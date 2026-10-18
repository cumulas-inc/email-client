//! Protected mail an account can send: the setup checks that show whether
//! its organization's mail server encrypts or labels mail that asks for it,
//! and its sensitivity labels.
//!
//! The app can't see the organization's mail flow rules or auto-labeling
//! policies, so nothing is offered until a check shows it works. A check
//! sends the mailbox's own address a control message without any request
//! and a test message per option with it, all alike but for the request,
//! then looks for them in the Inbox: an option works when its message
//! arrives encrypted (or labeled) and the control doesn't. Once read, the
//! test messages go to Deleted Items. A passed check lasts
//! [`CHECK_VALID_DAYS`].

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use rand::Rng;

use super::permissions::{NeedsPermission, is_listing_refusal};
use super::{CoreService, ProviderSet};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{
    Account, AccountFeature, AccountKind, AccountSettings, CatalogLabel, CheckItem, CheckNote,
    CheckResult, CheckState, DataKind, EmailAddress, EntityKind, FolderRole, LabelCatalog,
    LabelOption, LabelPolicy, OutgoingMessage, ProtectionKind, ProtectionSetup, local_id,
};
use crate::protection::{
    CHECK_TIMEOUT_MINUTES, CHECK_VALID_DAYS, MAX_CHECK_ITEMS, PROTECTED_SENDING_UNSUPPORTED,
    detect, has_reason, is_blank, own_label,
};
use crate::providers::{HeaderRequest, LabelsProvider, MessageContent, MessageLocator};
use crate::signatures::config::is_guid;
use crate::signatures::is_organization_tenant;
use crate::store::{CheckRecord, CheckRun, ControlOutcome, RunItem, StoredCatalog};

/// How long a listing of the labels is used before they're listed again.
const LABELS_MAX_AGE: Duration = Duration::hours(6);
/// The wait after a failed listing, doubled for each failure in a row up
/// to [`LABELS_MAX_AGE`].
const LABELS_RETRY: Duration = Duration::hours(1);
/// Test messages are looked for among mail received from this long before
/// the control was sent: the server's clock may differ from this one.
const LOOKUP_SLACK: Duration = Duration::minutes(2);
/// The text of every test message: the same for the control and each
/// option, naming none, so that no rule on words can tell them apart.
const CHECK_BODY: &str = "<p>Automated setup check from Cumulas Mail. You can delete it.</p>";
/// The letters of a test message's token (base32's).
const TOKEN_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
const TOKEN_LENGTH: usize = 8;

/// A test message's subject; the token finds it in the Inbox.
fn check_subject(token: &str) -> String {
    format!("Cumulas Mail setup check [{token}]")
}

fn new_token() -> String {
    let mut rng = rand::rng();
    (0..TOKEN_LENGTH)
        .map(|_| char::from(TOKEN_ALPHABET[rng.random_range(0..TOKEN_ALPHABET.len())]))
        .collect()
}

/// The organization (tenant) of a Microsoft 365 work or school account:
/// the only kind that can have labels or send protected mail.
pub(super) fn work_tenant(account: &Account) -> Option<&str> {
    match &account.settings {
        AccountSettings::Microsoft(settings) if is_organization_tenant(&settings.tenant_id) => {
            Some(&settings.tenant_id)
        }
        _ => None,
    }
}

/// Whether the account may use its sensitivity labels now.
fn labels_usable(account: &Account) -> bool {
    let feature = AccountFeature::SensitivityLabels;
    !account.needs_permission.contains(&feature) && !account.refused_features.contains(&feature)
}

/// Whether a failed or old listing of the labels is due to be redone.
fn labels_due(stored: &StoredCatalog, now: DateTime<Utc>) -> bool {
    if stored.failures > 0
        && let Some(failed_at) = stored.failed_at
    {
        let doubled = LABELS_RETRY * 2_i32.saturating_pow(stored.failures.min(16) - 1);
        return now - failed_at >= doubled.min(LABELS_MAX_AGE);
    }
    stored
        .listed_at
        .is_none_or(|listed| now - listed >= LABELS_MAX_AGE)
}

