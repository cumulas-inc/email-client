//! The planner and the reminders' sources.

use chrono::{FixedOffset, MappedLocalTime, NaiveDateTime};

use super::*;
use crate::model::{EntityKind, Importance, ShowAs, local_id};
use crate::store::Store;

fn at(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, day, hour, minute, 0)
        .unwrap()
}

/// 2026-10-15 09:00 UTC.
fn now() -> DateTime<Utc> {
    at(15, 9, 0)
}

fn event(remote: &str, start: DateTime<Utc>, minutes: i64, reminder: Option<u32>) -> Event {
    Event {
        id: local_id("a", EntityKind::Event, remote),
        account_id: "a".into(),
        calendar_id: "c".into(),
        remote_id: remote.into(),
        subject: remote.into(),
        location: Some("Room 3".into()),
        body_html: None,
        body_preview: None,
        start,
        end: start + Duration::minutes(minutes),
        is_all_day: false,
        organizer: None,
        attendees: vec![],
        show_as: ShowAs::Busy,
        response_status: ResponseStatus::Organizer,
        is_online_meeting: false,
        online_meeting_url: None,
        reminder_minutes: reminder,
        is_recurring: false,
        is_cancelled: false,
    }
}

fn all_day(remote: &str, day: u32, days: i64, reminder: u32) -> Event {
    let start = at(day, 0, 0);
    Event {
        is_all_day: true,
        end: start + Duration::days(days),
        ..event(remote, start, 0, Some(reminder))
    }
}

fn task(remote: &str, reminder: Option<DateTime<Utc>>, status: TaskStatus) -> Task {
    Task {
        id: local_id("a", EntityKind::Task, remote),
        account_id: "a".into(),
        list_id: "l".into(),
        remote_id: remote.into(),
        title: remote.into(),
        notes: None,
        status,
        importance: Importance::Normal,
        due_date: None,
        reminder_at: reminder,
        completed_at: None,
        created_at: at(1, 0, 0),
        updated_at: at(1, 0, 0),
    }
}

/// Plans in UTC, as of `now`, the last pass a minute earlier.
fn plan_at(
    sources: &[ReminderSource],
    states: &HashMap<String, ReminderState>,
    now: DateTime<Utc>,
) -> Plan {
    plan(
        sources,
        states,
        now,
        now - Duration::minutes(1),
        &Utc,
        &HashSet::new(),
        true,
    )
}

fn titles(reminders: &[ActiveReminder]) -> Vec<&str> {
    reminders.iter().map(|r| r.title.as_str()).collect()
}

/// Saves what the plan says became of the reminders.
fn apply(states: &mut HashMap<String, ReminderState>, plan: &Plan) {
    for update in &plan.updates {
        states.insert(update.key.clone(), update.clone());
    }
}

#[test]
fn keys_name_the_item_and_the_time() {
    let key = reminder_key(ReminderKind::Event, "5f0c-1", at(15, 8, 45));
    assert_eq!(key, format!("event:5f0c-1:{}", at(15, 8, 45).timestamp()));
    assert_eq!(
        parse_key(&key),
        Some((ReminderKind::Event, "5f0c-1".into(), at(15, 8, 45)))
    );
    for junk in ["", "event:x", "meeting:x:1", "task::1", "task:x:soon"] {
        assert_eq!(parse_key(junk), None, "{junk:?}");
    }
}

#[test]
fn reminders_come_due_at_their_time() {
    let events = [
        // Starts in 10 minutes, reminds 15 before: due 5 minutes ago.
        event("standup", now() + Duration::minutes(10), 15, Some(15)),
        // Starts in an hour: due in 45 minutes.
        event("review", now() + Duration::hours(1), 30, Some(15)),
        event("no reminder", now() + Duration::minutes(5), 30, None),
    ];
    let tasks = [task(
        "call back",
        Some(now() + Duration::minutes(20)),
        TaskStatus::NotStarted,
    )];
    let mut sources = event_sources(&events, &[], &[], now());
    sources.extend(task_sources(&tasks));
    let mut states = HashMap::new();
    let first = plan_at(&sources, &states, now());
    assert_eq!(titles(&first.newly_due), ["standup"]);
    assert_eq!(titles(&first.active), ["standup"]);
    let standup = &first.active[0];
    // Known by when the standup's own reminder time says it goes off.
    assert_eq!(
        standup.key,
        reminder_key(
            ReminderKind::Event,
            &events[0].id,
            events[0].start - Duration::minutes(15)
        )
    );
    assert_eq!(standup.fire_at, now() - Duration::minutes(5));
    assert_eq!(standup.starts_at, Some(now() + Duration::minutes(10)));
    assert_eq!(standup.location.as_deref(), Some("Room 3"));
    assert!(!standup.overdue && !standup.missed && standup.is_due());
    // Next, the standup starts (its reminder is overdue then).
    assert_eq!(first.next_wake, Some(now() + Duration::minutes(10)));
    assert_eq!(first.updates.len(), 1);
    assert_eq!(first.updates[0].status, ReminderStatus::Shown);
    apply(&mut states, &first);

    // Once shown, it stays due but isn't shown again.
    let later = now() + Duration::minutes(20);
    let second = plan_at(&sources, &states, later);
    assert_eq!(titles(&second.newly_due), ["call back"]);
    assert_eq!(titles(&second.active), ["standup", "call back"]);
    assert!(second.active[0].overdue, "the standup started");
    assert_eq!(second.next_wake, Some(now() + Duration::minutes(45)));
    apply(&mut states, &second);
    assert!(plan_at(&sources, &states, later).newly_due.is_empty());
}

#[test]
fn reminders_missed_in_sleep_go_off_once_together() {
    // Asleep from 01:00 to 09:00: five reminders went off meanwhile, for
    // things not over yet.
    let tasks: Vec<Task> = (1..=5)
        .map(|n| {
            task(
                &format!("task {n}"),
                Some(at(15, n, 30)),
                TaskStatus::NotStarted,
            )
        })
        .collect();
    let events = [event("all morning", at(15, 8, 0), 240, Some(15))];
    let mut sources = task_sources(&tasks);
    sources.extend(event_sources(&events, &[], &[], now()));
    let mut states = HashMap::new();
    let woke = plan(
        &sources,
        &states,
        now(),
        at(15, 1, 0),
        &Utc,
        &HashSet::new(),
        true,
    );
    assert_eq!(woke.newly_due.len(), 6);
    assert!(woke.newly_due.len() > SUMMARY_THRESHOLD);
    assert!(woke.active.iter().all(|r| r.overdue && !r.missed));
    apply(&mut states, &woke);
    // Once.
    let again = plan(
        &sources,
        &states,
        now() + Duration::minutes(1),
        now(),
        &Utc,
        &woke.due_keys(),
        true,
    );
    assert!(again.newly_due.is_empty());
    assert!(again.cleared.is_empty());
    assert_eq!(again.active.len(), 6);
}

