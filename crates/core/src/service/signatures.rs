//! Signatures: the person's own for every account, and their organization's
//! for Microsoft 365 work and school accounts (see [`crate::signatures`]);
//! when to ask the organization again; and what a message starts with.
//!
//! Only an answer changes what's known of the organization's signature.
//! When asking fails (no network, the service busy, Microsoft's sign-in
//! wanting the person), what was known stays, marked stale: a signature the
//! organization gave keeps working offline.
//!
//! While the organization signs the account's mail, or nothing is known
//! yet, or the last check failed, it's asked about 15 seconds after the app
//! starts, then every 6 hours, and in the background when a message is
//! started more than an hour after its last answer (while it signs the
//! mail). While it doesn't, or doesn't allow the app (in most organizations,
//! Microsoft refusing the token), it's asked once every 24 hours, also
//! across restarts, and never because the app started or a message was. A
//! failed check goes again 30 minutes later (or when the service says, or as
//! soon as the network is back). Signing in asks at once, whatever was
//! said. Times go by the wall clock, which keeps counting while the
//! computer sleeps; the scheduler looks at it at least every minute. A time
//! kept while the clock was ahead makes the check due at once.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};
use tokio::task::AbortHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::reminders::Clock;
use super::{CoreService, EventSink};
use crate::error::Result;
use crate::html::{
    SIGNATURE_SANITIZER_VERSION, html_to_text, sanitize_signature_html,
    sanitize_stored_signature_html,
};
use crate::model::{
    Account, AccountSettings, AccountSignature, ComposeSignature, ComposeSignatureKind, DataKind,
    OrganizationSignature, OrganizationSignatureState, ReplyKind,
};
use crate::signatures::{CheckOutcome, OrganizationSignatures, is_organization_tenant};
use crate::store::{Store, StoredOrganizationSignature};

/// The first scheduled check comes this long after the app starts.
pub(crate) const FIRST_CHECK_DELAY: StdDuration = StdDuration::from_secs(15);
/// How often an organization that signs the account's mail is asked again.
pub(crate) const MANAGED_EVERY: Duration = Duration::hours(6);
/// How often one that doesn't (or doesn't allow the app) is asked again,
/// whether the app restarted meanwhile or not.
pub(crate) const UNMANAGED_EVERY: Duration = Duration::hours(24);
/// How long after a failed check the next one comes, unless the service
/// said.
pub(crate) const RETRY_AFTER_FAILURE: Duration = Duration::minutes(30);
/// The shortest wait the service's `Retry-After` can ask for.
const MIN_RETRY_AFTER: Duration = Duration::minutes(1);
/// A check that ended less than this long ago isn't repeated.
pub(crate) const MIN_INTERVAL: StdDuration = StdDuration::from_secs(60);
/// Starting a message waits this long at most for a first check.
pub(crate) const COMPOSE_WAIT: StdDuration = StdDuration::from_secs(4);
/// Starting a message checks again, in the background, when the
/// organization signs the account's mail and its last answer is older than
/// this.
pub(crate) const COMPOSE_RECHECK: Duration = Duration::hours(1);
/// The longest the scheduler sleeps: its timer stands still while the
/// computer sleeps, so it looks at the clock at least this often.
const MAX_SLEEP: StdDuration = StdDuration::from_secs(60);

/// The signature state the service keeps in memory.
pub(crate) struct SignatureHub {
    /// The check in progress for each account; whoever asks meanwhile
    /// waits for it rather than starting another.
    checks: Mutex<HashMap<String, InFlight>>,
    /// Each account's organization service, as the connector made it
    /// (`None`: the account can't have a company signature).
    services: Mutex<HashMap<String, Option<Arc<dyn OrganizationSignatures>>>>,
    /// When each account's last check this run ended.
    finished: Mutex<HashMap<String, Instant>>,
    /// When the network last came back: checks that failed before then go
    /// again at once.
    network_back_at: Mutex<Option<DateTime<Utc>>>,
    /// Accounts removed this run, which no check is started for again
    /// (account ids are never reused).
    removed: Mutex<HashSet<String>>,
    next_check_id: AtomicU64,
    /// Wakes the scheduler.
    pub(crate) poke: Arc<Notify>,
    clock: RwLock<Clock>,
}

struct InFlight {
    id: u64,
    done: watch::Receiver<bool>,
    task: Option<AbortHandle>,
}

