//! Organization setup: an administrator signs in with the setup app
//! ([`crate::auth::admin_setup`]), and Cumulas Mail reads, creates and
//! repairs the organization's mail flow rules and auto-labeling policies
//! through Exchange Online's InvokeCommand interface, or writes a PowerShell
//! script that does the same. Only objects whose names start with
//! "Cumulas Mail: " are ever changed, and nothing is ever deleted.
//!
//! R1 to R18 in comments are what only a real tenant can show:
//! `docs/admin-setup-open-items.md`.

pub mod apply;
pub mod discovery;
pub mod invoke;
pub mod objects;
pub mod plan;
pub mod runlog;
pub mod script;
pub mod status;
pub mod wanted;

#[cfg(test)]
pub(crate) mod tests;

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

use serde_json::json;
use tokio_util::sync::CancellationToken;

pub use invoke::HostRules;
pub use runlog::{RunLog, details_text};
pub use status::Snapshot;

use crate::auth::admin_setup::{AdminSignIn, AdminTokens, Resource};
use crate::auth::microsoft::WallClock;
use crate::error::{Error, Result};
use crate::model::OrgSetupAdmin;
use crate::service::AdminEndpoints;
use invoke::{Anchor, InvokeClient, InvokeError, InvokeTarget};

/// A session unused this long is signed out, by either clock.
pub const IDLE_LIMIT: Duration = Duration::from_secs(60 * 60);
/// How often the idle watcher looks.
pub const IDLE_TICK: Duration = Duration::from_secs(60);
/// How long apply waits between listings of Security & Compliance objects
/// that don't show yet.
const RELIST_DELAY: Duration = Duration::from_secs(5);

/// The answer once the setup sign-in has ended (signed out, the dialog
/// closed, or idle too long).
pub fn signed_out() -> Error {
    Error::auth_required("Your setup sign-in ended. Sign in again to continue.")
        .with_details(json!({ "reason": "setupSignedOut" }))
}

/// How this session sends what Microsoft 365 turned out not to accept as
/// first sent (R8); each is pinned once a fallback worked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pins {
    /// Clear a setting with `""` or `[]` rather than `null`.
    pub clear_with_empty: bool,
    /// Leave out `Comments` / `Comment`.
    pub without_comments: bool,
    /// Send `HeaderMatchesPatterns` without `@odata.type`.
    pub plain_hashtable: bool,
}

/// An administrator's setup sign-in for one organization: at most one at a
/// time, in memory only. Dropping it drops its tokens.
pub struct Session {
    pub tenant_id: String,
    /// The work account the dialog was opened from.
    pub account_id: String,
    pub admin: OrgSetupAdmin,
    /// The ID token's oid: for the command that removes this
    /// administrator's approval.
    pub admin_object_id: String,
    pub client_id: String,
    tokens: Arc<AdminTokens>,
    http: reqwest::Client,
    exchange: InvokeClient,
    /// Only a success is kept: after a failed discovery, the next read
    /// tries again.
    compliance: tokio::sync::OnceCell<InvokeClient>,
    compliance_discovery: url::Url,
    hosts: HostRules,
    /// The organization's initial domain, for the Security & Compliance
    /// anchor.
    initial_domain: Mutex<Option<String>>,
    /// One sequence of Microsoft calls at a time (a read, an apply).
    pub(crate) calls: tokio::sync::Mutex<()>,
    /// Cancelled by `end_org_setup`, a new sign-in, or the idle watcher.
    cancel: CancellationToken,
    /// Last use by both clocks (`Instant` stops while the computer sleeps;
    /// the wall clock doesn't).
    last_used: Mutex<(Instant, SystemTime)>,
    wall_clock: WallClock,
    /// The last read, which plans are made from.
    snapshot: Mutex<Option<Arc<Snapshot>>>,
    pins: Mutex<Pins>,
    pub(crate) relist_delay: Duration,
    log: RunLog,
}

impl Session {
    /// A session for `sign_in`, talking where `endpoints` say.
    pub fn new(
        sign_in: AdminSignIn,
        account_id: &str,
        endpoints: &AdminEndpoints,
        log: RunLog,
        wall_clock: WallClock,
    ) -> Result<Self> {
        let http = invoke::http_client()?;
        let cancel = CancellationToken::new();
        let tokens = Arc::new(sign_in.tokens);
        let exchange = InvokeClient::new(
            http.clone(),
            InvokeTarget {
                resource: Resource::Exchange,
                base: endpoints.exchange.clone(),
                tenant_id: sign_in.tenant_id.clone(),
                anchors: Anchor::exchange(&sign_in.username, &sign_in.tenant_id),
            },
            Arc::clone(&tokens),
            endpoints.hosts.clone(),
            log.clone(),
            cancel.clone(),
        );
        let now = (Instant::now(), wall_clock());
        Ok(Self {
            tenant_id: sign_in.tenant_id,
            account_id: account_id.to_owned(),
            admin: OrgSetupAdmin {
                username: sign_in.username,
                display_name: sign_in.display_name,
            },
            admin_object_id: sign_in.object_id,
            client_id: endpoints.client_id.clone(),
            tokens,
            http,
            exchange,
            compliance: tokio::sync::OnceCell::new(),
            compliance_discovery: endpoints.compliance_discovery.clone(),
            hosts: endpoints.hosts.clone(),
            initial_domain: Mutex::new(None),
            calls: tokio::sync::Mutex::new(()),
            cancel,
            last_used: Mutex::new(now),
            wall_clock,
            snapshot: Mutex::new(None),
            pins: Mutex::default(),
            relist_delay: RELIST_DELAY,
            log,
        })
    }