/// Whether a check that passed at `at` no longer lets its option be
/// offered.
fn is_expired(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    at.is_some_and(|at| now - at > Duration::days(CHECK_VALID_DAYS))
}

/// A check as the UI shows it: an old pass shows as expired.
fn check_result(record: &CheckRecord, now: DateTime<Utc>) -> CheckResult {
    let expired = record.state == CheckState::Applied && is_expired(record.at, now);
    CheckResult {
        state: if expired {
            CheckState::Expired
        } else {
            record.state
        },
        at: record.at,
        note: record.note,
        test_message: record.test_message_id.is_some(),
    }
}

/// How a passed check stands now: `Ok` when its option may be offered,
/// else the state that keeps it from it (`Expired` for an old pass).
pub(super) fn offered(
    record: &CheckRecord,
    now: DateTime<Utc>,
) -> std::result::Result<(), CheckState> {
    match check_result(record, now).state {
        CheckState::Applied => Ok(()),
        other => Err(other),
    }
}

/// What an item's test message asks for, which the provider makes sure its
/// draft carries.
fn header_request(tenant: &str, item: &CheckItem, catalog: Option<&LabelCatalog>) -> HeaderRequest {
    match item {
        CheckItem::Action { action } => HeaderRequest {
            protect: Some(*action),
            label_id: None,
            tenant_id: tenant.to_owned(),
            label_encrypts: false,
        },
        CheckItem::Label { label_id } => HeaderRequest {
            protect: None,
            label_id: Some(label_id.clone()),
            tenant_id: tenant.to_owned(),
            label_encrypts: catalog
                .and_then(|c| c.label(label_id))
                .is_some_and(|l| l.has_protection),
        },
    }
}

/// What a test message that arrived shows of its item's option.
fn judge(
    item: &CheckItem,
    content: &MessageContent,
    control: &ControlOutcome,
    tenant: &str,
) -> (CheckState, Option<CheckNote>) {
    match item {
        CheckItem::Action { .. } => {
            let protection = arrived_protection(content);
            if protection != Some(ProtectionKind::RightsManaged) {
                (CheckState::NotApplied, None)
            } else if control.protected {
                (CheckState::Unknown, Some(CheckNote::OtherRule))
            } else if content.signals.requested_action.is_some() {
                // The rule documented for it removes the header; one that
                // encrypted the message for another reason doesn't.
                (CheckState::Unknown, Some(CheckNote::HeaderKept))
            } else {
                (CheckState::Applied, None)
            }
        }
        CheckItem::Label { label_id } => {
            let arrived = arrived_label(content, tenant);
            if arrived.as_deref() != Some(label_id.as_str()) {
                (CheckState::NotApplied, None)
            } else if control.label_id.as_deref() == Some(label_id.as_str()) {
                (CheckState::Unknown, Some(CheckNote::OtherRule))
            } else {
                (CheckState::Applied, None)
            }
        }
    }
}

fn arrived_protection(content: &MessageContent) -> Option<ProtectionKind> {
    let blank = is_blank(content.body_html.as_deref(), content.body_text.as_deref());
    detect(
        &content.signals,
        &content.attachments,
        content.body_html.as_deref(),
        blank,
        false,
    )
    .map(|p| p.kind)
}

fn arrived_label(content: &MessageContent, tenant: &str) -> Option<String> {
    content
        .signals
        .msip_labels
        .as_deref()
        .and_then(|raw| own_label(raw, tenant))
        .map(|l| l.id)
}

impl CoreService {
    /// The lock a check and its refreshes hold for an account, so that a
    /// sync's refresh and the UI's poll never interleave.
    fn protection_lock(&self, account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.protection_locks.lock().unwrap();
        Arc::clone(locks.entry(account_id.to_owned()).or_default())
    }

