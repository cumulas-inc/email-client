//! The reminder scheduler and what the UI can do with reminders.
//!
//! The scheduler runs [`crate::reminders::plan`] when the calendar or tasks
//! change, when the settings change, when the computer wakes up, and when a
//! reminder is due: it sleeps until the next one, a minute at most (timers
//! stand still while the computer sleeps). Each pass saves what became of
//! the reminders that came due and tells the sink
//! ([`super::EventSink::reminders_changed`]), which shows them.
//!
//! Dismissing and snoozing a Microsoft 365 event's reminder also tells
//! Microsoft 365, so that Outlook and phones stop reminding too. When that
//! fails, the next sync of the calendar tries again.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::CoreService;
use crate::error::{Error, ErrorCode, Result};
use crate::model::{Account, AccountKind, ActiveReminder, ReminderKind, Snooze};
use crate::providers::CalendarProvider;
use crate::reminders::{
    self, KEEP_STATES, LOOKAHEAD, LOOKBACK, ReminderState, ReminderStatus, ServerAction,
    ServerReminders,
};

/// The longest the scheduler sleeps: its timer stands still while the
/// computer sleeps, so it looks at the clock at least this often.
pub(crate) const MAX_SLEEP: StdDuration = StdDuration::from_secs(60);
/// Changes come in bursts (a sync stores calendars, then events): a pass
/// waits this long after the first.
pub(crate) const SETTLE: StdDuration = StdDuration::from_millis(250);
/// The most telling Microsoft 365 about dismissals and snoozes may hold up
/// the command that made them; what's left is retried at the next sync.
const SERVER_LIMIT: StdDuration = StdDuration::from_secs(10);
/// A server report on reminders older than this no longer counts: the
/// events' own reminder times do.
const REPORT_LIFE: Duration = Duration::days(1);
/// The longest snooze.
const MAX_SNOOZE: Duration = Duration::days(30);

pub(crate) type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// The scheduler's state, kept by the service.
pub(crate) struct ReminderHub {
    /// Wakes the scheduler: something it looks at changed.
    pub(crate) poke: Arc<Notify>,
    pass: Mutex<LastPass>,
    clock: RwLock<Clock>,
}

#[derive(Default)]
struct LastPass {
    active: Vec<ActiveReminder>,
    due: HashSet<String>,
    next_wake: Option<DateTime<Utc>>,
}

impl Default for ReminderHub {
    fn default() -> Self {
        Self {
            poke: Arc::new(Notify::new()),
            pass: Mutex::default(),
            clock: RwLock::new(Arc::new(Utc::now)),
        }
    }
}

impl ReminderHub {
    pub(crate) fn now(&self) -> DateTime<Utc> {
        let clock = Arc::clone(&self.clock.read().unwrap());
        clock()
    }

    /// For tests: reminders go by `clock` rather than the wall clock.
    #[cfg(test)]
    pub(crate) fn set_clock(&self, clock: Clock) {
        *self.clock.write().unwrap() = clock;
    }

    fn next_wake(&self) -> Option<DateTime<Utc>> {
        self.pass.lock().unwrap().next_wake
    }
}

impl CoreService {
    /// The reminders due now or snoozed, as of a fresh look.
    pub fn list_active_reminders(&self) -> Result<Vec<ActiveReminder>> {
        self.reminder_pass()
    }

    /// Snoozes a due reminder: it goes off again later. Returns the
    /// reminders then active.
    pub async fn snooze_reminder(&self, key: &str, snooze: Snooze) -> Result<Vec<ActiveReminder>> {
        let now = self.reminders.now();
        let until = match snooze {
            Snooze::Minutes(0) => {
                return Err(Error::invalid_input("Snooze for at least one minute."));
            }
            Snooze::Minutes(minutes) => now + Duration::minutes(minutes.into()),
            Snooze::Until(until) => until,
        };
        if until <= now {
            return Err(Error::invalid_input("Choose a time that hasn't passed."));
        }
        if until > now + MAX_SNOOZE {
            return Err(Error::invalid_input("Snooze for 30 days at most."));
        }
        let reminder = self
            .reminder_pass()?
            .into_iter()
            .find(|r| r.key == key)
            .ok_or_else(|| Error::not_found("That reminder is no longer due."))?;
        let mut state = self.state_for(
            &reminder,
            ReminderStatus::Snoozed,
            ServerAction::Snooze,
            now,
        )?;
        state.snooze_until = Some(until);
        self.store
            .put_reminder_states(std::slice::from_ref(&state))?;
        let active = self.reminder_pass()?;
        self.tell_servers(&[state.account_id]).await;
        Ok(active)
    }

