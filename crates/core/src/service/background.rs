//! What runs while the app does: the periodic full sync, push for every
//! account (see [`crate::push`]) and the resume detector.

use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime};

use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::targeted::{ServiceRunner, SyncQueue, SyncRunner, SyncScope};
use super::{CoreService, PushSource};
use crate::error::{ErrorCode, Result};
use crate::model::AccountKind;
use crate::push::backoff::Backoff;
use crate::push::hub::{AccountPush, join_within};
use crate::push::supervisor::{EventFn, MICROSOFT_POLL, StatusFn, Supervisor};
use crate::push::{PushHealth, PushMode, PushStatus, WakeReason, wake};

/// Lets the window render before the first sync.
const FIRST_SYNC_DELAY: Duration = Duration::from_secs(2);
/// The most `shutdown` waits for the background work to stop.
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(2);

impl CoreService {
    /// Starts the background work: the periodic full sync, push for every
    /// account and the resume detector. Call once, inside a Tokio runtime;
    /// later calls do nothing.
    pub fn start_background(self: &Arc<Self>) {
        let Ok(runtime) = Handle::try_current() else {
            tracing::error!("background work needs a Tokio runtime; not started");
            return;
        };
        let accounts = match self.store.list_accounts() {
            Ok(accounts) => accounts,
            Err(e) => {
                tracing::error!("listing accounts failed; background work not started: {e}");
                return;
            }
        };
        {
            let mut state = self.push.state.lock().unwrap();
            if state.runtime.is_some() || state.shut_down {
                return;
            }
            state.runtime = Some(runtime.clone());
            let _ = self.push.service.set(Arc::downgrade(self));

            let periodic = runtime.spawn(periodic_sync(
                Arc::downgrade(self),
                self.push.root.child_token(),
                self.push.online.subscribe(),
            ));
            let weak = Arc::downgrade(self);
            let resume = runtime.spawn(wake::watch_for_resume(
                SystemTime::now,
                self.push.root.child_token(),
                move |asleep| {
                    tracing::info!("woke up after about {} minutes", asleep.as_secs() / 60);
                    if let Some(service) = weak.upgrade() {
                        service.wake(WakeReason::Resume);
                    }
                },
            ));
            state.tasks.extend([periodic, resume]);
        }
        for account in accounts {
            self.start_push(&account.id);
        }
    }

    /// Restarts an account's push, e.g. after its credentials or settings
    /// changed. Stops it when the account is gone.
    pub async fn restart_push(&self, account_id: &str) {
        self.push.stop(Some(&[account_id.to_owned()])).await;
        self.start_push(account_id);
    }

    /// Restarts every account's push.
    pub(super) async fn restart_all_push(&self) {
        self.push.stop(None).await;
        let Ok(accounts) = self.store.list_accounts() else {
            return;
        };
        for account in accounts {
            self.start_push(&account.id);
        }
    }

    fn start_push(&self, account_id: &str) {
        let mut state = self.push.state.lock().unwrap();
        let (Some(runtime), false) = (state.runtime.clone(), state.shut_down) else {
            return;
        };
        let Some(weak) = self.push.service.get().cloned() else {
            return;
        };
        let Ok(account) = self.store.get_account(account_id) else {
            self.push.forget_status(account_id);
            return;
        };
        let cancel = self.push.root.child_token();
        let listener = cancel.child_token();
        let queue = SyncQueue::default();
        let runner: Arc<dyn SyncRunner> = Arc::new(ServiceRunner {
            service: weak.clone(),
            account_id: account.id.clone(),
        });
        let worker = runtime.spawn(
            queue
                .clone()
                .run(account.id.clone(), runner, cancel.clone()),
        );
        let supervisor = runtime.spawn(run_account(
            weak,
            account.id.clone(),
            account.kind,
            queue.clone(),
            listener.clone(),
            self.push.wake.subscribe(),
            self.push.online.subscribe(),
        ));
        let previous = state.accounts.insert(
            account.id.clone(),
            AccountPush {
                cancel,
                listener,
                queue,
                tasks: vec![worker, supervisor],
            },
        );
        // Two restarts raced; the newer one wins.
        if let Some(previous) = previous {
            previous.cancel.cancel();
        }
    }

    /// A sync found the account's credentials rejected: stop listening and
    /// polling until it is signed in again ([`Self::restart_push`]), rather
    /// than asking the server over and over. Full syncs still run.
    pub(super) fn park_push(&self, account_id: &str) {
        let parked = {
            let state = self.push.state.lock().unwrap();
            state
                .accounts
                .get(account_id)
                .filter(|push| !push.listener.is_cancelled())
                .map(|push| push.listener.cancel())
                .is_some()
        };
        if !parked {
            return;
        }
        tracing::info!(
            "push for account {account_id}: the server rejected the credentials; \
             waiting for the account to be signed in again"
        );
        let mode = self
            .push
            .status(account_id)
            .map_or(PushMode::Off, |status| status.mode);
        self.push.set_status(
            self.sink.as_ref(),
            account_id,
            mode,
            PushHealth::AuthRequired,
        );
    }

    /// The network came or went (from the desktop's network monitor). While
    /// offline, push pauses and the periodic sync skips its turns; coming
    /// back wakes everything up.
    pub fn network_changed(&self, available: bool) {
        let was_available = self.push.online.send_replace(available);
        match (was_available, available) {
            (false, true) => {
                tracing::info!("network is back");
                self.wake(WakeReason::Network);
            }
            (true, false) => tracing::info!("network is gone; pausing push"),
            _ => {}
        }
    }