#[test]
fn reminders_older_than_the_lookback_are_let_go() {
    let tasks = [
        task(
            "yesterday",
            Some(now() - Duration::hours(25)),
            TaskStatus::NotStarted,
        ),
        task(
            "last night",
            Some(now() - Duration::hours(23)),
            TaskStatus::NotStarted,
        ),
    ];
    let plan = plan(
        &task_sources(&tasks),
        &HashMap::new(),
        now(),
        now() - Duration::days(3),
        &Utc,
        &HashSet::new(),
        true,
    );
    assert_eq!(titles(&plan.newly_due), ["last night"]);
    assert_eq!(plan.updates.len(), 1);
}

#[test]
fn events_that_are_over_are_listed_but_not_shown() {
    // 07:00 to 08:00, reminded at 06:45, while the computer slept from
    // 06:00.
    let events = [event("early call", at(15, 7, 0), 60, Some(15))];
    let sources = event_sources(&events, &[], &[], now());
    let woke = plan(
        &sources,
        &HashMap::new(),
        now(),
        at(15, 6, 0),
        &Utc,
        &HashSet::new(),
        true,
    );
    assert!(woke.newly_due.is_empty());
    assert_eq!(titles(&woke.active), ["early call"]);
    assert!(woke.active[0].missed && woke.active[0].overdue);
    assert_eq!(woke.updates[0].status, ReminderStatus::Missed);
    let mut states = HashMap::new();
    apply(&mut states, &woke);
    let later = plan_at(&sources, &states, now() + Duration::minutes(5));
    assert!(later.newly_due.is_empty());
    assert!(later.active[0].missed);

    // Synced only after it was over (its reminder went off before the last
    // pass): not worth a reminder.
    let synced = plan(
        &sources,
        &HashMap::new(),
        now(),
        at(15, 8, 30),
        &Utc,
        &HashSet::new(),
        true,
    );
    assert_eq!(synced, Plan::default());
}

#[test]
fn an_event_found_after_its_reminder_time_still_reminds() {
    // Made just now, starting in five minutes, reminding fifteen before.
    let events = [event(
        "quick sync",
        now() + Duration::minutes(5),
        30,
        Some(15),
    )];
    let plan = plan_at(
        &event_sources(&events, &[], &[], now()),
        &HashMap::new(),
        now(),
    );
    assert_eq!(titles(&plan.newly_due), ["quick sync"]);
}

#[test]
fn dismissed_and_snoozed_reminders_stay_so_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let events = [
        event("standup", now() + Duration::minutes(10), 15, Some(15)),
        event("review", now() + Duration::minutes(5), 30, Some(15)),
    ];
    let sources = event_sources(&events, &[], &[], now());
    {
        let store = Store::open(&path).unwrap();
        store
            .insert_account(&crate::store::test_support::imap_account("a"))
            .unwrap();
        let first = plan_at(&sources, &HashMap::new(), now());
        assert_eq!(first.newly_due.len(), 2);
        store.put_reminder_states(&first.updates).unwrap();
        let update = |event: &Event| {
            first
                .updates
                .iter()
                .find(|u| u.item_id == event.id)
                .unwrap()
                .clone()
        };
        let mut dismissed = update(&events[1]);
        dismissed.status = ReminderStatus::Dismissed;
        let mut snoozed = update(&events[0]);
        snoozed.status = ReminderStatus::Snoozed;
        snoozed.snooze_until = Some(now() + Duration::minutes(5));
        store.put_reminder_states(&[dismissed, snoozed]).unwrap();
    }
    let store = Store::open(&path).unwrap();
    let states = store.reminder_states().unwrap();
    let restarted = plan_at(&sources, &states, now() + Duration::minutes(1));
    assert!(restarted.newly_due.is_empty());
    assert_eq!(titles(&restarted.active), ["standup"]);
    assert_eq!(
        restarted.active[0].snoozed_until,
        Some(now() + Duration::minutes(5))
    );
    assert_eq!(restarted.next_wake, Some(now() + Duration::minutes(5)));

    // The snooze ends: it goes off again, once.
    let back = plan_at(&sources, &states, now() + Duration::minutes(5));
    assert_eq!(titles(&back.newly_due), ["standup"]);
    assert_eq!(back.updates[0].status, ReminderStatus::Shown);
    assert_eq!(back.updates[0].snooze_until, None);
}

#[test]
fn snoozing_or_dismissing_clears_the_notification() {
    let events = [event(
        "standup",
        now() + Duration::minutes(10),
        15,
        Some(15),
    )];
    let sources = event_sources(&events, &[], &[], now());
    let mut states = HashMap::new();
    let first = plan_at(&sources, &states, now());
    apply(&mut states, &first);
    let key = first.newly_due[0].key.clone();
    let due = first.due_keys();

    let mut snoozed = states[&key].clone();
    snoozed.status = ReminderStatus::Snoozed;
    snoozed.snooze_until = Some(now() + Duration::minutes(5));
    states.insert(key.clone(), snoozed);
    let plan = plan(&sources, &states, now(), now(), &Utc, &due, true);
    assert_eq!(plan.cleared, std::slice::from_ref(&key));
    assert_eq!(plan.active.len(), 1, "snoozed ones stay listed");

    states.get_mut(&key).unwrap().status = ReminderStatus::Dismissed;
    let plan = super::plan(&sources, &states, now(), now(), &Utc, &due, true);
    assert_eq!(plan.cleared, [key]);
    assert!(plan.active.is_empty());
}

#[test]
fn moving_an_event_sets_its_reminder_again() {
    let standup = event("standup", now() + Duration::minutes(10), 15, Some(15));
    let mut states = HashMap::new();
    let first = plan_at(
        &event_sources(std::slice::from_ref(&standup), &[], &[], now()),
        &states,
        now(),
    );
    apply(&mut states, &first);
    let old_key = first.newly_due[0].key.clone();

    // Moved a minute later: a new reminder, which goes off.
    let moved = Event {
        start: standup.start + Duration::minutes(1),
        end: standup.end + Duration::minutes(1),
        ..standup
    };
    let plan = plan(
        &event_sources(&[moved], &[], &[], now()),
        &states,
        now() + Duration::seconds(30),
        now(),
        &Utc,
        &first.due_keys(),
        true,
    );
    assert_eq!(plan.newly_due.len(), 1);
    assert_ne!(plan.newly_due[0].key, old_key);
    assert_eq!(plan.cleared, [old_key]);
}