    /// Ends the session's work: a read in flight returns at once, an apply
    /// finishes the change it's in.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Waits until the session ends.
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    pub fn exchange(&self) -> &InvokeClient {
        &self.exchange
    }

    /// The Security & Compliance client, finding the organization's region
    /// first; a failure is tried again next time.
    pub async fn compliance(&self) -> std::result::Result<&InvokeClient, InvokeError> {
        self.compliance
            .get_or_try_init(|| async {
                let initial = self.initial_domain.lock().unwrap().clone();
                let anchors = Anchor::compliance(&self.admin.username, initial.as_deref());
                let discovery = discovery::compliance_host(
                    &self.http,
                    &self.tokens,
                    &self.tenant_id,
                    &anchors,
                    &self.compliance_discovery,
                    &self.log,
                );
                let base = tokio::select! {
                    biased;
                    _ = self.cancel.cancelled() => return Err(InvokeError::Cancelled),
                    base = discovery => base?,
                };
                Ok(InvokeClient::new(
                    self.http.clone(),
                    InvokeTarget {
                        resource: Resource::Compliance,
                        base,
                        tenant_id: self.tenant_id.clone(),
                        anchors,
                    },
                    Arc::clone(&self.tokens),
                    self.hosts.clone(),
                    self.log.clone(),
                    self.cancel.clone(),
                ))
            })
            .await
    }

    pub(crate) fn set_initial_domain(&self, domain: Option<String>) {
        *self.initial_domain.lock().unwrap() = domain;
    }

    pub fn log(&self) -> &RunLog {
        &self.log
    }

    pub fn pins(&self) -> Pins {
        *self.pins.lock().unwrap()
    }

    pub(crate) fn pin(&self, change: impl FnOnce(&mut Pins)) {
        change(&mut self.pins.lock().unwrap());
    }

    /// The last read, if any.
    pub fn snapshot(&self) -> Option<Arc<Snapshot>> {
        self.snapshot.lock().unwrap().clone()
    }

    pub(crate) fn keep_snapshot(&self, snapshot: Snapshot) -> Arc<Snapshot> {
        let snapshot = Arc::new(snapshot);
        *self.snapshot.lock().unwrap() = Some(Arc::clone(&snapshot));
        snapshot
    }

    /// The setup app's token store, to see that it goes with the session.
    pub fn token_store(&self) -> Weak<crate::secrets::MemorySecretStore> {
        self.tokens.store()
    }

    /// Notes a use now.
    pub fn touch(&self) {
        *self.last_used.lock().unwrap() = (Instant::now(), (self.wall_clock)());
    }

    /// Whether it went unused for [`IDLE_LIMIT`] by either clock.
    pub fn is_idle(&self) -> bool {
        let (instant, wall) = *self.last_used.lock().unwrap();
        instant.elapsed() >= IDLE_LIMIT
            || (self.wall_clock)()
                .duration_since(wall)
                .is_ok_and(|idle| idle >= IDLE_LIMIT)
    }
}

/// Where the service keeps the session: the watcher takes it out of here
/// when it goes idle.
pub type SessionSlot = Arc<Mutex<Option<Arc<Session>>>>;

/// Signs `session` out of `slot` (if it's still the one there): cancels it
/// and notes why.
pub fn sign_out(slot: &SessionSlot, session: &Arc<Session>, why: &str) {
    let mut held = slot.lock().unwrap();
    if held.as_ref().is_some_and(|s| Arc::ptr_eq(s, session)) {
        held.take();
    }
    drop(held);
    if !session.is_cancelled() {
        session.cancel();
        session.log().note(why);
    }
}

/// What the idle watcher notes.
pub const IDLE_NOTE: &str = "Signed out after 60 minutes without use.";

/// Watches `session` until it ends, and signs it out once it has gone
/// unused for [`IDLE_LIMIT`] by either clock (a suspended computer's
/// `Instant` stands still; its wall clock doesn't). Holds it weakly.
pub fn watch_idle(slot: SessionSlot, session: &Arc<Session>, tick: Duration) {
    let weak = Arc::downgrade(session);
    let cancel = session.cancel.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(tick) => {}
            }
            let Some(session) = weak.upgrade() else {
                return;
            };
            if session.is_idle() {
                sign_out(&slot, &session, IDLE_NOTE);
                return;
            }
        }
    });
}
