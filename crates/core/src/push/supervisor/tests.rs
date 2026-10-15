//! The supervisor with a scripted watcher, on paused time.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use super::*;
use crate::error::Error;

/// What one connection attempt of the fake watcher does.
enum Step {
    /// Fails at once.
    Fail(fn() -> EndReason),
    /// Connects, then ends after the given time.
    Connect(Duration, fn() -> EndReason),
}

/// Follows its steps, then connects and stays connected.
#[derive(Default)]
struct FakeWatcher {
    steps: Mutex<VecDeque<Step>>,
    starts: Mutex<Vec<Instant>>,
    cancelled: AtomicUsize,
}

impl FakeWatcher {
    fn new(steps: impl IntoIterator<Item = Step>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into_iter().collect()),
            ..Self::default()
        })
    }

    fn attempts(&self) -> usize {
        self.starts.lock().unwrap().len()
    }

    /// Time between the starts of consecutive attempts.
    fn gaps(&self) -> Vec<Duration> {
        let starts = self.starts.lock().unwrap();
        starts.windows(2).map(|w| w[1] - w[0]).collect()
    }
}

#[async_trait]
impl MailWatcher for FakeWatcher {
    async fn watch(&self, sink: &WatchSink, cancel: &CancellationToken) -> EndReason {
        self.starts.lock().unwrap().push(Instant::now());
        let step = self.steps.lock().unwrap().pop_front();
        let (stay, end) = match step {
            Some(Step::Fail(end)) => return end(),
            Some(Step::Connect(stay, end)) => (Some(stay), end),
            None => (None, (|| EndReason::Cancelled) as fn() -> EndReason),
        };
        sink.connected(PushMode::ImapIdle);
        sink.changed(PushEvent::CatchUp);
        tokio::select! {
            _ = cancel.cancelled() => {
                self.cancelled.fetch_add(1, Ordering::SeqCst);
                EndReason::Cancelled
            }
            _ = sleep_until(stay.map(|d| Instant::now() + d)) => end(),
        }
    }
}

fn failed() -> EndReason {
    EndReason::Failed(Error::network(
        "The connection to the IMAP server was lost.",
    ))
}

fn busy() -> EndReason {
    EndReason::TooManyConnections(Error::network("Too many connections."))
}