    /// Dismisses reminders; ones no longer active are passed over. Returns
    /// the reminders then active.
    pub async fn dismiss_reminders(&self, keys: &[String]) -> Result<Vec<ActiveReminder>> {
        let now = self.reminders.now();
        let active = self.reminder_pass()?;
        let mut states = Vec::new();
        for reminder in active.iter().filter(|r| keys.contains(&r.key)) {
            states.push(self.state_for(
                reminder,
                ReminderStatus::Dismissed,
                ServerAction::Dismiss,
                now,
            )?);
        }
        if states.is_empty() {
            return Ok(active);
        }
        self.store.put_reminder_states(&states)?;
        let active = self.reminder_pass()?;
        let mut accounts: Vec<String> = states.into_iter().map(|s| s.account_id).collect();
        accounts.sort();
        accounts.dedup();
        self.tell_servers(&accounts).await;
        Ok(active)
    }

    /// A state for `reminder`, with `action` pending when its server keeps
    /// reminders (an event of a Microsoft account).
    fn state_for(
        &self,
        reminder: &ActiveReminder,
        status: ReminderStatus,
        action: ServerAction,
        now: DateTime<Utc>,
    ) -> Result<ReminderState> {
        let on_server = reminder.kind == ReminderKind::Event
            && self.store.get_account(&reminder.account_id)?.kind == AccountKind::Microsoft;
        Ok(ReminderState {
            key: reminder.key.clone(),
            account_id: reminder.account_id.clone(),
            kind: reminder.kind,
            item_id: reminder.item_id.clone(),
            fire_at: reminder.fire_at,
            status,
            snooze_until: None,
            pending: on_server.then_some(action),
            updated_at: now,
        })
    }

    /// Tells these accounts' servers what's pending, briefly: what's left
    /// is retried at the next sync.
    async fn tell_servers(&self, account_ids: &[String]) {
        let tell = async {
            for account_id in account_ids {
                let calendar = match self.providers_for(account_id).await {
                    Ok(providers) => providers.calendar,
                    Err(e) => {
                        tracing::debug!(
                            "reminders of account {account_id}: no connection ({:?})",
                            e.code
                        );
                        continue;
                    }
                };
                if let Some(calendar) = calendar {
                    self.flush_reminder_actions(account_id, &calendar).await;
                }
            }
        };
        if tokio::time::timeout(SERVER_LIMIT, tell).await.is_err() {
            tracing::info!("telling the server about reminders took too long; retrying later");
        }
    }

    /// Tells the account's server about the dismissals and snoozes it
    /// hasn't heard of.
    pub(super) async fn flush_reminder_actions(
        &self,
        account_id: &str,
        calendar: &Arc<dyn CalendarProvider>,
    ) {
        let pending = match self.store.pending_reminder_actions(account_id) {
            Ok(pending) => pending,
            Err(e) => {
                tracing::warn!("reading pending reminder actions failed: {e}");
                return;
            }
        };
        let now = self.reminders.now();
        for state in pending {
            let Some(action) = state.pending else {
                continue;
            };
            let event = match self.store.get_event(&state.item_id) {
                Ok(event) => event,
                // Deleted meanwhile: nothing to tell.
                Err(e) if e.code == ErrorCode::NotFound => {
                    let _ = self.store.clear_reminder_pending(&state.key, action);
                    continue;
                }
                Err(e) => {
                    tracing::warn!("reading an event for its reminder failed: {e}");
                    continue;
                }
            };
            let result = match (action, state.snooze_until) {
                (ServerAction::Dismiss, _) => calendar.dismiss_reminder(&event.remote_id).await,
                (ServerAction::Snooze, Some(until)) if until > now => {
                    calendar.snooze_reminder(&event.remote_id, until).await
                }
                // The snooze is over already: the server's reminder is due
                // as it is.
                (ServerAction::Snooze, _) => Ok(()),
            };
            match result {
                Ok(()) => {}
                // The event or its reminder is gone, or the server keeps
                // no reminders: nothing more to tell.
                Err(e) if matches!(e.code, ErrorCode::NotFound | ErrorCode::Unsupported) => {}
                Err(e) => {
                    tracing::info!(
                        "could not tell account {account_id}'s server about a reminder ({:?}); \
                         trying again at the next sync",
                        e.code
                    );
                    if e.code == ErrorCode::Network {
                        return;
                    }
                    continue;
                }
            }
            if let Err(e) = self.store.clear_reminder_pending(&state.key, action) {
                tracing::warn!("saving a reminder failed: {e}");
            }
        }
    }