    /// The account's labels, while it may use them; `None` before they were
    /// first listed.
    pub(super) fn label_list(&self, account: &Account) -> Result<Option<LabelCatalog>> {
        if work_tenant(account).is_none() || !labels_usable(account) {
            return Ok(None);
        }
        Ok(self.store.label_catalog(&account.id)?.catalog)
    }

    /// What the account can send protected, from this device. Errors:
    /// `NotFound`.
    pub fn protection_setup(&self, account_id: &str) -> Result<ProtectionSetup> {
        let account = self.store.get_account(account_id)?;
        let mut setup = ProtectionSetup {
            account_id: account.id.clone(),
            supported: false,
            encrypt: CheckResult::default(),
            do_not_forward: CheckResult::default(),
            labels: Vec::new(),
            policy: LabelPolicy::default(),
            labels_listed_at: None,
            labels_error: None,
            checking: false,
        };
        if work_tenant(&account).is_none() {
            return Ok(setup);
        }
        let now = Utc::now();
        let checks = self.store.protection_checks(account_id)?;
        setup.supported = true;
        setup.encrypt = check_result(&checks.encrypt, now);
        setup.do_not_forward = check_result(&checks.do_not_forward, now);
        setup.checking = checks.run.is_some();
        if labels_usable(&account) {
            let stored = self.store.label_catalog(account_id)?;
            setup.labels_listed_at = stored.listed_at;
            setup.labels_error = stored.error;
            if let Some(catalog) = stored.catalog {
                setup.policy = catalog.policy;
                setup.labels = catalog
                    .labels
                    .into_iter()
                    .map(|label| LabelOption {
                        check: check_result(
                            &checks.labels.get(&label.id).cloned().unwrap_or_default(),
                            now,
                        ),
                        label,
                    })
                    .collect();
            }
        }
        Ok(setup)
    }

    /// Lists the account's sensitivity labels from Microsoft 365 now,
    /// whatever the backoff. Errors: `Unsupported` (IMAP and personal
    /// accounts), `PermissionRequired` (not granted, or refused), and a
    /// listing's failure, which is also kept as `labels_error`.
    pub async fn refresh_sensitivity_labels(&self, account_id: &str) -> Result<ProtectionSetup> {
        let account = self.store.get_account(account_id)?;
        if work_tenant(&account).is_none() {
            return Err(Error::unsupported(
                "Only Microsoft 365 work and school accounts have sensitivity labels.",
            ));
        }
        let providers = self.providers_for(account_id).await?;
        let Some(labels) = &providers.labels else {
            return Err(Error::unsupported(
                "Only Microsoft 365 work and school accounts have sensitivity labels.",
            ));
        };
        self.list_labels(&account, labels.as_ref()).await?;
        self.protection_setup(account_id)
    }

    /// Lists the labels when they're due (see [`labels_due`]), or when the
    /// feature is tried again after a refusal (`retrying`). A sync's part:
    /// its failures never fail the sync.
    pub(super) async fn sync_labels(
        &self,
        account: &Account,
        providers: &ProviderSet,
        retrying: &[AccountFeature],
    ) -> Result<()> {
        let Some(labels) = &providers.labels else {
            return Ok(());
        };
        let feature = AccountFeature::SensitivityLabels;
        let retried = retrying.contains(&feature);
        // Without the permission, or refused: nothing is kept of them.
        if !labels_usable(account) && !retried {
            return self.drop_feature_data(account, feature);
        }
        let stored = self.store.label_catalog(&account.id)?;
        if !retried && !labels_due(&stored, Utc::now()) {
            return Ok(());
        }
        match self.list_labels(account, labels.as_ref()).await {
            // Not usable now: handled, not a failure of the sync's.
            Err(e) if e.code == ErrorCode::PermissionRequired => Ok(()),
            other => other,
        }
    }

