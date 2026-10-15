//! Reminders for events and tasks: which are due, which became due just
//! now (to show as notifications), and when to look again.
//!
//! [`plan`] is a pure function of the reminders' sources (events and tasks
//! with reminders, see [`event_sources`] and [`task_sources`]), what became
//! of each reminder so far ([`ReminderState`], kept in the store) and the
//! time. The scheduler (`service::reminders`) runs it whenever something
//! changes, and at the times it asks for.
//!
//! A reminder is known by its key, `{event|task}:{local id}:{fire time}`:
//! moving an event, or changing a task's reminder, makes a new reminder
//! that goes off afresh.
//!
//! Microsoft 365 keeps reminders itself: its `reminderView` says when each
//! goes off (later once snoozed) and leaves out the dismissed ones, also
//! when that was done in Outlook or on a phone. Where the app has that
//! report for an event, it wins over the time worked out here.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};

use crate::model::{ActiveReminder, Event, ReminderKind, ResponseStatus, Task, TaskStatus};
use crate::providers::ServerReminder;

/// Reminders missed while the app wasn't running, or the computer slept,
/// still go off this long after their time; older ones are let go.
pub const LOOKBACK: Duration = Duration::hours(24);
/// Events starting up to this far ahead are looked at.
pub const LOOKAHEAD: Duration = Duration::days(15);
/// More reminders than this going off at once show as one notification.
pub const SUMMARY_THRESHOLD: usize = 3;
/// What became of a reminder is forgotten this long after it went off.
pub const KEEP_STATES: Duration = Duration::days(30);

/// The key of the reminder of `item_id` that goes off at `fire_at`.
pub fn reminder_key(kind: ReminderKind, item_id: &str, fire_at: DateTime<Utc>) -> String {
    format!("{}:{item_id}:{}", kind.as_str(), fire_at.timestamp())
}

/// The kind, item and fire time a reminder key names.
pub fn parse_key(key: &str) -> Option<(ReminderKind, String, DateTime<Utc>)> {
    let (kind, rest) = key.split_once(':')?;
    let (item_id, fire_at) = rest.rsplit_once(':')?;
    let kind = ReminderKind::parse(kind)?;
    let fire_at = Utc.timestamp_opt(fire_at.parse().ok()?, 0).single()?;
    (!item_id.is_empty()).then(|| (kind, item_id.to_owned(), fire_at))
}

/// What became of a reminder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderStatus {
    /// It went off and showed as a notification.
    Shown,
    /// Its event was over by the time it was found due: it's listed with
    /// the others, but never showed as a notification.
    Missed,
    /// Snoozed until `snooze_until`.
    Snoozed,
    Dismissed,
}

impl ReminderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shown => "shown",
            Self::Missed => "missed",
            Self::Snoozed => "snoozed",
            Self::Dismissed => "dismissed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "shown" => Self::Shown,
            "missed" => Self::Missed,
            "snoozed" => Self::Snoozed,
            "dismissed" => Self::Dismissed,
            _ => return None,
        })
    }
}

/// A dismissal or snooze the server (Microsoft 365) still has to hear of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerAction {
    Dismiss,
    Snooze,
}

impl ServerAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dismiss => "dismiss",
            Self::Snooze => "snooze",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dismiss" => Some(Self::Dismiss),
            "snooze" => Some(Self::Snooze),
            _ => None,
        }
    }
}

/// What became of one reminder, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderState {
    pub key: String,
    pub account_id: String,
    pub kind: ReminderKind,
    pub item_id: String,
    pub fire_at: DateTime<Utc>,
    pub status: ReminderStatus,
    pub snooze_until: Option<DateTime<Utc>>,
    pub pending: Option<ServerAction>,
    pub updated_at: DateTime<Utc>,
}

/// The reminders Microsoft 365 reported for an account's events starting
/// within `[window_start, window_end)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerReminders {
    pub account_id: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub fetched_at: DateTime<Utc>,
    pub reminders: Vec<ServerReminder>,
}

