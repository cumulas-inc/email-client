//! Targeted syncs: just what push said changed, usually the Inbox.
//!
//! Each account has a [`SyncQueue`] whose worker runs one sync at a time.
//! Requests are merged into a [`SyncScope`] while they wait, so a burst of
//! notifications (Graph sends created+updated pairs, IMAP sends EXISTS and
//! FETCH) becomes one sync, and a change arriving during a sync gives exactly
//! one follow-up. Syncs are spaced out, more so when changes never stop.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::CoreService;
use crate::error::{ErrorCode, Result};
use crate::model::{DataKind, Folder, FolderRole};
use crate::push::PushEvent;

/// Requests arriving this soon after the first one join its sync.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(750);
/// The least time between the starts of two syncs.
pub(crate) const SPACING: Duration = Duration::from_secs(3);
/// This many syncs within [`STORM_WINDOW`] (as many as [`SPACING`] allows)
/// slow the queue down to [`STORM_SPACING`] for [`STORM_DURATION`].
pub(crate) const STORM_SYNCS: usize = 20;
pub(crate) const STORM_WINDOW: Duration = Duration::from_secs(60);
pub(crate) const STORM_SPACING: Duration = Duration::from_secs(15);
pub(crate) const STORM_DURATION: Duration = Duration::from_secs(300);

/// What a targeted sync covers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncScope {
    pub inbox: bool,
    /// Remote ids of other folders that changed.
    pub folders: BTreeSet<String>,
    /// List the folders again (created, renamed or deleted).
    pub folder_list: bool,
    pub calendar: bool,
    /// Everything, like the periodic sync. Absorbs the rest.
    pub full: bool,
}

impl SyncScope {
    pub fn inbox() -> Self {
        Self {
            inbox: true,
            ..Self::default()
        }
    }