    /// Lists the labels and keeps them. Without the permission what's kept
    /// goes; a refusal (HTTP 403) marks the feature refused; another
    /// failure keeps the last list with the error, and backs off.
    async fn list_labels(&self, account: &Account, provider: &dyn LabelsProvider) -> Result<()> {
        let feature = AccountFeature::SensitivityLabels;
        let id = account.id.as_str();
        match provider.label_catalog().await {
            Ok(catalog) => {
                if self.store.save_label_catalog(id, &catalog, Utc::now())? {
                    self.notify(&[DataKind::Protection], Some(id));
                }
                if account.refused_features.contains(&feature)
                    && self.store.lift_refusal(id, feature)?
                {
                    self.notify(&[DataKind::Accounts], Some(id));
                }
                Ok(())
            }
            Err(e) if e.code == ErrorCode::PermissionRequired => {
                self.drop_feature_data(account, feature)?;
                Err(e)
            }
            Err(e) if is_listing_refusal(&e) => {
                self.refuse_feature(account, feature)?;
                Err(NeedsPermission {
                    feature,
                    account_id: account.id.clone(),
                    email: account.email.clone(),
                    refused: true,
                }
                .error())
            }
            Err(e) => {
                if self
                    .store
                    .label_catalog_failed(id, &e.message, Utc::now())?
                {
                    self.notify(&[DataKind::Protection], Some(id));
                }
                Err(e)
            }
        }
    }

    /// Sends the control and a test message for each item to the
    /// mailbox's own address, marking each waiting as it goes. Errors:
    /// `Unsupported` (not a work or school account), `InvalidInput` (none,
    /// more than [`MAX_CHECK_ITEMS`], a label not listed, a check already
    /// waiting), and the first send's error: the items not sent then keep
    /// what they had, and the run waits for those sent.
    pub async fn check_protection(
        &self,
        account_id: &str,
        items: &[CheckItem],
    ) -> Result<ProtectionSetup> {
        let lock = self.protection_lock(account_id);
        let _guard = lock.lock().await;
        let account = self.store.get_account(account_id)?;
        let Some(tenant) = work_tenant(&account).map(str::to_owned) else {
            return Err(Error::unsupported(PROTECTED_SENDING_UNSUPPORTED));
        };
        let catalog = self.label_list(&account)?;
        let mut wanted: Vec<CheckItem> = Vec::with_capacity(items.len());
        for item in items {
            let item = match item {
                CheckItem::Label { label_id } => {
                    let listed = catalog.as_ref().and_then(|c| c.label(label_id.trim()));
                    let Some(label) = listed else {
                        return Err(Error::invalid_input(format!(
                            "That sensitivity label isn't available for {}.",
                            account.email
                        )));
                    };
                    CheckItem::Label {
                        label_id: label.id.clone(),
                    }
                }
                action => action.clone(),
            };
            if !wanted.contains(&item) {
                wanted.push(item);
            }
        }
        if wanted.is_empty() {
            return Err(Error::invalid_input("Choose what to check."));
        }
        if wanted.len() > MAX_CHECK_ITEMS {
            return Err(Error::invalid_input(format!(
                "Cumulas Mail checks at most {MAX_CHECK_ITEMS} at a time."
            )));
        }
        if self.store.protection_checks(account_id)?.run.is_some() {
            return Err(Error::invalid_input(
                "Wait for the check that's running to finish.",
            ));
        }

        let providers = self.providers_for(account_id).await?;
        let own = match providers.mail.own_address().await {
            Ok(Some(address)) => address,
            Ok(None) => EmailAddress::new(None, &account.email),
            Err(e) => {
                tracing::info!(
                    "reading the mailbox's own address failed, using the account's: {e}"
                );
                EmailAddress::new(None, &account.email)
            }
        };
        let test_message = |token: &str| OutgoingMessage {
            account_id: account.id.clone(),
            to: vec![own.clone()],
            cc: vec![],
            bcc: vec![],
            subject: check_subject(token),
            body_html: CHECK_BODY.to_owned(),
            attachments: vec![],
            importance: None,
            reply_to_message_id: None,
            reply_kind: None,
            protect: None,
            label_id: None,
        };

        // The control: sent like any mail, without any request.
        let control_token = new_token();
        providers
            .mail
            .send(&test_message(&control_token), None)
            .await?;
        let started = self.store.update_protection_checks(account_id, |checks| {
            checks.run = Some(CheckRun {
                control_token: control_token.clone(),
                control_sent_at: Utc::now(),
                control: None,
                items: Vec::new(),
            });
        })?;
        if started.is_none() {
            return Err(Error::not_found("account"));
        }

        let mut failure = None;
        for item in &wanted {
            let token = new_token();
            let previous = self.store.update_protection_checks(account_id, |checks| {
                let previous = checks.record(item);
                let now = Utc::now();
                *checks.record_mut(item) = CheckRecord {
                    state: CheckState::Waiting,
                    at: Some(now),
                    note: None,
                    test_message_id: None,
                };
                if let Some(run) = &mut checks.run {
                    run.items.push(RunItem {
                        item: item.clone(),
                        token: token.clone(),
                        sent_at: now,
                    });
                }
                previous
            })?;
            let Some(previous) = previous else {
                break;
            };
            let request = header_request(&tenant, item, catalog.as_ref());
            // These go out whatever the setup says: the point is to test it.
            let sent = providers
                .mail
                .send_requesting(&test_message(&token), None, &request)
                .await;
            let Err(e) = sent else {
                continue;
            };
            let not_kept = has_reason(&e, "headersNotKept");
            self.store.update_protection_checks(account_id, |checks| {
                if let Some(run) = &mut checks.run {
                    run.items.retain(|i| i.token != token);
                }
                *checks.record_mut(item) = if not_kept {
                    CheckRecord {
                        state: CheckState::NotApplied,
                        at: Some(Utc::now()),
                        note: Some(CheckNote::HeaderNotKept),
                        test_message_id: None,
                    }
                } else {
                    // Nothing was learned of it.
                    previous
                };
            })?;
            if !not_kept {
                failure = Some(e);
                break;
            }
        }
        self.store.update_protection_checks(account_id, |checks| {
            if checks.run.as_ref().is_some_and(|run| run.items.is_empty()) {
                checks.run = None;
            }
        })?;
        self.notify(&[DataKind::Protection, DataKind::Mail], Some(account_id));
        match failure {
            Some(e) => Err(e),
            None => self.protection_setup(account_id),
        }
    }

