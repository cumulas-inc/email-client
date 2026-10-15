//! Signatures: the person's own for every account, and their organization's
//! for Microsoft 365 work and school accounts (see [`crate::signatures`]);
//! when to ask the organization again; and what a message starts with.
//!
//! Only an answer changes what's known of the organization's signature.
//! When asking fails (no network, the service busy, Microsoft's sign-in
//! wanting the person), what was known stays, marked stale: a signature the
//! organization gave keeps working offline.
//!
//! The organization is asked about 15 seconds after the app starts, then
//! every 6 hours while it signs the account's mail, every 24 hours while it
//! doesn't, and 30 minutes after a failure (or when the service says, or as
//! soon as the network is back). Signing in asks at once. Times go by the
//! wall clock, which keeps counting while the computer sleeps; the
//! scheduler looks at it at least every minute.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
use crate::html::{html_to_text, sanitize_signature_html};
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
/// How often one that doesn't (or doesn't allow the app) is asked again.
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
/// Starting a message checks again, in the background, when the last
/// answer is older than this.
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
    /// The network came back: failed checks go again at once.
    retry_failed: AtomicBool,
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
            retry_failed: AtomicBool::new(false),
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
        }
        self.finished
            .lock()
            .unwrap()
            .insert(account_id.to_owned(), Instant::now());
        self.poke.notify_one();
    }

    /// Forgets an account: its check in progress stops.
    pub(crate) fn forget(&self, account_id: &str) {
        if let Some(check) = self.checks.lock().unwrap().remove(account_id)
            && let Some(task) = check.task
        {
            task.abort();
        }
        self.services.lock().unwrap().remove(account_id);
        self.finished.lock().unwrap().remove(account_id);
    }

    /// Forgets every account's organization service (the Microsoft app
    /// registration changed).
    pub(crate) fn forget_services(&self) {
        self.services.lock().unwrap().clear();
    }

    /// The network came back.
    pub(crate) fn network_back(&self) {
        self.retry_failed.store(true, Ordering::SeqCst);
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
        let etag = before
            .etag
            .as_deref()
            .filter(|_| before.state != OrganizationSignatureState::Unchecked);
        let outcome = organization.check(etag).await;
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

/// What's known once a check ended with `outcome`.
fn apply(
    before: &StoredOrganizationSignature,
    outcome: CheckOutcome,
    now: DateTime<Utc>,
    scope: &str,
    account_id: &str,
) -> StoredOrganizationSignature {
    let answered = |state, html: Option<String>, on_replies, etag| {
        tracing::debug!("organization signature for account {account_id}: {state:?}");
        StoredOrganizationSignature {
            scope: Some(scope.to_owned()),
            state,
            html,
            on_replies,
            etag,
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
        CheckOutcome::AddedOnSend { etag } => {
            answered(OrganizationSignatureState::AddedOnSend, None, false, etag)
        }
        CheckOutcome::NotUsed { etag } => {
            answered(OrganizationSignatureState::NotUsed, None, false, etag)
        }
        CheckOutcome::NotAllowed => {
            answered(OrganizationSignatureState::NotAllowed, None, false, None)
        }
        CheckOutcome::Unchanged if before.state != OrganizationSignatureState::Unchecked => {
            StoredOrganizationSignature {
                checked_at: Some(now),
                attempted_at: Some(now),
                stale: false,
                retry_at: None,
                ..before.clone()
            }
        }
        // Nothing was asked about: a service that says so is wrong.
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
        o.on_replies && ready,
        o.stale,
    )
}

fn public(o: StoredOrganizationSignature) -> OrganizationSignature {
    let ready = o.state == OrganizationSignatureState::Ready;
    OrganizationSignature {
        state: o.state,
        html: ready.then(|| o.html.unwrap_or_default()),
        on_replies: o.on_replies && ready,
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

/// When the organization is to be asked next: soon after the app starts,
/// then by what it last said.
pub(crate) fn next_check(
    o: &StoredOrganizationSignature,
    asked_this_run: bool,
    retry_failed: bool,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    use OrganizationSignatureState as State;
    let since = |at: Option<DateTime<Utc>>, every: Duration| at.map_or(now, |at| at + every);
    let due = if !asked_this_run || (o.stale && retry_failed) {
        now
    } else if o.stale {
        o.retry_at
            .unwrap_or_else(|| since(o.attempted_at, RETRY_AFTER_FAILURE))
    } else {
        match o.state {
            State::Ready | State::AddedOnSend => since(o.checked_at, MANAGED_EVERY),
            State::NotUsed | State::NotAllowed => since(o.checked_at, UNMANAGED_EVERY),
            State::Unchecked | State::Unsupported => now,
        }
    };
    // Never before the service said.
    o.retry_at.map_or(due, |at| due.max(at))
}

/// The signature a message starts with.
pub(crate) fn compose_signature(
    signature: &AccountSignature,
    reply_kind: Option<ReplyKind>,
) -> ComposeSignature {
    let new_message = reply_kind.is_none();
    let organization = &signature.organization;
    match organization.state {
        OrganizationSignatureState::Ready => {
            if new_message || organization.on_replies {
                ComposeSignature {
                    kind: ComposeSignatureKind::Organization,
                    html: Some(organization.html.clone().unwrap_or_default()),
                }
            } else {
                ComposeSignature::none()
            }
        }
        OrganizationSignatureState::AddedOnSend => ComposeSignature {
            kind: ComposeSignatureKind::AddedOnSend,
            html: None,
        },
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
    /// organization's first answer; checks again in the background when
    /// its last is more than an hour old.
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
            } else {
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

    fn account_signature(&self, account: &Account) -> Result<AccountSignature> {
        let stored = self.store.account_signatures(&account.id)?;
        let organization = match self.organization_service(account) {
            Some(service) => public(effective(stored.organization, service.scope())),
            None => unsupported(),
        };
        Ok(AccountSignature {
            account_id: account.id.clone(),
            own_html: stored.own_html,
            own_on_replies: stored.own_on_replies,
            organization,
        })
    }

    /// The account's organization signature service; `None` for IMAP
    /// accounts, personal Microsoft accounts, and a build without the
    /// service's client ID. Those never reach the network for it.
    fn organization_service(&self, account: &Account) -> Option<Arc<dyn OrganizationSignatures>> {
        let AccountSettings::Microsoft(settings) = &account.settings else {
            return None;
        };
        if !is_organization_tenant(&settings.tenant_id) {
            return None;
        }
        let mut services = self.signatures.services.lock().unwrap();
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
        Ok(stored.retry_at.is_none_or(|at| at <= self.signatures.now()))
    }

    /// Starts a check of the account's organization signature on its own
    /// task, or joins the one in progress. The receiver turns true when it
    /// has ended.
    fn start_signature_check(
        &self,
        account_id: &str,
        organization: Arc<dyn OrganizationSignatures>,
    ) -> watch::Receiver<bool> {
        let mut checks = self.signatures.checks.lock().unwrap();
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
        let retry_failed = online && self.signatures.retry_failed.swap(false, Ordering::SeqCst);
        let now = self.signatures.now();
        let mut next: Option<DateTime<Utc>> = None;
        for account in accounts {
            let Some(organization) = self.organization_service(&account) else {
                continue;
            };
            // It pokes the scheduler when it ends.
            if self.signatures.in_flight(&account.id).is_some() {
                continue;
            }
            let stored = match self.store.account_signatures(&account.id) {
                Ok(stored) => effective(stored.organization, organization.scope()),
                Err(e) => {
                    tracing::warn!("signature check for account {}: {e}", account.id);
                    continue;
                }
            };
            let asked = self.signatures.asked_this_run(&account.id);
            let cooling =
                Duration::from_std(self.signatures.cooling(&account.id)).unwrap_or_default();
            let due = next_check(&stored, asked, retry_failed, now).max(now + cooling);
            if due <= now && online {
                drop(self.start_signature_check(&account.id, organization));
            } else if due > now {
                next = Some(next.map_or(due, |next| next.min(due)));
            }
        }
        next
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