/// A running supervisor and what it reported.
struct Run {
    cancel: CancellationToken,
    wake: watch::Sender<u64>,
    online: watch::Sender<bool>,
    statuses: Arc<Mutex<Vec<(PushMode, PushHealth)>>>,
    /// When each event arrived.
    events: Arc<Mutex<Vec<(Instant, PushEvent)>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Run {
    fn start(
        online: bool,
        backoff: Backoff,
        run: impl FnOnce(Supervisor) -> futures::future::BoxFuture<'static, ()>,
    ) -> Self {
        let cancel = CancellationToken::new();
        let (wake, wake_rx) = watch::channel(0);
        let (online, online_rx) = watch::channel(online);
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let supervisor = Supervisor::new(
            "acct",
            {
                let events = events.clone();
                Arc::new(move |event| events.lock().unwrap().push((Instant::now(), event)))
            },
            {
                let statuses = statuses.clone();
                Arc::new(move |mode, health| statuses.lock().unwrap().push((mode, health)))
            },
            cancel.clone(),
            wake_rx,
            online_rx,
        )
        .with_backoff(backoff);
        let task = tokio::spawn(run(supervisor));
        Self {
            cancel,
            wake,
            online,
            statuses,
            events,
            task,
        }
    }

    fn watching(watcher: &Arc<FakeWatcher>, online: bool, backoff: Backoff) -> Self {
        let watcher: Arc<dyn MailWatcher> = watcher.clone();
        Self::start(online, backoff, move |s| Box::pin(s.watch(watcher)))
    }

    fn last_status(&self) -> (PushMode, PushHealth) {
        self.statuses.lock().unwrap().last().cloned().unwrap()
    }

    /// When the Inbox was polled, relative to `origin`.
    fn poll_times(&self, origin: Instant) -> Vec<Duration> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| *e == PushEvent::InboxChanged)
            .map(|(t, _)| *t - origin)
            .collect()
    }

    /// The same in whole seconds.
    fn polls_since(&self, origin: Instant) -> Vec<u64> {
        self.poll_times(origin)
            .iter()
            .map(Duration::as_secs)
            .collect()
    }

    fn catch_ups(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| *e == PushEvent::CatchUp)
            .count()
    }

    async fn stop(self) {
        self.cancel.cancel();
        tokio::time::timeout(Duration::from_millis(10), self.task)
            .await
            .expect("stops at once")
            .unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn failures_back_off_with_full_jitter() {
    let watcher = FakeWatcher::new((0..8).map(|_| Step::Fail(failed)));
    let run = Run::watching(&watcher, true, Backoff::seeded(42));
    tokio::time::sleep(Duration::from_secs(3600)).await;
    let mut expected = Backoff::seeded(42);
    let expected: Vec<Duration> = (0..8).map(|_| expected.next_delay()).collect();
    let gaps = watcher.gaps();
    assert_eq!(
        &gaps[..8],
        &expected[..],
        "the attempt after each failure waits its delay"
    );
    // Long waits poll the Inbox every minute, short ones don't.
    let origin = watcher.starts.lock().unwrap()[0];
    let polls = run.poll_times(origin);
    let mut start = Duration::ZERO;
    for delay in &expected {
        let end = start + *delay;
        let during = polls.iter().filter(|t| **t >= start && **t < end).count();
        if *delay > POLL_WHILE_WAITING_OVER {
            let every_minute = delay.as_millis().div_ceil(60_000) as usize;
            assert_eq!(during, every_minute, "{delay:?} {polls:?}");
        } else {
            assert_eq!(during, 0, "{delay:?} {polls:?}");
        }
        start = end;
    }
    assert!(polls.iter().all(|t| *t < start), "no polls once connected");
    assert_eq!(run.last_status(), (PushMode::ImapIdle, PushHealth::Live));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_stable_connection_resets_the_backoff() {
    let steps = (0..6)
        .map(|_| Step::Fail(failed))
        .chain([Step::Connect(Duration::from_secs(121), failed)]);
    let watcher = FakeWatcher::new(steps);
    let run = Run::watching(&watcher, true, Backoff::seeded(9));
    tokio::time::sleep(Duration::from_secs(3600)).await;
    let gaps = watcher.gaps();
    // The connection lasted 121 s; the next attempt came within a second.
    let after_stable = gaps[6] - Duration::from_secs(121);
    assert!(after_stable <= Duration::from_secs(1), "{gaps:?}");
    assert_eq!(watcher.attempts(), 8);
    assert_eq!(run.catch_ups(), 2);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn rejected_credentials_park_the_supervisor() {
    let watcher = FakeWatcher::new([Step::Fail(|| EndReason::AuthRequired)]);
    let run = Run::watching(&watcher, true, Backoff::seeded(1));
    tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
    assert_eq!(watcher.attempts(), 1);
    assert_eq!(run.last_status().1, PushHealth::AuthRequired);
    // Not even a wake retries; restarting the account's push does.
    run.wake.send_modify(|n| *n += 1);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(watcher.attempts(), 1);
    assert!(run.events.lock().unwrap().is_empty(), "no polls either");
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn servers_without_idle_are_polled_every_two_minutes() {
    let watcher = FakeWatcher::new([Step::Fail(|| EndReason::Unsupported)]);
    let run = Run::watching(&watcher, true, Backoff::seeded(1));
    let origin = Instant::now();
    tokio::time::sleep(Duration::from_secs(500)).await;
    assert_eq!(run.polls_since(origin), [0, 120, 240, 360, 480]);
    assert_eq!(
        run.last_status(),
        (
            PushMode::Polling { every_secs: 120 },
            PushHealth::Unsupported
        )
    );
    assert_eq!(watcher.attempts(), 1);
    // A wake tries the watcher again (it may have been a network problem).
    run.wake.send_modify(|n| *n += 1);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(watcher.attempts(), 2);
    assert_eq!(run.last_status(), (PushMode::ImapIdle, PushHealth::Live));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_busy_server_gets_a_long_wait_with_polling() {
    let watcher = FakeWatcher::new([Step::Fail(busy)]);
    let run = Run::watching(&watcher, true, Backoff::seeded(1));
    let origin = Instant::now();
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(watcher.attempts(), 2);
    let gap = watcher.gaps()[0];
    assert!(gap >= BUSY_WAIT, "{gap:?}");
    let polls = run.polls_since(origin);
    assert_eq!(polls, [0, 60, 120, 180, 240]);
    assert!(
        run.statuses
            .lock()
            .unwrap()
            .iter()
            .any(
                |(mode, health)| *mode == PushMode::Polling { every_secs: 60 }
                    && matches!(health, PushHealth::Degraded { .. })
            )
    );
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn waking_up_reconnects_at_once() {
    let watcher = FakeWatcher::new((0..4).map(|_| Step::Fail(failed)));
    let run = Run::watching(&watcher, true, Backoff::seeded(3));
    // Let the backoff grow, then sit connected.
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(watcher.attempts(), 5);
    run.wake.send_modify(|n| *n += 1);
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(watcher.attempts(), 6);
    assert_eq!(watcher.cancelled.load(Ordering::SeqCst), 1);
    let gaps = watcher.gaps();
    let connected_at: Duration = gaps[..4].iter().sum();
    assert_eq!(gaps[4], Duration::from_secs(600) - connected_at);
    assert_eq!(run.catch_ups(), 2);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn waking_up_cuts_a_backoff_wait_short() {
    let watcher = FakeWatcher::new((0..9).map(|_| Step::Fail(failed)));
    let run = Run::watching(&watcher, true, Backoff::seeded(5));
    let mut replica = Backoff::seeded(5);
    let delays: Vec<Duration> = (0..9).map(|_| replica.next_delay()).collect();
    let until_ninth: Duration = delays[..8].iter().sum();
    tokio::time::sleep(until_ninth + Duration::from_millis(1)).await;
    assert_eq!(watcher.attempts(), 9);
    assert!(delays[8] > Duration::from_secs(2), "{delays:?}");
    run.wake.send_modify(|n| *n += 1);
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(watcher.attempts(), 10);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn offline_pauses_until_the_network_returns() {
    let watcher = FakeWatcher::new([]);
    let run = Run::watching(&watcher, false, Backoff::seeded(1));
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(watcher.attempts(), 0);
    assert!(matches!(
        run.last_status(),
        (PushMode::ImapIdle, PushHealth::Degraded { .. })
    ));

    run.online.send_replace(true);
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(watcher.attempts(), 1);
    assert_eq!(run.last_status().1, PushHealth::Live);

    // Going offline ends the connection; nothing is tried meanwhile.
    run.online.send_replace(false);
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(watcher.cancelled.load(Ordering::SeqCst), 1);
    assert_eq!(watcher.attempts(), 1);
    run.online.send_replace(true);
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(watcher.attempts(), 2);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn polling_checks_on_its_timer_and_when_woken() {
    let run = Run::start(true, Backoff::seeded(1), |s| {
        Box::pin(s.poll(MICROSOFT_POLL))
    });
    let origin = Instant::now();
    tokio::time::sleep(Duration::from_secs(70)).await;
    assert_eq!(run.polls_since(origin), [0, 30, 60]);
    assert_eq!(
        run.last_status(),
        (PushMode::Polling { every_secs: 30 }, PushHealth::Live)
    );
    run.wake.send_modify(|n| *n += 1);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(run.polls_since(origin), [0, 30, 60, 70]);
    run.online.send_replace(false);
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert_eq!(run.polls_since(origin).len(), 4);
    run.stop().await;
}
