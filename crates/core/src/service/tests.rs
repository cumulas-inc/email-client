//! Service tests against an in-memory fake server.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::auth::microsoft::SignInResult;
use crate::error::ErrorCode;
use crate::model::{
    AccountSettings, AttachmentMeta, EmailAddress, EventDraft, Folder, FolderRole,
    ImapAccountUpdate, ImapSettings, Importance, Message, MessageQuery, MessageSummary,
    OutgoingMessage, ServerSearchStatus, ShowAs, TaskDraft, TaskPatch, TaskStatus,
};
use crate::providers::{
    ChangeSubscriptions, FolderCounts, MailProvider, MessageBatch, MessageContent, MessageLocator,
    SyncWindow,
};
use crate::push::graph::{CONSENT_HINT, EVENTS_RESOURCE, INBOX_RESOURCE, RelaySource};
use crate::push::relay::RelayEvent;
use crate::push::{EndReason, MailWatcher, PushEvent, PushHealth, PushMode, WakeReason, WatchSink};
use crate::secrets::MemorySecretStore;

/// A tiny IMAP-like server: folders by remote id, messages by remote id.
#[derive(Default)]
pub(crate) struct FakeServer {
    pub folders: Vec<(String, FolderRole)>,
    pub messages: HashMap<String, (String, MessageSummary)>,
    pub sent: Vec<OutgoingMessage>,
    pub fail_sync: bool,
    /// Every message sync fails with this.
    pub failure: Option<Error>,
    /// Syncs of this folder (by remote id) fail.
    pub failing_folder: Option<String>,
    /// How often folder counts were asked for.
    pub count_requests: usize,
    /// How often the Inbox's messages were asked for.
    pub inbox_syncs: usize,
    /// Answering an Inbox sync, or counts, takes this long.
    pub inbox_delay: Option<std::time::Duration>,
    pub counts_delay: Option<std::time::Duration>,
    /// Room for one more connection: while a watcher is connected
    /// (`watching`), the operations connection is refused.
    pub one_free_slot: bool,
    pub watching: Arc<AtomicUsize>,
    /// Messages (folder, remote id) the next sync of the folder reports as
    /// moved out of it, as Graph's does after looking up its removals.
    pub moves: Vec<(String, String)>,
    /// Messages outside the sync window, by remote id: server search finds
    /// them, syncs don't.
    pub server_only: HashMap<String, (String, MessageSummary)>,
    /// Server searches wait for this, when set.
    pub search_gate: Option<Arc<tokio::sync::Notify>>,
    /// Server searches say when their answer is ready, then wait for
    /// `search_hold` before giving it, when set.
    pub search_answered: Option<Arc<tokio::sync::Notify>>,
    pub search_hold: Option<Arc<tokio::sync::Notify>>,
    /// Server searches of these accounts fail so.
    pub search_failures: HashMap<String, Error>,
    next_uid: u32,
}

/// What an IMAP server at its connection limit says (see
/// `providers::imap::connection`).
fn server_busy() -> Error {
    Error::network("The IMAP server imap.example.com isn't accepting more connections right now.")
        .with_details(serde_json::json!({ "reason": "serverBusy" }))
}

pub(crate) struct FakeMail {
    account_id: String,
    pub server: Arc<Mutex<FakeServer>>,
}

impl FakeMail {
    fn folder_id(&self, remote: &str) -> String {
        local_id(&self.account_id, EntityKind::Folder, remote)
    }

    /// Refuses the operations connection when the watcher has the last slot.
    fn connect(&self) -> Result<()> {
        let server = self.server.lock().unwrap();
        if server.one_free_slot && server.watching.load(Ordering::SeqCst) > 0 {
            return Err(server_busy());
        }
        Ok(())
    }
}

pub(crate) fn summary(
    account_id: &str,
    folder_remote: &str,
    remote: &str,
    subject: &str,
) -> MessageSummary {
    MessageSummary {
        id: local_id(account_id, EntityKind::Message, remote),
        account_id: account_id.into(),
        folder_id: local_id(account_id, EntityKind::Folder, folder_remote),
        remote_id: remote.into(),
        conversation_id: None,
        subject: subject.into(),
        from: Some(EmailAddress::new(
            Some("Grace Hopper"),
            "grace@navy.example",
        )),
        to: vec![EmailAddress::new(None, "me@example.com")],
        cc: vec![],
        preview: "preview".into(),
        received_at: Utc::now(),
        is_read: false,
        is_flagged: false,
        is_draft: false,
        has_attachments: true,
        importance: Importance::Normal,
    }
}

#[async_trait]
impl MailProvider for FakeMail {
    async fn list_folders(&self) -> Result<Vec<Folder>> {
        self.connect()?;
        let server = self.server.lock().unwrap();
        Ok(server
            .folders
            .iter()
            .map(|(remote, role)| Folder {
                id: self.folder_id(remote),
                account_id: self.account_id.clone(),
                remote_id: remote.clone(),
                parent_id: None,
                name: remote.clone(),
                role: *role,
                unread_count: server
                    .messages
                    .values()
                    .filter(|(f, m)| f == remote && !m.is_read)
                    .count() as u32,
                total_count: 0,
            })
            .collect())
    }

    async fn sync_messages(
        &self,
        folder: &Folder,
        _state: Option<&str>,
        _window: SyncWindow,
    ) -> Result<MessageBatch> {
        self.connect()?;
        let delay = {
            let mut server = self.server.lock().unwrap();
            if folder.remote_id != "INBOX" {
                None
            } else {
                server.inbox_syncs += 1;
                server.inbox_delay
            }
        };
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        let mut server = self.server.lock().unwrap();
        if server.fail_sync {
            return Err(Error::new(ErrorCode::AuthRequired, "token expired"));
        }
        if let Some(e) = &server.failure {
            return Err(e.clone());
        }
        if server.failing_folder.as_ref() == Some(&folder.remote_id) {
            return Err(Error::provider("This folder can't be synced."));
        }
        let upserts = server
            .messages
            .values()
            .filter(|(f, _)| *f == folder.remote_id)
            .map(|(_, m)| m.clone())
            .collect();
        let (moves, others): (Vec<_>, Vec<_>) = std::mem::take(&mut server.moves)
            .into_iter()
            .partition(|(from, _)| *from == folder.remote_id);
        server.moves = others;
        Ok(MessageBatch {
            upserts,
            // Like a delta: what changed, not the whole folder.
            full_resync: moves.is_empty(),
            moved_out: moves
                .iter()
                .map(|(_, remote)| server.messages[remote].1.clone())
                .collect(),
            sync_state: Some("state".into()),
            ..Default::default()
        })
    }

    async fn folder_counts(&self, folder: &Folder) -> Result<Option<FolderCounts>> {
        self.connect()?;
        let delay = self.server.lock().unwrap().counts_delay;
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        let mut server = self.server.lock().unwrap();
        server.count_requests += 1;
        let in_folder = || {
            server
                .messages
                .values()
                .filter(|(f, _)| *f == folder.remote_id)
        };
        Ok(Some(FolderCounts {
            unread: in_folder().filter(|(_, m)| !m.is_read).count() as u32,
            total: in_folder().count() as u32,
        }))
    }

    async fn fetch_message(&self, m: &MessageLocator) -> Result<MessageContent> {
        {
            let server = self.server.lock().unwrap();
            if !server.messages.contains_key(&m.remote_id)
                && !server.server_only.contains_key(&m.remote_id)
            {
                return Err(Error::not_found("message"));
            }
        }
        Ok(MessageContent {
            body_html: Some(
                r#"<p>Hello <img src="https://tracker.example/p.gif"></p><script>x()</script>"#
                    .into(),
            ),
            attachments: vec![AttachmentMeta {
                id: "att1".into(),
                name: "report.pdf".into(),
                content_type: "application/pdf".into(),
                size: 3,
                is_inline: false,
                content_id: None,
            }],
            ..Default::default()
        })
    }

    async fn fetch_attachment(&self, _: &MessageLocator, id: &str) -> Result<Vec<u8>> {
        assert_eq!(id, "att1");
        Ok(b"pdf".to_vec())
    }

    async fn set_read(&self, ms: &[MessageLocator], read: bool) -> Result<()> {
        let mut server = self.server.lock().unwrap();
        for m in ms {
            server.messages.get_mut(&m.remote_id).unwrap().1.is_read = read;
        }
        Ok(())
    }

    async fn set_flagged(&self, ms: &[MessageLocator], flagged: bool) -> Result<()> {
        let mut server = self.server.lock().unwrap();
        for m in ms {
            server.messages.get_mut(&m.remote_id).unwrap().1.is_flagged = flagged;
        }
        Ok(())
    }

    async fn move_messages(
        &self,
        ms: &[MessageLocator],
        dest: &str,
    ) -> Result<Vec<Option<String>>> {
        let mut server = self.server.lock().unwrap();
        let mut out = Vec::new();
        for m in ms {
            let (_, mut summary) = server.messages.remove(&m.remote_id).unwrap();
            server.next_uid += 1;
            let new_remote = format!("{dest}:{}", server.next_uid);
            summary.remote_id = new_remote.clone();
            summary.id = local_id(&self.account_id, EntityKind::Message, &new_remote);
            summary.folder_id = self.folder_id(dest);
            server
                .messages
                .insert(new_remote.clone(), (dest.to_owned(), summary));
            out.push(Some(new_remote));
        }
        Ok(out)
    }

    async fn delete_messages(&self, ms: &[MessageLocator]) -> Result<()> {
        let mut server = self.server.lock().unwrap();
        for m in ms {
            server.messages.remove(&m.remote_id);
        }
        Ok(())
    }