impl Default for SignatureHub {
    fn default() -> Self {
        Self {
            checks: Mutex::default(),
            services: Mutex::default(),
            finished: Mutex::default(),
            network_back_at: Mutex::default(),
            removed: Mutex::default(),
            next_check_id: AtomicU64::new(0),
            poke: Arc::new(Notify::new()),
            clock: RwLock::new(Arc::new(Utc::now)),
        }
    }
}

impl SignatureHub {
    pub(crate) fn now(&self) -> DateTime<Utc> {
        let clock = Arc::clone(&self.clock.read().unwrap());
        clock()
    }

    /// For tests: signatures go by `clock` rather than the wall clock.
    #[cfg(test)]
    pub(crate) fn set_clock(&self, clock: Clock) {
        *self.clock.write().unwrap() = clock;
    }

    fn in_flight(&self, account_id: &str) -> Option<watch::Receiver<bool>> {
        let checks = self.checks.lock().unwrap();
        checks.get(account_id).map(|c| c.done.clone())
    }

    /// How long until a check may start again (`MIN_INTERVAL` after the
    /// last ended); zero when it may now.
    fn cooling(&self, account_id: &str) -> StdDuration {
        let finished = self.finished.lock().unwrap();
        finished.get(account_id).map_or(StdDuration::ZERO, |ended| {
            MIN_INTERVAL.saturating_sub(ended.elapsed())
        })
    }

    fn asked_this_run(&self, account_id: &str) -> bool {
        self.finished.lock().unwrap().contains_key(account_id)
    }

    fn is_removed(&self, account_id: &str) -> bool {
        self.removed.lock().unwrap().contains(account_id)
    }

    /// A check ended: it's no longer in progress, and the scheduler looks
    /// at when the next is due.
    fn finish(&self, account_id: &str, id: u64) {
        {
            let mut checks = self.checks.lock().unwrap();
            if checks.get(account_id).is_none_or(|c| c.id != id) {
                // The account was removed meanwhile.
                return;
            }
            checks.remove(account_id);
            // Still under `checks`, so that removing the account forgets
            // this too.
            self.finished
                .lock()
                .unwrap()
                .insert(account_id.to_owned(), Instant::now());
        }
        self.poke.notify_one();
    }

    /// Forgets a removed account: its check in progress stops, and none
    /// starts again, even for whoever found the account just before.
    pub(crate) fn forget(&self, account_id: &str) {
        let check = {
            let mut checks = self.checks.lock().unwrap();
            // Under `checks`, where checks start: none starts after this.
            self.removed.lock().unwrap().insert(account_id.to_owned());
            checks.remove(account_id)
        };
        if let Some(task) = check.and_then(|check| check.task) {
            task.abort();
        }
        self.services.lock().unwrap().remove(account_id);
        self.finished.lock().unwrap().remove(account_id);
    }

    /// An account whose removal failed after [`Self::forget`]: it stays,
    /// so its checks go on.
    pub(crate) fn restore(&self, account_id: &str) {
        self.removed.lock().unwrap().remove(account_id);
        self.poke.notify_one();
    }

    /// Forgets every account's organization service (the Microsoft app
    /// registration changed).
    pub(crate) fn forget_services(&self) {
        self.services.lock().unwrap().clear();
    }

    /// The network came back.
    pub(crate) fn network_back(&self) {
        *self.network_back_at.lock().unwrap() = Some(self.now());
        self.poke.notify_one();
    }
}

/// What a check needs, without the service: it runs on its own task, and
/// finishes even when whoever started it stops waiting.
#[derive(Clone)]
struct CheckContext {
    store: Arc<Store>,
    sink: Arc<dyn EventSink>,
    hub: Arc<SignatureHub>,
}

impl CheckContext {
    async fn run(&self, account_id: &str, organization: &dyn OrganizationSignatures) {
        let scope = organization.scope().to_owned();
        let before = match self.store.account_signatures(account_id) {
            Ok(stored) => effective(stored.organization, &scope),
            Err(e) => {
                tracing::warn!("signature check for account {account_id}: {e}");
                return;
            }
        };
        let outcome = organization.check(etag_to_send(&before)).await;
        let after = apply(&before, outcome, self.hub.now(), &scope, account_id);
        if let Err(e) = self.store.save_organization_signature(account_id, &after) {
            // The account was removed meanwhile, most likely.
            tracing::debug!("signature check for account {account_id}: not kept ({e})");
            return;
        }
        if visible(&before) != visible(&after) {
            self.sink
                .data_changed(&[DataKind::Signatures], Some(account_id));
        }
    }
}