/// When an item's reminder goes off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Timing {
    /// A timed event; the reminder goes off `minutes` before it starts.
    Event {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        minutes: u32,
    },
    /// An all-day event, from local midnight of `first` to local midnight
    /// of `after_last`; the reminder goes off `minutes` before it starts.
    /// Worked out on every pass, so it follows daylight saving time and a
    /// computer that moved to another time zone.
    AllDay {
        first: NaiveDate,
        after_last: NaiveDate,
        minutes: u32,
    },
    /// A task's reminder.
    Task { at: DateTime<Utc> },
}

/// An event or task with a reminder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderSource {
    pub kind: ReminderKind,
    pub item_id: String,
    pub account_id: String,
    pub title: String,
    pub location: Option<String>,
    pub online_meeting_url: Option<String>,
    pub timing: Timing,
    /// When the server says the reminder goes off; wins over `timing`.
    pub server_fire_at: Option<DateTime<Utc>>,
}

/// The reminders of `events`, minus cancelled and declined ones. Where a
/// server report covers an event, it says when its reminder goes off, and
/// an event it leaves out has none (it was dismissed); elsewhere the
/// event's own reminder time counts.
pub fn event_sources(events: &[Event], reports: &[ServerReminders]) -> Vec<ReminderSource> {
    let reports: HashMap<&str, &ServerReminders> =
        reports.iter().map(|r| (r.account_id.as_str(), r)).collect();
    events
        .iter()
        .filter(|e| !e.is_cancelled && e.response_status != ResponseStatus::Declined)
        .filter_map(|e| {
            let report = reports
                .get(e.account_id.as_str())
                .filter(|r| r.window_start <= e.start && e.start < r.window_end);
            let server_fire_at = match report {
                Some(report) => Some(server_fire_time(e, &report.reminders)?),
                None => None,
            };
            let minutes = match (e.reminder_minutes, server_fire_at) {
                (Some(minutes), _) => minutes,
                // The server has a reminder the event doesn't show.
                (None, Some(_)) => 0,
                (None, None) => return None,
            };
            let timing = if e.is_all_day {
                // Stored as midnight UTC of the first day and of the day
                // after the last (see `model::Event`).
                let first = e.start.date_naive();
                Timing::AllDay {
                    first,
                    after_last: e.end.date_naive().max(first.succ_opt()?),
                    minutes,
                }
            } else {
                Timing::Event {
                    start: e.start,
                    end: e.end,
                    minutes,
                }
            };
            Some(ReminderSource {
                kind: ReminderKind::Event,
                item_id: e.id.clone(),
                account_id: e.account_id.clone(),
                title: e.subject.clone(),
                location: e.location.clone(),
                online_meeting_url: e.online_meeting_url.clone(),
                timing,
                server_fire_at,
            })
        })
        .collect()
}

/// The server's fire time for the event: matched by its id, or (for
/// occurrences of a series, whose ids may be another form) by its times and
/// subject. `None` when the server has no reminder for it.
fn server_fire_time(event: &Event, reminders: &[ServerReminder]) -> Option<DateTime<Utc>> {
    if let Some(r) = reminders.iter().find(|r| r.event_id == event.remote_id) {
        return Some(r.fire_at);
    }
    reminders
        .iter()
        .find(|r| {
            r.subject == event.subject
                && if event.is_all_day {
                    // The server gives midnight in the mailbox's time zone.
                    nearest_date(r.event_start) == event.start.date_naive()
                        && nearest_date(r.event_end) == event.end.date_naive()
                } else {
                    r.event_start == event.start && r.event_end == event.end
                }
        })
        .map(|r| r.fire_at)
}

/// The date of the midnight nearest to `at` (see
/// `providers::graph::types::nearest_date`).
fn nearest_date(at: DateTime<Utc>) -> NaiveDate {
    (at + Duration::hours(12)).date_naive()
}

