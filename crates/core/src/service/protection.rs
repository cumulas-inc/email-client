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
    LabelOption, LabelPolicy, OrganizationSignatureState, OutgoingMessage, ProtectAction,
    ProtectionKind, ProtectionSetup, local_id,
};
use crate::protection::{
    CHECK_SUBJECT_PREFIX, CHECK_TIMEOUT_MINUTES, CHECK_VALID_DAYS, CheckDetailsInput,
    CheckDetailsItem, CheckEvidence, MAX_CHECK_ITEMS, POLICIES_SETTLE,
    PROTECTED_SENDING_UNSUPPORTED, RULES_SETTLE, Settle, check_details, check_evidence,
    first_own_label, has_reason, relay_not_internal,
};
use crate::providers::{HeaderRequest, LabelsProvider, MessageContent, MessageLocator};
use crate::signatures::config::is_guid;
use crate::signatures::is_organization_tenant;
use crate::store::{
    CheckRecord, CheckRun, ControlOutcome, LastControl, RunItem, StoredCatalog, StoredChecks,
};

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
    format!("{CHECK_SUBJECT_PREFIX} [{token}]")
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

/// What decides whether the Cumulas Signatures relay made a pass stale:
/// whether the organization says the account's mail goes through it now,
/// and the last check's control message.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Relay<'c> {
    pub now: bool,
    pub control: Option<&'c LastControl>,
}

impl<'c> Relay<'c> {
    pub(super) fn of(now: bool, checks: &'c StoredChecks) -> Self {
        Self {
            now,
            control: checks.last_control.as_ref(),
        }
    }
}

/// Whether a pass was made stale by the Cumulas Signatures relay: the
/// account's mail goes through it now, and the pass's test message didn't.
/// Mail that goes through the relay may come back as outside mail, which
/// Cumulas Mail's rules don't act on, so the pass says nothing of it.
///
/// The organization says so as soon as it chooses the relay, while its
/// mail may not go through it for a while yet (routing waits for an
/// administrator or a directory sync). So a pass checked once it said so
/// drifts only when a later check's control message did go through the
/// relay; a pass from before drifts at once. Passes from before 0.5.0 kept
/// no evidence and never drift; a relay that went away leaves the rules
/// acting on the first pass, which works.
pub(super) fn relay_drift(record: &CheckRecord, relay: Relay<'_>) -> bool {
    let routed_since = || {
        relay.control.is_some_and(|control| {
            record
                .sent_at
                .or(record.at)
                .is_some_and(|sent| control.sent_at > sent)
                && control
                    .outcome
                    .evidence
                    .as_ref()
                    .is_some_and(|e| e.relay_stamped)
        })
    };
    record.state == CheckState::Applied
        && relay.now
        && record.evidence.as_ref().is_some_and(|e| !e.relay_stamped)
        && (!record.relay_expected || routed_since())
}

/// What a check decided, as of `now`: an old pass is expired, and so is one
/// the relay made stale (see [`relay_drift`]).
fn decided_state(record: &CheckRecord, now: DateTime<Utc>, relay: Relay<'_>) -> CheckState {
    if record.state == CheckState::Applied
        && (is_expired(record.at, now) || relay_drift(record, relay))
    {
        CheckState::Expired
    } else {
        record.state
    }
}

/// A check as the UI shows it: an old pass shows as expired, and one the
/// relay made stale too, with the note `RelayChanged`; an item whose test
/// message is on its way (`waiting`) shows as waiting, while the pass
/// before it, if any, still counts (`passed_at`).
fn check_result(
    record: &CheckRecord,
    waiting: Option<&RunItem>,
    now: DateTime<Utc>,
    relay: Relay<'_>,
) -> CheckResult {
    let passed_at = offered(record, now, relay).ok().and(record.at);
    match waiting {
        Some(waiting) => CheckResult {
            state: CheckState::Waiting,
            at: Some(waiting.sent_at),
            passed_at,
            ..CheckResult::default()
        },
        None => CheckResult {
            state: decided_state(record, now, relay),
            at: record.at,
            note: if relay_drift(record, relay) {
                Some(CheckNote::RelayChanged)
            } else {
                record.note
            },
            test_message: record.test_message_id.is_some(),
            passed_at,
            other_label_id: record.other_label_id.clone(),
            other_label_on_control: record.other_label_on_control,
            changed_at: record.changed_at,
        },
    }
}