/// What's kept of the organization's signature, for the service whose
/// scope is `scope`: nothing, when it came from another.
fn effective(stored: StoredOrganizationSignature, scope: &str) -> StoredOrganizationSignature {
    if stored.scope.as_deref() == Some(scope) {
        stored
    } else {
        StoredOrganizationSignature::default()
    }
}

/// What prefixes a kept ETag: the version of the sanitizer its answer went
/// through.
fn etag_prefix() -> String {
    format!("s{SIGNATURE_SANITIZER_VERSION}:")
}

/// The ETag a check sends: the last answer's, when this sanitizer kept it.
/// One kept by another (or by a version before the prefix) isn't sent, so
/// the service answers in full and the signature is sanitized again; until
/// then the one kept stays in use.
fn etag_to_send(before: &StoredOrganizationSignature) -> Option<&str> {
    let etag = before
        .etag
        .as_deref()
        .filter(|_| before.state != OrganizationSignatureState::Unchecked)?;
    etag.strip_prefix(etag_prefix().as_str())
}

/// What's known once a check ended with `outcome`.
fn apply(
    before: &StoredOrganizationSignature,
    outcome: CheckOutcome,
    now: DateTime<Utc>,
    scope: &str,
    account_id: &str,
) -> StoredOrganizationSignature {
    let answered = |state, html: Option<String>, on_replies, etag: Option<String>| {
        tracing::debug!("organization signature for account {account_id}: {state:?}");
        StoredOrganizationSignature {
            scope: Some(scope.to_owned()),
            state,
            html,
            on_replies,
            etag: etag.map(|etag| format!("{}{etag}", etag_prefix())),
            checked_at: Some(now),
            attempted_at: Some(now),
            stale: false,
            retry_at: None,
        }
    };
    let failed = |retry_after: Option<StdDuration>, reason: &str| {
        let retry_at = retry_after.map(|wait| {
            now + Duration::from_std(wait)
                .unwrap_or(MIN_RETRY_AFTER)
                .max(MIN_RETRY_AFTER)
        });
        tracing::info!("signature check for account {account_id}: {reason}");
        StoredOrganizationSignature {
            scope: Some(scope.to_owned()),
            attempted_at: Some(now),
            stale: true,
            retry_at,
            ..before.clone()
        }
    };
    match outcome {
        CheckOutcome::Signature {
            html,
            on_replies,
            base,
            etag,
        } => match sanitize_signature_html(&html, base.as_ref()) {
            Ok(html) => answered(
                OrganizationSignatureState::Ready,
                Some(html),
                on_replies,
                etag,
            ),
            Err(_) => failed(None, "the signature is too large to use"),
        },
        CheckOutcome::AddedOnSend { on_replies, etag } => answered(
            OrganizationSignatureState::AddedOnSend,
            None,
            on_replies,
            etag,
        ),
        CheckOutcome::NotUsed { etag } => {
            answered(OrganizationSignatureState::NotUsed, None, false, etag)
        }
        CheckOutcome::NotAllowed => {
            answered(OrganizationSignatureState::NotAllowed, None, false, None)
        }
        CheckOutcome::Unchanged if etag_to_send(before).is_some() => StoredOrganizationSignature {
            checked_at: Some(now),
            attempted_at: Some(now),
            stale: false,
            retry_at: None,
            ..before.clone()
        },
        // No ETag was sent: a service that says so is wrong.
        CheckOutcome::Unchanged => failed(None, "unexpected \"not modified\""),
        CheckOutcome::Failed {
            retry_after,
            reason,
        } => failed(retry_after, &reason),
    }
}

/// What the UI shows of it (when it was checked aside).
fn visible(
    o: &StoredOrganizationSignature,
) -> (OrganizationSignatureState, Option<&str>, bool, bool) {
    let ready = o.state == OrganizationSignatureState::Ready;
    (
        o.state,
        o.html.as_deref().filter(|_| ready),
        o.on_replies && o.state.is_managed(),
        o.stale,
    )
}

/// What the UI and the composer get of it: its signature sanitized again
/// by this version of the app.
fn public(o: StoredOrganizationSignature) -> OrganizationSignature {
    let ready = o.state == OrganizationSignatureState::Ready;
    OrganizationSignature {
        state: o.state,
        html: ready.then(|| {
            o.html
                .as_deref()
                .map(sanitize_stored_signature_html)
                .unwrap_or_default()
        }),
        // Whoever adds it, the app or the organization's mail server.
        on_replies: o.on_replies && o.state.is_managed(),
        checked_at: o.checked_at,
        stale: o.stale,
    }
}