#[test]
fn cancelled_and_declined_events_have_no_reminders() {
    let soon = now() + Duration::minutes(10);
    let events = [
        Event {
            is_cancelled: true,
            ..event("cancelled", soon, 30, Some(15))
        },
        Event {
            response_status: ResponseStatus::Declined,
            ..event("declined", soon, 30, Some(15))
        },
        Event {
            response_status: ResponseStatus::Tentative,
            ..event("tentative", soon, 30, Some(15))
        },
    ];
    let sources = event_sources(&events, &[], &[], now());
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].title, "tentative");
}

#[test]
fn all_day_events_remind_before_local_midnight() {
    // October 20, reminded 15 hours before (9:00 AM the day before).
    let sources = event_sources(&[all_day("trip", 20, 2, 15 * 60)], &[], &[], now());
    assert_eq!(
        sources[0].timing,
        Timing::AllDay {
            first: NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
            after_last: NaiveDate::from_ymd_opt(2026, 10, 22).unwrap(),
            minutes: 900,
        }
    );
    let east = FixedOffset::east_opt(10 * 3600).unwrap();
    let west = FixedOffset::west_opt(10 * 3600).unwrap();
    let in_zone = |tz: &FixedOffset, now| {
        let plan = plan(
            &sources,
            &HashMap::new(),
            now,
            now,
            tz,
            &HashSet::new(),
            true,
        );
        (plan.next_wake, plan.newly_due)
    };
    // Local midnight of the 20th is 14:00 UTC on the 19th at +10 and 10:00
    // UTC on the 20th at -10.
    let (wake, due) = in_zone(&east, now());
    assert!(due.is_empty());
    assert_eq!(wake, Some(at(19, 14, 0) - Duration::hours(15)));
    let (wake, _) = in_zone(&west, now());
    assert_eq!(wake, Some(at(20, 10, 0) - Duration::hours(15)));

    let (_, due) = in_zone(&east, at(18, 23, 30));
    assert_eq!(due[0].starts_at, Some(at(19, 14, 0)));
    assert_eq!(due[0].ends_at, Some(at(21, 14, 0)));
    assert!(!due[0].overdue);
    // The same plan in the other zone (the computer travelled) isn't due.
    let (_, due) = in_zone(&west, at(18, 23, 30));
    assert!(due.is_empty());
}

/// A time zone at +2 that falls back to +1 at 01:00 UTC on October 25,
/// 2026 (as Central Europe does), and skips from midnight to 01:00 on
/// October 28 (as some zones did).
#[derive(Debug, Clone, Copy)]
struct Europe;

impl Europe {
    fn offset_at_utc(utc: &NaiveDateTime) -> FixedOffset {
        let switch = at(25, 1, 0).naive_utc();
        let hours = if *utc < switch { 2 } else { 1 };
        FixedOffset::east_opt(hours * 3600).unwrap()
    }
}

impl TimeZone for Europe {
    type Offset = FixedOffset;

