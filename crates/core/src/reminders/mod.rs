//! Reminders for events and tasks: which are due, which became due just
//! now (to show as notifications), and when to look again.
//!
//! [`plan`] is a pure function of the reminders' sources (events and tasks
//! with reminders, see [`event_sources`] and [`task_sources`]), what became
//! of each reminder so far ([`ReminderState`], kept in the store) and the
//! time. The scheduler (`service::reminders`) runs it whenever something
//! changes, and at the times it asks for.
//!
//! A reminder is known by its key, `{event|task}:{local id}:{time}`, the
//! time being when the event's own reminder time says it goes off (for an
//! all-day event, counted from midnight UTC of its first day) or the
//! task's reminder time. So a reminder stays the same one whatever becomes
//! of the time it goes off: a snooze, the server's word on it, or the
//! computer's time zone. Moving an event, or changing its reminder or a
//! task's, makes a new reminder that goes off afresh.
//!
//! Microsoft 365 keeps reminders itself: its `reminderView` says when each
//! goes off (later once snoozed, here or elsewhere) and leaves out the
//! dismissed ones, also when that was done in Outlook or on a phone. Where
//! the app has that report for an event, it wins over the time worked out
//! here; a reminder that came due while the app wasn't looking waits a
//! little for a report that can vouch for it.

#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Duration, LocalResult, NaiveDate, NaiveTime, SubsecRound, TimeZone, Utc};

use crate::model::{
    ActiveReminder, Calendar, Event, ReminderKind, ResponseStatus, Task, TaskStatus,
};
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
/// A reminder found due within this of its time was found on time (the
/// scheduler looks at least this often while the computer is awake); one
/// found later came due while the app wasn't looking.
pub const ON_TIME: Duration = Duration::minutes(1);
/// A server report older than this no longer counts: the events' own
/// reminder times do.
pub const REPORT_LIFE: Duration = Duration::days(1);
/// Fire times this close are one: the server may round a snooze's end.
const SAME_TIME: Duration = Duration::minutes(1);

/// The key of the reminder of `item_id` at `at`: when the event's own
/// reminder time says it goes off, or the task's reminder time.
pub fn reminder_key(kind: ReminderKind, item_id: &str, at: DateTime<Utc>) -> String {
    format!("{}:{item_id}:{}", kind.as_str(), at.timestamp())
}

/// The kind, item and time a reminder key names.
pub fn parse_key(key: &str) -> Option<(ReminderKind, String, DateTime<Utc>)> {
    let (kind, rest) = key.split_once(':')?;
    let (item_id, at) = rest.rsplit_once(':')?;
    let kind = ReminderKind::parse(kind)?;
    let at = Utc.timestamp_opt(at.parse().ok()?, 0).single()?;
    (!item_id.is_empty()).then(|| (kind, item_id.to_owned(), at))
}

/// What became of a reminder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderStatus {
    /// It went off and showed as a notification.
    Shown,
    /// Its event was over by the time it was found due, well after its
    /// time: it's listed with the others, but never showed as a
    /// notification.
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
    /// When it last went off (or would have, for a missed one).
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
    /// The account's events in the window as the app had them when the
    /// report was fetched, by remote id: only an event as it was then can
    /// be missing from it for being dismissed, rather than for being newer,
    /// moved or reminding otherwise since.
    pub known_events: HashMap<String, KnownEvent>,
}

/// An event as the app had it when a server report was fetched (its times
/// to the second, as the store keeps them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownEvent {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub reminder_minutes: Option<u32>,
}

impl KnownEvent {
    pub fn of(event: &Event) -> Self {
        Self {
            start: event.start.trunc_subsecs(0),
            end: event.end.trunc_subsecs(0),
            reminder_minutes: event.reminder_minutes,
        }
    }
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
    /// computer that moved to another time zone (the reminder stays the
    /// same one).
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
    /// The server keeps the reminder too (Microsoft 365), and can dismiss
    /// or snooze it elsewhere.
    pub on_server: bool,
    /// What the server's report says of it, where one covers it; wins over
    /// `timing`.
    pub server: Option<ServerTiming>,
}

/// When the server says a reminder goes off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerTiming {
    /// When it goes off, snoozes aside.
    pub fire_at: DateTime<Utc>,
    /// Snoozed until then, here or elsewhere.
    pub snoozed_until: Option<DateTime<Utc>>,
    /// When the server said so: what became of the reminder since, it
    /// can't tell.
    pub as_of: DateTime<Utc>,
}

