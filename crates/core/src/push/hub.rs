//! The push state the service keeps: one entry per account (its sync queue
//! and the tasks serving it), the latest status of each, and the shared
//! wake and network signals. `service::background` drives it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, Weak};
use std::time::Duration;

use chrono::Utc;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{PushHealth, PushMode, PushStatus};
use crate::service::targeted::SyncQueue;
use crate::service::{CoreService, EventSink};

/// How long stopping an account's push may take before its tasks are left
/// to finish on their own.
pub(crate) const STOP_LIMIT: Duration = Duration::from_secs(3);

pub(crate) struct PushHub {
    pub(crate) state: Mutex<HubState>,
    statuses: Mutex<HashMap<String, PushStatus>>,
    /// Bumped to make every supervisor reconnect at once.
    pub(crate) wake: watch::Sender<u64>,
    pub(crate) online: watch::Sender<bool>,
    /// Parent of every task's token; cancelled on shutdown.
    pub(crate) root: CancellationToken,
    pub(crate) service: OnceLock<Weak<CoreService>>,
}

#[derive(Default)]
pub(crate) struct HubState {
    /// Set once the background work started.
    pub(crate) runtime: Option<Handle>,
    pub(crate) accounts: HashMap<String, AccountPush>,
    /// The periodic sync and the resume detector.
    pub(crate) tasks: Vec<JoinHandle<()>>,
    pub(crate) shut_down: bool,
}

/// One account's push.
pub(crate) struct AccountPush {
    pub(crate) cancel: CancellationToken,
    /// The supervisor's token, a child of `cancel`: parking it stops
    /// listening while the queue keeps serving full syncs.
    pub(crate) listener: CancellationToken,
    pub(crate) queue: SyncQueue,
    pub(crate) tasks: Vec<JoinHandle<()>>,
}

impl Default for PushHub {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            statuses: Mutex::default(),
            wake: watch::Sender::new(0),
            online: watch::Sender::new(true),
            root: CancellationToken::new(),
            service: OnceLock::new(),
        }
    }
}

impl PushHub {
    /// Records an account's status and tells the UI when it changed.
    pub(crate) fn set_status(
        &self,
        sink: &dyn EventSink,
        account_id: &str,
        mode: PushMode,
        health: PushHealth,
    ) {
        let status = {
            let mut statuses = self.statuses.lock().unwrap();
            if statuses
                .get(account_id)
                .is_some_and(|s| s.mode == mode && s.health == health)
            {
                return;
            }
            let status = PushStatus {
                account_id: account_id.to_owned(),
                mode,
                health,
                detail: None,
                since: Utc::now(),
            };
            statuses.insert(account_id.to_owned(), status.clone());
            status
        };
        sink.push_status(&status);
    }

    pub(crate) fn status(&self, account_id: &str) -> Option<PushStatus> {
        self.statuses.lock().unwrap().get(account_id).cloned()
    }

    pub(crate) fn forget_status(&self, account_id: &str) {
        self.statuses.lock().unwrap().remove(account_id);
    }

    /// Stops the given accounts' push (all when `None`): cancels their
    /// tasks and waits for them, briefly.
    pub(crate) async fn stop(&self, account_ids: Option<&[String]>) {
        let stopped: Vec<AccountPush> = {
            let mut state = self.state.lock().unwrap();
            match account_ids {
                Some(ids) => ids
                    .iter()
                    .filter_map(|id| state.accounts.remove(id))
                    .collect(),
                None => state.accounts.drain().map(|(_, push)| push).collect(),
            }
        };
        for push in &stopped {
            push.cancel.cancel();
        }
        let tasks = stopped.into_iter().flat_map(|push| push.tasks);
        join_within(tasks, STOP_LIMIT).await;
    }

    /// Asks every account's queue for `scope`.
    pub(crate) fn request(&self, scope: &crate::service::SyncScope) {
        let state = self.state.lock().unwrap();
        for push in state.accounts.values() {
            push.queue.push(scope.clone());
        }
    }
}

/// Waits for `tasks` to finish, at most `limit`.
pub(crate) async fn join_within(tasks: impl IntoIterator<Item = JoinHandle<()>>, limit: Duration) {
    let all = futures::future::join_all(tasks);
    if tokio::time::timeout(limit, all).await.is_err() {
        tracing::debug!("background tasks did not stop within {limit:?}");
    }
}