    fn from_offset(_: &FixedOffset) -> Self {
        Europe
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
        self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> MappedLocalTime<FixedOffset> {
        let gap_start = NaiveDate::from_ymd_opt(2026, 10, 28)
            .unwrap()
            .and_time(NaiveTime::MIN);
        if *local >= gap_start && *local < gap_start + Duration::hours(1) {
            return MappedLocalTime::None;
        }
        let candidates: Vec<FixedOffset> = [2, 1]
            .into_iter()
            .map(|h| FixedOffset::east_opt(h * 3600).unwrap())
            .filter(|o| Self::offset_at_utc(&(*local - *o)) == *o)
            .collect();
        match candidates.as_slice() {
            [one] => MappedLocalTime::Single(*one),
            [a, b] => MappedLocalTime::Ambiguous(*a, *b),
            _ => MappedLocalTime::None,
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
        Self::offset_at_utc(&utc.and_time(NaiveTime::MIN))
    }

    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
        Self::offset_at_utc(utc)
    }
}

#[test]
fn all_day_events_follow_daylight_saving_time() {
    let date = |day| NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
    // Before the change midnight is 22:00 UTC; after it, 23:00 UTC.
    assert_eq!(local_midnight(&Europe, date(24)), at(23, 22, 0));
    assert_eq!(local_midnight(&Europe, date(25)), at(24, 22, 0));
    assert_eq!(local_midnight(&Europe, date(26)), at(25, 23, 0));
    // Midnight that doesn't exist: the first time of the day that does.
    assert_eq!(local_midnight(&Europe, date(28)), at(28, 0, 0));

    // The day of the change is 25 hours long; the reminder at midnight.
    let sources = event_sources(&[all_day("fall back", 25, 1, 0)], &[], &[], now());
    let plan = plan(
        &sources,
        &HashMap::new(),
        at(24, 22, 0),
        at(24, 21, 59),
        &Europe,
        &HashSet::new(),
        true,
    );
    let due = &plan.newly_due[0];
    assert_eq!(due.fire_at, at(24, 22, 0));
    assert_eq!(due.ends_at, Some(at(25, 23, 0)));
    assert_eq!(
        due.ends_at.unwrap() - due.starts_at.unwrap(),
        Duration::hours(25)
    );
    // The day after: an hour later in UTC.
    let next = event_sources(&[all_day("after", 26, 1, 0)], &[], &[], now());
    let plan = plan_at(&next, &HashMap::new(), at(25, 22, 30));
    assert!(
        plan.newly_due.is_empty(),
        "midnight in UTC isn't midnight there"
    );
    let plan = super::plan(
        &next,
        &HashMap::new(),
        at(25, 22, 30),
        at(25, 22, 29),
        &Europe,
        &HashSet::new(),
        true,
    );
    assert!(plan.newly_due.is_empty());
    assert_eq!(plan.next_wake, Some(at(25, 23, 0)));
}

#[test]
fn task_reminders_go_off_and_completing_the_task_clears_them() {
    let open = task(
        "pay rent",
        Some(now() - Duration::minutes(1)),
        TaskStatus::InProgress,
    );
    let mut states = HashMap::new();
    let first = plan_at(&task_sources(std::slice::from_ref(&open)), &states, now());
    assert_eq!(titles(&first.newly_due), ["pay rent"]);
    let reminder = &first.newly_due[0];
    assert_eq!(reminder.kind, ReminderKind::Task);
    assert_eq!(reminder.starts_at, None);
    assert!(reminder.overdue);
    apply(&mut states, &first);

    let done = Task {
        status: TaskStatus::Completed,
        ..open
    };
    let plan = plan(
        &task_sources(&[done]),
        &states,
        now(),
        now(),
        &Utc,
        &first.due_keys(),
        true,
    );
    assert!(plan.active.is_empty());
    assert_eq!(plan.cleared, std::slice::from_ref(&reminder.key));
    // Tasks without a reminder have none.
    assert!(task_sources(&[task("x", None, TaskStatus::NotStarted)]).is_empty());
}

fn server_reminder(event: &Event, id: &str, fire_at: DateTime<Utc>) -> ServerReminder {
    ServerReminder {
        event_id: id.into(),
        fire_at,
        change_key: None,
        event_start: event.start,
        event_end: event.end,
        subject: event.subject.clone(),
    }
}

/// Account "a"'s report, fetched at `fetched_at`, when the app had `known`.
fn report(
    fetched_at: DateTime<Utc>,
    reminders: Vec<ServerReminder>,
    known: &[&Event],
) -> ServerReminders {
    ServerReminders {
        account_id: "a".into(),
        window_start: fetched_at - LOOKBACK,
        window_end: fetched_at + LOOKAHEAD,
        fetched_at,
        reminders,
        known_events: known
            .iter()
            .map(|e| (e.remote_id.clone(), KnownEvent::of(e)))
            .collect(),
    }
}

/// When the server says each source's reminder goes off, and until when
/// it's snoozed.
fn server_times(sources: &[ReminderSource]) -> Vec<(&str, Option<ServerTiming>)> {
    sources
        .iter()
        .map(|s| (s.title.as_str(), s.server))
        .collect()
}

fn timing(fire_at: DateTime<Utc>, snoozed_until: Option<DateTime<Utc>>) -> Option<ServerTiming> {
    Some(ServerTiming {
        fire_at,
        snoozed_until,
        as_of: now(),
    })
}

#[test]
fn the_servers_reminders_win_and_dismissals_elsewhere_count() {
    let snoozed = event(
        "snoozed in outlook",
        now() + Duration::minutes(10),
        30,
        Some(15),
    );
    let dismissed = event(
        "dismissed on the phone",
        now() + Duration::minutes(10),
        30,
        Some(15),
    );
    let occurrence = Event {
        is_recurring: true,
        ..event("weekly", now() + Duration::minutes(10), 30, Some(15))
    };
    let later = event("in three weeks", now() + Duration::days(21), 30, Some(15));
    let other = Event {
        account_id: "b".into(),
        ..event("other account", now() + Duration::minutes(10), 30, Some(15))
    };
    let birthday = Event {
        is_recurring: true,
        ..all_day("birthday", 16, 1, 15 * 60)
    };
    let report = report(
        now(),
        vec![
            server_reminder(&snoozed, "snoozed in outlook", now() + Duration::minutes(8)),
            // The series' occurrence under another id.
            server_reminder(
                &occurrence,
                "AAMkOccurrence==",
                now() - Duration::minutes(5),
            ),
            // Midnight in the mailbox's time zone, -7.
            ServerReminder {
                event_start: at(16, 7, 0),
                event_end: at(17, 7, 0),
                ..server_reminder(&birthday, "birthday-occurrence", at(15, 16, 0))
            },
        ],
        &[&snoozed, &dismissed, &occurrence, &birthday],
    );
    let events = [
        snoozed.clone(),
        dismissed,
        occurrence,
        later.clone(),
        other,
        birthday,
    ];
    let sources = event_sources(&events, &[], std::slice::from_ref(&report), now());
    assert_eq!(
        server_times(&sources),
        [
            (
                "snoozed in outlook",
                timing(
                    now() - Duration::minutes(5),
                    Some(now() + Duration::minutes(8))
                )
            ),
            ("weekly", timing(now() - Duration::minutes(5), None)),
            // Outside what the server reported on: its own time.
            ("in three weeks", None),
            ("other account", None),
            ("birthday", timing(at(15, 16, 0), None)),
        ]
    );
    assert!(
        sources
            .iter()
            .all(|s| s.on_server != (s.title == "other account"))
    );
    let plan = plan_at(&sources, &HashMap::new(), now());
    let mut due = titles(&plan.newly_due);
    due.sort();
    assert_eq!(due, ["other account", "weekly"]);
    assert_eq!(plan.next_wake, Some(now() + Duration::minutes(8)));

    // A day on, the report no longer counts.
    let later_on = now() + REPORT_LIFE;
    let sources = event_sources(&events, &[], std::slice::from_ref(&report), later_on);
    assert!(sources.iter().all(|s| s.server.is_none()));
    assert_eq!(sources.len(), events.len());
}

/// A pass after another, as the scheduler runs them.
struct Run {
    states: HashMap<String, ReminderState>,
    due: HashSet<String>,
    checked_until: DateTime<Utc>,
    shown: Vec<String>,
}

impl Run {
    fn new(checked_until: DateTime<Utc>) -> Self {
        Self {
            states: HashMap::new(),
            due: HashSet::new(),
            checked_until,
            shown: Vec::new(),
        }
    }

    /// A pass in UTC that holds what waits for the server.
    fn pass(&mut self, events: &[Event], reports: &[ServerReminders], now: DateTime<Utc>) -> Plan {
        self.pass_in(events, &[], reports, now, &Utc, true)
    }

    fn pass_in<Tz: TimeZone>(
        &mut self,
        events: &[Event],
        calendars: &[Calendar],
        reports: &[ServerReminders],
        now: DateTime<Utc>,
        tz: &Tz,
        hold: bool,
    ) -> Plan {
        let sources = event_sources(events, calendars, reports, now);
        let plan = plan(
            &sources,
            &self.states,
            now,
            self.checked_until,
            tz,
            &self.due,
            hold,
        );
        apply(&mut self.states, &plan);
        self.due = plan.due_keys();
        if plan.held.is_empty() {
            self.checked_until = now;
        }
        self.shown
            .extend(plan.newly_due.iter().map(|r| r.title.clone()));
        plan
    }

    /// What `snooze_reminder` saves, once the server heard of it.
    fn snooze(&mut self, key: &str, until: DateTime<Utc>) {
        let state = self.states.get_mut(key).unwrap();
        state.status = ReminderStatus::Snoozed;
        state.snooze_until = Some(until);
    }