/// The reminders of `events`, minus cancelled and declined ones.
///
/// Where a server report that still counts (see [`REPORT_LIFE`]) covers
/// an event, it says when its reminder goes off; an event it leaves out
/// has none (it was dismissed), if the report knew of the event as it is.
/// Elsewhere, and for an event new, moved or reminding otherwise since the
/// report, the event's own reminder time counts. `calendars` say which is
/// each account's default calendar, which the report surely covers.
pub fn event_sources(
    events: &[Event],
    calendars: &[Calendar],
    reports: &[ServerReminders],
    now: DateTime<Utc>,
) -> Vec<ReminderSource> {
    let reports: HashMap<&str, &ServerReminders> =
        reports.iter().map(|r| (r.account_id.as_str(), r)).collect();
    let matched = match_server_reminders(events, calendars, &reports);
    events
        .iter()
        .filter(|e| !e.is_cancelled && e.response_status != ResponseStatus::Declined)
        .filter_map(|e| {
            let report = reports.get(e.account_id.as_str());
            let covering = report.filter(|r| {
                r.fetched_at > now - REPORT_LIFE
                    && r.window_start <= e.start
                    && e.start < r.window_end
            });
            let server = match (covering, matched.get(e.id.as_str())) {
                // Unless it was said of the event before it moved.
                (Some(report), Some(r)) => {
                    same_times(e, r).then(|| server_timing(e, r, report.fetched_at))
                }
                (Some(report), None)
                    if report.known_events.get(&e.remote_id) == Some(&KnownEvent::of(e)) =>
                {
                    return None;
                }
                _ => None,
            };
            let minutes = match (e.reminder_minutes, server) {
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
                on_server: report.is_some(),
                server,
            })
        })
        .collect()
}

/// Which of the server's reminders is each event's (by local id): the one
/// that names the event's id, or for an occurrence of a series, whose id
/// may take another form there, one with its subject and times that no
/// other event has. That goes to an occurrence in a calendar the report
/// surely covers (the account's default calendar, or one with events it
/// names) before a copy of it in a calendar shared with the user, of which
/// Microsoft 365 doesn't remind.
fn match_server_reminders<'a>(
    events: &'a [Event],
    calendars: &[Calendar],
    reports: &HashMap<&str, &'a ServerReminders>,
) -> HashMap<&'a str, &'a ServerReminder> {
    let mut matched: HashMap<&str, &ServerReminder> = HashMap::new();
    let mut claimed: HashSet<(&str, &str)> = HashSet::new();
    let mut covered: HashSet<&str> = calendars
        .iter()
        .filter(|c| c.is_default)
        .map(|c| c.id.as_str())
        .collect();
    for e in events {
        let Some(report) = reports.get(e.account_id.as_str()) else {
            continue;
        };
        if let Some(r) = report.reminders.iter().find(|r| r.event_id == e.remote_id) {
            matched.insert(&e.id, r);
            claimed.insert((&report.account_id, &r.event_id));
            covered.insert(&e.calendar_id);
        }
    }
    for report in reports.values() {
        for r in &report.reminders {
            if claimed.contains(&(report.account_id.as_str(), r.event_id.as_str())) {
                continue;
            }
            let (sure, others): (Vec<&Event>, Vec<&Event>) = events
                .iter()
                .filter(|e| {
                    e.account_id == report.account_id
                        && e.is_recurring
                        && e.reminder_minutes.is_some()
                        && !matched.contains_key(e.id.as_str())
                        && e.subject == r.subject
                        && same_times(e, r)
                })
                .partition(|e| covered.contains(e.calendar_id.as_str()));
            if let Some(e) = sure.first().or(others.first()) {
                matched.insert(&e.id, r);
            }
        }
    }
    matched
}

/// Whether the server's reminder is of the event as it is (the server
/// gives an all-day event's times as midnight in the mailbox's time zone).
fn same_times(event: &Event, r: &ServerReminder) -> bool {
    if event.is_all_day {
        nearest_date(r.event_start) == event.start.date_naive()
            && nearest_date(r.event_end) == event.end.date_naive()
    } else {
        r.event_start == event.start && r.event_end == event.end
    }
}

