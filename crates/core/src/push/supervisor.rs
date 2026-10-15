//! One account's push: keeps its watcher connected, or polls, and says how
//! it is going.
//!
//! | Watcher ends with    | Then                                                  |
//! | -------------------- | ----------------------------------------------------- |
//! | `AuthRequired`       | Park until the account is restarted (signed in again) |
//! | `Unsupported`        | Poll the Inbox every 2 minutes                        |
//! | `TooManyConnections` | Wait at least 5 minutes, polling every minute         |
//! | `Failed`             | Back off; poll every minute while waits exceed 30 s   |
//!
//! Waking up (resume, network back) reconnects at once with the backoff
//! reset; going offline pauses everything until the network returns.

use std::future::pending;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use super::backoff::Backoff;
use super::{EndReason, MailWatcher, PushEvent, PushHealth, PushMode, WatchSignal, WatchSink};

/// How often Microsoft accounts check the Inbox without the push service.
pub const MICROSOFT_POLL: Duration = Duration::from_secs(30);
/// How often the Inbox is checked when the server has no IDLE.
pub const UNSUPPORTED_POLL: Duration = Duration::from_secs(120);
/// How often it is checked while reconnecting takes a while.
pub const DEGRADED_POLL: Duration = Duration::from_secs(60);
/// Waits longer than this before reconnecting also poll the Inbox.
pub const POLL_WHILE_WAITING_OVER: Duration = Duration::from_secs(30);
/// The least wait after the server refused another connection.
pub const BUSY_WAIT: Duration = Duration::from_secs(300);

/// Receives the changes the supervisor hears of (and its polls).
pub type EventFn = Arc<dyn Fn(PushEvent) + Send + Sync>;
/// Receives the supervisor's mode and health whenever it reports them.
pub type StatusFn = Arc<dyn Fn(PushMode, PushHealth) + Send + Sync>;

pub struct Supervisor {
    /// For logs.
    account_id: String,
    events: EventFn,
    status: StatusFn,
    cancel: CancellationToken,
    /// Bumped to reconnect at once (resume, network change).
    wake: watch::Receiver<u64>,
    online: watch::Receiver<bool>,
    backoff: Backoff,
}

/// Why a pause ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pause {
    Elapsed,
    Woken,
    Offline,
    Stopped,
}

/// Why the supervisor ended a watcher's connection itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interrupt {
    Woken,
    Offline,
}

/// What the watcher said about its connection.
#[derive(Default)]
struct Connection {
    since: Option<Instant>,
    mode: Option<PushMode>,
}