fn unsupported() -> OrganizationSignature {
    OrganizationSignature {
        state: OrganizationSignatureState::Unsupported,
        html: None,
        on_replies: false,
        checked_at: None,
        stale: false,
    }
}

/// When the organization is to be asked next, by what it last said:
/// soon after the app starts, unless it said it doesn't sign the account's
/// mail (or doesn't allow the app), which is asked once a day whatever the
/// app did meanwhile.
pub(crate) fn next_check(
    o: &StoredOrganizationSignature,
    asked_this_run: bool,
    retry_failed: bool,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    use OrganizationSignatureState as State;
    // A time after now was kept while the clock was ahead: what's due
    // after it is due now (the check keeps the right time), rather than
    // once the clock has caught up.
    let since = |at: Option<DateTime<Utc>>, every: Duration| {
        at.filter(|at| *at <= now).map_or(now, |at| at + every)
    };
    let retry_at = retry_at(o, now);
    let due = if o.stale {
        if !asked_this_run || retry_failed {
            now
        } else {
            retry_at.unwrap_or_else(|| since(o.attempted_at, RETRY_AFTER_FAILURE))
        }
    } else {
        match o.state {
            State::Ready | State::AddedOnSend if asked_this_run => {
                since(o.checked_at, MANAGED_EVERY)
            }
            // Mostly Microsoft refusing the token in an organization
            // without the service's app: asking at every start would only
            // fill its sign-in logs.
            State::NotUsed | State::NotAllowed => since(o.checked_at, UNMANAGED_EVERY),
            State::Ready | State::AddedOnSend | State::Unchecked | State::Unsupported => now,
        }
    };
    // Never before the service said.
    retry_at.map_or(due, |at| due.max(at))
}

/// When the service said to ask again, if it did; not when it said so
/// while the clock was ahead (its check is after now), since how long it
/// asked to wait from now can't be told.
fn retry_at(o: &StoredOrganizationSignature, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    o.retry_at
        .filter(|_| o.attempted_at.is_none_or(|at| at <= now))
}

/// The signature a message starts with. While the organization signs the
/// account's mail, the own signature isn't used, not even on the replies
/// and forwards it leaves unsigned.
pub(crate) fn compose_signature(
    signature: &AccountSignature,
    reply_kind: Option<ReplyKind>,
) -> ComposeSignature {
    let new_message = reply_kind.is_none();
    let organization = &signature.organization;
    let signed = new_message || organization.on_replies;
    match organization.state {
        OrganizationSignatureState::Ready if signed => ComposeSignature {
            kind: ComposeSignatureKind::Organization,
            html: Some(organization.html.clone().unwrap_or_default()),
        },
        // Its mail server leaves the message as it is.
        OrganizationSignatureState::AddedOnSend if signed => ComposeSignature {
            kind: ComposeSignatureKind::AddedOnSend,
            html: None,
        },
        OrganizationSignatureState::Ready | OrganizationSignatureState::AddedOnSend => {
            ComposeSignature::none()
        }
        _ if !signature.own_html.is_empty() && (new_message || signature.own_on_replies) => {
            ComposeSignature {
                kind: ComposeSignatureKind::Own,
                html: Some(signature.own_html.clone()),
            }
        }
        _ => ComposeSignature::none(),
    }
}

async fn wait_for(mut done: watch::Receiver<bool>) {
    // An error means the check was stopped (its account removed).
    let _ = done.wait_for(|done| *done).await;
}

impl CoreService {
    /// The account's signatures: its own, and what its organization last
    /// said.
    pub fn get_account_signature(&self, account_id: &str) -> Result<AccountSignature> {
        let account = self.store.get_account(account_id)?;
        self.account_signature(&account)
    }

    /// Saves the account's own signature, sanitized (more than 512 KB is
    /// refused); empty for none.
    pub fn update_own_signature(
        &self,
        account_id: &str,
        html: &str,
        on_replies: bool,
    ) -> Result<AccountSignature> {
        let account = self.store.get_account(account_id)?;
        let mut html = sanitize_signature_html(html, None)?;
        // An editor's empty paragraph is no signature.
        if html_to_text(&html).trim().is_empty() && !html.contains("<img") {
            html.clear();
        }
        let before = self.store.account_signatures(account_id)?;
        if before.own_html != html || before.own_on_replies != on_replies {
            self.store
                .save_own_signature(account_id, &html, on_replies)?;
            self.notify(&[DataKind::Signatures], Some(account_id));
        }
        self.account_signature(&account)
    }