    /// Looks for the waiting check's messages and records what became of
    /// them; the ones read go to Deleted Items.
    pub async fn refresh_protection_check(&self, account_id: &str) -> Result<ProtectionSetup> {
        let lock = self.protection_lock(account_id);
        let _guard = lock.lock().await;
        let account = self.store.get_account(account_id)?;
        let Some(run) = self.store.protection_checks(account_id)?.run else {
            return self.protection_setup(account_id);
        };
        let Some(tenant) = work_tenant(&account).map(str::to_owned) else {
            return self.protection_setup(account_id);
        };
        let providers = self.providers_for(account_id).await?;
        let mut tokens: Vec<String> = run.items.iter().map(|i| i.token.clone()).collect();
        if run.control.is_none() {
            tokens.push(run.control_token.clone());
        }
        let found = match providers
            .mail
            .find_in_inbox(&tokens, run.control_sent_at - LOOKUP_SLACK)
            .await
        {
            Ok(found) => found,
            Err(e) => {
                tracing::info!("looking for the setup check's messages failed: {e}");
                HashMap::new()
            }
        };

        let mut changed = false;
        let mut moved = false;
        let mut control = run.control.clone();
        if control.is_none()
            && let Some(locator) = found.get(&run.control_token)
            && let Some(content) = self.fetch_arrived(&providers, locator).await
        {
            control = Some(ControlOutcome {
                protected: arrived_protection(&content) == Some(ProtectionKind::RightsManaged),
                label_id: arrived_label(&content, &tenant),
            });
            changed = true;
            moved |= self
                .discard_test_message(&account, &providers, locator)
                .await
                .is_some();
        }
        let mut decided: Vec<(String, CheckRecord)> = Vec::new();
        if let Some(control) = &control {
            for waiting in &run.items {
                let Some(locator) = found.get(&waiting.token) else {
                    continue;
                };
                let Some(content) = self.fetch_arrived(&providers, locator).await else {
                    continue;
                };
                let (state, note) = judge(&waiting.item, &content, control, &tenant);
                let new_id = self
                    .discard_test_message(&account, &providers, locator)
                    .await;
                moved |= new_id.is_some();
                decided.push((
                    waiting.token.clone(),
                    CheckRecord {
                        state,
                        at: Some(Utc::now()),
                        note,
                        test_message_id: Some(
                            new_id
                                .flatten()
                                .unwrap_or_else(|| locator.remote_id.clone()),
                        ),
                    },
                ));
            }
        }

        let now = Utc::now();
        let timeout = Duration::minutes(CHECK_TIMEOUT_MINUTES);
        let control_late = control.is_none() && now - run.control_sent_at > timeout;
        let not_arrived = CheckRecord {
            state: CheckState::Unknown,
            at: Some(now),
            note: Some(CheckNote::NotArrived),
            test_message_id: None,
        };
        let updated = self.store.update_protection_checks(account_id, |checks| {
            // Only the run this looked for: another started meanwhile
            // can't be, under the lock, but its tokens decide.
            let Some(current) = checks.run.as_mut() else {
                return false;
            };
            if current.control_token != run.control_token {
                return false;
            }
            let mut changed = changed;
            current.control = control.clone();
            let mut records = Vec::new();
            current.items.retain(|waiting| {
                let decided = decided.iter().find(|(token, _)| *token == waiting.token);
                let record = match decided {
                    Some((_, record)) => record.clone(),
                    None if control_late || now - waiting.sent_at > timeout => not_arrived.clone(),
                    None => return true,
                };
                records.push((waiting.item.clone(), record));
                false
            });
            changed |= !records.is_empty();
            for (item, record) in records {
                *checks.record_mut(&item) = record;
            }
            if checks.run.as_ref().is_some_and(|run| run.items.is_empty()) {
                checks.run = None;
                changed = true;
            }
            changed
        })?;
        if updated == Some(true) {
            self.notify(&[DataKind::Protection], Some(account_id));
        }
        if moved {
            self.notify(&[DataKind::Mail], Some(account_id));
        }
        self.protection_setup(account_id)
    }

