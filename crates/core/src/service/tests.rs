//! Service tests against an in-memory fake server.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, NaiveDate, Utc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::auth::microsoft::SignInResult;
use crate::error::ErrorCode;
use crate::model::{
    AccountSettings, AttachmentMeta, EmailAddress, EventDraft, Folder, FolderRole,
    ImapAccountUpdate, ImapSettings, Importance, Message, MessageQuery, MessageSummary,
    OutgoingMessage, ShowAs, TaskDraft, TaskPatch, TaskStatus,
};
use crate::providers::{
    FolderCounts, MailProvider, MessageBatch, MessageContent, MessageLocator, SyncWindow,
};
use crate::push::{EndReason, MailWatcher, PushEvent, PushHealth, PushMode, WakeReason, WatchSink};
use crate::secrets::MemorySecretStore;

/// A tiny IMAP-like server: folders by remote id, messages by remote id.
#[derive(Default)]
pub(crate) struct FakeServer {
    pub folders: Vec<(String, FolderRole)>,
    pub messages: HashMap<String, (String, MessageSummary)>,
    pub sent: Vec<OutgoingMessage>,
    pub fail_sync: bool,
    /// How often folder counts were asked for.
    pub count_requests: usize,
    next_uid: u32,
}

pub(crate) struct FakeMail {
    account_id: String,
    pub server: Arc<Mutex<FakeServer>>,
}

impl FakeMail {
    fn folder_id(&self, remote: &str) -> String {
        local_id(&self.account_id, EntityKind::Folder, remote)
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
        let server = self.server.lock().unwrap();
        if server.fail_sync {
            return Err(Error::new(ErrorCode::AuthRequired, "token expired"));
        }
        Ok(MessageBatch {
            upserts: server
                .messages
                .values()
                .filter(|(f, _)| *f == folder.remote_id)
                .map(|(_, m)| m.clone())
                .collect(),
            full_resync: true,
            sync_state: Some("state".into()),
            ..Default::default()
        })
    }

    async fn folder_counts(&self, folder: &Folder) -> Result<Option<FolderCounts>> {
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
        assert!(
            self.server
                .lock()
                .unwrap()
                .messages
                .contains_key(&m.remote_id)
        );
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

    async fn send(&self, message: &OutgoingMessage, original: Option<&Message>) -> Result<()> {
        if message.reply_to_message_id.is_some() {
            assert!(original.is_some());
        }
        self.server.lock().unwrap().sent.push(message.clone());
        Ok(())
    }
}

pub(crate) struct FakeConnector {
    pub server: Arc<Mutex<FakeServer>>,
    pub reject_login: std::sync::atomic::AtomicBool,
    /// Passwords passed to `verify_imap`, in order.
    pub verified_passwords: Mutex<Vec<String>>,
    /// Push for every account; polling when unset.
    pub watcher: Mutex<Option<Arc<dyn MailWatcher>>>,
}

impl FakeConnector {
    pub fn new(server: Arc<Mutex<FakeServer>>, reject_login: bool) -> Self {
        Self {
            server,
            reject_login: reject_login.into(),
            verified_passwords: Mutex::default(),
            watcher: Mutex::default(),
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
            calendar: None,
            contacts: None,
            tasks: None,
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

    async fn microsoft_sign_in(&self, _: &BrowserOpener, _: Option<&str>) -> Result<SignInResult> {
        // Like a user who never finishes signing in in the browser.
        std::future::pending().await
    }

    async fn push_source(&self, account: &Account) -> Result<PushSource> {
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

#[derive(Default)]
struct RecordingSink {
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
}

/// A watcher whose changes the test sends; counts its connections.
#[derive(Default)]
struct ScriptedWatcher {
    events: Mutex<Option<mpsc::UnboundedReceiver<PushEvent>>>,
    sender: Mutex<Option<mpsc::UnboundedSender<PushEvent>>>,
    connections: AtomicUsize,
    cancelled: AtomicUsize,
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
    let id = h.service.search_messages("welcome", None).unwrap()[0]
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
    let original = h.service.search_messages("invoice", None).unwrap()[0]
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
async fn targeted_syncs_wait_for_the_account_lock() {
    let h = harness();
    let account = account_with_mail(&h).await;
    add_message(&h, &account, "INBOX", "INBOX:3", "Fresh");
    let lock = h.service.account_lock(&account.id);
    let guard = lock.lock().await;
    // The periodic sync skips an account that is busy...
    h.service.sync_account(&account.id).await.unwrap();
    assert!(h.sink.new_mail.lock().unwrap().is_empty());
    // ...but a targeted sync waits for it.
    let release = async {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(h.sink.new_mail.lock().unwrap().is_empty(), "still waiting");
        drop(guard);
    };
    let (synced, ()) = tokio::join!(
        h.service.sync_targets(&account.id, SyncScope::inbox()),
        release
    );
    synced.unwrap();
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

    let started = tokio::time::Instant::now();
    b.service.shutdown().await;
    assert!(started.elapsed() <= std::time::Duration::from_secs(2));
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