/// The reminders of open tasks.
pub fn task_sources(tasks: &[Task]) -> Vec<ReminderSource> {
    tasks
        .iter()
        .filter(|t| t.status != TaskStatus::Completed)
        .filter_map(|t| {
            Some(ReminderSource {
                kind: ReminderKind::Task,
                item_id: t.id.clone(),
                account_id: t.account_id.clone(),
                title: t.title.clone(),
                location: None,
                online_meeting_url: None,
                timing: Timing::Task { at: t.reminder_at? },
                server_fire_at: None,
            })
        })
        .collect()
}

/// Local midnight at the start of `date` in `tz`; where the clocks skip
/// midnight, the first time that day that exists.
pub fn local_midnight<Tz: TimeZone>(tz: &Tz, date: NaiveDate) -> DateTime<Utc> {
    let mut at = date.and_time(NaiveTime::MIN);
    for _ in 0..4 {
        match tz.from_local_datetime(&at) {
            LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => return t.with_timezone(&Utc),
            LocalResult::None => at += Duration::minutes(30),
        }
    }
    at.and_utc()
}

/// What [`plan`] found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// Every reminder that is due, or snoozed: due ones first, by when their
    /// event starts (or the task's reminder time), then snoozed ones by when
    /// they come back.
    pub active: Vec<ActiveReminder>,
    /// The ones that just became due: to show as notifications.
    pub newly_due: Vec<ActiveReminder>,
    /// Keys of reminders that were due at the last pass and no longer are
    /// (dismissed, snoozed, completed, moved, deleted): their notifications
    /// can go.
    pub cleared: Vec<String>,
    /// When to look again: the next reminder time or end of a snooze, or
    /// an event starting.
    pub next_wake: Option<DateTime<Utc>>,
    /// What became of the reminders that just became due, to save.
    pub updates: Vec<ReminderState>,
}

impl Plan {
    /// The keys of the reminders due now.
    pub fn due_keys(&self) -> HashSet<String> {
        self.active
            .iter()
            .filter(|r| r.is_due())
            .map(|r| r.key.clone())
            .collect()
    }
}