    pub fn full() -> Self {
        Self {
            full: true,
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn merge(&mut self, other: SyncScope) {
        if self.full || other.full {
            *self = Self::full();
            return;
        }
        self.inbox |= other.inbox;
        self.folders.extend(other.folders);
        self.folder_list |= other.folder_list;
        self.calendar |= other.calendar;
    }
}

impl From<PushEvent> for SyncScope {
    fn from(event: PushEvent) -> Self {
        let mut scope = SyncScope::default();
        match event {
            // A catch-up only needs the Inbox: the periodic sync covers the
            // rest, and a resume asks for a full sync separately.
            PushEvent::InboxChanged | PushEvent::CatchUp => scope.inbox = true,
            PushEvent::FolderChanged { remote_id } => {
                scope.folders.insert(remote_id);
            }
            PushEvent::FolderListChanged => scope.folder_list = true,
            PushEvent::CalendarChanged => scope.calendar = true,
        }
        scope
    }
}

/// Runs the syncs a [`SyncQueue`] asks for.
#[async_trait]
pub(crate) trait SyncRunner: Send + Sync {
    async fn run(&self, scope: SyncScope);
}

/// Hands sync requests to one account's worker. Cheap to clone; requesting
/// never blocks.
#[derive(Clone, Default)]
pub(crate) struct SyncQueue {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    /// What the next sync covers; empty when nothing is waiting.
    pending: Mutex<SyncScope>,
    notify: Notify,
}

impl SyncQueue {
    pub(crate) fn push(&self, scope: impl Into<SyncScope>) {
        self.shared.pending.lock().unwrap().merge(scope.into());
        self.shared.notify.notify_one();
    }

    fn has_work(&self) -> bool {
        !self.shared.pending.lock().unwrap().is_empty()
    }

    /// The worker: runs requested syncs one at a time until `cancel` fires,
    /// which also abandons a sync in progress. `label` names the account in
    /// logs.
    pub(crate) async fn run(
        self,
        label: String,
        runner: Arc<dyn SyncRunner>,
        cancel: CancellationToken,
    ) {
        let mut last_start: Option<Instant> = None;
        let mut recent: VecDeque<Instant> = VecDeque::new();
        let mut calm_until: Option<Instant> = None;
        loop {
            while !self.has_work() {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = self.shared.notify.notified() => {}
                }
            }
            let storm = calm_until.is_some_and(|until| Instant::now() < until);
            let spacing = if storm { STORM_SPACING } else { SPACING };
            let mut start = Instant::now() + DEBOUNCE;
            if let Some(last) = last_start {
                start = start.max(last + spacing);
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep_until(start) => {}
            }
            let scope = std::mem::take(&mut *self.shared.pending.lock().unwrap());

            let now = Instant::now();
            last_start = Some(now);
            recent.push_back(now);
            while recent.front().is_some_and(|t| now - *t > STORM_WINDOW) {
                recent.pop_front();
            }
            if !storm && recent.len() >= STORM_SYNCS {
                tracing::info!(
                    "account {label} keeps changing; syncing it at most every {}s for {} minutes",
                    STORM_SPACING.as_secs(),
                    STORM_DURATION.as_secs() / 60
                );
                calm_until = Some(now + STORM_DURATION);
                recent.clear();
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = runner.run(scope) => {}
            }
        }
    }
}

/// Runs a queue's syncs on the service.
pub(crate) struct ServiceRunner {
    pub service: Weak<CoreService>,
    pub account_id: String,
}

#[async_trait]
impl SyncRunner for ServiceRunner {
    async fn run(&self, scope: SyncScope) {
        let Some(service) = self.service.upgrade() else {
            return;
        };
        if let Err(e) = service.sync_targets(&self.account_id, scope).await {
            match e.code {
                // Removed meanwhile.
                ErrorCode::NotFound => {}
                // The periodic sync reports it to the user.
                ErrorCode::AuthRequired => service.park_push(&self.account_id),
                // Push reconnects and catches up by itself.
                ErrorCode::Network => {
                    tracing::debug!("targeted sync of account {} failed: {e}", self.account_id);
                }
                _ => tracing::warn!("targeted sync of account {} failed: {e}", self.account_id),
            }
        }
    }
}

impl CoreService {
    /// Syncs what push reported as changed. Unlike [`Self::sync_account`] it
    /// waits for a sync of the account in progress instead of skipping, so a
    /// change is never lost. Never lists all folders unless asked to.
    pub(crate) async fn sync_targets(&self, account_id: &str, scope: SyncScope) -> Result<()> {
        if scope.full {
            return self.sync_account_waiting(account_id).await;
        }
        let lock = self.account_lock(account_id);
        let _guard = lock.lock().await;
        let account = self.store.get_account(account_id)?;
        let providers = self.providers_for(account_id).await?;

        let mut folders: Vec<Folder> = Vec::new();
        let mut list_folders = scope.folder_list;
        for remote_id in &scope.folders {
            match self.store.find_folder_by_remote_id(account_id, remote_id)? {
                Some(folder) => folders.push(folder),
                // New to us: list the folders to learn about it.
                None => list_folders = true,
            }
        }
        if list_folders {
            let listed = providers.mail.list_folders().await?;
            self.store.replace_folders(account_id, &listed)?;
            self.notify(&[DataKind::Mail], Some(account_id));
            folders.clear();
            for remote_id in &scope.folders {
                if let Some(folder) = self.store.find_folder_by_remote_id(account_id, remote_id)? {
                    folders.push(folder);
                }
            }
        }
        if scope.inbox {
            // Without an Inbox the account's first sync is yet to list the
            // folders; it syncs the Inbox itself.
            if let Some(inbox) = self
                .store
                .find_folder_by_role(account_id, FolderRole::Inbox)?
                && !folders.iter().any(|f| f.id == inbox.id)
            {
                folders.insert(0, inbox);
            }
        }
        for folder in &folders {
            self.sync_one_folder(&account, &providers, folder, true)
                .await?;
        }
        if scope.calendar {
            self.refresh_calendars(&account, &providers).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Records the scopes it runs; each sync takes `duration`.
    struct Recorder {
        runs: Mutex<Vec<(Instant, SyncScope)>>,
        duration: Duration,
        running: AtomicUsize,
    }

    impl Recorder {
        fn new(duration: Duration) -> Arc<Self> {
            Arc::new(Self {
                runs: Mutex::default(),
                duration,
                running: AtomicUsize::new(0),
            })
        }

        fn scopes(&self) -> Vec<SyncScope> {
            self.runs
                .lock()
                .unwrap()
                .iter()
                .map(|(_, s)| s.clone())
                .collect()
        }

        fn starts(&self) -> Vec<Instant> {
            self.runs.lock().unwrap().iter().map(|(t, _)| *t).collect()
        }
    }

    #[async_trait]
    impl SyncRunner for Recorder {
        async fn run(&self, scope: SyncScope) {
            assert_eq!(
                self.running.fetch_add(1, Ordering::SeqCst),
                0,
                "one at a time"
            );
            self.runs.lock().unwrap().push((Instant::now(), scope));
            tokio::time::sleep(self.duration).await;
            self.running.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn start(
        recorder: &Arc<Recorder>,
    ) -> (SyncQueue, CancellationToken, tokio::task::JoinHandle<()>) {
        let queue = SyncQueue::default();
        let cancel = CancellationToken::new();
        let task = tokio::spawn(queue.clone().run(
            "test".into(),
            Arc::clone(recorder) as Arc<dyn SyncRunner>,
            cancel.clone(),
        ));
        (queue, cancel, task)
    }

    fn folder(remote_id: &str) -> SyncScope {
        PushEvent::FolderChanged {
            remote_id: remote_id.into(),
        }
        .into()
    }

    #[test]
    fn scopes_merge_and_full_absorbs_everything() {
        let mut scope = SyncScope::from(PushEvent::InboxChanged);
        scope.merge(folder("Archive"));
        scope.merge(PushEvent::FolderListChanged.into());
        scope.merge(PushEvent::CalendarChanged.into());
        assert!(scope.inbox && scope.folder_list && scope.calendar && !scope.full);
        assert_eq!(scope.folders, BTreeSet::from(["Archive".to_owned()]));
        assert_eq!(SyncScope::from(PushEvent::CatchUp), SyncScope::inbox());

        let mut full = scope.clone();
        full.merge(SyncScope::full());
        assert_eq!(full, SyncScope::full());
        let mut after = SyncScope::full();
        after.merge(scope);
        assert_eq!(after, SyncScope::full());
        assert!(SyncScope::default().is_empty() && !after.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_pings_gives_one_sync() {
        let recorder = Recorder::new(Duration::from_millis(500));
        let (queue, cancel, task) = start(&recorder);
        let begin = Instant::now();
        for _ in 0..10 {
            queue.push(PushEvent::InboxChanged);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        queue.push(folder("Archive"));
        tokio::time::sleep(Duration::from_secs(10)).await;
        let scopes = recorder.scopes();
        assert_eq!(scopes.len(), 1, "{scopes:?}");
        assert!(scopes[0].inbox && scopes[0].folders.contains("Archive"));
        // The first sync starts once the debounce has passed.
        assert_eq!(recorder.starts()[0] - begin, DEBOUNCE);
        cancel.cancel();
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_ping_during_a_sync_gives_exactly_one_follow_up() {
        let recorder = Recorder::new(Duration::from_secs(5));
        let (queue, cancel, task) = start(&recorder);
        queue.push(PushEvent::InboxChanged);
        tokio::time::sleep(DEBOUNCE + Duration::from_millis(100)).await;
        assert_eq!(recorder.scopes().len(), 1);
        // Several pings while the first sync runs.
        queue.push(PushEvent::InboxChanged);
        queue.push(PushEvent::CalendarChanged);
        tokio::time::sleep(Duration::from_secs(1)).await;
        queue.push(PushEvent::InboxChanged);
        tokio::time::sleep(Duration::from_secs(30)).await;
        let scopes = recorder.scopes();
        assert_eq!(scopes.len(), 2, "{scopes:?}");
        assert!(scopes[1].inbox && scopes[1].calendar);
        let starts = recorder.starts();
        // Right after the first one finished (it took longer than the spacing).
        assert_eq!(starts[1] - starts[0], Duration::from_secs(5) + DEBOUNCE);
        cancel.cancel();
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn syncs_are_spaced_out() {
        let recorder = Recorder::new(Duration::from_millis(100));
        let (queue, cancel, task) = start(&recorder);
        queue.push(PushEvent::InboxChanged);
        tokio::time::sleep(DEBOUNCE + Duration::from_millis(200)).await;
        queue.push(PushEvent::InboxChanged);
        tokio::time::sleep(Duration::from_secs(10)).await;
        let starts = recorder.starts();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[1] - starts[0], SPACING);
        cancel.cancel();
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn the_storm_guard_slows_constant_changes_down() {
        let recorder = Recorder::new(Duration::from_millis(100));
        let (queue, cancel, task) = start(&recorder);
        // A change every half second for ten minutes.
        let pinger = tokio::spawn({
            let queue = queue.clone();
            async move {
                for _ in 0..1200 {
                    queue.push(PushEvent::InboxChanged);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        });
        pinger.await.unwrap();
        let starts = recorder.starts();
        let gaps: Vec<Duration> = starts.windows(2).map(|w| w[1] - w[0]).collect();
        // The first minute runs at the normal spacing...
        assert!(
            gaps[..STORM_SYNCS - 1].iter().all(|g| *g == SPACING),
            "{gaps:?}"
        );
        // ...then the guard spaces syncs out for five minutes...
        let slow = gaps.iter().position(|g| *g == STORM_SPACING).unwrap();
        assert_eq!(slow, STORM_SYNCS - 1);
        let calm = &gaps[slow..slow + 20];
        assert!(calm.iter().all(|g| *g == STORM_SPACING), "{calm:?}");
        // ...and the normal spacing returns afterwards.
        assert!(gaps[slow + 20..].contains(&SPACING));
        // About 20 + 20 + 20 + 20 syncs in ten minutes instead of 200.
        assert!(starts.len() < 100, "{}", starts.len());
        cancel.cancel();
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn full_requests_absorb_targeted_ones() {
        let recorder = Recorder::new(Duration::from_millis(100));
        let (queue, cancel, task) = start(&recorder);
        queue.push(PushEvent::InboxChanged);
        queue.push(SyncScope::full());
        queue.push(folder("Archive"));
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(recorder.scopes(), [SyncScope::full()]);
        cancel.cancel();
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_stops_the_worker_mid_sync() {
        let recorder = Recorder::new(Duration::from_secs(3600));
        let (queue, cancel, task) = start(&recorder);
        queue.push(PushEvent::InboxChanged);
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(recorder.running.load(Ordering::SeqCst), 1);
        cancel.cancel();
        tokio::time::timeout(Duration::from_millis(10), task)
            .await
            .expect("stops at once")
            .unwrap();
    }
}