    /// Asks the account's organization for its signature now, unless a
    /// check ended in the last minute (or the service asked to wait), and
    /// returns what's known then. Never fails for the service or the
    /// network: a failed check leaves what was known, marked stale.
    pub async fn check_organization_signature(&self, account_id: &str) -> Result<AccountSignature> {
        let account = self.store.get_account(account_id)?;
        if let Some(organization) = self.organization_service(&account) {
            let done = match self.signatures.in_flight(account_id) {
                Some(done) => Some(done),
                None if self.may_check_now(account_id, organization.scope())? => {
                    Some(self.start_signature_check(account_id, organization))
                }
                None => None,
            };
            if let Some(done) = done {
                wait_for(done).await;
            }
        }
        self.account_signature(&account)
    }

    /// The signature a new message (`reply_kind` `None`), reply or forward
    /// from the account starts with. Waits up to four seconds for the
    /// organization's first answer; checks again in the background when it
    /// signs the account's mail and its last answer is more than an hour
    /// old.
    pub async fn get_compose_signature(
        &self,
        account_id: &str,
        reply_kind: Option<ReplyKind>,
    ) -> Result<ComposeSignature> {
        let account = self.store.get_account(account_id)?;
        let mut signature = self.account_signature(&account)?;
        if let Some(organization) = self.organization_service(&account) {
            let state = signature.organization.state;
            let may_check = || self.may_check_now(account_id, organization.scope());
            if state == OrganizationSignatureState::Unchecked {
                let done = match self.signatures.in_flight(account_id) {
                    Some(done) => Some(done),
                    None if may_check()? => {
                        Some(self.start_signature_check(account_id, Arc::clone(&organization)))
                    }
                    None => None,
                };
                if let Some(done) = done
                    && tokio::time::timeout(COMPOSE_WAIT, wait_for(done))
                        .await
                        .is_ok()
                {
                    signature = self.account_signature(&account)?;
                }
            } else if state.is_managed() {
                // Not otherwise: what it said is asked again once a day.
                let old = signature
                    .organization
                    .checked_at
                    .is_none_or(|at| self.signatures.now() - at > COMPOSE_RECHECK);
                if old && self.signatures.in_flight(account_id).is_none() && may_check()? {
                    // For the next message; this one goes with what's known.
                    drop(self.start_signature_check(account_id, organization));
                }
            }
        }
        Ok(compose_signature(&signature, reply_kind))
    }

    /// Asks the account's organization at once (after a sign-in), without
    /// waiting for the answer.
    pub(super) fn check_signature_soon(&self, account_id: &str) {
        let Ok(account) = self.store.get_account(account_id) else {
            return;
        };
        if let Some(organization) = self.organization_service(&account)
            && self.signatures.in_flight(account_id).is_none()
        {
            drop(self.start_signature_check(account_id, organization));
        }
    }

    /// The account's signatures as kept, each sanitized again: a sanitizer
    /// stricter than the one that kept them applies at once.
    fn account_signature(&self, account: &Account) -> Result<AccountSignature> {
        let stored = self.store.account_signatures(&account.id)?;
        let organization = match self.organization_service(account) {
            Some(service) => public(effective(stored.organization, service.scope())),
            None => unsupported(),
        };
        Ok(AccountSignature {
            account_id: account.id.clone(),
            own_html: sanitize_stored_signature_html(&stored.own_html),
            own_on_replies: stored.own_on_replies,
            organization,
        })
    }

    /// The account's organization signature service; `None` for IMAP
    /// accounts, personal Microsoft accounts, a build without the
    /// service's client ID, and an account being removed. Those never reach
    /// the network for it.
    fn organization_service(&self, account: &Account) -> Option<Arc<dyn OrganizationSignatures>> {
        let AccountSettings::Microsoft(settings) = &account.settings else {
            return None;
        };
        if !is_organization_tenant(&settings.tenant_id) {
            return None;
        }
        let mut services = self.signatures.services.lock().unwrap();
        if self.signatures.is_removed(&account.id) {
            return None;
        }
        services
            .entry(account.id.clone())
            .or_insert_with(|| self.connector.organization_signatures(account))
            .clone()
    }