    /// Every term somewhere in the subject, preview or sender, newest
    /// first; the cursor is where the next page starts.
    async fn search(
        &self,
        query: &crate::search::SearchQuery,
        _folders: &[Folder],
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<crate::providers::SearchPage> {
        let gate = self.server.lock().unwrap().search_gate.clone();
        if let Some(gate) = gate {
            gate.notified().await;
        }
        let (page, answered, hold) = {
            let server = self.server.lock().unwrap();
            (
                self.search_page(&server, query, cursor, limit),
                server.search_answered.clone(),
                server.search_hold.clone(),
            )
        };
        if let Some(answered) = answered {
            answered.notify_one();
        }
        if let Some(hold) = hold {
            hold.notified().await;
        }
        page
    }

    async fn send(&self, message: &OutgoingMessage, original: Option<&Message>) -> Result<()> {
        if message.reply_to_message_id.is_some() {
            assert!(original.is_some());
        }
        self.server.lock().unwrap().sent.push(message.clone());
        Ok(())
    }
}

impl FakeMail {
    /// See `search` above.
    fn search_page(
        &self,
        server: &FakeServer,
        query: &crate::search::SearchQuery,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<crate::providers::SearchPage> {
        if let Some(e) = server.search_failures.get(&self.account_id) {
            return Err(e.clone());
        }
        let matches = |m: &MessageSummary| {
            let from = m
                .from
                .as_ref()
                .map(|f| format!("{f:?}"))
                .unwrap_or_default();
            let text = format!("{} {} {from}", m.subject, m.preview).to_lowercase();
            query.terms.iter().all(|t| text.contains(&t.text))
                && (!query.has_attachment || m.has_attachments)
        };
        let mut found: Vec<MessageSummary> = server
            .messages
            .values()
            .chain(server.server_only.values())
            .map(|(_, m)| m)
            .filter(|m| m.account_id == self.account_id && matches(m))
            .cloned()
            .collect();
        found.sort_by(|a, b| b.received_at.cmp(&a.received_at).then(a.id.cmp(&b.id)));
        let start: usize = cursor.map_or(0, |c| c.parse().unwrap());
        let end = (start + limit as usize).min(found.len());
        Ok(crate::providers::SearchPage {
            messages: found[start..end].to_vec(),
            next: (end < found.len()).then(|| end.to_string()),
        })
    }
}

pub(crate) struct FakeConnector {
    pub server: Arc<Mutex<FakeServer>>,
    pub reject_login: std::sync::atomic::AtomicBool,
    /// Passwords passed to `verify_imap`, in order.
    pub verified_passwords: Mutex<Vec<String>>,
    /// Push for every account; polling when unset.
    pub watcher: Mutex<Option<Arc<dyn MailWatcher>>>,
    /// Push for Microsoft accounts through the push service, when set.
    pub relay: Mutex<Option<RelaySource>>,
    /// Graph's change subscriptions, for every account.
    pub subscriptions: Mutex<Option<Arc<dyn ChangeSubscriptions>>>,
    /// Whether each Microsoft sign-in asked consent for the push service.
    pub push_consent: Mutex<Vec<bool>>,
    /// What the next Microsoft sign-in returns; it never finishes when unset.
    pub sign_in: Mutex<Option<SignInResult>>,
    /// The calendar of Microsoft accounts, when set.
    pub calendar: Mutex<Option<Arc<dyn crate::providers::CalendarProvider>>>,
    /// The signature service of Microsoft work accounts' organizations,
    /// when set.
    pub organization_signatures: Mutex<Option<Arc<dyn crate::signatures::OrganizationSignatures>>>,
    /// How often it was asked for.
    pub organization_requests: AtomicUsize,
}

impl FakeConnector {
    pub fn new(server: Arc<Mutex<FakeServer>>, reject_login: bool) -> Self {
        Self {
            server,
            reject_login: reject_login.into(),
            verified_passwords: Mutex::default(),
            watcher: Mutex::default(),
            relay: Mutex::default(),
            subscriptions: Mutex::default(),
            push_consent: Mutex::default(),
            sign_in: Mutex::default(),
            calendar: Mutex::default(),
            organization_signatures: Mutex::default(),
            organization_requests: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Connector for FakeConnector {
    async fn providers(&self, account: &Account) -> Result<ProviderSet> {
        Ok(ProviderSet {
            mail: Arc::new(FakeMail {
                account_id: account.id.clone(),
                server: Arc::clone(&self.server),
            }),
            calendar: match account.kind {
                AccountKind::Microsoft => self.calendar.lock().unwrap().clone(),
                AccountKind::Imap => None,
            },
            contacts: None,
            tasks: None,
            subscriptions: self.subscriptions.lock().unwrap().clone(),
        })
    }

    async fn verify_imap(&self, credentials: &ImapCredentials) -> Result<()> {
        self.verified_passwords
            .lock()
            .unwrap()
            .push(credentials.password.clone());
        if self.reject_login.load(std::sync::atomic::Ordering::Relaxed) {
            Err(Error::auth_required("Authentication failed"))
        } else {
            Ok(())
        }
    }

    async fn microsoft_sign_in(
        &self,
        _: &BrowserOpener,
        _: Option<&str>,
        push_consent: bool,
    ) -> Result<SignInResult> {
        self.push_consent.lock().unwrap().push(push_consent);
        let result = self.sign_in.lock().unwrap().take();
        match result {
            Some(result) => Ok(result),
            // Like a user who never finishes signing in in the browser.
            None => std::future::pending().await,
        }
    }

    fn organization_signatures(
        &self,
        _: &Account,
    ) -> Option<Arc<dyn crate::signatures::OrganizationSignatures>> {
        self.organization_requests.fetch_add(1, Ordering::SeqCst);
        self.organization_signatures.lock().unwrap().clone()
    }

    async fn push_source(&self, account: &Account) -> Result<PushSource> {
        if account.kind == AccountKind::Microsoft
            && let Some(relay) = self.relay.lock().unwrap().clone()
        {
            return Ok(PushSource::Relay(relay));
        }
        match self.watcher.lock().unwrap().clone() {
            Some(watcher) => Ok(PushSource::Watch(watcher)),
            None => Ok(PushSource::Poll(std::time::Duration::from_secs(
                match account.kind {
                    AccountKind::Microsoft => 30,
                    AccountKind::Imap => 120,
                },
            ))),
        }
    }
}

/// One `reminders_changed`: the active keys, the titles of the newly due
/// and the cleared keys.
#[derive(Debug, Clone, PartialEq)]
struct RemindersChanged {
    active: Vec<ActiveReminder>,
    newly_due: Vec<String>,
    cleared: Vec<String>,
}

#[derive(Default)]
struct RecordingSink {
    reminders: Mutex<Vec<RemindersChanged>>,
    new_mail: Mutex<Vec<String>>,
    alerts: Mutex<Vec<MailAlert>>,
    settled: Mutex<Vec<String>>,
    push: Mutex<Vec<crate::push::PushStatus>>,
    full_syncs: AtomicUsize,
}

impl EventSink for RecordingSink {
    fn data_changed(&self, _: &[DataKind], _: Option<&str>) {}
    fn sync_status(&self, status: &SyncStatus) {
        if status.state == SyncState::Idle {
            self.full_syncs.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn new_mail(&self, _: &Account, alert: &MailAlert) {
        let mut seen = self.new_mail.lock().unwrap();
        seen.extend(alert.messages().iter().map(|m| m.subject.clone()));
        self.alerts.lock().unwrap().push(alert.clone());
    }
    fn mail_settled(&self, message_ids: &[String]) {
        self.settled.lock().unwrap().extend_from_slice(message_ids);
    }
    fn push_status(&self, status: &crate::push::PushStatus) {
        self.push.lock().unwrap().push(status.clone());
    }
    fn reminders_changed(
        &self,
        active: &[ActiveReminder],
        newly_due: &[ActiveReminder],
        cleared: &[String],
    ) {
        self.reminders.lock().unwrap().push(RemindersChanged {
            active: active.to_vec(),
            newly_due: newly_due.iter().map(|r| r.title.clone()).collect(),
            cleared: cleared.to_vec(),
        });
    }
}

impl RecordingSink {
    /// The titles of every reminder shown so far, in order.
    fn shown_reminders(&self) -> Vec<String> {
        self.reminders
            .lock()
            .unwrap()
            .iter()
            .flat_map(|c| c.newly_due.clone())
            .collect()
    }

    fn cleared_reminders(&self) -> Vec<String> {
        self.reminders
            .lock()
            .unwrap()
            .iter()
            .flat_map(|c| c.cleared.clone())
            .collect()
    }
}

/// A watcher whose changes the test sends; counts its connections.
#[derive(Default)]
struct ScriptedWatcher {
    events: Mutex<Option<mpsc::UnboundedReceiver<PushEvent>>>,
    sender: Mutex<Option<mpsc::UnboundedSender<PushEvent>>>,
    connections: AtomicUsize,
    cancelled: AtomicUsize,
    /// Connections open now.
    open: Arc<AtomicUsize>,
}

impl ScriptedWatcher {
    fn new() -> Arc<Self> {
        let watcher = Arc::new(Self::default());
        watcher.reset_channel();
        watcher
    }

    fn reset_channel(&self) {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.sender.lock().unwrap() = Some(tx);
        *self.events.lock().unwrap() = Some(rx);
    }

    fn send(&self, event: PushEvent) {
        let sender = self.sender.lock().unwrap();
        sender.as_ref().unwrap().send(event).unwrap();
    }
}

#[async_trait]
impl MailWatcher for ScriptedWatcher {
    async fn watch(&self, sink: &WatchSink, cancel: &CancellationToken) -> EndReason {
        self.connections.fetch_add(1, Ordering::SeqCst);
        let Some(mut events) = self.events.lock().unwrap().take() else {
            cancel.cancelled().await;
            return EndReason::Cancelled;
        };
        self.open.fetch_add(1, Ordering::SeqCst);
        sink.connected(PushMode::ImapIdle);
        sink.changed(PushEvent::CatchUp);
        let end = loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    self.cancelled.fetch_add(1, Ordering::SeqCst);
                    break EndReason::Cancelled;
                }
                event = events.recv() => match event {
                    Some(event) => sink.changed(event),
                    None => break EndReason::Failed(Error::network("closed")),
                },
            }
        };
        self.open.fetch_sub(1, Ordering::SeqCst);
        // The next connection gets a fresh channel.
        self.reset_channel();
        end
    }
}

struct Harness {
    service: CoreService,
    connector: Arc<FakeConnector>,
    server: Arc<Mutex<FakeServer>>,
    secrets: Arc<MemorySecretStore>,
    sink: Arc<RecordingSink>,
}

fn harness() -> Harness {
    let server = Arc::new(Mutex::new(FakeServer {
        folders: vec![
            ("INBOX".into(), FolderRole::Inbox),
            ("Archive".into(), FolderRole::Archive),
            ("Trash".into(), FolderRole::Trash),
        ],
        ..Default::default()
    }));
    let secrets = Arc::new(MemorySecretStore::default());
    let sink = Arc::new(RecordingSink::default());
    let connector = Arc::new(FakeConnector::new(Arc::clone(&server), false));
    let service = CoreService::new(
        Arc::new(Store::open_in_memory().unwrap()),
        secrets.clone(),
        connector.clone(),
        sink.clone(),
    );
    Harness {
        service,
        connector,
        server,
        secrets,
        sink,
    }
}

fn new_imap_account(email: &str) -> NewImapAccount {
    NewImapAccount {
        display_name: "Me".into(),
        email: email.into(),
        password: "hunter2".into(),
        settings: ImapSettings {
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            imap_tls: TlsMode::Tls,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            smtp_tls: TlsMode::StartTls,
            username: email.into(),
        },
    }
}

async fn account_with_mail(h: &Harness) -> Account {
    let account = h
        .service
        .add_imap_account(new_imap_account("me@example.com"))
        .await
        .unwrap();
    {
        let mut server = h.server.lock().unwrap();
        for (remote, subject) in [("INBOX:1", "Welcome"), ("INBOX:2", "Invoice")] {
            let m = summary(&account.id, "INBOX", remote, subject);
            server.messages.insert(remote.into(), ("INBOX".into(), m));
        }
    }
    h.service.sync_account(&account.id).await.unwrap();
    account
}

fn inbox(h: &Harness, account: &Account) -> Folder {
    h.service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Inbox)
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn adding_imap_account_stores_password_and_local_containers() {
    let h = harness();
    let account = h
        .service
        .add_imap_account(new_imap_account("me@example.com"))
        .await
        .unwrap();
    assert_eq!(
        h.secrets
            .get(&keys::imap_password(&account.id))
            .await
            .unwrap()
            .as_deref(),
        Some("hunter2")
    );
    assert_eq!(h.service.list_calendars().unwrap().len(), 1);
    assert_eq!(h.service.list_task_lists().unwrap().len(), 1);

    let dup = h
        .service
        .add_imap_account(new_imap_account("ME@example.com"))
        .await
        .unwrap_err();
    assert_eq!(dup.code, ErrorCode::InvalidInput);

    let mut insecure = new_imap_account("other@example.com");
    insecure.settings.imap_tls = TlsMode::None;
    assert_eq!(
        h.service.add_imap_account(insecure).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );

    h.service.remove_account(&account.id).await.unwrap();
    assert!(h.service.list_accounts().unwrap().is_empty());
    assert!(h.service.list_calendars().unwrap().is_empty());
    assert_eq!(
        h.secrets
            .get(&keys::imap_password(&account.id))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn rejected_login_does_not_create_account() {
    let h = harness();
    let service = CoreService::new(
        Arc::new(Store::open_in_memory().unwrap()),
        h.secrets.clone(),
        Arc::new(FakeConnector::new(h.server.clone(), true)),
        Arc::new(NullSink),
    );
    let err = service
        .add_imap_account(new_imap_account("me@example.com"))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthRequired);
    assert!(service.list_accounts().unwrap().is_empty());
}

#[tokio::test]
async fn sync_populates_folders_and_messages_and_reports_new_mail() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let inbox = inbox(&h, &account);
    assert_eq!(inbox.unread_count, 2);
    let page = h
        .service
        .list_messages(
            &inbox.id,
            &MessageQuery {
                limit: 50,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(page.total, 2);
    // Initial sync is not "new mail".
    assert!(h.sink.new_mail.lock().unwrap().is_empty());

    let m = summary(&account.id, "INBOX", "INBOX:3", "Fresh");
    h.server
        .lock()
        .unwrap()
        .messages
        .insert("INBOX:3".into(), ("INBOX".into(), m));
    h.service.sync_account(&account.id).await.unwrap();
    assert_eq!(*h.sink.new_mail.lock().unwrap(), vec!["Fresh".to_string()]);

    let statuses = h.service.sync_statuses().unwrap();
    assert_eq!(statuses[0].state, SyncState::Idle);
    assert!(statuses[0].last_synced_at.is_some());
}

#[tokio::test]
async fn sync_failure_is_recorded_on_the_account() {
    let h = harness();
    let account = account_with_mail(&h).await;
    h.server.lock().unwrap().fail_sync = true;
    let err = h.service.sync_account(&account.id).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthRequired);
    let account = h.service.store().get_account(&account.id).unwrap();
    assert!(account.sync_error.unwrap().contains("Sign in again"));
    assert_eq!(
        h.service.sync_statuses().unwrap()[0].state,
        SyncState::Error
    );
}

#[tokio::test]
async fn reading_a_message_fetches_sanitises_and_caches_it() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let id = h.service.search_messages("welcome", None, &[]).unwrap()[0]
        .id
        .clone();

    let message = h.service.get_message(&id, false).await.unwrap();
    let html = message.body_html.unwrap();
    assert!(html.contains("Hello"));
    assert!(!html.contains("script"));
    assert!(!html.contains("tracker"));
    assert!(message.has_blocked_content);

    let with_images = h.service.get_message(&id, true).await.unwrap();
    assert!(with_images.body_html.unwrap().contains("tracker"));

    let attachment = h.service.get_attachment(&id, "att1").await.unwrap();
    assert_eq!(attachment.data, b"pdf");
    assert_eq!(attachment.meta.name, "report.pdf");
    let _ = account;
}

#[tokio::test]
async fn flags_moves_and_deletes_go_to_server_then_store() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let inbox = inbox(&h, &account);
    let ids: Vec<String> = h
        .service
        .list_messages(&inbox.id, &MessageQuery::default())
        .unwrap()
        .items
        .into_iter()
        .map(|m| m.id)
        .collect();

    h.service.set_read(&ids, true).await.unwrap();
    assert!(
        h.server
            .lock()
            .unwrap()
            .messages
            .values()
            .all(|(_, m)| m.is_read)
    );
    assert_eq!(
        h.service
            .store()
            .get_folder(&inbox.id)
            .unwrap()
            .unread_count,
        0
    );

    h.service.set_flagged(&ids[..1], true).await.unwrap();
    assert!(
        h.service
            .store()
            .get_message_summary(&ids[0])
            .unwrap()
            .is_flagged
    );

    h.service.archive_messages(&ids[..1]).await.unwrap();
    let archive = h
        .service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Archive)
        .unwrap()
        .unwrap();
    assert_eq!(
        h.service
            .list_messages(&archive.id, &MessageQuery::default())
            .unwrap()
            .total,
        1
    );