/// How the last decided check stands now: `Ok` when its option may be
/// offered, else the state that keeps it from it (`Expired` for an old
/// pass, or one the relay made stale). A check on its way changes nothing
/// until it's decided.
pub(super) fn offered(
    record: &CheckRecord,
    now: DateTime<Utc>,
    relay: Relay<'_>,
) -> std::result::Result<(), CheckState> {
    match decided_state(record, now, relay) {
        CheckState::Applied => Ok(()),
        other => Err(other),
    }
}

/// The waiting check's test message for `item`, if one is on its way.
pub(super) fn waiting_for<'r>(checks: &'r StoredChecks, item: &CheckItem) -> Option<&'r RunItem> {
    checks
        .run
        .as_ref()
        .and_then(|run| run.items.iter().find(|waiting| waiting.item == *item))
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

/// What a test message that arrived showed of its item's option, with what
/// the notes need.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Judged {
    state: CheckState,
    note: Option<CheckNote>,
    /// With `OtherLabel`: the organization's label that arrived instead.
    other_label_id: Option<String>,
    /// With `OtherLabel`: the control arrived with that label too.
    other_label_on_control: bool,
    /// The setup change recent enough to matter (see [`recent_change`]),
    /// whatever was decided, so that the note can add the wait.
    changed_at: Option<DateTime<Utc>>,
}

/// The change this computer made to the organization's setup recent enough
/// to explain a test message sent at `sent_at` arriving without its option:
/// mail flow rules can take an hour to start working, auto-labeling
/// policies a day. A change after the message was sent (and before it was
/// judged, `now`) counts too: it may have arrived in between.
fn recent_change(
    item: &CheckItem,
    settle: &Settle,
    sent_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let (at, window) = match item {
        CheckItem::Action { .. } => (settle.rules_changed_at, RULES_SETTLE),
        CheckItem::Label { .. } => (settle.policies_changed_at, POLICIES_SETTLE),
    };
    at.filter(|at| *at <= now && sent_at - *at < window)
}