    /// Looks again at once: every account reconnects with its backoff reset,
    /// then gets a full sync (after any sync in progress).
    pub fn wake(&self, reason: WakeReason) {
        tracing::debug!("waking up push ({reason:?})");
        self.push.wake.send_modify(|n| *n = n.wrapping_add(1));
        self.push.request(&SyncScope::full());
    }

    /// Stops all background work: cancels syncs, closes push connections
    /// (logging out of IMAP) and waits up to two seconds for that. Nothing
    /// starts again afterwards.
    pub async fn shutdown(&self) {
        let tasks = {
            let mut state = self.push.state.lock().unwrap();
            state.shut_down = true;
            let mut tasks: Vec<_> = state.tasks.drain(..).collect();
            for (_, push) in state.accounts.drain() {
                tasks.extend(push.tasks);
            }
            tasks
        };
        self.push.root.cancel();
        join_within(tasks, SHUTDOWN_LIMIT).await;
    }

    /// The push status of every account that has one.
    pub fn push_statuses(&self) -> Result<Vec<PushStatus>> {
        Ok(self
            .store
            .list_accounts()?
            .into_iter()
            .filter_map(|account| self.push.status(&account.id))
            .collect())
    }

    /// How the account hears about changes; `None` when instant updates
    /// are off.
    async fn push_source(&self, account_id: &str) -> Result<Option<PushSource>> {
        if !self.store.get_settings()?.instant_push {
            return Ok(None);
        }
        let account = self.store.get_account(account_id)?;
        self.connector.push_source(&account).await.map(Some)
    }

    /// Syncs every account in full, each after any sync it has in progress.
    async fn sync_all_waiting(&self) {
        let accounts = match self.store.list_accounts() {
            Ok(accounts) => accounts,
            Err(e) => {
                tracing::error!("listing accounts for sync failed: {e}");
                return;
            }
        };
        for account in accounts {
            if let Err(e) = self.sync_account_waiting(&account.id).await {
                super::sync::log_sync_failure(&account.id, &e);
            }
        }
    }
}

/// Syncs everything every `sync_interval_minutes`, skipping turns while
/// offline.
async fn periodic_sync(
    service: Weak<CoreService>,
    cancel: CancellationToken,
    online: watch::Receiver<bool>,
) {
    tokio::select! {
        _ = cancel.cancelled() => return,
        _ = tokio::time::sleep(FIRST_SYNC_DELAY) => {}
    }
    loop {
        let Some(service) = service.upgrade() else {
            return;
        };
        if *online.borrow() {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = service.sync_all_waiting() => {}
            }
        }
        let minutes = service
            .get_settings()
            .map(|s| s.sync_interval_minutes)
            .unwrap_or(5)
            .max(1);
        drop(service);
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_secs(u64::from(minutes) * 60)) => {}
        }
    }
}

/// One account's push: finds out how it hears about changes, then
/// supervises that until cancelled.
async fn run_account(
    service: Weak<CoreService>,
    account_id: String,
    kind: AccountKind,
    queue: SyncQueue,
    cancel: CancellationToken,
    wake: watch::Receiver<u64>,
    online: watch::Receiver<bool>,
) {
    let status: StatusFn = {
        let service = service.clone();
        let account_id = account_id.clone();
        let cancel = cancel.clone();
        Arc::new(move |mode, health| {
            // Once stopped or parked, the status belongs to whoever did it.
            if cancel.is_cancelled() {
                return;
            }
            if let Some(service) = service.upgrade() {
                service
                    .push
                    .set_status(service.sink.as_ref(), &account_id, mode, health);
            }
        })
    };
    // Shown until the source is known.
    let planned = match kind {
        AccountKind::Imap => PushMode::ImapIdle,
        AccountKind::Microsoft => PushMode::Polling {
            every_secs: MICROSOFT_POLL.as_secs() as u32,
        },
    };
    let mut backoff = Backoff::new();
    let source = loop {
        let Some(strong) = service.upgrade() else {
            return;
        };
        let result = tokio::select! {
            _ = cancel.cancelled() => return,
            result = strong.push_source(&account_id) => result,
        };
        drop(strong);
        match result {
            Ok(Some(source)) => break source,
            Ok(None) => {
                status(PushMode::Off, PushHealth::Live);
                return;
            }
            Err(e) if e.code == ErrorCode::NotFound => return,
            Err(e) if e.code == ErrorCode::AuthRequired => {
                tracing::info!("push for account {account_id}: credentials are missing");
                status(planned, PushHealth::AuthRequired);
                return;
            }
            Err(e) => {
                let delay = backoff.next_delay();
                tracing::warn!(
                    "push for account {account_id}: could not start ({e}); retrying in {}s",
                    delay.as_secs()
                );
                status(planned.clone(), PushHealth::Degraded { reason: e.message });
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    };
    let events: EventFn = Arc::new(move |event| queue.push(event));
    let supervisor = Supervisor::new(account_id, events, status, cancel, wake, online);
    match source {
        PushSource::Watch(watcher) => supervisor.watch(watcher).await,
        PushSource::Poll(every) => supervisor.poll(every).await,
    }
}