    // Delete from inbox moves to trash; deleting from trash removes it.
    h.service.delete_messages(&ids[1..]).await.unwrap();
    let trash = h
        .service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Trash)
        .unwrap()
        .unwrap();
    let in_trash = h
        .service
        .list_messages(&trash.id, &MessageQuery::default())
        .unwrap();
    assert_eq!(in_trash.total, 1);
    h.service
        .delete_messages(&[in_trash.items[0].id.clone()])
        .await
        .unwrap();
    assert_eq!(
        h.service
            .list_messages(&trash.id, &MessageQuery::default())
            .unwrap()
            .total,
        0
    );
    assert_eq!(h.server.lock().unwrap().messages.len(), 1);
}

#[tokio::test]
async fn sending_validates_recipients_and_links_replies() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let original = h.service.search_messages("invoice", None, &[]).unwrap()[0]
        .id
        .clone();
    let mut message = OutgoingMessage {
        account_id: account.id.clone(),
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "Re: Invoice".into(),
        body_html: "<p>Paid</p>".into(),
        attachments: vec![],
        importance: None,
        reply_to_message_id: Some(original),
        reply_kind: Some(crate::model::ReplyKind::Reply),
    };
    assert_eq!(
        h.service
            .send_message(message.clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    message.to = vec![EmailAddress::new(None, "not an address")];
    assert!(h.service.send_message(message.clone()).await.is_err());
    message.to = vec![EmailAddress::new(Some("Grace"), "grace@navy.example")];
    h.service.send_message(message).await.unwrap();
    assert_eq!(h.server.lock().unwrap().sent.len(), 1);
}

#[tokio::test]
async fn local_calendar_and_tasks_for_imap_accounts() {
    let h = harness();
    h.service
        .add_imap_account(new_imap_account("me@example.com"))
        .await
        .unwrap();
    let calendar = h.service.list_calendars().unwrap().remove(0);
    let start = Utc::now();
    let draft = EventDraft {
        calendar_id: calendar.id.clone(),
        subject: "Dentist".into(),
        location: Some("Main St".into()),
        body_html: None,
        start,
        end: start + Duration::hours(1),
        is_all_day: false,
        attendees: vec![],
        reminder_minutes: Some(15),
        show_as: ShowAs::Busy,
        is_online_meeting: false,
    };
    let event = h.service.create_event(draft.clone()).await.unwrap();
    assert_eq!(
        h.service
            .list_events(start - Duration::hours(1), start + Duration::hours(2), None)
            .unwrap(),
        vec![event.clone()]
    );
    let bad = EventDraft {
        end: start - Duration::hours(1),
        ..draft.clone()
    };
    assert!(h.service.create_event(bad).await.is_err());
    let moved = h
        .service
        .update_event(
            &event.id,
            EventDraft {
                subject: "Dentist (moved)".into(),
                ..draft
            },
        )
        .await
        .unwrap();
    assert_eq!(moved.id, event.id);
    h.service.delete_event(&event.id).await.unwrap();

    let list = h.service.list_task_lists().unwrap().remove(0);
    let task = h
        .service
        .create_task(TaskDraft {
            list_id: list.id.clone(),
            title: "Renew passport".into(),
            notes: None,
            importance: Some(Importance::High),
            due_date: NaiveDate::from_ymd_opt(2026, 11, 1),
            reminder_at: None,
        })
        .await
        .unwrap();
    let done = h
        .service
        .update_task(
            &task.id,
            TaskPatch {
                status: Some(TaskStatus::Completed),
                clear_due_date: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(done.status, TaskStatus::Completed);
    assert!(done.completed_at.is_some());
    assert_eq!(done.due_date, None);
    assert!(
        h.service
            .list_tasks(Some(&list.id), false)
            .unwrap()
            .is_empty()
    );
    assert_eq!(h.service.list_tasks(Some(&list.id), true).unwrap().len(), 1);
}

#[tokio::test]
async fn updating_imap_account_verifies_and_stores_new_password() {
    let h = harness();
    let account = h
        .service
        .add_imap_account(new_imap_account("me@example.com"))
        .await
        .unwrap();
    h.service
        .store()
        .set_account_sync_result(&account.id, None, Some("Authentication failed"))
        .unwrap();

    // A rejected password leaves the stored one alone.
    h.connector
        .reject_login
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let update = ImapAccountUpdate {
        password: Some("wrong".into()),
        settings: None,
    };
    let err = h
        .service
        .update_imap_account(&account.id, update)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthRequired);
    let stored = h
        .secrets
        .get(&keys::imap_password(&account.id))
        .await
        .unwrap();
    assert_eq!(stored.as_deref(), Some("hunter2"));

    h.connector
        .reject_login
        .store(false, std::sync::atomic::Ordering::Relaxed);
    let mut settings = match &account.settings {
        AccountSettings::Imap(s) => s.clone(),
        AccountSettings::Microsoft(_) => unreachable!(),
    };
    settings.imap_host = "mail.example.com".into();
    let updated = h
        .service
        .update_imap_account(
            &account.id,
            ImapAccountUpdate {
                password: Some("new-secret".into()),
                settings: Some(settings),
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(&updated.settings, AccountSettings::Imap(s) if s.imap_host == "mail.example.com")
    );
    assert_eq!(updated.sync_error, None);
    let stored = h
        .secrets
        .get(&keys::imap_password(&account.id))
        .await
        .unwrap();
    assert_eq!(stored.as_deref(), Some("new-secret"));
    assert_eq!(
        h.service
            .store()
            .get_account(&account.id)
            .unwrap()
            .sync_error,
        None
    );

    // Settings-only change re-verifies with the stored password.
    h.service
        .update_imap_account(&account.id, ImapAccountUpdate::default())
        .await
        .unwrap();
    assert_eq!(
        h.connector
            .verified_passwords
            .lock()
            .unwrap()
            .last()
            .map(String::as_str),
        Some("new-secret")
    );

    let mut insecure = ImapAccountUpdate::default();
    let mut s = match &updated.settings {
        AccountSettings::Imap(s) => s.clone(),
        AccountSettings::Microsoft(_) => unreachable!(),
    };
    s.smtp_tls = TlsMode::None;
    insecure.settings = Some(s);
    assert_eq!(
        h.service
            .update_imap_account(&account.id, insecure)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
}

#[tokio::test]
async fn microsoft_sign_in_can_be_cancelled() {
    let h = harness();
    let service = &h.service;
    let (result, ()) = tokio::join!(service.add_microsoft_account(&|_| Ok(()), None), async {
        tokio::task::yield_now().await;
        service.cancel_microsoft_sign_in();
    });
    assert_eq!(result.unwrap_err().code, ErrorCode::SignInCancelled);
    // Nothing left pending; canceling again is a no-op.
    service.cancel_microsoft_sign_in();
    assert!(service.list_accounts().unwrap().is_empty());
}

#[tokio::test]
async fn appearance_settings_are_validated_and_saved() {
    use crate::model::{InterfaceFont, ReadingFont, TextSize};

    let h = harness();
    let chosen = Settings {
        interface_font: InterfaceFont::Family {
            family: "Noto Sans".into(),
        },
        reading_font: ReadingFont::Interface,
        text_size: TextSize::Larger,
        ..Settings::default()
    };
    assert_eq!(
        h.service.update_settings(chosen.clone()).await.unwrap(),
        chosen
    );
    assert_eq!(h.service.get_settings().unwrap(), chosen);

    for family in ["", " Noto Sans", "Noto\nSans", "Noto\u{202E}Sans"] {
        let bad = Settings {
            reading_font: ReadingFont::Family {
                family: family.into(),
            },
            ..chosen.clone()
        };
        let err = h.service.update_settings(bad).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput, "{family:?}");
    }
    let bad = Settings {
        interface_font: InterfaceFont::Family {
            family: "x".repeat(500),
        },
        ..Settings::default()
    };
    assert_eq!(
        h.service.update_settings(bad).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
    // A rejected change leaves the saved settings alone.
    assert_eq!(h.service.get_settings().unwrap(), chosen);
}

fn add_message(h: &Harness, account: &Account, folder: &str, remote: &str, subject: &str) {
    let m = summary(&account.id, folder, remote, subject);
    h.server
        .lock()
        .unwrap()
        .messages
        .insert(remote.into(), (folder.into(), m));
}

#[tokio::test]
async fn a_targeted_inbox_sync_reports_new_mail_and_refreshes_counts() {
    let h = harness();
    let account = account_with_mail(&h).await;
    h.service
        .store()
        .set_folder_counts(&inbox(&h, &account).id, 0, 0)
        .unwrap();
    add_message(&h, &account, "INBOX", "INBOX:3", "Fresh");
    add_message(&h, &account, "Archive", "Archive:1", "Elsewhere");

    h.service
        .sync_targets(&account.id, SyncScope::inbox())
        .await
        .unwrap();
    assert_eq!(*h.sink.new_mail.lock().unwrap(), ["Fresh"]);
    assert!(matches!(
        h.sink.alerts.lock().unwrap()[0],
        MailAlert::Each(ref m) if m.len() == 1
    ));
    // Only the Inbox was synced, and its counts came from the server.
    let archive = h
        .service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Archive)
        .unwrap()
        .unwrap();
    assert_eq!(
        h.service
            .list_messages(&archive.id, &MessageQuery::default())
            .unwrap()
            .total,
        0
    );
    let inbox = inbox(&h, &account);
    assert_eq!((inbox.unread_count, inbox.total_count), (3, 3));
    assert_eq!(h.server.lock().unwrap().count_requests, 1);

    // Nothing changed: no new counts, no notification.
    h.service
        .sync_targets(&account.id, SyncScope::inbox())
        .await
        .unwrap();
    assert_eq!(h.server.lock().unwrap().count_requests, 1);
    assert_eq!(h.sink.new_mail.lock().unwrap().len(), 1);

    // A folder named by NOTIFY.
    let mut scope = SyncScope::default();
    scope.folders.insert("Archive".into());
    h.service.sync_targets(&account.id, scope).await.unwrap();
    assert_eq!(
        h.service
            .list_messages(&archive.id, &MessageQuery::default())
            .unwrap()
            .total,
        1
    );
    // Archive's first sync isn't news, nor outside the Inbox.
    assert_eq!(h.sink.new_mail.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn mail_read_or_removed_elsewhere_is_settled() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let welcome = local_id(&account.id, EntityKind::Message, "INBOX:1");
    let invoice = local_id(&account.id, EntityKind::Message, "INBOX:2");
    h.server
        .lock()
        .unwrap()
        .messages
        .get_mut("INBOX:1")
        .unwrap()
        .1
        .is_read = true;
    h.service
        .sync_targets(&account.id, SyncScope::inbox())
        .await
        .unwrap();
    assert_eq!(
        *h.sink.settled.lock().unwrap(),
        std::slice::from_ref(&welcome)
    );
    assert_eq!(inbox(&h, &account).unread_count, 1);

    h.server.lock().unwrap().messages.remove("INBOX:2");
    h.service
        .sync_targets(&account.id, SyncScope::inbox())
        .await
        .unwrap();
    assert_eq!(*h.sink.settled.lock().unwrap(), [welcome, invoice]);
}

#[tokio::test]
async fn reading_moving_and_deleting_here_settle_notifications() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let ids: Vec<String> = h
        .service
        .list_messages(&inbox(&h, &account).id, &MessageQuery::default())
        .unwrap()
        .items
        .into_iter()
        .map(|m| m.id)
        .collect();
    h.service.set_read(&ids[..1], false).await.unwrap();
    assert!(h.sink.settled.lock().unwrap().is_empty(), "already unread");
    h.service.set_read(&ids[..1], true).await.unwrap();
    assert_eq!(*h.sink.settled.lock().unwrap(), ids[..1]);
    h.service.archive_messages(&ids[1..]).await.unwrap();
    assert_eq!(*h.sink.settled.lock().unwrap(), ids);
}

#[tokio::test]
async fn syncs_wait_for_the_account_lock() {
    let h = harness();
    let account = account_with_mail(&h).await;
    add_message(&h, &account, "INBOX", "INBOX:3", "Fresh");
    let syncs = h.sink.full_syncs.load(Ordering::SeqCst);
    let lock = h.service.account_lock(&account.id);
    let guard = lock.lock().await;
    // Neither a sync the user asks for nor a targeted sync is dropped while
    // the account is busy: both wait for it.
    let release = async {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(h.sink.new_mail.lock().unwrap().is_empty(), "still waiting");
        assert_eq!(h.sink.full_syncs.load(Ordering::SeqCst), syncs);
        drop(guard);
    };
    let (full, targeted, ()) = tokio::join!(
        h.service.sync_account(&account.id),
        h.service.sync_targets(&account.id, SyncScope::inbox()),
        release
    );
    full.unwrap();
    targeted.unwrap();
    assert_eq!(h.sink.full_syncs.load(Ordering::SeqCst), syncs + 1);
    assert_eq!(*h.sink.new_mail.lock().unwrap(), ["Fresh"]);
}

#[tokio::test]
async fn targeted_syncs_learn_about_new_folders() {
    let h = harness();
    let account = account_with_mail(&h).await;
    h.server
        .lock()
        .unwrap()
        .folders
        .push(("Projects".into(), FolderRole::Other));
    add_message(&h, &account, "Projects", "Projects:1", "Plan");
    let mut scope = SyncScope::default();
    scope.folders.insert("Projects".into());
    h.service.sync_targets(&account.id, scope).await.unwrap();
    let projects = h
        .service
        .store()
        .find_folder_by_remote_id(&account.id, "Projects")
        .unwrap()
        .expect("listed");
    assert_eq!(
        h.service
            .list_messages(&projects.id, &MessageQuery::default())
            .unwrap()
            .total,
        1
    );
}

#[tokio::test]
async fn mail_moved_in_elsewhere_refreshes_both_folders_counts() {
    let h = harness();
    let account = account_with_mail(&h).await;
    // Like Graph's, this message's id is the same in every folder.
    add_message(&h, &account, "Archive", "MSG-1", "Not junk");
    h.service.sync_account(&account.id).await.unwrap();
    let archive = h
        .service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Archive)
        .unwrap()
        .unwrap();
    assert_eq!(archive.unread_count, 1);

    // Moved to the Inbox on the phone.
    {
        let mut server = h.server.lock().unwrap();
        let (folder, message) = server.messages.get_mut("MSG-1").unwrap();
        *folder = "INBOX".into();
        message.folder_id = inbox(&h, &account).id;
    }
    h.service
        .sync_targets(&account.id, SyncScope::inbox())
        .await
        .unwrap();
    let inbox = inbox(&h, &account);
    assert_eq!((inbox.unread_count, inbox.total_count), (3, 3));
    let archive = h.service.store().get_folder(&archive.id).unwrap();
    assert_eq!((archive.unread_count, archive.total_count), (0, 0));
    // A message moved in isn't new mail.
    assert!(h.sink.new_mail.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_targeted_sync_after_a_move_refreshes_both_folders() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let welcome = local_id(&account.id, EntityKind::Message, "INBOX:1");
    // Read here, so its body is stored.
    h.service.get_message(&welcome, false).await.unwrap();
    // Moved to the Archive on the phone (keeping its id, as on Graph);
    // only the Inbox is synced.
    {
        let mut server = h.server.lock().unwrap();
        let (folder, message) = server.messages.get_mut("INBOX:1").unwrap();
        *folder = "Archive".into();
        message.folder_id = local_id(&account.id, EntityKind::Folder, "Archive");
        server.moves.push(("INBOX".into(), "INBOX:1".into()));
    }
    h.service
        .sync_targets(&account.id, SyncScope::inbox())
        .await
        .unwrap();
    let store = h.service.store();
    let archive = store
        .find_folder_by_role(&account.id, FolderRole::Archive)
        .unwrap()
        .unwrap();
    let moved = store.get_message_summary(&welcome).unwrap();
    assert_eq!(moved.folder_id, archive.id);
    assert!(store.get_message_content(&welcome).unwrap().is_some());
    // Both folders' counts came from the server.
    let inbox = inbox(&h, &account);
    assert_eq!((inbox.unread_count, inbox.total_count), (1, 1));
    assert_eq!((archive.unread_count, archive.total_count), (1, 1));
    assert_eq!(h.server.lock().unwrap().count_requests, 2);
    // Its new-mail notification can go.
    assert_eq!(*h.sink.settled.lock().unwrap(), [welcome]);
}

// --- server search ---------------------------------------------------------------

/// A message only on the server, received `days_ago`.
fn on_server_only(
    h: &Harness,
    account: &Account,
    folder: &str,
    remote: &str,
    subject: &str,
    days_ago: i64,
) -> String {
    let mut m = summary(&account.id, folder, remote, subject);
    m.received_at = Utc::now() - Duration::days(days_ago);
    let id = m.id.clone();
    h.server
        .lock()
        .unwrap()
        .server_only
        .insert(remote.into(), (folder.into(), m));
    id
}

fn local_ids(h: &Harness, text: &str, include: &[String]) -> Vec<String> {
    h.service
        .search_messages(text, None, include)
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect()
}

#[tokio::test]
async fn server_search_stores_results_outside_the_window_and_local_search_includes_them() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let old = on_server_only(
        &h,
        &account,
        "Archive",
        "Archive:7",
        "Old zebracorn invoice",
        400,
    );
    assert!(local_ids(&h, "zebracorn", &[]).is_empty());

    let found = h
        .service
        .search_server("s-1", "zebracorn", &account.id, false)
        .await
        .unwrap();
    assert_eq!(found.status, ServerSearchStatus::Done);
    assert_eq!(found.message_ids, [old.as_str()]);
    assert_eq!((found.added, found.has_more), (1, false));
    assert!(found.error.is_none());
    assert_eq!(found.search_id, "s-1");
    assert_eq!(found.account_id, account.id);

    // Stored and indexed: a store row like any other, in its folder.
    assert_eq!(
        local_ids(&h, "zebracorn", &found.message_ids),
        [old.as_str()]
    );
    assert_eq!(local_ids(&h, "zebracorn", &[]), [old.as_str()]);
    let archive = h
        .service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Archive)
        .unwrap()
        .unwrap();
    assert_eq!(
        h.service
            .store()
            .get_message_summary(&old)
            .unwrap()
            .folder_id,
        archive.id
    );
    // Found again: nothing new.
    let again = h
        .service
        .search_server("s-1", "zebracorn", &account.id, false)
        .await
        .unwrap();
    assert_eq!((again.message_ids.len(), again.added), (1, 0));
    // A full sync of a window without it keeps it.
    h.service.sync_account(&account.id).await.unwrap();
    assert_eq!(local_ids(&h, "zebracorn", &[]), [old]);

    let json = serde_json::to_value(&found).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "searchId": "s-1",
            "accountId": account.id,
            "status": "done",
            "messageIds": found.message_ids,
            "added": 1,
            "hasMore": false,
            "error": null,
        })
    );
}

#[tokio::test]
async fn a_new_search_cancels_the_previous_one() {
    let h = harness();
    let account = account_with_mail(&h).await;
    on_server_only(
        &h,
        &account,
        "Archive",
        "Archive:7",
        "Old zebracorn invoice",
        400,
    );
    h.server.lock().unwrap().search_gate = Some(Arc::new(tokio::sync::Notify::new()));
    let first = h
        .service
        .search_server("a", "zebracorn", &account.id, false);
    let second = async {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        h.server.lock().unwrap().search_gate = None;
        h.service
            .search_server("b", "zebracorn", &account.id, false)
            .await
    };
    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap();
    assert_eq!(first.status, ServerSearchStatus::Cancelled);
    assert!(first.message_ids.is_empty() && first.error.is_none());
    let second = second.unwrap();
    assert_eq!(second.status, ServerSearchStatus::Done);
    // Nothing of the first was stored: the second added it.
    assert_eq!(second.added, 1);
    // A call for the first arriving late doesn't replace the second.
    let late = h
        .service
        .search_server("a", "zebracorn", &account.id, false)
        .await
        .unwrap();
    assert_eq!(late.status, ServerSearchStatus::Cancelled);
    let more = h
        .service
        .search_server("b", "zebracorn", &account.id, true)
        .await
        .unwrap();
    assert_eq!(
        (more.status, more.message_ids.len()),
        (ServerSearchStatus::Done, 0)
    );
}