/// Works out the reminders as of `now`.
///
/// - `states`: what became of reminders so far, by key.
/// - `checked_until`: when the last pass ran (`now` the first time). A
///   reminder that went off since then was missed only while the app
///   wasn't looking; one that went off before it, and that nothing is known
///   of, was found later (a synced or new event).
/// - `previously_due`: the keys due at the last pass.
///
/// A reminder that comes due goes off once: it's shown, unless its event
/// is already over (then it's listed as missed, but only when it went off
/// since `checked_until`: an event that was over before the app heard of
/// it isn't worth a reminder). One that went off more than [`LOOKBACK`]
/// ago is let go.
pub fn plan<Tz: TimeZone>(
    sources: &[ReminderSource],
    states: &HashMap<String, ReminderState>,
    now: DateTime<Utc>,
    checked_until: DateTime<Utc>,
    tz: &Tz,
    previously_due: &HashSet<String>,
) -> Plan {
    let mut plan = Plan::default();
    let wake = |at: DateTime<Utc>, plan: &mut Plan| {
        if at > now && plan.next_wake.is_none_or(|w| at < w) {
            plan.next_wake = Some(at);
        }
    };
    // Snoozed on the server too: its report then says the reminder goes off
    // when the snooze ends, which makes a new key.
    let snoozed_items: HashMap<&str, DateTime<Utc>> = states
        .values()
        .filter(|s| s.status == ReminderStatus::Snoozed)
        .filter_map(|s| Some((s.item_id.as_str(), s.snooze_until?)))
        .filter(|(_, until)| *until > now)
        .collect();

    for source in sources {
        let (computed, starts_at, ends_at) = match source.timing {
            Timing::Event {
                start,
                end,
                minutes,
            } => (
                start - Duration::minutes(minutes.into()),
                Some(start),
                Some(end),
            ),
            Timing::AllDay {
                first,
                after_last,
                minutes,
            } => {
                let start = local_midnight(tz, first);
                (
                    start - Duration::minutes(minutes.into()),
                    Some(start),
                    Some(local_midnight(tz, after_last)),
                )
            }
            Timing::Task { at } => (at, None, None),
        };
        let fire_at = source.server_fire_at.unwrap_or(computed);
        let key = reminder_key(source.kind, &source.item_id, fire_at);
        let reminder = |snoozed_until: Option<DateTime<Utc>>, missed: bool| ActiveReminder {
            key: key.clone(),
            kind: source.kind,
            item_id: source.item_id.clone(),
            account_id: source.account_id.clone(),
            title: source.title.clone(),
            starts_at,
            ends_at,
            location: source.location.clone(),
            online_meeting_url: source.online_meeting_url.clone(),
            fire_at,
            snoozed_until,
            overdue: starts_at.unwrap_or(fire_at) <= now,
            missed,
        };
        let state = |status| ReminderState {
            key: key.clone(),
            account_id: source.account_id.clone(),
            kind: source.kind,
            item_id: source.item_id.clone(),
            fire_at,
            status,
            snooze_until: None,
            pending: None,
            updated_at: now,
        };
        match states.get(&key) {
            Some(s) if s.status == ReminderStatus::Dismissed => {}
            Some(s) if s.status == ReminderStatus::Snoozed => match s.snooze_until {
                Some(until) if until > now => {
                    wake(until, &mut plan);
                    plan.active.push(reminder(Some(until), false));
                }
                // The snooze is over: it goes off again.
                _ => {
                    let due = reminder(None, false);
                    plan.updates.push(state(ReminderStatus::Shown));
                    plan.newly_due.push(due.clone());
                    plan.active.push(due);
                }
            },
            Some(s) => {
                let missed = s.status == ReminderStatus::Missed;
                plan.active.push(reminder(None, missed));
            }
            None if fire_at > now => {
                wake(fire_at, &mut plan);
                if let Some(until) = snoozed_items.get(source.item_id.as_str())
                    && (*until - fire_at).num_seconds().abs() <= 60
                {
                    plan.active.push(reminder(Some(fire_at), false));
                }
            }
            None if fire_at < now - LOOKBACK => {}
            None if ends_at.is_some_and(|end| end <= now) => {
                if fire_at > checked_until {
                    plan.updates.push(state(ReminderStatus::Missed));
                    plan.active.push(reminder(None, true));
                }
            }
            None => {
                let due = reminder(None, false);
                plan.updates.push(state(ReminderStatus::Shown));
                plan.newly_due.push(due.clone());
                plan.active.push(due);
            }
        }
    }

    // An event starting makes its reminder overdue.
    let starts: Vec<DateTime<Utc>> = plan
        .active
        .iter()
        .filter(|r| r.is_due())
        .filter_map(|r| r.starts_at)
        .collect();
    for start in starts {
        wake(start, &mut plan);
    }
    plan.active.sort_by(|a, b| {
        let order = |r: &ActiveReminder| {
            (
                r.snoozed_until.is_some(),
                r.snoozed_until.or(r.starts_at).unwrap_or(r.fire_at),
            )
        };
        order(a).cmp(&order(b)).then_with(|| a.key.cmp(&b.key))
    });
    plan.newly_due.sort_by(|a, b| {
        (a.starts_at.unwrap_or(a.fire_at), &a.key).cmp(&(b.starts_at.unwrap_or(b.fire_at), &b.key))
    });
    let due = plan.due_keys();
    let mut cleared: Vec<String> = previously_due
        .iter()
        .filter(|k| !due.contains(*k))
        .cloned()
        .collect();
    cleared.sort();
    plan.cleared = cleared;
    plan
}