/// When the server says the event's reminder goes off, as of `as_of`: a
/// time later than the event's start less its reminder time is a snooze.
fn server_timing(event: &Event, r: &ServerReminder, as_of: DateTime<Utc>) -> ServerTiming {
    let unsnoozed = event
        .reminder_minutes
        .map(|minutes| r.event_start - Duration::minutes(minutes.into()));
    match unsnoozed {
        Some(fire_at) if r.fire_at > fire_at + SAME_TIME => ServerTiming {
            fire_at,
            snoozed_until: Some(r.fire_at),
            as_of,
        },
        _ => ServerTiming {
            fire_at: r.fire_at,
            snoozed_until: None,
            as_of,
        },
    }
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
                on_server: false,
                server: None,
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
    /// What became of the reminders that just became due (or snoozed
    /// elsewhere), to save.
    pub updates: Vec<ReminderState>,
    /// Accounts with reminders that came due while the app wasn't looking
    /// and that wait for a fresh report from their server, which alone can
    /// say whether they were dismissed or snoozed elsewhere meanwhile.
    /// They're left out of the rest of the plan until then.
    pub held: Vec<String>,
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
/// - `hold`: whether reminders that came due while the app wasn't looking
///   wait for a fresh word from the server that keeps them (see
///   [`Plan::held`]); the scheduler waits only so long.
///
/// A reminder that comes due goes off once: it's shown, unless it came due
/// while the app wasn't looking and its event is over by now (then it's
/// listed as missed, but only when it went off since `checked_until`: an
/// event that was over before the app heard of it isn't worth a reminder).
/// One missed more than [`LOOKBACK`] ago is let go, and so is one found
/// that late for something under way. A snooze on the server, here or
/// elsewhere, moves when it goes off, but it stays the same reminder.
pub fn plan<Tz: TimeZone>(
    sources: &[ReminderSource],
    states: &HashMap<String, ReminderState>,
    now: DateTime<Utc>,
    checked_until: DateTime<Utc>,
    tz: &Tz,
    previously_due: &HashSet<String>,
    hold: bool,
) -> Plan {
    let mut plan = Plan::default();
    let mut held = BTreeSet::new();
    let wake = |at: DateTime<Utc>, plan: &mut Plan| {
        if at > now && plan.next_wake.is_none_or(|w| at < w) {
            plan.next_wake = Some(at);
        }
    };

    for source in sources {
        // When it goes off by the item's own reminder time, and by what
        // it's known: for an all-day event, the same wherever the computer
        // is.
        let (computed, starts_at, ends_at, keyed_at) = match source.timing {
            Timing::Event {
                start,
                end,
                minutes,
            } => {
                let fire_at = start - Duration::minutes(minutes.into());
                (fire_at, Some(start), Some(end), fire_at)
            }
            Timing::AllDay {
                first,
                after_last,
                minutes,
            } => {
                let before = Duration::minutes(minutes.into());
                let start = local_midnight(tz, first);
                (
                    start - before,
                    Some(start),
                    Some(local_midnight(tz, after_last)),
                    first.and_time(NaiveTime::MIN).and_utc() - before,
                )
            }
            Timing::Task { at } => (at, None, None, at),
        };
        let key = reminder_key(source.kind, &source.item_id, keyed_at);
        let fire_at = source.server.map_or(computed, |s| s.fire_at);
        let server_snooze = source.server.and_then(|s| s.snoozed_until);
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
        let state = |status, went_off, snooze_until| ReminderState {
            key: key.clone(),
            account_id: source.account_id.clone(),
            kind: source.kind,
            item_id: source.item_id.clone(),
            fire_at: went_off,
            status,
            snooze_until,
            pending: None,
            updated_at: now,
        };
        // Snoozed on the server until `until` since it went off here.
        let snoozed_elsewhere = |went_off, until, plan: &mut Plan| {
            plan.updates
                .push(state(ReminderStatus::Snoozed, went_off, Some(until)));
            plan.active.push(reminder(Some(until), false));
            wake(until, plan);
        };

        // When it comes due now, if it does.
        let known = states.get(&key);
        let due_at = match known {
            Some(s) if s.status == ReminderStatus::Dismissed => None,
            Some(s) if s.status == ReminderStatus::Snoozed => match s.snooze_until {
                Some(until) if until > now => {
                    wake(until, &mut plan);
                    plan.active.push(reminder(Some(until), false));
                    None
                }
                // The snooze is over: it goes off again, unless it was
                // snoozed again elsewhere once the server heard of this.
                until => {
                    let until = until.unwrap_or(s.fire_at);
                    match server_snooze
                        .filter(|later| s.pending.is_none() && *later > until + SAME_TIME)
                        .filter(|later| *later > now)
                    {
                        Some(later) => {
                            snoozed_elsewhere(s.fire_at, later, &mut plan);
                            None
                        }
                        None => Some(until),
                    }
                }
            },
            Some(s) => {
                match server_snooze.filter(|until| *until > s.fire_at + SAME_TIME && *until > now) {
                    Some(until) => snoozed_elsewhere(s.fire_at, until, &mut plan),
                    // A snooze elsewhere that's over by the time the app
                    // hears of it changes nothing: it was due here all
                    // along.
                    None => plan
                        .active
                        .push(reminder(None, s.status == ReminderStatus::Missed)),
                }
                None
            }
            // Snoozed elsewhere before it went off here: due when that ends.
            None => match server_snooze.unwrap_or(fire_at) {
                at if at > now => {
                    wake(at, &mut plan);
                    None
                }
                at if at < now - LOOKBACK
                    && (at > checked_until || !starts_at.is_some_and(|start| start > now)) =>
                {
                    None
                }
                at => Some(at),
            },
        };
        let Some(due_at) = due_at else {
            continue;
        };
        let late = now - due_at > ON_TIME;
        let over = late && ends_at.is_some_and(|end| end <= now);
        // An event that was over before the app last looked, and so before
        // it heard of it, isn't worth a reminder, whatever the server would
        // say of it. One that ended since (a reminder held for the server
        // across a sleep or a restart) is listed as missed.
        if over && known.is_none() && ends_at.is_some_and(|end| end <= checked_until) {
            continue;
        }
        if hold && late && source.on_server && source.server.is_none_or(|s| s.as_of < due_at) {
            held.insert(source.account_id.clone());
            continue;
        }
        if over {
            plan.updates
                .push(state(ReminderStatus::Missed, due_at, None));
            plan.active.push(reminder(None, true));
        } else {
            let due = reminder(None, false);
            plan.updates
                .push(state(ReminderStatus::Shown, due_at, None));
            plan.newly_due.push(due.clone());
            plan.active.push(due);
        }
    }
    plan.held = held.into_iter().collect();

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