    /// After syncing an account's calendars: tells its server about
    /// dismissals and snoozes, then reads its reminders as the server keeps
    /// them. A failure keeps the last report (until it's a day old, when
    /// the events' own reminder times count).
    pub(super) async fn refresh_server_reminders(
        &self,
        account: &Account,
        calendar: &Arc<dyn CalendarProvider>,
    ) {
        self.flush_reminder_actions(&account.id, calendar).await;
        let now = self.reminders.now();
        let (window_start, window_end) = (now - LOOKBACK, now + LOOKAHEAD);
        match calendar.reminders(window_start, window_end).await {
            Ok(Some(reminders)) => {
                let report = ServerReminders {
                    account_id: account.id.clone(),
                    window_start,
                    window_end,
                    fetched_at: now,
                    reminders,
                };
                if let Err(e) = self.store.replace_server_reminders(&report) {
                    tracing::warn!("saving the server's reminders failed: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => tracing::info!(
                "could not read account {}'s reminders ({:?}); keeping the last ones",
                account.id,
                e.code
            ),
        }
    }

    /// Forgets what became of reminders long gone, and of deleted events and
    /// tasks.
    pub(super) fn prune_reminders(&self) {
        let cutoff = self.reminders.now() - KEEP_STATES;
        match self.store.prune_reminder_states(cutoff) {
            Ok(0) => {}
            Ok(n) => tracing::debug!("forgot {n} old reminders"),
            Err(e) => tracing::warn!("pruning reminders failed: {e}"),
        }
    }

    /// Looks at the reminders: saves what became of those that came due,
    /// and tells the sink when anything changed. Returns the active ones.
    pub(crate) fn reminder_pass(&self) -> Result<Vec<ActiveReminder>> {
        let mut last = self
            .reminders
            .pass
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let now = self.reminders.now();
        let settings = self.store.get_settings()?;
        if !settings.reminders_enabled {
            let mut cleared: Vec<String> = last.due.drain().collect();
            cleared.sort();
            if !cleared.is_empty() || !last.active.is_empty() {
                self.sink.reminders_changed(&[], &[], &cleared);
            }
            *last = LastPass::default();
            // Turned on again later, what came due meanwhile isn't missed.
            self.store.set_reminders_checked_until(now)?;
            return Ok(Vec::new());
        }

        let events = self
            .store
            .list_events(now - LOOKBACK, now + LOOKAHEAD, None)?;
        // Snoozed task reminders can come back long after their time.
        let tasks = self.store.tasks_with_reminders(now - KEEP_STATES)?;
        let reports: Vec<ServerReminders> = self
            .store
            .server_reminders()?
            .into_iter()
            .filter(|r| r.fetched_at > now - REPORT_LIFE)
            .collect();
        let mut sources = reminders::event_sources(&events, &reports);
        sources.extend(reminders::task_sources(&tasks));
        let states: HashMap<String, ReminderState> = self.store.reminder_states()?;
        let checked_until = self
            .store
            .reminders_checked_until()?
            .unwrap_or(now)
            .min(now);
        let plan = reminders::plan(
            &sources,
            &states,
            now,
            checked_until,
            &chrono::Local,
            &last.due,
        );
        if !plan.updates.is_empty() {
            self.store.put_reminder_states(&plan.updates)?;
        }
        self.store.set_reminders_checked_until(now)?;
        if plan.active != last.active || !plan.newly_due.is_empty() || !plan.cleared.is_empty() {
            self.sink
                .reminders_changed(&plan.active, &plan.newly_due, &plan.cleared);
        }
        *last = LastPass {
            due: plan.due_keys(),
            active: plan.active.clone(),
            next_wake: plan.next_wake,
        };
        Ok(plan.active)
    }
}

/// Runs the reminder passes until `cancel` fires.
pub(super) async fn reminder_loop(service: Weak<CoreService>, cancel: CancellationToken) {
    loop {
        let Some(strong) = service.upgrade() else {
            return;
        };
        let next = match strong.reminder_pass() {
            Ok(_) => strong.reminders.next_wake(),
            Err(e) => {
                tracing::warn!("looking at reminders failed: {e}");
                None
            }
        };
        let now = strong.reminders.now();
        let poke = Arc::clone(&strong.reminders.poke);
        drop(strong);
        let sleep = next
            .map(|at| (at - now).to_std().unwrap_or_default())
            .unwrap_or(MAX_SLEEP)
            .min(MAX_SLEEP);
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(sleep) => {}
            _ = poke.notified() => {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(SETTLE) => {}
                }
            }
        }
    }
}