    /// A test message as it arrived; `None` when it can't be judged yet:
    /// not fetched, or fetched without its named properties, whose absence
    /// could make a protected arrival look ordinary.
    async fn fetch_arrived(
        &self,
        providers: &ProviderSet,
        locator: &MessageLocator,
    ) -> Option<MessageContent> {
        match providers.mail.fetch_message(locator).await {
            Ok(content) if content.signals.properties_unavailable => {
                tracing::debug!("a setup check's message came without its properties");
                None
            }
            Ok(content) => Some(content),
            Err(e) => {
                tracing::info!("reading a setup check's message failed: {e}");
                None
            }
        }
    }

    /// Moves a test message that was read to Deleted Items, here too when
    /// it's stored. `Some` with its new remote id (when the server said)
    /// once moved; `None` when it couldn't be.
    async fn discard_test_message(
        &self,
        account: &Account,
        providers: &ProviderSet,
        locator: &MessageLocator,
    ) -> Option<Option<String>> {
        let trash = match self
            .store
            .find_folder_by_role(&account.id, FolderRole::Trash)
        {
            Ok(Some(trash)) => trash,
            Ok(None) => {
                tracing::debug!("no Deleted Items folder to move a setup check's message to");
                return None;
            }
            Err(e) => {
                tracing::warn!("finding Deleted Items failed: {e}");
                return None;
            }
        };
        let moved = providers
            .mail
            .move_messages(std::slice::from_ref(locator), &trash.remote_id)
            .await;
        let new_id = match moved {
            Ok(ids) => ids.into_iter().next().flatten(),
            Err(e) => {
                tracing::info!("moving a setup check's message to Deleted Items failed: {e}");
                return None;
            }
        };
        // Stored already (synced into the Inbox): moved here too, and its
        // new mail notification withdrawn.
        let id = local_id(&account.id, EntityKind::Message, &locator.remote_id);
        if self.store.get_message_summary(&id).is_ok() {
            if let Err(e) = self.store.relocate_message(&id, &trash, new_id.as_deref()) {
                tracing::warn!("moving a stored setup check message failed: {e}");
            }
            self.sink.mail_settled(&[id]);
        }
        Some(new_id)
    }