/// What a test message that arrived shows of its item's option, from what
/// it showed (`evidence`) against the control's. First match wins: what
/// arrived decides; then, for what arrived without it, the likeliest
/// reason: the relay's return as outside mail (whether rules scoped by
/// address act on it is unverified, so it's the stronger explanation), a
/// setup change too recent, then whether the request was still on it.
fn judge(
    item: &CheckItem,
    evidence: &CheckEvidence,
    control: &ControlOutcome,
    settle: &Settle,
    sent_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Judged {
    let changed_at = recent_change(item, settle, sent_at, now);
    let decided = |state: CheckState, note: Option<CheckNote>| Judged {
        state,
        note,
        other_label_id: None,
        other_label_on_control: false,
        changed_at,
    };
    let without_it = || {
        let relay = relay_not_internal(evidence)
            || control.evidence.as_ref().is_some_and(relay_not_internal);
        let note = if relay {
            CheckNote::RelayNotInternal
        } else if changed_at.is_some() {
            CheckNote::TooSoon
        } else if evidence.request_arrived {
            CheckNote::RequestIgnored
        } else {
            CheckNote::RequestRemoved
        };
        decided(CheckState::NotApplied, Some(note))
    };
    match item {
        CheckItem::Action { .. } => match evidence.detected {
            Some(ProtectionKind::RightsManaged) if control.protected => {
                decided(CheckState::Unknown, Some(CheckNote::OtherRule))
            }
            // The rule documented for it removes the header; one that
            // encrypted the message for another reason doesn't.
            Some(ProtectionKind::RightsManaged) if evidence.request_arrived => {
                decided(CheckState::Unknown, Some(CheckNote::HeaderKept))
            }
            Some(ProtectionKind::RightsManaged) => decided(CheckState::Applied, None),
            Some(_) => decided(CheckState::Unknown, Some(CheckNote::OtherProtection)),
            None => without_it(),
        },
        CheckItem::Label { label_id } => {
            let on_control = |id: &str| {
                control
                    .label_id
                    .as_deref()
                    .is_some_and(|c| c.eq_ignore_ascii_case(id))
            };
            match first_own_label(evidence) {
                Some(own) if own.eq_ignore_ascii_case(label_id) && on_control(own) => {
                    decided(CheckState::Unknown, Some(CheckNote::OtherRule))
                }
                Some(own) if own.eq_ignore_ascii_case(label_id) => {
                    decided(CheckState::Applied, None)
                }
                Some(own) => Judged {
                    other_label_id: Some(own.to_owned()),
                    other_label_on_control: on_control(own),
                    ..decided(CheckState::NotApplied, Some(CheckNote::OtherLabel))
                },
                // Purview's encryption hides the label from the headers
                // Cumulas Mail can read.
                None if evidence.detected == Some(ProtectionKind::RightsManaged) => {
                    decided(CheckState::Unknown, Some(CheckNote::LabelUnreadable))
                }
                None => without_it(),
            }
        }
    }
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
        let relay = Relay::of(self.relay_now(account_id), &checks);
        setup.supported = true;
        let result = |item: &CheckItem| {
            check_result(&checks.record(item), waiting_for(&checks, item), now, relay)
        };
        setup.encrypt = result(&CheckItem::Action {
            action: ProtectAction::Encrypt,
        });
        setup.do_not_forward = result(&CheckItem::Action {
            action: ProtectAction::DoNotForward,
        });
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
                        check: result(&CheckItem::Label {
                            label_id: label.id.clone(),
                        }),
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
    /// mailbox's own address, adding each to the run as it goes; what each
    /// item's last check decided stands until its test message is judged.
    /// Errors: `Unsupported` (not a work or school account), `InvalidInput`
    /// (none, more than [`MAX_CHECK_ITEMS`], a label not listed, a check
    /// already waiting), and the first send's error: the items not sent
    /// then keep what they had, and the run waits for those sent.
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
            let added = self.store.update_protection_checks(account_id, |checks| {
                if let Some(run) = &mut checks.run {
                    run.items.push(RunItem {
                        item: item.clone(),
                        token: token.clone(),
                        sent_at: Utc::now(),
                    });
                }
            })?;
            if added.is_none() {
                break;
            }
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
                // Otherwise nothing was learned of it: it keeps its record.
                if not_kept {
                    *checks.record_mut(item) = CheckRecord {
                        state: CheckState::NotApplied,
                        at: Some(Utc::now()),
                        note: Some(CheckNote::HeaderNotKept),
                        ..CheckRecord::default()
                    };
                }
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
        // Judged now: kept as the last control too.
        let mut control_judged = false;
        if control.is_none()
            && let Some(locator) = found.get(&run.control_token)
            && let Some((content, transport)) = self.fetch_arrived(&providers, locator).await
        {
            let evidence = check_evidence(
                &content,
                transport.as_deref(),
                &tenant,
                run.control_sent_at,
                Utc::now(),
            );
            control = Some(ControlOutcome {
                protected: evidence.detected == Some(ProtectionKind::RightsManaged),
                label_id: first_own_label(&evidence).map(str::to_owned),
                evidence: Some(evidence),
            });
            changed = true;
            control_judged = true;
            moved |= self
                .discard_test_message(&account, &providers, locator)
                .await
                .is_some();
        }
        let settle = self.settle(&tenant);
        // Whether the organization said its mail goes through the relay, as
        // each pass is judged.
        let relay_now = self.relay_now(account_id);
        let mut decided: Vec<(String, CheckRecord)> = Vec::new();
        if let Some(control) = &control {
            for waiting in &run.items {
                let Some(locator) = found.get(&waiting.token) else {
                    continue;
                };
                let Some((content, transport)) = self.fetch_arrived(&providers, locator).await
                else {
                    continue;
                };
                let now = Utc::now();
                let evidence = check_evidence(
                    &content,
                    transport.as_deref(),
                    &tenant,
                    waiting.sent_at,
                    now,
                );
                let judged = judge(
                    &waiting.item,
                    &evidence,
                    control,
                    &settle,
                    waiting.sent_at,
                    now,
                );
                let new_id = self
                    .discard_test_message(&account, &providers, locator)
                    .await;
                moved |= new_id.is_some();
                decided.push((
                    waiting.token.clone(),
                    CheckRecord {
                        state: judged.state,
                        at: Some(now),
                        note: judged.note,
                        test_message_id: Some(
                            new_id
                                .flatten()
                                .unwrap_or_else(|| locator.remote_id.clone()),
                        ),
                        evidence: Some(evidence),
                        other_label_id: judged.other_label_id,
                        other_label_on_control: judged.other_label_on_control,
                        changed_at: judged.changed_at,
                        sent_at: Some(waiting.sent_at),
                        relay_expected: relay_now,
                    },
                ));
            }
        }

        let now = Utc::now();
        let timeout = Duration::minutes(CHECK_TIMEOUT_MINUTES);
        let control_late = control.is_none() && now - run.control_sent_at > timeout;
        let not_arrived = |sent_at: DateTime<Utc>| CheckRecord {
            state: CheckState::Unknown,
            at: Some(now),
            note: Some(CheckNote::NotArrived),
            sent_at: Some(sent_at),
            ..CheckRecord::default()
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
            // Each item that's done: what was decided of it, or `None` when
            // its message (sent then) never came.
            let mut records: Vec<(CheckItem, DateTime<Utc>, Option<CheckRecord>)> = Vec::new();
            current.items.retain(|waiting| {
                let decided = decided.iter().find(|(token, _)| *token == waiting.token);
                let record = match decided {
                    Some((_, record)) => Some(record.clone()),
                    None if control_late || now - waiting.sent_at > timeout => None,
                    None => return true,
                };
                records.push((waiting.item.clone(), waiting.sent_at, record));
                false
            });
            changed |= !records.is_empty();
            for (item, sent_at, record) in records {
                let record = record.unwrap_or_else(|| {
                    let previous = checks.record(&item);
                    // Nothing was learned of it: a pass that still counts
                    // stays, saying this test didn't arrive.
                    if offered(&previous, now, Relay::of(relay_now, checks)).is_ok() {
                        CheckRecord {
                            note: Some(CheckNote::NotArrived),
                            ..previous
                        }
                    } else {
                        not_arrived(sent_at)
                    }
                });
                *checks.record_mut(&item) = record;
            }
            if control_judged && let Some(outcome) = &control {
                checks.last_control = Some(LastControl {
                    sent_at: run.control_sent_at,
                    outcome: outcome.clone(),
                });
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

    /// A test message as it arrived, and its transport headers when
    /// Microsoft 365 gives them; `None` when it can't be judged yet: not
    /// fetched, or fetched without its named properties, whose absence could
    /// make a protected arrival look ordinary. The transport headers are
    /// only a better source: without them, the headers Graph parsed are
    /// used.
    async fn fetch_arrived(
        &self,
        providers: &ProviderSet,
        locator: &MessageLocator,
    ) -> Option<(MessageContent, Option<String>)> {
        let content = match providers.mail.fetch_message(locator).await {
            Ok(content) if content.signals.properties_unavailable => {
                tracing::debug!("a setup check's message came without its properties");
                return None;
            }
            Ok(content) => content,
            Err(e) => {
                tracing::info!("reading a setup check's message failed: {e}");
                return None;
            }
        };
        let transport = match providers.mail.transport_headers(locator).await {
            Ok(transport) => transport,
            Err(e) => {
                tracing::info!("reading a setup check's message's transport headers failed: {e}");
                None
            }
        };
        Some((content, transport))
    }

    /// Whether the account's organization adds its signature on send (the
    /// Cumulas Signatures relay), as it last said: then the account's mail
    /// goes through the relay.
    pub(super) fn relay_now(&self, account_id: &str) -> bool {
        self.store
            .account_signatures(account_id)
            .is_ok_and(|s| s.organization.state == OrganizationSignatureState::AddedOnSend)
    }

    /// "Copy details for IT" (see [`check_details`]): what the account's
    /// last checks found. Errors: `NotFound`, `Unsupported` (not a work or
    /// school account).
    pub(super) fn check_details_text(&self, account_id: &str) -> Result<String> {
        let account = self.store.get_account(account_id)?;
        let Some(tenant) = work_tenant(&account) else {
            return Err(Error::unsupported(PROTECTED_SENDING_UNSUPPORTED));
        };
        let now = Utc::now();
        let checks = self.store.protection_checks(account_id)?;
        let relay = Relay::of(self.relay_now(account_id), &checks);
        let catalog = self.label_list(&account)?;
        let settle = self.settle(tenant);
        let detail = |title: String, item: CheckItem| {
            let record = checks.record(&item);
            CheckDetailsItem {
                title,
                result: check_result(&record, waiting_for(&checks, &item), now, relay),
                item,
                record,
            }
        };
        let mut items: Vec<CheckDetailsItem> =
            [ProtectAction::Encrypt, ProtectAction::DoNotForward]
                .into_iter()
                .map(|action| detail(action.name().to_owned(), CheckItem::Action { action }))
                .collect();
        // Labels only once checked: an organization may list many.
        for label in catalog.iter().flat_map(|c| &c.labels) {
            let item = CheckItem::Label {
                label_id: label.id.clone(),
            };
            if checks.record(&item).state != CheckState::NotChecked
                || waiting_for(&checks, &item).is_some()
            {
                items.push(detail(
                    format!("Label {} ({})", label.title(), label.id),
                    item,
                ));
            }
        }
        Ok(check_details(&CheckDetailsInput {
            email: &account.email,
            tenant,
            version: env!("CARGO_PKG_VERSION"),
            control: checks.last_control.as_ref(),
            items,
            catalog: catalog.as_ref(),
            settle: &settle,
        }))
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
