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
    let mut sources = event_sources(&events, &[]);
    sources.extend(task_sources(&tasks));
    let mut states = HashMap::new();
    let first = plan_at(&sources, &states, now());
    assert_eq!(titles(&first.newly_due), ["standup"]);
    assert_eq!(titles(&first.active), ["standup"]);
    let standup = &first.active[0];
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
    sources.extend(event_sources(&events, &[]));
    let mut states = HashMap::new();
    let woke = plan(
        &sources,
        &states,
        now(),
        at(15, 1, 0),
        &Utc,
        &HashSet::new(),
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
    );
    assert_eq!(titles(&plan.newly_due), ["last night"]);
    assert_eq!(plan.updates.len(), 1);
}

#[test]
fn events_that_are_over_are_listed_but_not_shown() {
    // 07:00 to 08:00, reminded at 06:45, while the computer slept from
    // 06:00.
    let events = [event("early call", at(15, 7, 0), 60, Some(15))];
    let sources = event_sources(&events, &[]);
    let woke = plan(
        &sources,
        &HashMap::new(),
        now(),
        at(15, 6, 0),
        &Utc,
        &HashSet::new(),
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
    let plan = plan_at(&event_sources(&events, &[]), &HashMap::new(), now());
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
    let sources = event_sources(&events, &[]);
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
    let sources = event_sources(&events, &[]);
    let mut states = HashMap::new();
    let first = plan_at(&sources, &states, now());
    apply(&mut states, &first);
    let key = first.newly_due[0].key.clone();
    let due = first.due_keys();

    let mut snoozed = states[&key].clone();
    snoozed.status = ReminderStatus::Snoozed;
    snoozed.snooze_until = Some(now() + Duration::minutes(5));
    states.insert(key.clone(), snoozed);
    let plan = plan(&sources, &states, now(), now(), &Utc, &due);
    assert_eq!(plan.cleared, std::slice::from_ref(&key));
    assert_eq!(plan.active.len(), 1, "snoozed ones stay listed");

    states.get_mut(&key).unwrap().status = ReminderStatus::Dismissed;
    let plan = super::plan(&sources, &states, now(), now(), &Utc, &due);
    assert_eq!(plan.cleared, [key]);
    assert!(plan.active.is_empty());
}

#[test]
fn moving_an_event_sets_its_reminder_again() {
    let standup = event("standup", now() + Duration::minutes(10), 15, Some(15));
    let mut states = HashMap::new();
    let first = plan_at(
        &event_sources(std::slice::from_ref(&standup), &[]),
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
        &event_sources(&[moved], &[]),
        &states,
        now() + Duration::seconds(30),
        now(),
        &Utc,
        &first.due_keys(),
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
    let sources = event_sources(&events, &[]);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].title, "tentative");
}

#[test]
fn all_day_events_remind_before_local_midnight() {
    // October 20, reminded 15 hours before (9:00 AM the day before).
    let sources = event_sources(&[all_day("trip", 20, 2, 15 * 60)], &[]);
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
        let plan = plan(&sources, &HashMap::new(), now, now, tz, &HashSet::new());
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
    let sources = event_sources(&[all_day("fall back", 25, 1, 0)], &[]);
    let plan = plan(
        &sources,
        &HashMap::new(),
        at(24, 22, 0),
        at(24, 21, 59),
        &Europe,
        &HashSet::new(),
    );
    let due = &plan.newly_due[0];
    assert_eq!(due.fire_at, at(24, 22, 0));
    assert_eq!(due.ends_at, Some(at(25, 23, 0)));
    assert_eq!(
        due.ends_at.unwrap() - due.starts_at.unwrap(),
        Duration::hours(25)
    );
    // The day after: an hour later in UTC.
    let next = event_sources(&[all_day("after", 26, 1, 0)], &[]);
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
    let birthday = all_day("birthday", 16, 1, 15 * 60);
    let report = ServerReminders {
        account_id: "a".into(),
        window_start: now() - LOOKBACK,
        window_end: now() + LOOKAHEAD,
        fetched_at: now(),
        reminders: vec![
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
    };
    let events = [
        snoozed.clone(),
        dismissed,
        occurrence,
        later.clone(),
        other,
        birthday,
    ];
    let sources = event_sources(&events, &[report]);
    let fires: Vec<(&str, Option<DateTime<Utc>>)> = sources
        .iter()
        .map(|s| (s.title.as_str(), s.server_fire_at))
        .collect();
    assert_eq!(
        fires,
        [
            ("snoozed in outlook", Some(now() + Duration::minutes(8))),
            ("weekly", Some(now() - Duration::minutes(5))),
            // Outside what the server reported on: its own time.
            ("in three weeks", None),
            ("other account", None),
            ("birthday", Some(at(15, 16, 0))),
        ]
    );
    let plan = plan_at(&sources, &HashMap::new(), now());
    let mut due = titles(&plan.newly_due);
    due.sort();
    assert_eq!(due, ["other account", "weekly"]);
    assert_eq!(plan.next_wake, Some(now() + Duration::minutes(8)));
}

#[test]
fn a_snooze_the_server_heard_of_stays_snoozed() {
    let standup = event("standup", now() + Duration::minutes(10), 15, Some(15));
    let sources = event_sources(std::slice::from_ref(&standup), &[]);
    let mut states = HashMap::new();
    let first = plan_at(&sources, &states, now());
    apply(&mut states, &first);
    let key = first.newly_due[0].key.clone();
    let until = now() + Duration::minutes(5);
    let snoozed = states.get_mut(&key).unwrap();
    snoozed.status = ReminderStatus::Snoozed;
    snoozed.snooze_until = Some(until);

    // The server now says the reminder goes off when the snooze ends.
    let report = ServerReminders {
        account_id: "a".into(),
        window_start: now() - LOOKBACK,
        window_end: now() + LOOKAHEAD,
        fetched_at: now(),
        reminders: vec![server_reminder(&standup, "standup", until)],
    };
    let sources = event_sources(std::slice::from_ref(&standup), &[report]);
    let plan = plan_at(&sources, &states, now() + Duration::minutes(1));
    assert!(plan.newly_due.is_empty());
    assert_eq!(plan.active.len(), 1);
    assert_eq!(plan.active[0].snoozed_until, Some(until));
    assert_ne!(plan.active[0].key, key);
    assert_eq!(plan.next_wake, Some(until));
    // And it goes off then.
    let plan = plan_at(&sources, &states, until);
    assert_eq!(titles(&plan.newly_due), ["standup"]);
}