#[tokio::test]
async fn cancel_server_search_cancels_pending_calls() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let old = on_server_only(
        &h,
        &account,
        "Archive",
        "Archive:7",
        "Old zebracorn invoice",
        400,
    );
    let gate = Arc::new(tokio::sync::Notify::new());
    h.server.lock().unwrap().search_gate = Some(gate.clone());
    let pending = h
        .service
        .search_server("a", "zebracorn", &account.id, false);
    let cancel = async {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        h.service.cancel_server_search("a");
        // The server answering now changes nothing.
        gate.notify_waiters();
    };
    let (pending, ()) = tokio::join!(pending, cancel);
    assert_eq!(pending.unwrap().status, ServerSearchStatus::Cancelled);
    assert!(h.service.store().get_message_summary(&old).is_err());
    h.server.lock().unwrap().search_gate = None;
    let again = h
        .service
        .search_server("a", "zebracorn", &account.id, false)
        .await
        .unwrap();
    assert_eq!(again.status, ServerSearchStatus::Cancelled);
    // Canceled before it started, a search never starts.
    h.service.cancel_server_search("c");
    let early = h
        .service
        .search_server("c", "zebracorn", &account.id, false)
        .await
        .unwrap();
    assert_eq!(early.status, ServerSearchStatus::Cancelled);
    assert!(h.service.store().get_message_summary(&old).is_err());
}

#[tokio::test]
async fn server_search_errors_are_per_account() {
    let h = harness();
    let work = account_with_mail(&h).await;
    let home = h
        .service
        .add_imap_account(new_imap_account("home@example.com"))
        .await
        .unwrap();
    h.server.lock().unwrap().search_failures.insert(
        home.id.clone(),
        Error::network("Could not reach imap.example.com."),
    );
    let (work_result, home_result) = tokio::join!(
        h.service.search_server("s", "invoice", &work.id, false),
        h.service.search_server("s", "invoice", &home.id, false),
    );
    let work_result = work_result.unwrap();
    assert_eq!(work_result.status, ServerSearchStatus::Done);
    assert_eq!(work_result.message_ids.len(), 1);
    let home_result = home_result.unwrap();
    assert_eq!(home_result.status, ServerSearchStatus::Failed);
    let error = home_result.error.clone().unwrap();
    assert_eq!(error.code, ErrorCode::Network);
    let json = serde_json::to_value(&home_result).unwrap();
    assert_eq!(
        json["error"],
        serde_json::json!({ "code": "network", "message": "Could not reach imap.example.com." })
    );
    assert_eq!(json["status"], "failed");
    assert_eq!(json["messageIds"], serde_json::json!([]));
}

#[tokio::test(start_paused = true)]
async fn server_search_times_out() {
    let h = harness();
    let account = account_with_mail(&h).await;
    h.server.lock().unwrap().search_gate = Some(Arc::new(tokio::sync::Notify::new()));
    let started = tokio::time::Instant::now();
    let result = h
        .service
        .search_server("s", "invoice", &account.id, false)
        .await
        .unwrap();
    assert_eq!(result.status, ServerSearchStatus::Failed);
    let error = result.error.unwrap();
    assert_eq!(error.code, ErrorCode::Network);
    // The UI's Try again button says the rest.
    assert_eq!(
        error.message,
        "The server didn't answer in time; it may be busy."
    );
    assert!(started.elapsed() >= SERVER_SEARCH_TIMEOUT);
    assert!(started.elapsed() < std::time::Duration::from_secs(46));
}

#[tokio::test]
async fn show_more_continues_from_the_cursor_and_stops() {
    let h = harness();
    let account = account_with_mail(&h).await;
    for i in 0..60 {
        on_server_only(
            &h,
            &account,
            "Archive",
            &format!("Archive:{i}"),
            &format!("Purchase order {i}"),
            100 + i,
        );
    }
    let search = |more| h.service.search_server("s", "purchase", &account.id, more);
    let first = search(false).await.unwrap();
    assert_eq!(
        (first.message_ids.len(), first.added, first.has_more),
        (50, 50, true)
    );
    let second = search(true).await.unwrap();
    assert_eq!(
        (second.message_ids.len(), second.added, second.has_more),
        (10, 10, false)
    );
    assert!(
        second
            .message_ids
            .iter()
            .all(|id| !first.message_ids.contains(id))
    );
    let third = search(true).await.unwrap();
    assert_eq!(third.status, ServerSearchStatus::Done);
    assert!(third.message_ids.is_empty() && !third.has_more);
    // Every one is a search result now, newest first.
    let all: Vec<String> = first
        .message_ids
        .into_iter()
        .chain(second.message_ids)
        .collect();
    assert_eq!(local_ids(&h, "purchase", &[]).len(), 60);
    assert_eq!(local_ids(&h, "", &all), all);
}

#[tokio::test]
async fn an_empty_or_too_broad_query_is_done_with_nothing() {
    let h = harness();
    let account = account_with_mail(&h).await;
    for text in ["", "  ", "to:", "has:pdf", "a b", "from:a"] {
        let result = h
            .service
            .search_server("s", text, &account.id, false)
            .await
            .unwrap();
        assert_eq!(result.status, ServerSearchStatus::Done, "{text:?}");
        assert!(result.message_ids.is_empty() && !result.has_more);
    }
}

#[tokio::test]
async fn unknown_account_rejects() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let error = h
        .service
        .search_server("s", "invoice", "nobody", false)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    for id in [String::new(), "x".repeat(101)] {
        let error = h
            .service
            .search_server(&id, "invoice", &account.id, false)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }
    let longest = "x".repeat(100);
    assert!(
        h.service
            .search_server(&longest, "invoice", &account.id, false)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn opening_a_search_result_touches_it_and_a_gone_one_is_removed() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let kept = on_server_only(&h, &account, "Archive", "Archive:7", "Zebracorn notes", 400);
    let gone = on_server_only(
        &h,
        &account,
        "Archive",
        "Archive:8",
        "Zebracorn invoice",
        401,
    );
    let found = h
        .service
        .search_server("s", "zebracorn", &account.id, false)
        .await
        .unwrap();
    assert_eq!(found.message_ids, [kept.clone(), gone.clone()]);
    // Long unseen, then opened: kept by the pruning that follows a sync.
    h.service
        .store()
        .execute_for_tests("UPDATE messages SET search_seen_at = 0 WHERE search_only = 1");
    let message = h.service.get_message(&kept, false).await.unwrap();
    assert!(message.body_html.unwrap().contains("Hello"));

    // Deleted on the server, outside the window: noticed when opened. The
    // folder's counts are the server's, which may not count it any more.
    h.server.lock().unwrap().server_only.remove("Archive:8");
    let archive = h
        .service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Archive)
        .unwrap()
        .unwrap();
    h.service
        .store()
        .set_folder_counts(&archive.id, 3, 10)
        .unwrap();
    let error = h.service.get_message(&gone, false).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(error.message, "This message is no longer on the server.");
    assert!(h.service.store().get_message_summary(&gone).is_err());
    assert!(h.sink.settled.lock().unwrap().contains(&gone));
    let archive = h.service.store().get_folder(&archive.id).unwrap();
    assert_eq!((archive.unread_count, archive.total_count), (3, 10));

    h.service.sync_account(&account.id).await.unwrap();
    assert_eq!(local_ids(&h, "zebracorn", &[]), [kept]);
}

#[tokio::test]
async fn stale_search_results_are_pruned_after_sync() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let stale = on_server_only(&h, &account, "Archive", "Archive:7", "Zebracorn notes", 400);
    let fresh = on_server_only(&h, &account, "INBOX", "INBOX:70", "Zebracorn invoice", 300);
    h.service
        .search_server("s", "zebracorn", &account.id, false)
        .await
        .unwrap();
    let fifteen_days_ago = (Utc::now() - Duration::days(SEARCH_RESULT_TTL_DAYS + 1)).timestamp();
    h.service.store().execute_for_tests(&format!(
        "UPDATE messages SET search_seen_at = {fifteen_days_ago} WHERE remote_id = 'Archive:7'"
    ));
    let inbox_before = inbox(&h, &account);
    h.service.sync_account(&account.id).await.unwrap();
    assert!(h.service.store().get_message_summary(&stale).is_err());
    assert!(h.service.store().get_message_summary(&fresh).is_ok());
    let inbox_after = inbox(&h, &account);
    assert_eq!(inbox_before.total_count, inbox_after.total_count);
}

/// The user deletes a message while a server search whose answer lists it
/// is on its way: the answer, stored after, doesn't bring it back.
#[tokio::test]
async fn a_search_answered_before_a_deletion_doesnt_bring_it_back() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let answered = Arc::new(tokio::sync::Notify::new());
    let hold = Arc::new(tokio::sync::Notify::new());
    {
        let mut server = h.server.lock().unwrap();
        server.search_answered = Some(answered.clone());
        server.search_hold = Some(hold.clone());
    }
    let invoice = local_id(&account.id, EntityKind::Message, "INBOX:2");
    let search = h.service.search_server("s", "invoice", &account.id, false);
    let delete = async {
        answered.notified().await;
        h.service
            .delete_messages(std::slice::from_ref(&invoice))
            .await
            .unwrap();
        hold.notify_one();
    };
    let (found, ()) = tokio::join!(search, delete);
    let found = found.unwrap();
    assert_eq!(found.status, ServerSearchStatus::Done);
    assert!(found.message_ids.is_empty(), "{:?}", found.message_ids);
    // Only its copy in Trash, before and after a sync.
    let trash = h
        .service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Trash)
        .unwrap()
        .unwrap();
    for _ in 0..2 {
        assert!(h.service.store().get_message_summary(&invoice).is_err());
        let hits = h.service.search_messages("invoice", None, &[]).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].folder_id, trash.id);
        h.service.sync_account(&account.id).await.unwrap();
    }
}

/// New mail a server search finds before a sync does is still new mail to
/// the sync; old mail it finds never is.
#[tokio::test]
async fn new_mail_found_by_a_search_first_still_alerts() {
    let h = harness();
    let account = account_with_mail(&h).await;
    add_message(&h, &account, "INBOX", "INBOX:3", "Fresh zebracorn");
    on_server_only(&h, &account, "INBOX", "INBOX:4", "Old zebracorn", 400);
    let found = h
        .service
        .search_server("s", "zebracorn", &account.id, false)
        .await
        .unwrap();
    assert_eq!(found.added, 2);
    // A sync of a window that now reaches the old one too.
    {
        let mut server = h.server.lock().unwrap();
        let old = server.server_only.remove("INBOX:4").unwrap();
        server.messages.insert("INBOX:4".into(), old);
    }
    h.service.sync_account(&account.id).await.unwrap();
    assert_eq!(*h.sink.new_mail.lock().unwrap(), ["Fresh zebracorn"]);
}

#[tokio::test]
async fn search_results_are_pruned_even_when_the_sync_fails() {
    let h = harness();
    let account = account_with_mail(&h).await;
    let stale = on_server_only(&h, &account, "Archive", "Archive:7", "Zebracorn notes", 400);
    h.service
        .search_server("s", "zebracorn", &account.id, false)
        .await
        .unwrap();
    h.service
        .store()
        .execute_for_tests("UPDATE messages SET search_seen_at = 0 WHERE search_only = 1");
    // One folder keeps failing while the others sync.
    h.server.lock().unwrap().failing_folder = Some("Archive".into());
    assert!(h.service.sync_account(&account.id).await.is_err());
    assert!(h.service.store().get_message_summary(&stale).is_err());

    // Nor does a sign-in that expired keep them.
    h.service
        .search_server("t", "zebracorn", &account.id, false)
        .await
        .unwrap();
    h.service
        .store()
        .execute_for_tests("UPDATE messages SET search_seen_at = 0 WHERE search_only = 1");
    h.server.lock().unwrap().fail_sync = true;
    let error = h.service.sync_account(&account.id).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthRequired);
    assert!(h.service.store().get_message_summary(&stale).is_err());
}

/// A service with background work and a scripted push watcher.
struct Background {
    h: Harness,
    service: Arc<CoreService>,
    watcher: Arc<ScriptedWatcher>,
}

fn background() -> Background {
    let h = harness();
    let watcher = ScriptedWatcher::new();
    *h.connector.watcher.lock().unwrap() = Some(watcher.clone());
    h.server.lock().unwrap().watching = Arc::clone(&watcher.open);
    // A second service on the same fakes, shared as background work needs.
    let service = Arc::new(CoreService::new(
        Arc::clone(h.service.store()),
        h.secrets.clone(),
        h.connector.clone(),
        h.sink.clone(),
    ));
    Background {
        h,
        service,
        watcher,
    }
}

impl Background {
    async fn account(&self) -> Account {
        let account = self
            .service
            .add_imap_account(new_imap_account("me@example.com"))
            .await
            .unwrap();
        add_message(&self.h, &account, "INBOX", "INBOX:1", "Welcome");
        account
    }

    fn health(&self, account: &Account) -> Option<(PushMode, PushHealth)> {
        self.service
            .push_statuses()
            .unwrap()
            .into_iter()
            .find(|s| s.account_id == account.id)
            .map(|s| (s.mode, s.health))
    }