    /// Whether a check may start now: none ended in the last minute, and
    /// the service didn't ask to wait.
    fn may_check_now(&self, account_id: &str, scope: &str) -> Result<bool> {
        if !self.signatures.cooling(account_id).is_zero() {
            return Ok(false);
        }
        let stored = effective(
            self.store.account_signatures(account_id)?.organization,
            scope,
        );
        let now = self.signatures.now();
        Ok(retry_at(&stored, now).is_none_or(|at| at <= now))
    }

    /// Starts a check of the account's organization signature on its own
    /// task, or joins the one in progress. The receiver turns true when it
    /// has ended; at once for an account being removed, which no check
    /// starts for (it would redeem, and keep, the sign-in being deleted).
    fn start_signature_check(
        &self,
        account_id: &str,
        organization: Arc<dyn OrganizationSignatures>,
    ) -> watch::Receiver<bool> {
        let mut checks = self.signatures.checks.lock().unwrap();
        if self.signatures.is_removed(account_id) {
            return watch::channel(true).1;
        }
        if let Some(check) = checks.get(account_id) {
            return check.done.clone();
        }
        let (done_tx, done) = watch::channel(false);
        let Ok(runtime) = Handle::try_current() else {
            tracing::error!("signature checks need a Tokio runtime");
            return done;
        };
        let id = self
            .signatures
            .next_check_id
            .fetch_add(1, Ordering::Relaxed);
        let context = CheckContext {
            store: Arc::clone(&self.store),
            sink: Arc::clone(&self.sink),
            hub: Arc::clone(&self.signatures),
        };
        let cancel = self.push.root.child_token();
        let account = account_id.to_owned();
        // Still holding `checks`: the task can't finish before it's listed.
        let task = runtime.spawn(async move {
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = context.run(&account, organization.as_ref()) => {}
            }
            context.hub.finish(&account, id);
            let _ = done_tx.send(true);
        });
        checks.insert(
            account_id.to_owned(),
            InFlight {
                id,
                done: done.clone(),
                task: Some(task.abort_handle()),
            },
        );
        done
    }

    /// Starts the checks that are due; returns when the next one is.
    fn start_due_signature_checks(&self) -> Option<DateTime<Utc>> {
        let accounts = match self.store.list_accounts() {
            Ok(accounts) => accounts,
            Err(e) => {
                tracing::warn!("listing accounts for signature checks failed: {e}");
                return None;
            }
        };
        let online = *self.push.online.borrow();
        let network_back_at = *self.signatures.network_back_at.lock().unwrap();
        let now = self.signatures.now();
        accounts
            .iter()
            .filter_map(|account| {
                self.start_signature_check_if_due(account, online, network_back_at, now)
            })
            .min()
    }

    /// Starts the account's check if it's due (and the computer `online`);
    /// returns when it is, if later.
    fn start_signature_check_if_due(
        &self,
        account: &Account,
        online: bool,
        network_back_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        let organization = self.organization_service(account)?;
        // It pokes the scheduler when it ends.
        if self.signatures.in_flight(&account.id).is_some() {
            return None;
        }
        let stored = match self.store.account_signatures(&account.id) {
            Ok(stored) => effective(stored.organization, organization.scope()),
            Err(e) => {
                tracing::warn!("signature check for account {}: {e}", account.id);
                return None;
            }
        };
        let asked = self.signatures.asked_this_run(&account.id);
        // Failed while the network was away.
        let retry_failed = network_back_at
            .is_some_and(|back| stored.stale && stored.attempted_at.is_none_or(|at| at < back));
        let cooling = Duration::from_std(self.signatures.cooling(&account.id)).unwrap_or_default();
        let due = next_check(&stored, asked, retry_failed, now).max(now + cooling);
        if due <= now && online {
            drop(self.start_signature_check(&account.id, organization));
        }
        (due > now).then_some(due)
    }
}

/// Asks every account's organization when due (see the module's
/// documentation), until `cancel` fires.
pub(super) async fn signature_loop(service: Weak<CoreService>, cancel: CancellationToken) {
    tokio::select! {
        _ = cancel.cancelled() => return,
        _ = tokio::time::sleep(FIRST_CHECK_DELAY) => {}
    }
    loop {
        let Some(strong) = service.upgrade() else {
            return;
        };
        let next = strong.start_due_signature_checks();
        let now = strong.signatures.now();
        let poke = Arc::clone(&strong.signatures.poke);
        drop(strong);
        let sleep = next
            .map(|at| (at - now).to_std().unwrap_or_default())
            .unwrap_or(MAX_SLEEP)
            .min(MAX_SLEEP);
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(sleep) => {}
            _ = poke.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests;