impl Supervisor {
    pub fn new(
        account_id: impl Into<String>,
        events: EventFn,
        status: StatusFn,
        cancel: CancellationToken,
        wake: watch::Receiver<u64>,
        online: watch::Receiver<bool>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            events,
            status,
            cancel,
            wake,
            online,
            backoff: Backoff::new(),
        }
    }

    /// Uses `backoff` instead of a randomly seeded one.
    pub fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    fn report(&self, mode: PushMode, health: PushHealth) {
        (self.status)(mode, health);
    }

    /// Keeps `watcher` connected until cancelled.
    pub async fn watch(mut self, watcher: Arc<dyn MailWatcher>) {
        let mut mode = PushMode::ImapIdle;
        loop {
            if !self.wait_online(&mode).await {
                return;
            }
            // Wakes from before this attempt are moot: it starts afresh.
            self.wake.borrow_and_update();
            self.report(mode.clone(), PushHealth::Connecting);
            let attempt = self.cancel.child_token();
            let connection = Arc::new(Mutex::new(Connection::default()));
            let sink = {
                let connection = Arc::clone(&connection);
                let events = Arc::clone(&self.events);
                let status = Arc::clone(&self.status);
                WatchSink::new(move |signal| match signal {
                    WatchSignal::Connected(mode) => {
                        *connection.lock().unwrap() = Connection {
                            since: Some(Instant::now()),
                            mode: Some(mode.clone()),
                        };
                        status(mode, PushHealth::Live);
                    }
                    WatchSignal::Changed(event) => events(event),
                })
            };

            let (end, interrupt) = {
                let run = watcher.watch(&sink, &attempt);
                tokio::pin!(run);
                let mut interrupt = None;
                loop {
                    tokio::select! {
                        end = &mut run => break (end, interrupt),
                        _ = woken(&mut self.wake), if interrupt.is_none() => {
                            interrupt = Some(Interrupt::Woken);
                            attempt.cancel();
                        }
                        _ = went_offline(&mut self.online), if interrupt.is_none() => {
                            interrupt = Some(Interrupt::Offline);
                            attempt.cancel();
                        }
                    }
                }
            };
            let connection = std::mem::take(&mut *connection.lock().unwrap());
            if let Some(connected) = connection.mode {
                mode = connected;
            }
            let up_for = connection.since.map(|t| t.elapsed()).unwrap_or_default();
            self.backoff.connection_ended(up_for);
            if self.cancel.is_cancelled() {
                return;
            }
            match interrupt {
                Some(Interrupt::Woken) => {
                    tracing::debug!("push for account {}: reconnecting", self.account_id);
                    self.backoff.reset();
                    continue;
                }
                // `wait_online` reports it.
                Some(Interrupt::Offline) => continue,
                None => {}
            }

            let pause = match end {
                EndReason::AuthRequired => {
                    tracing::info!(
                        "push for account {}: the server rejected the credentials; \
                         waiting for the account to be signed in again",
                        self.account_id
                    );
                    self.report(mode.clone(), PushHealth::AuthRequired);
                    self.cancel.cancelled().await;
                    return;
                }
                EndReason::Unsupported => {
                    tracing::info!(
                        "push for account {}: the server has no IDLE; polling the Inbox",
                        self.account_id
                    );
                    self.report(polling(UNSUPPORTED_POLL), PushHealth::Unsupported);
                    self.pause(None, Some(UNSUPPORTED_POLL)).await
                }
                EndReason::TooManyConnections(e) => {
                    let delay = self.backoff.next_delay().max(BUSY_WAIT);
                    tracing::warn!(
                        "push for account {}: {e} Polling the Inbox; trying again in {}s",
                        self.account_id,
                        delay.as_secs()
                    );
                    self.report(
                        polling(DEGRADED_POLL),
                        PushHealth::Degraded { reason: e.message },
                    );
                    self.pause(Some(delay), Some(DEGRADED_POLL)).await
                }
                EndReason::Failed(e) => self.retry_later(&mode, e.message).await,
                // The watcher stopped by itself; treat it like a failure so a
                // misbehaving watcher can't make this loop spin.
                EndReason::Cancelled => {
                    self.retry_later(&mode, "The connection was closed.".to_owned())
                        .await
                }
            };
            if pause == Pause::Stopped {
                return;
            }
        }
    }

    /// Waits out the backoff after a failure, polling the Inbox if that
    /// takes long.
    async fn retry_later(&mut self, mode: &PushMode, reason: String) -> Pause {
        let delay = self.backoff.next_delay();
        tracing::info!(
            "push for account {}: {reason} Reconnecting in {:.1}s",
            self.account_id,
            delay.as_secs_f32()
        );
        if delay > POLL_WHILE_WAITING_OVER {
            self.report(polling(DEGRADED_POLL), PushHealth::Degraded { reason });
            self.pause(Some(delay), Some(DEGRADED_POLL)).await
        } else {
            self.report(mode.clone(), PushHealth::Degraded { reason });
            self.pause(Some(delay), None).await
        }
    }

    /// Checks the Inbox every `every` until cancelled; at once when started,
    /// woken or back online.
    pub async fn poll(mut self, every: Duration) {
        // A zero interval would be a busy loop (and panics in `interval`).
        let every = every.max(Duration::from_secs(1));
        let mode = polling(every);
        loop {
            if !self.wait_online(&mode).await {
                return;
            }
            self.wake.borrow_and_update();
            self.report(mode.clone(), PushHealth::Live);
            if self.pause(None, Some(every)).await == Pause::Stopped {
                return;
            }
        }
    }

    /// Returns at once when online; otherwise reports it and waits. False
    /// when cancelled meanwhile.
    async fn wait_online(&mut self, mode: &PushMode) -> bool {
        if *self.online.borrow_and_update() {
            return !self.cancel.is_cancelled();
        }
        tracing::debug!("push for account {}: offline, pausing", self.account_id);
        self.report(
            mode.clone(),
            PushHealth::Degraded {
                reason: "No network connection.".to_owned(),
            },
        );
        tokio::select! {
            _ = self.cancel.cancelled() => false,
            online = self.online.wait_for(|online| *online) => online.is_ok(),
        }
    }

    /// Waits for `delay` (forever if `None`), checking the Inbox at once and
    /// then every `poll` if set.
    async fn pause(&mut self, delay: Option<Duration>, poll: Option<Duration>) -> Pause {
        let deadline = delay.map(|d| Instant::now() + d);
        let mut ticker = poll.map(|every| {
            let mut ticker = tokio::time::interval(every);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            ticker
        });
        loop {
            tokio::select! {
                // In this order when several are ready: the wait ending
                // beats a poll due at the same moment.
                biased;
                _ = self.cancel.cancelled() => return Pause::Stopped,
                _ = woken(&mut self.wake) => {
                    self.backoff.reset();
                    return Pause::Woken;
                }
                _ = went_offline(&mut self.online) => return Pause::Offline,
                _ = sleep_until(deadline) => return Pause::Elapsed,
                _ = tick(&mut ticker) => (self.events)(PushEvent::InboxChanged),
            }
        }
    }
}

fn polling(every: Duration) -> PushMode {
    PushMode::Polling {
        every_secs: every.as_secs() as u32,
    }
}

/// Resolves when the wake counter changes; never if its sender is gone.
async fn woken(wake: &mut watch::Receiver<u64>) {
    if wake.changed().await.is_err() {
        pending::<()>().await;
    }
}

/// Resolves while offline; never if the sender is gone.
async fn went_offline(online: &mut watch::Receiver<bool>) {
    if online.wait_for(|online| !*online).await.is_err() {
        pending::<()>().await;
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => pending().await,
    }
}

async fn tick(ticker: &mut Option<tokio::time::Interval>) {
    match ticker {
        Some(ticker) => {
            ticker.tick().await;
        }
        None => pending().await,
    }
}

#[cfg(test)]
mod tests;