    /// The Outlook on the web link of the item's arrived test message.
    /// Errors: `NotFound` when none arrived, `Provider` when Microsoft 365
    /// gives no link.
    pub async fn check_message_web_link(
        &self,
        account_id: &str,
        item: &CheckItem,
    ) -> Result<String> {
        let account = self.store.get_account(account_id)?;
        if work_tenant(&account).is_none() {
            return Err(Error::unsupported(PROTECTED_SENDING_UNSUPPORTED));
        }
        let item = match item {
            CheckItem::Label { label_id } => CheckItem::Label {
                label_id: label_id.trim().to_ascii_lowercase(),
            },
            action => action.clone(),
        };
        let Some(remote_id) = self
            .store
            .protection_checks(account_id)?
            .record(&item)
            .test_message_id
        else {
            return Err(Error::new(
                ErrorCode::NotFound,
                "There's no test message to open. Check again.",
            ));
        };
        let folder_remote_id = self
            .store
            .find_folder_by_role(account_id, FolderRole::Trash)?
            .map_or_else(|| "deleteditems".to_owned(), |f| f.remote_id);
        let link = self
            .providers_for(account_id)
            .await?
            .mail
            .web_link(&MessageLocator {
                folder_remote_id,
                remote_id,
            })
            .await;
        match link {
            Ok(Some(link)) => Ok(link),
            Ok(None) => Err(Error::provider(
                "Microsoft 365 didn't give a link to this message.",
            )),
            // Deleted since, from Deleted Items too.
            Err(e) if e.code == ErrorCode::NotFound => Err(Error::new(
                ErrorCode::NotFound,
                "There's no test message to open. Check again.",
            )),
            Err(e) => Err(e),
        }
    }

    /// Runs, with a sync of a work or school account, what keeps its
    /// protection setup current: the labels when due, and the waiting
    /// check. Failures are logged, never the sync's.
    pub(super) async fn sync_protection(
        &self,
        account: &Account,
        providers: &ProviderSet,
        retrying: &[AccountFeature],
    ) {
        if account.kind != AccountKind::Microsoft || work_tenant(account).is_none() {
            return;
        }
        if let Err(e) = self.sync_labels(account, providers, retrying).await {
            tracing::info!(
                "listing the sensitivity labels of account {} failed: {e}",
                account.id
            );
        }
        let waiting = self
            .store
            .protection_checks(&account.id)
            .is_ok_and(|checks| checks.run.is_some());
        if waiting && let Err(e) = self.refresh_protection_check(&account.id).await {
            tracing::info!(
                "refreshing the setup check of account {} failed: {e}",
                account.id
            );
        }
    }
}

/// The title of a listed label, for messages.
pub(super) fn label_title(catalog: Option<&LabelCatalog>, id: &str) -> Option<String> {
    catalog.and_then(|c| c.label(id)).map(CatalogLabel::title)
}

/// Whether `id` names a listed label: a GUID (any case) in `catalog`.
pub(super) fn listed_label<'c>(
    catalog: Option<&'c LabelCatalog>,
    id: &str,
) -> Option<&'c CatalogLabel> {
    let id = id.trim();
    if !is_guid(id) {
        return None;
    }
    catalog.and_then(|c| c.label(id))
}

#[cfg(test)]
mod tests;