    async fn settle(&self) {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

#[tokio::test(start_paused = true)]
async fn push_brings_new_mail_between_full_syncs() {
    let b = background();
    let account = b.account().await;
    b.service.start_background();
    // The first full sync runs after two seconds.
    b.settle().await;
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), 1);
    assert_eq!(
        b.health(&account),
        Some((PushMode::ImapIdle, PushHealth::Live))
    );
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 1);
    assert!(b.h.sink.new_mail.lock().unwrap().is_empty());

    add_message(&b.h, &account, "INBOX", "INBOX:2", "Pushed");
    b.watcher.send(PushEvent::InboxChanged);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(*b.h.sink.new_mail.lock().unwrap(), ["Pushed"]);
    // No full sync was needed for it.
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), 1);
    let pushed = b.h.sink.push.lock().unwrap().clone();
    assert!(pushed.iter().all(|s| s.account_id == account.id));
    assert_eq!(pushed.last().unwrap().health, PushHealth::Live);

    // Then the periodic sync comes around.
    tokio::time::sleep(std::time::Duration::from_secs(5 * 60)).await;
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), 2);

    // On paused time, a task that ignored the shutdown would show as the
    // full two-second limit.
    let started = tokio::time::Instant::now();
    b.service.shutdown().await;
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(b.watcher.cancelled.load(Ordering::SeqCst), 1);
    // Nothing runs or starts afterwards.
    b.service.restart_push(&account.id).await;
    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 1);
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn waking_up_reconnects_and_syncs_everything() {
    let b = background();
    let account = b.account().await;
    b.service.start_background();
    b.settle().await;
    let syncs = b.h.sink.full_syncs.load(Ordering::SeqCst);

    b.service.wake(WakeReason::Resume);
    b.settle().await;
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 2);
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), syncs + 1);

    // Offline: push pauses and the periodic sync skips its turn.
    b.service.network_changed(false);
    b.settle().await;
    assert!(matches!(
        b.health(&account),
        Some((_, PushHealth::Degraded { .. }))
    ));
    tokio::time::sleep(std::time::Duration::from_secs(10 * 60)).await;
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), syncs + 1);
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 2);
    // Back online: reconnect and catch up.
    b.service.network_changed(true);
    b.settle().await;
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 3);
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), syncs + 2);
    assert_eq!(
        b.health(&account),
        Some((PushMode::ImapIdle, PushHealth::Live))
    );
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn settings_and_account_changes_restart_push() {
    let b = background();
    let account = b.account().await;
    b.service.start_background();
    b.settle().await;
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 1);

    // Unrelated settings leave the connection alone.
    let settings = b.service.get_settings().unwrap();
    b.service
        .update_settings(Settings {
            notify_sound: false,
            ..settings.clone()
        })
        .await
        .unwrap();
    b.settle().await;
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 1);

    // Instant updates off: no connection, the periodic sync alone.
    b.service
        .update_settings(Settings {
            instant_push: false,
            ..settings.clone()
        })
        .await
        .unwrap();
    b.settle().await;
    assert_eq!(b.watcher.cancelled.load(Ordering::SeqCst), 1);
    assert_eq!(b.health(&account), Some((PushMode::Off, PushHealth::Live)));
    b.service
        .update_settings(Settings {
            instant_push: true,
            ..settings
        })
        .await
        .unwrap();
    b.settle().await;
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 2);

    // New credentials reconnect.
    b.service
        .update_imap_account(
            &account.id,
            ImapAccountUpdate {
                password: Some("new".into()),
                settings: None,
            },
        )
        .await
        .unwrap();
    b.settle().await;
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 3);

    // Removing the account stops its push and forgets its status.
    b.service.remove_account(&account.id).await.unwrap();
    b.settle().await;
    assert_eq!(b.watcher.cancelled.load(Ordering::SeqCst), 3);
    assert!(b.service.push_statuses().unwrap().is_empty());
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn an_account_whose_removal_failed_keeps_its_push() {
    let b = background();
    let account = b.account().await;
    b.service.start_background();
    b.settle().await;
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 1);
    b.service.store().execute_for_tests(
        "CREATE TEMP TRIGGER keep_accounts BEFORE DELETE ON accounts
         BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
    );
    assert!(b.service.remove_account(&account.id).await.is_err());
    b.settle().await;
    // Push was stopped for the removal, and starts again.
    assert_eq!(b.watcher.cancelled.load(Ordering::SeqCst), 1);
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 2);
    assert_eq!(
        b.health(&account),
        Some((PushMode::ImapIdle, PushHealth::Live))
    );
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn accounts_without_a_watcher_poll_the_inbox() {
    let b = background();
    *b.h.connector.watcher.lock().unwrap() = None;
    let account = b.account().await;
    b.service.start_background();
    b.settle().await;
    assert_eq!(
        b.health(&account),
        Some((PushMode::Polling { every_secs: 120 }, PushHealth::Live))
    );
    add_message(&b.h, &account, "INBOX", "INBOX:2", "Polled");
    tokio::time::sleep(std::time::Duration::from_secs(121)).await;
    assert_eq!(*b.h.sink.new_mail.lock().unwrap(), ["Polled"]);
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn rejected_credentials_park_push_until_the_account_is_fixed() {
    let b = background();
    *b.h.connector.watcher.lock().unwrap() = None;
    let account = b.account().await;
    b.service.start_background();
    b.settle().await;
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), 1);

    // The server now rejects us: the next poll parks push...
    b.h.server.lock().unwrap().fail_sync = true;
    tokio::time::sleep(std::time::Duration::from_secs(121)).await;
    assert!(matches!(
        b.health(&account),
        Some((PushMode::Polling { .. }, PushHealth::AuthRequired))
    ));
    // ...so it stops asking: the poll due at four minutes would bring this
    // in (the next full sync is at five).
    b.h.server.lock().unwrap().fail_sync = false;
    add_message(&b.h, &account, "INBOX", "INBOX:2", "Unseen");
    tokio::time::sleep(std::time::Duration::from_secs(150)).await;
    assert!(b.h.sink.new_mail.lock().unwrap().is_empty());

    // Fixing the account brings it back.
    b.service
        .update_imap_account(&account.id, ImapAccountUpdate::default())
        .await
        .unwrap();
    b.settle().await;
    assert_eq!(*b.h.sink.new_mail.lock().unwrap(), ["Unseen"]);
    assert_eq!(
        b.health(&account),
        Some((PushMode::Polling { every_secs: 120 }, PushHealth::Live))
    );
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn push_parked_by_a_passing_rejection_resumes_after_a_good_sync() {
    let b = background();
    *b.h.connector.watcher.lock().unwrap() = None;
    let account = b.account().await;
    b.service.start_background();
    b.settle().await;

    // The server rejects one poll, say during an outage at its end...
    b.h.server.lock().unwrap().fail_sync = true;
    tokio::time::sleep(std::time::Duration::from_secs(121)).await;
    assert!(matches!(
        b.health(&account),
        Some((_, PushHealth::AuthRequired))
    ));
    b.h.server.lock().unwrap().fail_sync = false;
    // ...and the next full sync, at five minutes, works: push carries on.
    tokio::time::sleep(std::time::Duration::from_secs(180)).await;
    assert_eq!(
        b.health(&account),
        Some((PushMode::Polling { every_secs: 120 }, PushHealth::Live))
    );
    add_message(&b.h, &account, "INBOX", "INBOX:2", "Polled");
    tokio::time::sleep(std::time::Duration::from_secs(121)).await;
    assert_eq!(*b.h.sink.new_mail.lock().unwrap(), ["Polled"]);
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), 2);
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn push_gives_way_when_the_server_refuses_the_sync_connection() {
    let b = background();
    let account = b.account().await;
    // The server has room for one more connection, and push takes it first.
    b.h.server.lock().unwrap().one_free_slot = true;
    b.service.start_background();
    b.settle().await;
    // The first full sync was refused: push gave up its connection, and the
    // sync tried again and worked.
    assert_eq!(b.watcher.cancelled.load(Ordering::SeqCst), 1);
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), 1);
    let polling = |health: Option<(PushMode, PushHealth)>| {
        matches!(
            health,
            Some((
                PushMode::Polling { every_secs: 60 },
                PushHealth::Degraded { .. }
            ))
        )
    };
    assert!(polling(b.health(&account)));
    // Meanwhile the Inbox is checked every minute.
    add_message(&b.h, &account, "INBOX", "INBOX:2", "Polled");
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    assert_eq!(*b.h.sink.new_mail.lock().unwrap(), ["Polled"]);
    // Push tries again after five minutes, and gives way again.
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 1);
    tokio::time::sleep(std::time::Duration::from_secs(5 * 60)).await;
    assert_eq!(b.watcher.connections.load(Ordering::SeqCst), 2);
    assert_eq!(b.watcher.cancelled.load(Ordering::SeqCst), 2);
    assert!(polling(b.health(&account)));
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn polls_back_off_while_syncs_keep_failing() {
    let b = background();
    *b.h.connector.watcher.lock().unwrap() = None;
    // A Microsoft account, whose Inbox is checked every 30 seconds.
    let account = Account {
        id: "ms".into(),
        kind: AccountKind::Microsoft,
        display_name: "Me".into(),
        email: "me@outlook.example".into(),
        color: "#0b7a75".into(),
        settings: AccountSettings::Microsoft(MicrosoftSettings {
            tenant_id: "consumers".into(),
            user_id: "me".into(),
        }),
        created_at: Utc::now(),
        last_synced_at: None,
        sync_error: None,
    };
    b.service.store().insert_account(&account).unwrap();
    b.service.sync_account(&account.id).await.unwrap();
    let inbox_syncs = || b.h.server.lock().unwrap().inbox_syncs;

    // Microsoft's sign-in keeps refusing the app.
    b.h.server.lock().unwrap().failure = Some(Error::provider(
        "unauthorized_client: AADSTS7000112: Application is disabled.",
    ));
    let before = inbox_syncs();
    b.service.start_background();
    tokio::time::sleep(std::time::Duration::from_secs(30 * 60)).await;
    // Without backing off: 60 polls plus 6 full syncs.
    let failing = inbox_syncs() - before;
    assert!(failing < 40, "{failing}");

    // Once it works again, so do the polls.
    b.h.server.lock().unwrap().failure = None;
    tokio::time::sleep(std::time::Duration::from_secs(10 * 60)).await;
    let before = inbox_syncs();
    tokio::time::sleep(std::time::Duration::from_secs(5 * 60)).await;
    assert!(inbox_syncs() - before >= 10, "{}", inbox_syncs() - before);
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_sync_asked_for_during_a_push_sync_runs_after_it() {
    let b = background();
    let account = b.account().await;
    b.service.start_background();
    b.settle().await;
    let syncs = b.h.sink.full_syncs.load(Ordering::SeqCst);

    // Push syncs the Inbox, which takes a while.
    b.h.server.lock().unwrap().inbox_delay = Some(std::time::Duration::from_secs(10));
    b.watcher.send(PushEvent::InboxChanged);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    // Meanwhile a message is sent, and the app syncs to show the copy (or
    // the user presses Sync): that doesn't wait...
    add_message(&b.h, &account, "Archive", "Archive:1", "Sent copy");
    let asked = tokio::time::Instant::now();
    b.service.sync_account(&account.id).await.unwrap();
    assert!(asked.elapsed() < std::time::Duration::from_secs(1));
    b.h.server.lock().unwrap().inbox_delay = None;
    // ...and isn't dropped either: it runs once the push sync is done.
    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    assert_eq!(b.h.sink.full_syncs.load(Ordering::SeqCst), syncs + 1);
    let archive = b
        .service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Archive)
        .unwrap()
        .unwrap();
    assert_eq!(
        b.service
            .list_messages(&archive.id, &MessageQuery::default())
            .unwrap()
            .total,
        1
    );
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn restarting_push_mid_sync_keeps_the_new_mail_alert() {
    let b = background();
    let account = b.account().await;
    b.service.start_background();
    b.settle().await;

    // New mail, stored by a push sync that then waits for the counts...
    b.h.server.lock().unwrap().counts_delay = Some(std::time::Duration::from_secs(10));
    add_message(&b.h, &account, "INBOX", "INBOX:2", "Pushed");
    b.watcher.send(PushEvent::InboxChanged);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    // ...while the user saves the account, which restarts its push.
    b.service
        .update_imap_account(&account.id, ImapAccountUpdate::default())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    assert_eq!(*b.h.sink.new_mail.lock().unwrap(), ["Pushed"]);
    // Later syncs don't alert it again.
    tokio::time::sleep(std::time::Duration::from_secs(10 * 60)).await;
    assert_eq!(*b.h.sink.new_mail.lock().unwrap(), ["Pushed"]);
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn changing_settings_mid_sync_lets_the_sync_finish() {
    let b = background();
    b.account().await;
    b.service.start_background();
    b.settle().await;

    // Waking up syncs everything; the Inbox takes a while.
    b.h.server.lock().unwrap().inbox_delay = Some(std::time::Duration::from_secs(2));
    b.service.wake(WakeReason::Resume);
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(
        b.service.sync_statuses().unwrap()[0].state,
        SyncState::Syncing
    );
    // Meanwhile a setting that restarts push changes.
    let settings = b.service.get_settings().unwrap();
    b.service
        .update_settings(Settings {
            use_push_relay: !settings.use_push_relay,
            ..settings
        })
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    assert_eq!(b.service.sync_statuses().unwrap()[0].state, SyncState::Idle);
    b.service.shutdown().await;
}

/// A keyring that is locked, and whose unlock prompt the user dismisses
/// after a few seconds, every time.
#[derive(Default)]
struct LockedKeyring {
    reads: Mutex<HashMap<String, usize>>,
}

#[async_trait]
impl SecretStore for LockedKeyring {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        *self
            .reads
            .lock()
            .unwrap()
            .entry(key.to_owned())
            .or_default() += 1;
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        Err(Error::new(
            ErrorCode::Storage,
            "could not access the system keyring: the prompt was dismissed",
        ))
    }

    async fn set(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }

    async fn delete(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_locked_keyring_is_only_asked_at_the_periodic_sync() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut settings = store.get_settings().unwrap();
    settings.microsoft_client_id = Some("00000000-0000-0000-0000-000000000000".into());
    store.save_settings(&settings).unwrap();
    // A Microsoft account and an IMAP account, both synced before.
    let microsoft = AccountSettings::Microsoft(MicrosoftSettings {
        tenant_id: "consumers".into(),
        user_id: "me".into(),
    });
    let mut imap = new_imap_account("me@example.com").settings;
    (imap.imap_host, imap.imap_port, imap.imap_tls) = ("127.0.0.1".into(), 1, TlsMode::None);
    for (id, settings) in [("ms", microsoft), ("imap", AccountSettings::Imap(imap))] {
        store
            .insert_account(&Account {
                id: id.into(),
                kind: match settings {
                    AccountSettings::Microsoft(_) => AccountKind::Microsoft,
                    AccountSettings::Imap(_) => AccountKind::Imap,
                },
                display_name: "Me".into(),
                email: format!("me@{id}.example"),
                color: "#0b7a75".into(),
                settings,
                created_at: Utc::now(),
                last_synced_at: Some(Utc::now()),
                sync_error: None,
            })
            .unwrap();
        store
            .replace_folders(
                id,
                &[Folder {
                    id: local_id(id, EntityKind::Folder, "INBOX"),
                    account_id: id.into(),
                    remote_id: "INBOX".into(),
                    parent_id: None,
                    name: "Inbox".into(),
                    role: FolderRole::Inbox,
                    unread_count: 0,
                    total_count: 0,
                }],
            )
            .unwrap();
    }
    let keyring = Arc::new(LockedKeyring::default());
    let service = Arc::new(CoreService::new(
        Arc::clone(&store),
        keyring.clone(),
        Arc::new(LiveConnector::new(store, keyring.clone())),
        Arc::new(NullSink),
    ));
    service.start_background();
    tokio::time::sleep(std::time::Duration::from_secs(15 * 60)).await;
    // The periodic sync asks at two seconds, then every five minutes; push
    // asks once when it starts, and then waits for the periodic sync.
    let reads = keyring.reads.lock().unwrap().clone();
    assert_eq!(reads.len(), 2, "{reads:?}");
    assert!(reads.values().all(|n| *n <= 4), "{reads:?}");
    for status in service.push_statuses().unwrap() {
        assert!(
            matches!(status.health, PushHealth::Degraded { .. }),
            "{status:?}"
        );
    }
    service.shutdown().await;
}

#[tokio::test]
async fn the_installation_id_is_stable() {
    let h = harness();
    let id = h.service.installation_id().unwrap();
    assert_eq!(id.len(), 22);
    assert_eq!(h.service.installation_id().unwrap(), id);
}

#[tokio::test]
async fn snooze_time_is_validated() {
    let h = harness();
    let err = h
        .service
        .update_settings(Settings {
            default_snooze_minutes: 0,
            ..Settings::default()
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

// --- Microsoft accounts through the push service ------------------------------

use crate::auth::SyncTokenError;
use crate::push::fakes::{FakeGraph, FakeTokens, ScriptedRelay};

/// A service whose Microsoft accounts get push through a scripted push
/// service, with Graph's subscriptions in memory.
struct RelayBackground {
    b: Background,
    relay: Arc<ScriptedRelay>,
    tokens: Arc<FakeTokens>,
    graph: Arc<FakeGraph>,
}

fn relay_background() -> RelayBackground {
    let b = background();
    let (relay, tokens, graph) = (ScriptedRelay::new(), FakeTokens::new(), FakeGraph::new());
    *b.h.connector.relay.lock().unwrap() = Some(RelaySource {
        relay: relay.clone(),
        tokens: tokens.clone(),
    });
    *b.h.connector.subscriptions.lock().unwrap() = Some(graph.clone());
    RelayBackground {
        b,
        relay,
        tokens,
        graph,
    }
}

/// A Microsoft account (signed in elsewhere: the fakes need no token).
fn microsoft_account(h: &Harness, email: &str) -> Account {
    let account = Account {
        id: Uuid::new_v4().to_string(),
        kind: AccountKind::Microsoft,
        display_name: "Adele".into(),
        email: email.into(),
        color: "#0b7a75".into(),
        settings: AccountSettings::Microsoft(MicrosoftSettings {
            tenant_id: "11111111-2222-3333-4444-555555555555".into(),
            user_id: "33333333-0000-0000-0000-00000000000a".into(),
        }),
        created_at: Utc::now(),
        last_synced_at: None,
        sync_error: None,
    };
    h.service.store().insert_account(&account).unwrap();
    add_message(h, &account, "INBOX", "INBOX:1", "Welcome");
    account
}

#[tokio::test(start_paused = true)]
async fn a_change_from_the_push_service_syncs_the_inbox_and_alerts() {
    let r = relay_background();
    let account = microsoft_account(&r.b.h, "adele@contoso.com");
    r.b.service.start_background();
    r.b.settle().await;
    assert_eq!(r.b.h.sink.full_syncs.load(Ordering::SeqCst), 1);
    assert_eq!(
        r.b.health(&account),
        Some((PushMode::GraphRelay, PushHealth::Live))
    );
    let resources: Vec<String> = r
        .graph
        .state
        .lock()
        .unwrap()
        .subs
        .iter()
        .map(|s| s.resource.clone())
        .collect();
    assert_eq!(resources, [INBOX_RESOURCE, EVENTS_RESOURCE]);
    assert!(r.b.h.sink.new_mail.lock().unwrap().is_empty());
    let syncs = r.b.h.server.lock().unwrap().inbox_syncs;

    add_message(&r.b.h, &account, "INBOX", "INBOX:2", "Pushed");
    r.relay.send(RelayEvent::Change {
        subscription: r.graph.id_of(INBOX_RESOURCE),
    });
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(*r.b.h.sink.new_mail.lock().unwrap(), ["Pushed"]);
    // A targeted sync of the Inbox, not a full one.
    assert_eq!(r.b.h.server.lock().unwrap().inbox_syncs, syncs + 1);
    assert_eq!(r.b.h.sink.full_syncs.load(Ordering::SeqCst), 1);

    // While live, the Inbox isn't polled.
    tokio::time::sleep(std::time::Duration::from_secs(4 * 60)).await;
    assert_eq!(r.b.h.server.lock().unwrap().inbox_syncs, syncs + 1);

    let started = tokio::time::Instant::now();
    r.b.service.shutdown().await;
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    // Stopping leaves the subscriptions for the next start.
    assert_eq!(r.graph.state.lock().unwrap().subs.len(), 2);
    assert_eq!(r.relay.open(), 0);
}

#[tokio::test(start_paused = true)]
async fn missing_consent_polls_and_offers_it_at_the_next_sign_in() {
    let r = relay_background();
    r.tokens.refuse(SyncTokenError::ConsentMissing);
    let account = microsoft_account(&r.b.h, "adele@contoso.com");
    r.b.service.start_background();
    r.b.settle().await;
    let status =
        r.b.service
            .push_statuses()
            .unwrap()
            .into_iter()
            .find(|s| s.account_id == account.id)
            .unwrap();
    assert_eq!(status.mode, PushMode::Polling { every_secs: 30 });
    assert_eq!(status.health, PushHealth::ConsentMissing);
    assert_eq!(status.detail.as_deref(), Some(CONSENT_HINT));
    // Signing in again asks for it.
    assert!(!status.consent_refused);
    assert_eq!(status.admin_consent, None);
    // Not an account error: syncing goes on.
    assert!(r.b.service.list_accounts().unwrap()[0].sync_error.is_none());
    let syncs = r.b.h.server.lock().unwrap().inbox_syncs;
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    assert_eq!(r.b.h.server.lock().unwrap().inbox_syncs, syncs + 2);

    // Signing in again as that account asks consent for the push service
    // too; as anyone else, not.
    for hint in [Some("ADELE@contoso.com"), Some("grace@contoso.com"), None] {
        let service = &r.b.service;
        let (result, ()) = tokio::join!(service.add_microsoft_account(&|_| Ok(()), hint), async {
            tokio::task::yield_now().await;
            service.cancel_microsoft_sign_in();
        });
        assert_eq!(result.unwrap_err().code, ErrorCode::SignInCancelled);
    }
    assert_eq!(
        *r.b.h.connector.push_consent.lock().unwrap(),
        [true, false, false]
    );
    r.b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn refused_push_consent_says_why_and_is_not_asked_again() {
    let r = relay_background();
    // Before the sign-in, and again as push restarts after it.
    r.tokens.refuse(SyncTokenError::ConsentMissing);
    r.tokens.refuse(SyncTokenError::ConsentMissing);
    let account = microsoft_account(&r.b.h, "adele@contoso.com");
    r.b.service.start_background();
    r.b.settle().await;

    // Signing in again asks consent for the push service, which the
    // organization's settings refuse; the sign-in itself goes ahead.
    let refusal = Error::new(
        ErrorCode::AdminConsentRequired,
        "Your organization requires an administrator to approve instant updates.",
    )
    .with_details(serde_json::json!({
        "adminConsentUrl": "https://login.microsoftonline.com/contoso.com/v2.0/adminconsent?client_id=x",
        "tenant": "contoso.com",
        "canRequestApproval": false,
    }));
    *r.b.h.connector.sign_in.lock().unwrap() = Some(SignInResult {
        email: "adele@contoso.com".into(),
        display_name: "Adele".into(),
        tenant_id: "11111111-2222-3333-4444-555555555555".into(),
        user_id: "33333333-0000-0000-0000-00000000000a".into(),
        access_token: "access".into(),
        refresh_token: "refresh".into(),
        expires_in: std::time::Duration::from_secs(3600),
        consent_refused: Some(refusal.clone()),
    });
    let signed_in =
        r.b.service
            .add_microsoft_account(&|_| Ok(()), Some("adele@contoso.com"))
            .await
            .unwrap();
    assert_eq!(signed_in.id, account.id);
    r.b.settle().await;
    let status = || {
        r.b.service
            .push_statuses()
            .unwrap()
            .into_iter()
            .find(|s| s.account_id == account.id)
            .unwrap()
    };
    assert_eq!(status().health, PushHealth::ConsentMissing);
    // Why, not "sign in again", which would be refused again, with the link
    // an administrator approves it at.
    assert_eq!(status().detail.as_deref(), Some(refusal.message.as_str()));
    assert!(status().consent_refused);
    assert_eq!(
        status().admin_consent,
        Some(crate::push::AdminConsent {
            url: "https://login.microsoftonline.com/contoso.com/v2.0/adminconsent?client_id=x"
                .into(),
            tenant: "contoso.com".into(),
        })
    );

    // A later sign-in doesn't ask again, which would open the browser twice.
    let service = &r.b.service;
    let (result, ()) = tokio::join!(
        service.add_microsoft_account(&|_| Ok(()), Some("adele@contoso.com")),
        async {
            tokio::task::yield_now().await;
            service.cancel_microsoft_sign_in();
        }
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::SignInCancelled);
    assert_eq!(*r.b.h.connector.push_consent.lock().unwrap(), [true, false]);

    // Once an administrator approves, push goes live (after the 6-hour
    // wait) and the refusal goes.
    tokio::time::sleep(std::time::Duration::from_secs(7 * 60 * 60)).await;
    assert_eq!(status().health, PushHealth::Live);
    assert_eq!(status().admin_consent, None);
    assert!(!status().consent_refused);
    assert!(!r.b.service.push.consent_refused(&account.id));
    r.b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn turning_the_push_service_off_unsubscribes_and_polls() {
    let r = relay_background();
    let account = microsoft_account(&r.b.h, "adele@contoso.com");
    r.b.service.start_background();
    r.b.settle().await;
    let ids: Vec<String> = [INBOX_RESOURCE, EVENTS_RESOURCE]
        .iter()
        .map(|r_| r.graph.id_of(r_))
        .collect();
    assert_eq!(
        r.b.service
            .store()
            .push_subscriptions(&account.id)
            .unwrap()
            .len(),
        2
    );

    let settings = r.b.service.get_settings().unwrap();
    r.b.service
        .update_settings(Settings {
            use_push_relay: false,
            ..settings.clone()
        })
        .await
        .unwrap();
    r.b.settle().await;
    let mut deleted = r.graph.state.lock().unwrap().deleted.clone();
    deleted.sort();
    assert_eq!(deleted, ids);
    assert!(
        r.b.service
            .store()
            .push_subscriptions(&account.id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        r.b.health(&account),
        Some((PushMode::Polling { every_secs: 30 }, PushHealth::Live))
    );
    assert_eq!(r.relay.open(), 0);

    // On again: subscribed again, and live.
    r.b.service
        .update_settings(Settings {
            use_push_relay: true,
            ..settings
        })
        .await
        .unwrap();
    r.b.settle().await;
    assert_eq!(
        r.b.health(&account),
        Some((PushMode::GraphRelay, PushHealth::Live))
    );
    assert_eq!(r.graph.state.lock().unwrap().subs.len(), 2);
    r.b.service.shutdown().await;
}

#[tokio::test]
async fn removing_a_microsoft_account_deletes_its_subscriptions() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::auth::StaticToken;
    use crate::providers::graph::GraphProvider;
    use crate::store::PushSubscription;

    let h = harness();
    let server = MockServer::start().await;
    *h.connector.subscriptions.lock().unwrap() = Some(Arc::new(GraphProvider::with_base_url(
        "acct",
        Arc::new(StaticToken("graph-token".into())),
        server.uri(),
    )));
    Mock::given(method("DELETE"))
        .and(path("/subscriptions/inbox-sub"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    // Already gone: still fine.
    Mock::given(method("DELETE"))
        .and(path("/subscriptions/events-sub"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": { "code": "ResourceNotFound", "message": "Not found" }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let account = microsoft_account(&h, "adele@contoso.com");
    let sub = |id: &str, resource: &str| PushSubscription {
        id: id.into(),
        resource: resource.into(),
        expires_at: Utc::now(),
    };
    h.service
        .store()
        .replace_push_subscriptions(
            &account.id,
            &[
                sub("inbox-sub", INBOX_RESOURCE),
                sub("events-sub", EVENTS_RESOURCE),
            ],
        )
        .unwrap();
    h.service.remove_account(&account.id).await.unwrap();
    assert!(h.service.list_accounts().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_silent_graph_holds_up_removing_an_account_five_seconds_at_most() {
    use crate::auth::StaticToken;
    use crate::providers::graph::GraphProvider;
    use crate::store::PushSubscription;

    // Graph takes the request and never answers.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let silent = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let h = harness();
    *h.connector.subscriptions.lock().unwrap() = Some(Arc::new(GraphProvider::with_base_url(
        "acct",
        Arc::new(StaticToken("graph-token".into())),
        format!("http://{address}"),
    )));
    let account = microsoft_account(&h, "grace@contoso.com");
    h.service
        .store()
        .replace_push_subscriptions(
            &account.id,
            &[PushSubscription {
                id: "x".into(),
                resource: INBOX_RESOURCE.into(),
                expires_at: Utc::now(),
            }],
        )
        .unwrap();
    let started = tokio::time::Instant::now();
    h.service.remove_account(&account.id).await.unwrap();
    let took = started.elapsed();
    assert!(
        took >= std::time::Duration::from_secs(5) && took < std::time::Duration::from_secs(6),
        "{took:?}"
    );
    assert!(h.service.list_accounts().unwrap().is_empty());
    silent.abort();
}

/// A keyring whose writes of a refresh token wait until the test lets them
/// through.
struct GatedKeyring {
    values: MemorySecretStore,
    /// Told when a write reaches the gate.
    writing: tokio::sync::Notify,
    gate: tokio::sync::Semaphore,
}

impl GatedKeyring {
    fn new() -> Self {
        Self {
            values: MemorySecretStore::default(),
            writing: tokio::sync::Notify::new(),
            gate: tokio::sync::Semaphore::new(0),
        }
    }
}

#[async_trait]
impl SecretStore for GatedKeyring {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        self.values.get(key).await
    }

    async fn set(&self, key: &str, value: &str) -> Result<()> {
        if key.ends_with(":refresh-token") {
            self.writing.notify_one();
            self.gate.acquire().await.unwrap().forget();
        }
        self.values.set(key, value).await
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.values.delete(key).await
    }
}

/// A service on `keyring` with a Microsoft account signed in as
/// `refresh-0`.
async fn signed_in_service(
    keyring: &Arc<GatedKeyring>,
) -> (Arc<CoreService>, Arc<FakeConnector>, Account) {
    let connector = Arc::new(FakeConnector::new(Arc::default(), false));
    let service = Arc::new(CoreService::new(
        Arc::new(Store::open_in_memory().unwrap()),
        keyring.clone(),
        connector.clone(),
        Arc::new(NullSink),
    ));
    let account = Account {
        id: Uuid::new_v4().to_string(),
        kind: AccountKind::Microsoft,
        display_name: "Adele".into(),
        email: "adele@contoso.com".into(),
        color: "#0b7a75".into(),
        settings: AccountSettings::Microsoft(MicrosoftSettings {
            tenant_id: "11111111-2222-3333-4444-555555555555".into(),
            user_id: "33333333-0000-0000-0000-00000000000a".into(),
        }),
        created_at: Utc::now(),
        last_synced_at: None,
        sync_error: None,
    };
    service.store().insert_account(&account).unwrap();
    keyring
        .values
        .set(&keys::refresh_token(&account.id), "refresh-0")
        .await
        .unwrap();
    (service, connector, account)
}

/// Removes the account while `saving` is held at the gate saving a refresh
/// token, then lets it through; returns the refresh token kept after.
async fn remove_while_saving<T: Send + 'static>(
    keyring: &Arc<GatedKeyring>,
    service: &Arc<CoreService>,
    account: &Account,
    saving: tokio::task::JoinHandle<T>,
) -> (T, Option<String>) {
    keyring.writing.notified().await;
    let removing = tokio::spawn({
        let service = Arc::clone(service);
        let id = account.id.clone();
        async move { service.remove_account(&id).await }
    });
    // Without the account's lock, the removal would be done by now.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    keyring.gate.add_permits(1);
    let saved = saving.await.unwrap();
    removing.await.unwrap().unwrap();
    assert!(service.list_accounts().unwrap().is_empty());
    let kept = keyring
        .values
        .get(&keys::refresh_token(&account.id))
        .await
        .unwrap();
    (saved, kept)
}

#[tokio::test]
async fn a_refresh_ending_as_its_account_is_removed_keeps_no_sign_in() {
    use crate::auth::TokenSource;
    use crate::auth::microsoft::{MicrosoftAuthConfig, MicrosoftTokenSource};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Microsoft rotates the refresh token.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token_type": "Bearer",
            "expires_in": 3599,
            "access_token": "access-1",
            "refresh_token": "refresh-1",
        })))
        .mount(&server)
        .await;
    let keyring = Arc::new(GatedKeyring::new());
    let (service, _, account) = signed_in_service(&keyring).await;
    let tokens = Arc::new(MicrosoftTokenSource::new(
        MicrosoftAuthConfig {
            client_id: "client".into(),
            tenant: "common".into(),
            authority_host: server.uri(),
            consent_scopes: Vec::new(),
        },
        account.id.clone(),
        keyring.clone(),
    ));
    // The refresh has found refresh-0 still kept, and is saving refresh-1,
    // when the account is removed.
    let refreshing = tokio::spawn(async move { tokens.access_token().await });
    let (token, kept) = remove_while_saving(&keyring, &service, &account, refreshing).await;
    assert_eq!(token.unwrap(), "access-1");
    assert_eq!(kept, None);
}

#[tokio::test]
async fn a_sign_in_ending_as_its_account_is_removed_keeps_no_sign_in() {
    let keyring = Arc::new(GatedKeyring::new());
    let (service, connector, account) = signed_in_service(&keyring).await;
    // Signing in again, as the same person...
    *connector.sign_in.lock().unwrap() = Some(SignInResult {
        email: "adele@contoso.com".into(),
        display_name: "Adele".into(),
        tenant_id: "11111111-2222-3333-4444-555555555555".into(),
        user_id: "33333333-0000-0000-0000-00000000000a".into(),
        access_token: "access".into(),
        refresh_token: "refresh-new".into(),
        expires_in: std::time::Duration::from_secs(3600),
        consent_refused: None,
    });
    // ...is saving the new refresh token when the account is removed.
    let signing_in = tokio::spawn({
        let service = Arc::clone(&service);
        async move { service.add_microsoft_account(&|_| Ok(()), None).await }
    });
    let (_, kept) = remove_while_saving(&keyring, &service, &account, signing_in).await;
    assert_eq!(kept, None);
}

#[tokio::test]
async fn a_sign_in_finding_its_account_removed_keeps_no_sign_in() {
    let keyring = Arc::new(GatedKeyring::new());
    // Writes go straight through.
    keyring.gate.add_permits(16);
    let (service, connector, account) = signed_in_service(&keyring).await;
    *connector.sign_in.lock().unwrap() = Some(SignInResult {
        email: "adele@contoso.com".into(),
        display_name: "Adele".into(),
        tenant_id: "11111111-2222-3333-4444-555555555555".into(),
        user_id: "33333333-0000-0000-0000-00000000000a".into(),
        access_token: "access".into(),
        refresh_token: "refresh-new".into(),
        expires_in: std::time::Duration::from_secs(3600),
        consent_refused: None,
    });
    // A removal holds the account's lock, deleting its secrets, when the
    // sign-in, having updated the account, comes to save its refresh token.
    let removing = secrets::lock_account(&account.id).await;
    let signing_in = tokio::spawn({
        let service = Arc::clone(&service);
        async move { service.add_microsoft_account(&|_| Ok(()), None).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    service.store().delete_account(&account.id).unwrap();
    keyring
        .values
        .delete(&keys::refresh_token(&account.id))
        .await
        .unwrap();
    drop(removing);
    // The sign-in finds the account gone and keeps nothing.
    let error = signing_in.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    let kept = keyring
        .values
        .get(&keys::refresh_token(&account.id))
        .await
        .unwrap();
    assert_eq!(kept, None);
}

// --- reminders ------------------------------------------------------------------

use crate::model::{Calendar, Event, ResponseStatus, Snooze};
use crate::providers::{CalendarProvider, ServerReminder};

/// 2026-10-15 09:00 UTC, when the reminder tests start.
fn reminder_epoch() -> DateTime<Utc> {
    use chrono::TimeZone;
    Utc.with_ymd_and_hms(2026, 10, 15, 9, 0, 0).unwrap()
}

/// Reminders go by a clock that starts at [`reminder_epoch`] and follows
/// Tokio's (paused) time, plus whatever is put in the returned jump (time
/// the computer slept).
fn virtual_clock(service: &CoreService) -> Arc<Mutex<Duration>> {
    let origin = tokio::time::Instant::now();
    let jump = Arc::new(Mutex::new(Duration::zero()));
    let slept = Arc::clone(&jump);
    service.reminders.set_clock(Arc::new(move || {
        reminder_epoch()
            + Duration::from_std(tokio::time::Instant::now() - origin).unwrap()
            + *slept.lock().unwrap()
    }));
    jump
}

/// Reminders go by a clock stopped at `at`.
fn stopped_clock(service: &CoreService, at: DateTime<Utc>) {
    service.reminders.set_clock(Arc::new(move || at));
}

fn local_event(
    calendar: &Calendar,
    subject: &str,
    start: DateTime<Utc>,
    reminder: Option<u32>,
) -> EventDraft {
    EventDraft {
        calendar_id: calendar.id.clone(),
        subject: subject.into(),
        location: None,
        body_html: None,
        start,
        end: start + Duration::minutes(30),
        is_all_day: false,
        attendees: vec![],
        reminder_minutes: reminder,
        show_as: ShowAs::Busy,
        is_online_meeting: false,
    }
}

#[tokio::test(start_paused = true)]
async fn the_scheduler_wakes_when_a_reminder_is_due() {
    let b = background();
    let account = b.account().await;
    let jump = virtual_clock(&b.service);
    let calendar = b.service.list_calendars().unwrap().remove(0);
    assert_eq!(calendar.account_id, account.id);
    // Starts in 20 minutes, reminds 15 before.
    b.service
        .create_event(local_event(
            &calendar,
            "Dentist",
            reminder_epoch() + Duration::minutes(20),
            Some(15),
        ))
        .await
        .unwrap();
    b.service.start_background();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert!(b.h.sink.shown_reminders().is_empty());

    tokio::time::sleep(std::time::Duration::from_secs(4 * 60 + 57)).await;
    assert!(b.h.sink.shown_reminders().is_empty(), "not yet");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert_eq!(b.h.sink.shown_reminders(), ["Dentist"]);
    let active = b.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].fire_at, reminder_epoch() + Duration::minutes(5));
    assert!(!active[0].overdue);

    // Asleep for two hours: a reminder that came due meanwhile goes off
    // as soon as the computer wakes up, well before the next minute.
    b.service
        .create_event(local_event(
            &calendar,
            "Lunch",
            reminder_epoch() + Duration::minutes(120),
            Some(15),
        ))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    *jump.lock().unwrap() = Duration::hours(2);
    b.service.wake(WakeReason::Resume);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(b.h.sink.shown_reminders(), ["Dentist", "Lunch"]);
    // The dentist's appointment is over by now; it stays listed.
    let titles: Vec<String> = b
        .service
        .list_active_reminders()
        .unwrap()
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert_eq!(titles, ["Dentist", "Lunch"]);
    b.service.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn turning_reminders_off_stops_them() {
    let b = background();
    b.account().await;
    virtual_clock(&b.service);
    let calendar = b.service.list_calendars().unwrap().remove(0);
    for (subject, minutes) in [("Soon", 10), ("Later", 30)] {
        b.service
            .create_event(local_event(
                &calendar,
                subject,
                reminder_epoch() + Duration::minutes(minutes),
                Some(15),
            ))
            .await
            .unwrap();
    }
    b.service.start_background();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(b.h.sink.shown_reminders(), ["Soon"]);
    let soon = b.service.list_active_reminders().unwrap()[0].key.clone();

    let settings = b.service.get_settings().unwrap();
    b.service
        .update_settings(Settings {
            reminders_enabled: false,
            ..settings.clone()
        })
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(b.h.sink.cleared_reminders(), [soon]);
    assert!(b.service.list_active_reminders().unwrap().is_empty());
    // "Later" comes due meanwhile: no notification.
    tokio::time::sleep(std::time::Duration::from_secs(20 * 60)).await;
    assert_eq!(b.h.sink.shown_reminders(), ["Soon"]);
    assert_eq!(
        b.h.sink.reminders.lock().unwrap().last().unwrap().active,
        []
    );

    // On again: what's due is listed, what was shown isn't shown again.
    b.service.update_settings(settings).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let active = b.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 2);
    assert_eq!(b.h.sink.shown_reminders(), ["Soon", "Later"]);
    b.service.shutdown().await;
}

#[tokio::test]
async fn local_reminders_are_snoozed_and_dismissed() {
    let h = harness();
    h.service
        .add_imap_account(new_imap_account("me@example.com"))
        .await
        .unwrap();
    let now = reminder_epoch();
    stopped_clock(&h.service, now);
    let calendar = h.service.list_calendars().unwrap().remove(0);
    for subject in ["Standup", "Review"] {
        h.service
            .create_event(local_event(
                &calendar,
                subject,
                now + Duration::minutes(5),
                Some(15),
            ))
            .await
            .unwrap();
    }
    let list = h.service.list_task_lists().unwrap().remove(0);
    let task = h
        .service
        .create_task(TaskDraft {
            list_id: list.id.clone(),
            title: "Call the bank".into(),
            notes: None,
            importance: None,
            due_date: None,
            reminder_at: Some(now - Duration::minutes(1)),
        })
        .await
        .unwrap();
    assert_eq!(task.reminder_at, Some(now - Duration::minutes(1)));
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 3);
    let key = |title: &str| {
        active
            .iter()
            .find(|r| r.title == title)
            .unwrap()
            .key
            .clone()
    };

    for bad in [
        Snooze::Minutes(0),
        Snooze::Until(now - Duration::minutes(1)),
        Snooze::Until(now + Duration::days(31)),
    ] {
        let err = h
            .service
            .snooze_reminder(&key("Standup"), bad)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput, "{bad:?}");
    }
    let gone = h
        .service
        .snooze_reminder("event:nothing:1", Snooze::Minutes(5))
        .await
        .unwrap_err();
    assert_eq!(gone.code, ErrorCode::NotFound);

    let active = h
        .service
        .snooze_reminder(&key("Standup"), Snooze::Minutes(10))
        .await
        .unwrap();
    let standup = active.iter().find(|r| r.title == "Standup").unwrap();
    assert_eq!(standup.snoozed_until, Some(now + Duration::minutes(10)));
    assert_eq!(
        h.service
            .store()
            .reminder_state(&standup.key)
            .unwrap()
            .unwrap()
            .pending,
        None,
        "nothing to tell a local calendar"
    );

    let active = h
        .service
        .dismiss_reminders(&[key("Review"), "event:nothing:1".into()])
        .await
        .unwrap();
    let titles: Vec<&str> = active.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["Call the bank", "Standup"]);
    assert_eq!(h.sink.cleared_reminders(), [key("Standup"), key("Review")]);

    // Completing the task clears its reminder.
    h.service
        .update_task(
            &task.id,
            TaskPatch {
                status: Some(TaskStatus::Completed),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 1);
    assert!(h.sink.cleared_reminders().contains(&key("Call the bank")));

    // A task's reminder changes and clears like its other fields.
    let changed = h
        .service
        .update_task(
            &task.id,
            TaskPatch {
                reminder_at: Some(now + Duration::hours(1)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.reminder_at, Some(now + Duration::hours(1)));
    let cleared = h
        .service
        .update_task(
            &task.id,
            TaskPatch {
                clear_reminder: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(cleared.reminder_at, None);
}

/// A Microsoft 365 calendar: one calendar, its events, and the reminders the
/// server keeps for them.
#[derive(Default)]
struct FakeCalendar {
    calendar: Mutex<Option<Calendar>>,
    events: Mutex<Vec<Event>>,
    reminders: Mutex<Vec<ServerReminder>>,
    /// Dismissing and snoozing fail while set.
    offline: std::sync::atomic::AtomicBool,
    /// Reading the reminders fails while set.
    view_fails: std::sync::atomic::AtomicBool,
    /// How often the reminders were read.
    views: AtomicUsize,
    dismissed: Mutex<Vec<String>>,
    snoozed: Mutex<Vec<(String, DateTime<Utc>)>>,
}

impl FakeCalendar {
    fn add(&self, account: &Account, remote: &str, start: DateTime<Utc>, minutes: u32) -> Event {
        let calendar = self.calendar.lock().unwrap().clone().unwrap();
        let event = Event {
            id: local_id(&account.id, EntityKind::Event, remote),
            account_id: account.id.clone(),
            calendar_id: calendar.id,
            remote_id: remote.into(),
            subject: remote.into(),
            location: None,
            body_html: None,
            body_preview: None,
            start,
            end: start + Duration::minutes(30),
            is_all_day: false,
            organizer: None,
            attendees: vec![],
            show_as: ShowAs::Busy,
            response_status: ResponseStatus::Organizer,
            is_online_meeting: true,
            online_meeting_url: Some("https://teams.example/join/1".into()),
            reminder_minutes: Some(minutes),
            is_recurring: false,
            is_cancelled: false,
        };
        self.events.lock().unwrap().push(event.clone());
        self.reminders.lock().unwrap().push(ServerReminder {
            event_id: remote.into(),
            fire_at: start - Duration::minutes(minutes.into()),
            change_key: None,
            event_start: event.start,
            event_end: event.end,
            subject: event.subject.clone(),
        });
        event
    }

    fn fail(&self) -> Result<()> {
        if self.offline.load(Ordering::SeqCst) {
            Err(Error::network("offline"))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl CalendarProvider for FakeCalendar {
    async fn list_calendars(&self) -> Result<Vec<Calendar>> {
        Ok(self.calendar.lock().unwrap().clone().into_iter().collect())
    }

    async fn list_events(
        &self,
        _: &Calendar,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Event>> {
        Ok(self
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.start < end && e.end > start)
            .cloned()
            .collect())
    }

    async fn create_event(&self, _: &Calendar, _: &EventDraft) -> Result<Event> {
        unimplemented!()
    }

    async fn update_event(&self, _: &Calendar, _: &str, _: &EventDraft) -> Result<Event> {
        unimplemented!()
    }

    async fn delete_event(&self, _: &Calendar, _: &str) -> Result<()> {
        unimplemented!()
    }

    async fn respond_to_event(
        &self,
        _: &Calendar,
        _: &str,
        _: crate::model::EventResponse,
        _: Option<&str>,
        _: bool,
    ) -> Result<()> {
        Ok(())
    }

    async fn reminders(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Option<Vec<ServerReminder>>> {
        self.views.fetch_add(1, Ordering::SeqCst);
        if self.view_fails.load(Ordering::SeqCst) {
            return Err(Error::provider("Microsoft Graph returned HTTP 503."));
        }
        Ok(Some(
            self.reminders
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.event_start >= start && r.event_start < end)
                .cloned()
                .collect(),
        ))
    }

    async fn dismiss_reminder(&self, remote_id: &str) -> Result<()> {
        self.fail()?;
        self.dismissed.lock().unwrap().push(remote_id.into());
        self.reminders
            .lock()
            .unwrap()
            .retain(|r| r.event_id != remote_id);
        Ok(())
    }

    async fn snooze_reminder(&self, remote_id: &str, until: DateTime<Utc>) -> Result<()> {
        self.fail()?;
        self.snoozed.lock().unwrap().push((remote_id.into(), until));
        for r in self.reminders.lock().unwrap().iter_mut() {
            if r.event_id == remote_id {
                r.fire_at = until;
            }
        }
        Ok(())
    }
}

/// A Microsoft account with a calendar on [`FakeCalendar`].
fn microsoft_calendar(h: &Harness) -> (Account, Arc<FakeCalendar>) {
    let account = microsoft_account(h, "adele@contoso.com");
    let fake = Arc::new(FakeCalendar::default());
    *fake.calendar.lock().unwrap() = Some(Calendar {
        id: local_id(&account.id, EntityKind::Calendar, "cal"),
        account_id: account.id.clone(),
        remote_id: "cal".into(),
        name: "Calendar".into(),
        color: "#0b7a75".into(),
        can_edit: true,
        is_default: true,
    });
    *h.connector.calendar.lock().unwrap() = Some(fake.clone());
    (account, fake)
}

#[tokio::test]
async fn microsoft_365_hears_of_dismissals_and_snoozes_and_keeps_trying() {
    let h = harness();
    let (account, fake) = microsoft_calendar(&h);
    let now = reminder_epoch();
    stopped_clock(&h.service, now);
    fake.add(&account, "standup", now + Duration::minutes(10), 15);
    fake.add(&account, "review", now + Duration::minutes(5), 15);
    h.service.sync_account(&account.id).await.unwrap();
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 2);
    let key = |title: &str| {
        active
            .iter()
            .find(|r| r.title == title)
            .unwrap()
            .key
            .clone()
    };

    // Offline: dismissed here, and Microsoft 365 hears of it later.
    fake.offline.store(true, Ordering::SeqCst);
    let left = h
        .service
        .dismiss_reminders(&[key("standup")])
        .await
        .unwrap();
    assert_eq!(left.len(), 1);
    assert!(fake.dismissed.lock().unwrap().is_empty());
    let pending = h
        .service
        .store()
        .pending_reminder_actions(&account.id)
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].pending,
        Some(crate::reminders::ServerAction::Dismiss)
    );
    // Still dismissed after a sync that couldn't tell the server.
    h.service.sync_account(&account.id).await.unwrap();
    assert_eq!(h.service.list_active_reminders().unwrap().len(), 1);

    fake.offline.store(false, Ordering::SeqCst);
    h.service.sync_account(&account.id).await.unwrap();
    assert_eq!(*fake.dismissed.lock().unwrap(), ["standup"]);
    assert!(
        h.service
            .store()
            .pending_reminder_actions(&account.id)
            .unwrap()
            .is_empty()
    );

    // Snoozed: Microsoft 365 hears of it at once, and it stays snoozed,
    // the same reminder, once the server reports it at its new time.
    let active = h
        .service
        .snooze_reminder(&key("review"), Snooze::Minutes(5))
        .await
        .unwrap();
    let until = now + Duration::minutes(5);
    assert_eq!(
        *fake.snoozed.lock().unwrap(),
        [("review".to_owned(), until)]
    );
    assert_eq!(active[0].snoozed_until, Some(until));
    h.service.sync_account(&account.id).await.unwrap();
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].snoozed_until, Some(until));
    assert_eq!(active[0].key, key("review"));

    // When the snooze is over it goes off again, once.
    let shown = h.sink.shown_reminders().len();
    stopped_clock(&h.service, until);
    let active = h.service.list_active_reminders().unwrap();
    assert!(active[0].is_due());
    assert_eq!(h.sink.shown_reminders()[shown..], ["review"]);
    h.service.list_active_reminders().unwrap();
    assert_eq!(h.sink.shown_reminders().len(), shown + 1);
}

#[tokio::test]
async fn microsoft_365_s_reminders_win_over_the_events() {
    let h = harness();
    let (account, fake) = microsoft_calendar(&h);
    let now = reminder_epoch();
    stopped_clock(&h.service, now);
    let standup = fake.add(&account, "standup", now + Duration::minutes(10), 15);
    fake.add(&account, "review", now + Duration::minutes(10), 15);
    h.service.sync_account(&account.id).await.unwrap();
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 2);
    let standup_key = active
        .iter()
        .find(|r| r.title == "standup")
        .unwrap()
        .key
        .clone();
    assert_eq!(
        active[0].online_meeting_url.as_deref(),
        Some("https://teams.example/join/1")
    );

    // Dismissed in Outlook; the review snoozed on a phone.
    fake.reminders
        .lock()
        .unwrap()
        .retain(|r| r.event_id != "standup");
    fake.reminders.lock().unwrap()[0].fire_at = now + Duration::minutes(20);
    h.service.sync_account(&account.id).await.unwrap();
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].title, "review");
    assert_eq!(active[0].snoozed_until, Some(now + Duration::minutes(20)));
    let cleared = h.sink.cleared_reminders();
    assert!(cleared.contains(&standup_key));
    assert!(cleared.contains(&active[0].key));

    // Without a usable report the events' own times count: the account's
    // report forgotten, the standup reminds again (here, as already shown).
    h.service
        .store()
        .forget_server_reminders(&account.id)
        .unwrap();
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 2);
    assert_eq!(active[0].fire_at, standup.start - Duration::minutes(15));
    assert_eq!(h.sink.shown_reminders().len(), 2);
}

/// Snoozed here, and Microsoft 365 heard of it, but the calendar didn't
/// sync again until the snooze was over (no push, or a long interval).
#[tokio::test]
async fn a_snooze_goes_off_once_when_the_report_catches_up_late() {
    let h = harness();
    let (account, fake) = microsoft_calendar(&h);
    let now = reminder_epoch();
    stopped_clock(&h.service, now);
    fake.add(&account, "review", now + Duration::minutes(5), 15);
    h.service.sync_account(&account.id).await.unwrap();
    let key = h.service.list_active_reminders().unwrap()[0].key.clone();
    h.service
        .snooze_reminder(&key, Snooze::Minutes(5))
        .await
        .unwrap();
    let until = now + Duration::minutes(5);
    assert_eq!(
        *fake.snoozed.lock().unwrap(),
        [("review".to_owned(), until)]
    );

    // The snooze is over: it goes off again.
    stopped_clock(&h.service, until);
    h.service.list_active_reminders().unwrap();
    assert_eq!(h.sink.shown_reminders(), ["review", "review"]);
    let cleared = h.sink.cleared_reminders();

    // A minute later the report says it went off when the snooze ended:
    // the same reminder, which stays as it is.
    stopped_clock(&h.service, until + Duration::minutes(1));
    h.service.sync_account(&account.id).await.unwrap();
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(h.sink.shown_reminders(), ["review", "review"]);
    assert_eq!(h.sink.cleared_reminders(), cleared);
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].key, key);
    assert!(active[0].is_due());
}

/// reminderView fails at the sync that brings a meeting: the meeting's own
/// reminder time counts until a report knows of it, while a reminder the
/// last report found dismissed stays so.
#[tokio::test]
async fn a_failed_reminder_view_silences_no_new_meeting() {
    let h = harness();
    let (account, fake) = microsoft_calendar(&h);
    let now = reminder_epoch();
    stopped_clock(&h.service, now);
    // Dismissed in Outlook before the app saw it go off.
    fake.add(&account, "standup", now + Duration::minutes(10), 15);
    fake.reminders.lock().unwrap().clear();
    h.service.sync_account(&account.id).await.unwrap();
    assert!(h.service.list_active_reminders().unwrap().is_empty());

    // An invitation to a meeting in ten minutes, with a fifteen-minute
    // reminder; Graph doesn't answer reminderView at this sync.
    fake.add(&account, "urgent", now + Duration::minutes(10), 15);
    fake.view_fails.store(true, Ordering::SeqCst);
    h.service.sync_account(&account.id).await.unwrap();
    h.service.list_active_reminders().unwrap();
    // Its reminder went off five minutes ago: it waits a minute at most
    // for the server's word on it, which doesn't come.
    stopped_clock(&h.service, now + Duration::minutes(1));
    let active = h.service.list_active_reminders().unwrap();
    let titles: Vec<&str> = active.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["urgent"]);
    assert_eq!(h.sink.shown_reminders(), ["urgent"]);
}

/// Asleep while a reminder went off and was dismissed on a phone: on
/// waking, the report from before the sleep can't vouch for it, so it
/// waits for a fresh one rather than going off at once.
#[tokio::test]
async fn reminders_dismissed_elsewhere_while_asleep_stay_dismissed() {
    let h = harness();
    let (account, fake) = microsoft_calendar(&h);
    // 09:00 to 09:30, reminding at 08:45; the report is from 08:30.
    stopped_clock(&h.service, reminder_epoch() - Duration::minutes(30));
    fake.add(&account, "standup", reminder_epoch(), 15);
    fake.add(&account, "review", reminder_epoch(), 15);
    h.service.sync_account(&account.id).await.unwrap();
    assert!(h.service.list_active_reminders().unwrap().is_empty());

    // Asleep from 08:35 to 09:05; the standup's reminder was dismissed on
    // the phone meanwhile.
    fake.reminders
        .lock()
        .unwrap()
        .retain(|r| r.event_id != "standup");
    let woke = reminder_epoch() + Duration::minutes(5);
    stopped_clock(&h.service, woke);
    assert!(h.service.list_active_reminders().unwrap().is_empty());
    assert!(h.sink.shown_reminders().is_empty());
    // A fresh report: the review's goes off, the standup's never does.
    h.service.sync_account(&account.id).await.unwrap();
    h.service.list_active_reminders().unwrap();
    assert_eq!(h.sink.shown_reminders(), ["review"]);
    stopped_clock(&h.service, woke + Duration::minutes(2));
    h.service.list_active_reminders().unwrap();
    assert_eq!(h.sink.shown_reminders(), ["review"]);
}

/// Without a fresh report (here reminderView fails), a reminder that came
/// due during the sleep goes off a minute later.
#[tokio::test]
async fn reminders_missed_while_asleep_wait_a_minute_at_most() {
    let h = harness();
    let (account, fake) = microsoft_calendar(&h);
    stopped_clock(&h.service, reminder_epoch() - Duration::minutes(30));
    fake.add(&account, "review", reminder_epoch(), 15);
    h.service.sync_account(&account.id).await.unwrap();
    assert!(h.service.list_active_reminders().unwrap().is_empty());

    let woke = reminder_epoch() + Duration::minutes(5);
    stopped_clock(&h.service, woke);
    fake.view_fails.store(true, Ordering::SeqCst);
    h.service.sync_account(&account.id).await.unwrap();
    h.service.list_active_reminders().unwrap();
    stopped_clock(&h.service, woke + Duration::seconds(30));
    h.service.list_active_reminders().unwrap();
    assert!(h.sink.shown_reminders().is_empty(), "not yet");
    stopped_clock(&h.service, woke + Duration::minutes(1));
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(h.sink.shown_reminders(), ["review"]);
    // Missed while asleep, not found later: the meeting is on.
    assert!(!active[0].missed);
}

/// On waking, the scheduler reads the server's reminders again itself
/// rather than wait for the full sync, which syncs the mail first.
#[tokio::test(start_paused = true)]
async fn on_waking_the_scheduler_asks_the_server_first() {
    let b = background();
    let (account, fake) = microsoft_calendar(&b.h);
    let jump = virtual_clock(&b.service);
    // 09:30 to 10:00, reminding at 09:15.
    fake.add(
        &account,
        "standup",
        reminder_epoch() + Duration::minutes(30),
        15,
    );
    b.service.sync_account(&account.id).await.unwrap();
    b.service.start_background();
    b.settle().await;
    assert!(b.h.sink.shown_reminders().is_empty());

    // Asleep until 09:20; dismissed on the phone meanwhile. Once awake,
    // the network takes a few seconds to come back, and the mail a long
    // time to sync.
    b.h.server.lock().unwrap().inbox_delay = Some(std::time::Duration::from_secs(600));
    fake.reminders.lock().unwrap().clear();
    fake.view_fails.store(true, Ordering::SeqCst);
    let views = fake.views.load(Ordering::SeqCst);
    *jump.lock().unwrap() = Duration::minutes(20);
    b.service.wake(WakeReason::Resume);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert_eq!(fake.views.load(Ordering::SeqCst), views + 1);
    assert!(b.h.sink.shown_reminders().is_empty());
    fake.view_fails.store(false, Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    assert!(fake.views.load(Ordering::SeqCst) > views + 1, "tried again");
    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
    assert!(b.h.sink.shown_reminders().is_empty());
    b.service.shutdown().await;
}

/// Asleep again within the minute that reminders wait for the server's
/// word after a wake: on the next wake they wait afresh, and a dismissal on
/// the phone during either sleep counts.
#[tokio::test(start_paused = true)]
async fn reminders_wait_afresh_after_each_wake() {
    let b = background();
    let (account, fake) = microsoft_calendar(&b.h);
    let jump = virtual_clock(&b.service);
    // 09:30 to 10:00 reminding at 09:15; 10:30 to 11:00 reminding at 10:15.
    fake.add(
        &account,
        "standup",
        reminder_epoch() + Duration::minutes(30),
        15,
    );
    fake.add(
        &account,
        "review",
        reminder_epoch() + Duration::minutes(90),
        15,
    );
    b.service.sync_account(&account.id).await.unwrap();
    b.service.start_background();
    b.settle().await;
    assert!(b.h.sink.shown_reminders().is_empty());

    // Asleep until 09:20. Once awake, the network doesn't come back, and
    // the mail takes long to sync.
    b.h.server.lock().unwrap().inbox_delay = Some(std::time::Duration::from_secs(600));
    fake.view_fails.store(true, Ordering::SeqCst);
    *jump.lock().unwrap() = Duration::minutes(20);
    b.service.wake(WakeReason::Resume);
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    assert!(b.h.sink.shown_reminders().is_empty(), "the standup's waits");

    // Asleep again, for an hour; both reminders are dismissed on the phone
    // meanwhile. Once awake, the network takes a few seconds to come back.
    fake.reminders.lock().unwrap().clear();
    *jump.lock().unwrap() = Duration::minutes(80);
    b.service.wake(WakeReason::Resume);
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert!(b.h.sink.shown_reminders().is_empty(), "both wait afresh");
    fake.view_fails.store(false, Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
    assert!(b.h.sink.shown_reminders().is_empty());
    assert!(b.service.list_active_reminders().unwrap().is_empty());
    b.service.shutdown().await;
}

/// A meeting synced only once it was over, while reminderView fails: it
/// isn't worth a reminder, so nothing waits on the server for it, and the
/// scheduler doesn't keep asking.
#[tokio::test(start_paused = true)]
async fn an_event_found_after_it_was_over_waits_for_nothing() {
    let b = background();
    let (account, fake) = microsoft_calendar(&b.h);
    virtual_clock(&b.service);
    fake.add(
        &account,
        "standup",
        reminder_epoch() + Duration::hours(3),
        15,
    );
    b.service.sync_account(&account.id).await.unwrap();
    b.service.list_active_reminders().unwrap();
    // A call logged in Outlook, 07:30 to 08:00.
    fake.view_fails.store(true, Ordering::SeqCst);
    fake.add(
        &account,
        "logged call",
        reminder_epoch() - Duration::minutes(90),
        15,
    );
    b.service.sync_account(&account.id).await.unwrap();
    // The periodic sync doesn't get as far as the calendar.
    b.h.server.lock().unwrap().inbox_delay = Some(std::time::Duration::from_secs(3600));
    let views = fake.views.load(Ordering::SeqCst);
    b.service.start_background();
    tokio::time::sleep(std::time::Duration::from_secs(600)).await;
    assert_eq!(fake.views.load(Ordering::SeqCst), views);
    assert!(b.h.sink.shown_reminders().is_empty());
    assert!(b.service.list_active_reminders().unwrap().is_empty());
    b.service.shutdown().await;
}

/// A To Do task's reminder left in the panel for a month, then snoozed:
/// it comes back when the snooze is over.
#[tokio::test]
async fn a_task_reminder_snoozed_a_month_after_its_time_comes_back() {
    let h = harness();
    h.service
        .add_imap_account(new_imap_account("me@example.com"))
        .await
        .unwrap();
    let due = reminder_epoch();
    stopped_clock(&h.service, due);
    let list = h.service.list_task_lists().unwrap().remove(0);
    h.service
        .create_task(TaskDraft {
            list_id: list.id.clone(),
            title: "Renew the passport".into(),
            notes: None,
            importance: None,
            due_date: None,
            reminder_at: Some(due),
        })
        .await
        .unwrap();
    let key = h.service.list_active_reminders().unwrap()[0].key.clone();
    assert_eq!(h.sink.shown_reminders(), ["Renew the passport"]);

    let snoozed_at = due + Duration::days(30) - Duration::hours(1);
    stopped_clock(&h.service, snoozed_at);
    h.service
        .snooze_reminder(&key, Snooze::Minutes(24 * 60))
        .await
        .unwrap();
    // Thirty days after its time it's still snoozed, and kept.
    stopped_clock(&h.service, due + Duration::days(30) + Duration::hours(1));
    h.service.prune_reminders();
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 1);
    assert!(!active[0].is_due());

    stopped_clock(&h.service, snoozed_at + Duration::days(1));
    h.service.prune_reminders();
    let active = h.service.list_active_reminders().unwrap();
    assert_eq!(active.len(), 1);
    assert!(active[0].is_due());
    assert_eq!(
        h.sink.shown_reminders(),
        ["Renew the passport", "Renew the passport"]
    );
}

/// The scheduler sleeps up to a minute; a snooze shorter than that still
/// ends on time.
#[tokio::test(start_paused = true)]
async fn a_short_snooze_ends_on_time() {
    let b = background();
    b.account().await;
    virtual_clock(&b.service);
    let calendar = b.service.list_calendars().unwrap().remove(0);
    b.service
        .create_event(local_event(
            &calendar,
            "Standup",
            reminder_epoch() + Duration::minutes(10),
            Some(15),
        ))
        .await
        .unwrap();
    b.service.start_background();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(b.h.sink.shown_reminders(), ["Standup"]);

    let key = b.service.list_active_reminders().unwrap()[0].key.clone();
    let until = b.service.reminders.now() + Duration::seconds(30);
    b.service
        .snooze_reminder(&key, Snooze::Until(until))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(31)).await;
    assert_eq!(b.h.sink.shown_reminders(), ["Standup", "Standup"]);
    b.service.shutdown().await;
}