    fn dismiss(&mut self, key: &str) {
        self.states.get_mut(key).unwrap().status = ReminderStatus::Dismissed;
    }
}

#[test]
fn a_snooze_the_server_heard_of_stays_snoozed() {
    let standup = event("standup", now() + Duration::minutes(10), 15, Some(15));
    let events = [standup.clone()];
    let first = report(
        now(),
        vec![server_reminder(
            &standup,
            "standup",
            now() - Duration::minutes(5),
        )],
        &[&standup],
    );
    let mut run = Run::new(now() - Duration::minutes(1));
    let key = run.pass(&events, &[first], now()).newly_due[0].key.clone();
    let until = now() + Duration::minutes(5);
    run.snooze(&key, until);

    // The server says the reminder goes off when the snooze ends: still
    // the same reminder.
    let snoozed = report(
        now() + Duration::minutes(1),
        vec![server_reminder(&standup, "standup", until)],
        &[&standup],
    );
    let plan = run.pass(
        &events,
        std::slice::from_ref(&snoozed),
        now() + Duration::minutes(1),
    );
    assert!(plan.newly_due.is_empty());
    assert_eq!(plan.active.len(), 1);
    assert_eq!(plan.active[0].key, key);
    assert_eq!(plan.active[0].snoozed_until, Some(until));
    assert_eq!(plan.next_wake, Some(until));
    // And it goes off then, once.
    run.pass(&events, std::slice::from_ref(&snoozed), until);
    run.pass(&events, &[snoozed], until + Duration::minutes(3));
    assert_eq!(run.shown, ["standup", "standup"]);
}

fn minutes(n: i64) -> Duration {
    Duration::minutes(n)
}

/// The standup at 09:10 that reminds at 08:55, and the report that says so.
fn standup_and_report() -> (Event, ServerReminders) {
    let standup = event("standup", now() + minutes(10), 30, Some(15));
    let report = report(
        now(),
        vec![server_reminder(&standup, "standup", now() - minutes(5))],
        &[&standup],
    );
    (standup, report)
}

#[test]
fn a_snooze_here_goes_off_once_when_the_report_catches_up_late() {
    let (standup, first) = standup_and_report();
    let events = [standup.clone()];
    let mut run = Run::new(now() - minutes(1));
    let key = run
        .pass(&events, std::slice::from_ref(&first), now())
        .newly_due[0]
        .key
        .clone();
    let until = now() + minutes(5);
    run.snooze(&key, until);
    let plan = run.pass(
        &events,
        std::slice::from_ref(&first),
        now() + Duration::seconds(1),
    );
    assert_eq!(plan.cleared, std::slice::from_ref(&key));
    // No sync before the snooze ends: it goes off again.
    run.pass(&events, &[first], until);
    assert_eq!(run.shown, ["standup", "standup"]);

    // 09:07: the report says the reminder goes off at 09:05.
    let t = until + minutes(2);
    let late = report(
        t,
        vec![server_reminder(&standup, "standup", until)],
        &[&standup],
    );
    let plan = run.pass(&events, &[late], t);
    assert_eq!(run.shown, ["standup", "standup"]);
    assert!(plan.cleared.is_empty());
    assert_eq!(plan.active.len(), 1);
    assert_eq!(plan.active[0].key, key);
    assert!(plan.active[0].is_due());
}

/// Snoozed on a phone, with no sync here until that snooze was over: the
/// reminder was due here all along, and the phone went off again.
#[test]
fn a_snooze_elsewhere_that_ends_unseen_changes_nothing() {
    let (standup, first) = standup_and_report();
    let events = [standup.clone()];
    let mut run = Run::new(now() - minutes(1));
    let key = run
        .pass(&events, std::slice::from_ref(&first), now())
        .newly_due[0]
        .key
        .clone();
    run.pass(&events, &[first], now() + minutes(6));
    // 09:08: it was snoozed on the phone from 09:01 to 09:06.
    let t = now() + minutes(8);
    let late = report(
        t,
        vec![server_reminder(&standup, "standup", now() + minutes(6))],
        &[&standup],
    );
    let plan = run.pass(&events, &[late], t);
    assert_eq!(run.shown, ["standup"]);
    assert!(plan.cleared.is_empty());
    assert_eq!(plan.active[0].key, key);
    assert!(plan.active[0].is_due());
}

/// Snoozed on a phone, with a sync meanwhile: it's snoozed here too, and
/// goes off once when the snooze ends.
#[test]
fn a_snooze_elsewhere_snoozes_here_too() {
    let (standup, first) = standup_and_report();
    let events = [standup.clone()];
    let mut run = Run::new(now() - minutes(1));
    let key = run.pass(&events, &[first], now()).newly_due[0].key.clone();
    let until = now() + minutes(6);
    let snoozed = report(
        now() + minutes(2),
        vec![server_reminder(&standup, "standup", until)],
        &[&standup],
    );
    let plan = run.pass(&events, std::slice::from_ref(&snoozed), now() + minutes(2));
    assert_eq!(plan.cleared, std::slice::from_ref(&key));
    assert_eq!(plan.active.len(), 1);
    assert_eq!(plan.active[0].key, key);
    assert_eq!(plan.active[0].snoozed_until, Some(until));
    assert_eq!(plan.next_wake, Some(until));
    assert_eq!(run.states[&key].pending, None, "the server knows");
    run.pass(&events, std::slice::from_ref(&snoozed), until);
    run.pass(&events, &[snoozed], until + minutes(2));
    assert_eq!(run.shown, ["standup", "standup"]);
}

/// Snoozed here, the server not told yet, after a longer snooze on a phone:
/// this snooze, the latest, counts.
#[test]
fn a_snooze_here_the_server_hasnt_heard_of_wins() {
    let (standup, first) = standup_and_report();
    let events = [standup.clone()];
    let mut run = Run::new(now() - minutes(1));
    let key = run.pass(&events, &[first], now()).newly_due[0].key.clone();
    let phone = report(
        now() + minutes(1),
        vec![server_reminder(&standup, "standup", now() + minutes(30))],
        &[&standup],
    );
    run.pass(&events, std::slice::from_ref(&phone), now() + minutes(1));
    run.snooze(&key, now() + minutes(5));
    run.states.get_mut(&key).unwrap().pending = Some(ServerAction::Snooze);
    run.pass(&events, std::slice::from_ref(&phone), now() + minutes(5));
    assert_eq!(run.shown, ["standup", "standup"]);
}

/// The same meeting in the user's calendar and in a colleague's, shared
/// with the user: the server reminds of the user's.
#[test]
fn a_shared_calendars_copy_of_a_meeting_has_no_reminder() {
    let mine = event("AAMk-mine", now() + minutes(10), 30, Some(15));
    let theirs = Event {
        id: local_id("a", EntityKind::Event, "AAMk-boss"),
        remote_id: "AAMk-boss".into(),
        calendar_id: "boss".into(),
        reminder_minutes: None,
        ..mine.clone()
    };
    let calendars = [calendar("c", true), calendar("boss", false)];
    let report = report(
        now(),
        vec![server_reminder(&mine, "AAMk-mine", now() - minutes(5))],
        &[&mine, &theirs],
    );
    let mut run = Run::new(now() - minutes(1));
    let plan = run.pass_in(
        &[mine.clone(), theirs],
        &calendars,
        &[report],
        now(),
        &Utc,
        true,
    );
    assert_eq!(plan.newly_due.len(), 1);
    assert_eq!(plan.newly_due[0].item_id, mine.id);
}

fn calendar(id: &str, is_default: bool) -> Calendar {
    Calendar {
        id: id.into(),
        account_id: "a".into(),
        remote_id: id.into(),
        name: id.into(),
        color: "#000000".into(),
        can_edit: true,
        is_default,
    }
}

/// An occurrence of a series in both calendars, neither under the id the
/// report gives: the reminder is the user's own calendar's, once.
#[test]
fn a_series_in_two_calendars_has_one_reminder() {
    let mine = Event {
        is_recurring: true,
        ..event("AAMk-mine-occurrence", now() + minutes(10), 30, Some(15))
    };
    let theirs = Event {
        id: local_id("a", EntityKind::Event, "AAMk-boss-occurrence"),
        remote_id: "AAMk-boss-occurrence".into(),
        calendar_id: "boss".into(),
        ..mine.clone()
    };
    let calendars = [calendar("boss", false), calendar("c", true)];
    let report = report(
        now(),
        vec![server_reminder(
            &mine,
            "AAMk-series-occurrence",
            now() - minutes(5),
        )],
        &[&mine, &theirs],
    );
    for events in [
        [theirs.clone(), mine.clone()],
        [mine.clone(), theirs.clone()],
    ] {
        let sources = event_sources(&events, &calendars, std::slice::from_ref(&report), now());
        let reminded: Vec<&str> = sources.iter().map(|s| s.item_id.as_str()).collect();
        assert_eq!(reminded, [mine.id.as_str()]);
        assert!(sources[0].server.is_some());
    }
    // With the default calendar unknown, one that the report names an
    // event of is just as sure.
    let single = event("single", now() + minutes(20), 30, Some(15));
    let report = ServerReminders {
        reminders: vec![
            server_reminder(&mine, "AAMk-series-occurrence", now() - minutes(5)),
            server_reminder(&single, "single", now() + minutes(5)),
        ],
        ..report
    };
    let events = [theirs, mine.clone(), single];
    let sources = event_sources(&events, &[], &[report], now());
    let reminded: Vec<&str> = sources.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(reminded, ["AAMk-mine-occurrence", "single"]);
}

#[test]
fn a_zero_length_event_reminding_at_its_start_goes_off() {
    // "Call John", 15:00 to 15:00, reminding at 15:00.
    let call = event("call john", at(15, 15, 0), 0, Some(0));
    let events = [call.clone()];
    let with_report = report(
        at(15, 14, 0),
        vec![server_reminder(&call, "call john", call.start)],
        &[&call],
    );
    for reports in [vec![], vec![with_report]] {
        let before = at(15, 14, 59);
        let mut run = Run::new(before - minutes(1));
        assert_eq!(
            run.pass(&events, &reports, before).next_wake,
            Some(call.start)
        );
        // The scheduler wakes then, a few milliseconds late.
        let plan = run.pass(&events, &reports, call.start + Duration::milliseconds(3));
        assert_eq!(run.shown, ["call john"]);
        assert!(!plan.active[0].missed);
    }
}

/// A report from before a sync that brought a meeting, or a change to one
/// (reminderView failed then), judges only the events the app had when it
/// was fetched, as they were then.
#[test]
fn a_report_judges_only_the_events_it_knew_of() {
    let dismissed = event("dismissed", now() + minutes(10), 30, Some(15));
    let new = event("new", now() + minutes(10), 30, Some(15));
    // Moved an hour later since.
    let moved = event("moved", now() + minutes(70), 30, Some(15));
    let moved_then = Event {
        start: now() + minutes(10),
        end: now() + minutes(40),
        ..moved.clone()
    };
    // Its reminder dismissed at 08:15, then pushed back from 08:30 to 09:05.
    let pushed_back = event("pushed back", now() + minutes(5), 30, Some(15));
    let pushed_back_then = Event {
        start: now() - minutes(30),
        end: now(),
        ..pushed_back.clone()
    };
    // With no reminder then; one set since.
    let reminded = event("reminded since", now() + minutes(30), 30, Some(45));
    let reminded_then = Event {
        reminder_minutes: None,
        ..reminded.clone()
    };
    let report = report(
        now() - minutes(30),
        vec![server_reminder(&moved_then, "moved", now() - minutes(5))],
        &[&dismissed, &moved_then, &pushed_back_then, &reminded_then],
    );
    let events = [dismissed, new, moved, pushed_back, reminded];
    let sources = event_sources(&events, &[], std::slice::from_ref(&report), now());
    assert_eq!(
        server_times(&sources),
        [
            ("new", None),
            ("moved", None),
            ("pushed back", None),
            ("reminded since", None)
        ]
    );
    assert!(sources.iter().all(|s| s.on_server));
    // Their own reminder times count. With no fresh report to wait for,
    // those that went off already go off; the moved one waits for its
    // time.
    let mut run = Run::new(now() - minutes(1));
    let reports = [report];
    let plan = run.pass_in(&events, &[], &reports, now(), &Utc, false);
    assert_eq!(
        titles(&plan.newly_due),
        ["pushed back", "new", "reminded since"]
    );
    run.pass_in(&events, &[], &reports, now() + minutes(54), &Utc, false);
    assert_eq!(run.shown.len(), 3);
    run.pass_in(&events, &[], &reports, now() + minutes(55), &Utc, false);
    assert_eq!(run.shown, ["pushed back", "new", "reminded since", "moved"]);
}

#[test]
fn on_waking_reminders_wait_for_a_fresh_report() {
    // 09:00 to 10:00, reminding at 08:45; the report is from 08:30, the lid
    // closed at 08:35, and the reminder was dismissed on the phone.
    let standup = event("standup", at(15, 9, 0), 60, Some(15));
    let review = event("review", at(15, 9, 5), 60, Some(0));
    let events = [standup.clone(), review.clone()];
    let stale = report(
        at(15, 8, 30),
        vec![
            server_reminder(&standup, "standup", at(15, 8, 45)),
            server_reminder(&review, "review", at(15, 9, 5)),
        ],
        &[&standup, &review],
    );
    let mut run = Run::new(at(15, 8, 35));
    // Awake at 09:05: the review's reminder is on time.
    let woke = at(15, 9, 5) + Duration::milliseconds(300);
    let plan = run.pass(&events, std::slice::from_ref(&stale), woke);
    assert_eq!(run.shown, ["review"]);
    assert_eq!(plan.held, ["a"]);
    assert_eq!(titles(&plan.active), ["review"]);
    assert_eq!(run.checked_until, at(15, 8, 35));
    // The fresh report leaves the standup's out.
    let fresh = report(
        woke + Duration::seconds(2),
        vec![server_reminder(&review, "review", at(15, 9, 5))],
        &[&standup, &review],
    );
    let plan = run.pass(&events, &[fresh], woke + Duration::seconds(3));
    assert_eq!(run.shown, ["review"]);
    assert!(plan.held.is_empty());

    // No fresh report in time: it goes off, as missed while asleep.
    let mut run = Run::new(at(15, 8, 35));
    run.pass(
        std::slice::from_ref(&standup),
        std::slice::from_ref(&stale),
        woke,
    );
    assert!(run.shown.is_empty());
    let plan = run.pass_in(&[standup], &[], &[stale], at(15, 9, 6), &Utc, false);
    assert_eq!(run.shown, ["standup"]);
    assert!(!plan.active[0].missed);
}

/// A reminder held for the server and carried across a sleep is listed as
/// missed, not dropped, once its meeting is over.
#[test]
fn a_held_reminder_carried_across_a_sleep_is_missed_not_dropped() {
    // An invitation for 09:00 to 09:30, reminding at 08:45, arrives at 09:05;
    // the last report (08:30) never heard of it, and reading a new one fails.
    let invite = event("invite", at(15, 9, 0), 30, Some(15));
    let stale = report(at(15, 8, 30), Vec::new(), &[]);
    let mut run = Run::new(at(15, 9, 4));
    let plan = run.pass(
        std::slice::from_ref(&invite),
        std::slice::from_ref(&stale),
        at(15, 9, 5),
    );
    assert_eq!(plan.held, ["a"]);
    assert_eq!(run.checked_until, at(15, 9, 4));
    // The lid closes before a report comes; at 10:00 the meeting is over.
    let plan = run.pass_in(
        std::slice::from_ref(&invite),
        &[],
        &[stale],
        at(15, 10, 0),
        &Utc,
        false,
    );
    assert!(run.shown.is_empty());
    assert_eq!(titles(&plan.active), ["invite"]);
    assert!(plan.active[0].missed);

    // One over before the app last looked is still let go.
    let mut run = Run::new(at(15, 9, 40));
    let plan = run.pass_in(&[invite], &[], &[], at(15, 10, 0), &Utc, false);
    assert!(plan.active.is_empty());
}

/// Only reminders the server keeps wait for it.
#[test]
fn reminders_the_server_doesnt_keep_never_wait() {
    let standup = event("standup", at(15, 9, 0), 60, Some(15));
    let task = task("call back", Some(at(15, 8, 50)), TaskStatus::NotStarted);
    let mut sources = event_sources(std::slice::from_ref(&standup), &[], &[], now());
    sources.extend(task_sources(&[task]));
    let plan = plan(
        &sources,
        &HashMap::new(),
        at(15, 9, 5),
        at(15, 8, 35),
        &Utc,
        &HashSet::new(),
        true,
    );
    assert_eq!(titles(&plan.newly_due), ["call back", "standup"]);
    assert!(plan.held.is_empty());
}

#[test]
fn a_dismissed_all_day_reminder_stays_dismissed_in_another_time_zone() {
    // The 16th, reminding 18 hours before.
    let events = [all_day("trip", 16, 1, 18 * 60)];
    let new_york = FixedOffset::west_opt(4 * 3600).unwrap();
    let london = FixedOffset::east_opt(3600).unwrap();
    // In New York the reminder goes off at 10:00 UTC the day before.
    let mut run = Run::new(at(15, 10, 29));
    let plan = run.pass_in(&events, &[], &[], at(15, 10, 30), &new_york, true);
    run.dismiss(&plan.newly_due[0].key);
    let plan = run.pass_in(&events, &[], &[], at(15, 20, 0), &london, true);
    assert!(plan.active.is_empty());
    assert_eq!(run.shown, ["trip"]);

    // And the other way: dismissed in London (05:00 UTC), then New York.
    let mut run = Run::new(at(15, 5, 29));
    let plan = run.pass_in(&events, &[], &[], at(15, 5, 30), &london, true);
    run.dismiss(&plan.newly_due[0].key);
    for t in [at(15, 9, 0), at(15, 10, 0), at(15, 10, 1)] {
        run.pass_in(&events, &[], &[], t, &new_york, true);
    }
    assert_eq!(run.shown, ["trip"]);
}

/// An all-day reminder that went off stays the same one whether the time
/// comes from the computer's zone or from the mailbox's.
#[test]
fn an_all_day_reminder_goes_off_once_whatever_the_zone_and_report() {
    let trip = all_day("trip", 16, 1, 18 * 60);
    let events = [trip.clone()];
    let local = FixedOffset::west_opt(4 * 3600).unwrap();
    // Started after a day off, with no report that counts: local times.
    let mut run = Run::new(at(15, 11, 59));
    run.pass_in(&events, &[], &[], at(15, 12, 0), &local, true);
    assert_eq!(run.shown, ["trip"]);
    // The first sync's report: midnight where the mailbox is, in UTC.
    let report = report(
        at(15, 12, 0),
        vec![ServerReminder {
            event_start: at(16, 0, 0),
            event_end: at(17, 0, 0),
            ..server_reminder(&trip, "trip", at(15, 6, 0))
        }],
        &[&trip],
    );
    let plan = run.pass_in(
        &events,
        &[],
        std::slice::from_ref(&report),
        at(15, 12, 1),
        &local,
        true,
    );
    assert!(plan.cleared.is_empty());
    // The computer moves to London; the report ages out.
    let london = FixedOffset::east_opt(3600).unwrap();
    run.pass_in(&events, &[], &[report], at(15, 13, 0), &london, true);
    run.pass_in(&events, &[], &[], at(15, 14, 0), &local, true);
    assert_eq!(run.shown, ["trip"]);
}

#[test]
fn a_snooze_that_ends_after_the_event_is_missed_only_when_asleep() {
    // 09:00 to 09:30, reminding at 08:45, snoozed 5 minutes; asleep from
    // 08:46 to 11:00.
    let call = event("call", at(15, 9, 0), 30, Some(15));
    let events = [call.clone()];
    let mut run = Run::new(at(15, 8, 44));
    let key = run.pass(&events, &[], at(15, 8, 45)).newly_due[0]
        .key
        .clone();
    run.snooze(&key, at(15, 8, 50));
    run.pass(&events, &[], at(15, 8, 46));
    let plan = run.pass(&events, &[], at(15, 11, 0));
    assert_eq!(run.shown, ["call"]);
    assert!(plan.active[0].missed);
    assert_eq!(run.states[&key].status, ReminderStatus::Missed);

    // Awake, and snoozed an hour, past the end: it goes off when asked to,
    // for local and Microsoft 365 events alike.
    let first = report(
        at(15, 8, 45),
        vec![server_reminder(&call, "call", at(15, 8, 45))],
        &[&call],
    );
    let snoozed = report(
        at(15, 8, 50),
        vec![server_reminder(&call, "call", at(15, 9, 45))],
        &[&call],
    );
    for (first, snoozed) in [(vec![], vec![]), (vec![first], vec![snoozed])] {
        let mut run = Run::new(at(15, 8, 44));
        let key = run.pass(&events, &first, at(15, 8, 45)).newly_due[0]
            .key
            .clone();
        run.snooze(&key, at(15, 9, 45));
        run.pass(&events, &snoozed, at(15, 8, 50));
        let plan = run.pass(&events, &snoozed, at(15, 9, 45));
        assert_eq!(run.shown, ["call", "call"]);
        assert!(!plan.active[0].missed);
    }
}

#[test]
fn a_reminder_more_than_a_day_before_a_new_event_goes_off() {
    // Made in Outlook three days ahead, with a one-week reminder.
    let flight = event("flight", now() + Duration::days(3), 120, Some(7 * 24 * 60));
    let events = [flight];
    let mut run = Run::new(now() - minutes(1));
    let plan = run.pass(&events, &[], now());
    assert_eq!(run.shown, ["flight"]);
    assert!(!plan.active[0].overdue);
    run.pass(&events, &[], now() + minutes(1));
    assert_eq!(run.shown, ["flight"]);
    // Missed while asleep for five days: let go.
    let mut run = Run::new(now() - Duration::days(5));
    let plan = run.pass(&events, &[], now());
    assert!(plan.active.is_empty() && run.shown.is_empty());
}

/// A reminder set again once it went off, here or in Outlook (which
/// reminds again then), is a new one: it goes off, dismissed or not.
#[test]
fn a_reminder_set_again_goes_off_again() {
    // Tomorrow's flight at 10:00, reminding a day before; then fifteen
    // minutes before.
    let flight = event("flight", at(16, 10, 0), 120, Some(24 * 60));
    let soon = Event {
        reminder_minutes: Some(15),
        ..flight.clone()
    };
    let mut run = Run::new(at(15, 9, 59));
    let key = run
        .pass(std::slice::from_ref(&flight), &[], at(15, 10, 0))
        .newly_due[0]
        .key
        .clone();
    run.dismiss(&key);
    run.pass(std::slice::from_ref(&flight), &[], at(15, 10, 1));
    let plan = run.pass(std::slice::from_ref(&soon), &[], at(15, 10, 2));
    assert_eq!(plan.next_wake, Some(at(16, 9, 45)));
    run.pass(std::slice::from_ref(&soon), &[], at(16, 9, 45));
    assert_eq!(run.shown, ["flight", "flight"]);

    // On Microsoft 365, where the server says so too.
    let first = report(
        at(15, 9, 0),
        vec![server_reminder(&flight, "flight", at(15, 10, 0))],
        &[&flight],
    );
    let mut run = Run::new(at(15, 9, 59));
    let key = run
        .pass(std::slice::from_ref(&flight), &[first], at(15, 10, 0))
        .newly_due[0]
        .key
        .clone();
    run.dismiss(&key);
    let dismissed = report(at(15, 10, 5), vec![], &[&flight]);
    let plan = run.pass(std::slice::from_ref(&flight), &[dismissed], at(15, 10, 5));
    assert!(plan.active.is_empty());
    let again = report(
        at(16, 9, 0),
        vec![server_reminder(&soon, "flight", at(16, 9, 45))],
        &[&soon],
    );
    let plan = run.pass(
        std::slice::from_ref(&soon),
        std::slice::from_ref(&again),
        at(16, 9, 0),
    );
    assert_eq!(plan.next_wake, Some(at(16, 9, 45)));
    run.pass(std::slice::from_ref(&soon), &[again], at(16, 9, 45));
    assert_eq!(run.shown, ["flight", "flight"]);
}

/// A Microsoft 365 meeting synced only once it was over (while
/// reminderView failed) isn't worth a reminder, so it doesn't wait for the
/// server's word on it either, then or later.
#[test]
fn an_event_found_after_it_was_over_never_waits() {
    let call = event("logged call", at(15, 7, 30), 30, Some(15));
    let standup = event("standup", at(15, 12, 0), 30, Some(15));
    // The last good report, from before the call was synced.
    let stale = report(
        at(15, 7, 0),
        vec![server_reminder(&standup, "standup", at(15, 11, 45))],
        &[&standup],
    );
    let events = [call, standup];
    let mut run = Run::new(at(15, 8, 59));
    for t in [now(), now() + minutes(1), now() + minutes(2)] {
        for hold in [true, false] {
            let plan = run.pass_in(&events, &[], std::slice::from_ref(&stale), t, &Utc, hold);
            assert!(plan.held.is_empty(), "{t} {hold}");
            assert!(plan.active.is_empty() && plan.updates.is_empty());
        }
    }
    // Missed while asleep, it waits: whether it was dismissed elsewhere
    // decides whether it's listed.
    let mut run = Run::new(at(15, 7, 0));
    let plan = run.pass(&events, std::slice::from_ref(&stale), now());
    assert_eq!(plan.held, ["a"]);
}
