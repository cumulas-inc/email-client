//! The time zone all-day events and task due dates are anchored in: the
//! mailbox's, else the computer's as a Windows name.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{ACCOUNT, graph_error, ok_json, setup};
use crate::model::{Calendar, EntityKind, EventDraft, ShowAs, local_id};
use crate::providers::CalendarProvider;
use crate::providers::graph::{GraphProvider, tz_zone, windows_time_zone};

#[test]
fn iana_zones_map_to_windows_zones() {
    for (iana, windows) in [
        ("Europe/Amsterdam", "W. Europe Standard Time"),
        ("Europe/Berlin", "W. Europe Standard Time"),
        // Current names CLDR doesn't use, through tzdata's links.
        ("Asia/Kolkata", "India Standard Time"),
        ("Europe/Kyiv", "FLE Standard Time"),
        ("America/Nuuk", "Greenland Standard Time"),
        // CLDR's own old ones.
        ("Asia/Calcutta", "India Standard Time"),
        ("Europe/Kiev", "FLE Standard Time"),
        ("Etc/UTC", "UTC"),
        ("UTC", "UTC"),
        // Old names only tzdata's links know.
        ("US/Eastern", "Eastern Standard Time"),
        ("America/Santa_Isabel", "Pacific Standard Time (Mexico)"),
        // The first and last entries.
        ("Africa/Abidjan", "Greenwich Standard Time"),
        ("Zulu", "UTC"),
    ] {
        assert_eq!(windows_time_zone(Some(iana)), windows, "{iana}");
    }
    for unknown in ["Mars/Olympus_Mons", "", "europe/berlin", "CET-1CEST"] {
        assert_eq!(windows_time_zone(Some(unknown)), "UTC", "{unknown}");
    }
    assert_eq!(windows_time_zone(None), "UTC");
}

#[test]
fn tz_names_a_zone_with_or_without_a_colon() {
    assert_eq!(
        tz_zone(Some("Europe/Berlin")),
        Some("W. Europe Standard Time")
    );
    assert_eq!(
        tz_zone(Some(":Europe/Berlin")),
        Some("W. Europe Standard Time")
    );
    assert_eq!(tz_zone(Some("UTC")), Some("UTC"));
    // A POSIX rule, or nothing: the system's setting counts.
    assert_eq!(tz_zone(Some("CET-1CEST,M3.5.0,M10.5.0/3")), None);
    assert_eq!(tz_zone(Some("")), None);
    assert_eq!(tz_zone(None), None);
}

/// The IANA names Graph's `dateTimeTimeZone` takes besides Windows names
/// ("Additional time zones" in microsoft-graph-docs-contrib's
/// `datetimetimezone.md`). Three the page spells with an old name in
/// parentheses appear here by their IANA name: Asia/Tashkent (listed as
/// Asia/Toshkent), Asia/Almaty (Asia/Astana) and Asia/Yangon.
const GRAPH_ADDITIONAL_TIME_ZONES: &[&str] = &[
    "Etc/GMT+12",
    "Etc/GMT+11",
    "Pacific/Honolulu",
    "America/Anchorage",
    "America/Santa_Isabel",
    "America/Los_Angeles",
    "America/Phoenix",
    "America/Chihuahua",
    "America/Denver",
    "America/Guatemala",
    "America/Chicago",
    "America/Mexico_City",
    "America/Regina",
    "America/Bogota",
    "America/New_York",
    "America/Indiana/Indianapolis",
    "America/Caracas",
    "America/Asuncion",
    "America/Halifax",
    "America/Cuiaba",
    "America/La_Paz",
    "America/Santiago",
    "America/St_Johns",
    "America/Sao_Paulo",
    "America/Argentina/Buenos_Aires",
    "America/Cayenne",
    "America/Godthab",
    "America/Montevideo",
    "America/Bahia",
    "Etc/GMT+2",
    "Atlantic/Azores",
    "Atlantic/Cape_Verde",
    "Africa/Casablanca",
    "Etc/GMT",
    "Europe/London",
    "Atlantic/Reykjavik",
    "Europe/Berlin",
    "Europe/Budapest",
    "Europe/Paris",
    "Europe/Warsaw",
    "Africa/Lagos",
    "Africa/Windhoek",
    "Europe/Bucharest",
    "Asia/Beirut",
    "Africa/Cairo",
    "Asia/Damascus",
    "Africa/Johannesburg",
    "Europe/Kyiv",
    "Europe/Istanbul",
    "Asia/Jerusalem",
    "Asia/Amman",
    "Asia/Baghdad",
    "Europe/Kaliningrad",
    "Asia/Riyadh",
    "Africa/Nairobi",
    "Asia/Tehran",
    "Asia/Dubai",
    "Asia/Baku",
    "Europe/Moscow",
    "Indian/Mauritius",
    "Asia/Tbilisi",
    "Asia/Yerevan",
    "Asia/Kabul",
    "Asia/Karachi",
    "Asia/Tashkent",
    "Asia/Kolkata",
    "Asia/Colombo",
    "Asia/Kathmandu",
    "Asia/Almaty",
    "Asia/Dhaka",
    "Asia/Yekaterinburg",
    "Asia/Yangon",
    "Asia/Bangkok",
    "Asia/Novosibirsk",
    "Asia/Shanghai",
    "Asia/Krasnoyarsk",
    "Asia/Singapore",
    "Australia/Perth",
    "Asia/Taipei",
    "Asia/Ulaanbaatar",
    "Asia/Irkutsk",
    "Asia/Tokyo",
    "Asia/Seoul",
    "Australia/Adelaide",
    "Australia/Darwin",
    "Australia/Brisbane",
    "Australia/Sydney",
    "Pacific/Port_Moresby",
    "Australia/Hobart",
    "Asia/Yakutsk",
    "Pacific/Guadalcanal",
    "Asia/Vladivostok",
    "Pacific/Auckland",
    "Etc/GMT-12",
    "Pacific/Fiji",
    "Asia/Magadan",
    "Pacific/Tongatapu",
    "Pacific/Apia",
    "Pacific/Kiritimati",
];

#[test]
fn every_zone_graph_names_maps() {
    for iana in GRAPH_ADDITIONAL_TIME_ZONES {
        assert!(
            super::super::windows_zones::windows_zone(iana).is_some(),
            "{iana}"
        );
    }
}

fn calendar() -> Calendar {
    Calendar {
        id: local_id(ACCOUNT, EntityKind::Calendar, "AAMkCal"),
        account_id: ACCOUNT.into(),
        remote_id: "AAMkCal".into(),
        name: "Calendar".into(),
        color: "#0b7a75".into(),
        can_edit: true,
        is_default: true,
    }
}

fn all_day() -> EventDraft {
    EventDraft {
        calendar_id: calendar().id,
        subject: "Offsite".into(),
        location: None,
        body_html: None,
        start: Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap(),
        end: Utc.with_ymd_and_hms(2026, 10, 22, 0, 0, 0).unwrap(),
        is_all_day: true,
        attendees: vec![],
        reminder_minutes: None,
        show_as: ShowAs::Free,
        is_online_meeting: false,
    }
}

/// Answers updates of the event, checking they are anchored in `zone`;
/// counts them in `updates`.
async fn mount_update(server: &MockServer, zone: &'static str, updates: Arc<AtomicUsize>) {
    Mock::given(method("PATCH"))
        .and(path("/me/calendars/AAMkCal/events/AAMkEvent-offsite"))
        .respond_with(move |r: &wiremock::Request| {
            let body: Value = r.body_json().unwrap();
            assert_eq!(body["start"]["timeZone"], zone);
            assert_eq!(body["end"]["timeZone"], zone);
            updates.fetch_add(1, Ordering::SeqCst);
            ok_json(json!({
                "id": "AAMkEvent-offsite",
                "subject": "Offsite",
                "start": { "dateTime": "2026-10-20T00:00:00.0000000", "timeZone": "UTC" },
                "end": { "dateTime": "2026-10-22T00:00:00.0000000", "timeZone": "UTC" },
                "isAllDay": true
            }))
        })
        .mount(server)
        .await;
}

async fn update_twice(provider: &GraphProvider) {
    for _ in 0..2 {
        provider
            .update_event(&calendar(), "AAMkEvent-offsite", &all_day())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn without_permission_for_the_mailbox_zone_the_computers_is_used() {
    let (server, mut provider) = setup().await;
    provider.computer_time_zone = || "Tokyo Standard Time".into();
    Mock::given(method("GET"))
        .and(path("/me/mailboxSettings/timeZone"))
        .respond_with(graph_error(
            403,
            "ErrorAccessDenied",
            "Access is denied. Check credentials and try again.",
        ))
        // Once per provider: its providers are made afresh when the
        // permission is granted.
        .expect(1)
        .mount(&server)
        .await;
    let updates = Arc::new(AtomicUsize::new(0));
    mount_update(&server, "Tokyo Standard Time", updates.clone()).await;
    update_twice(&provider).await;
    assert_eq!(updates.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn other_failures_use_the_computers_zone_and_ask_again() {
    let (server, mut provider) = setup().await;
    provider.computer_time_zone = || "Tokyo Standard Time".into();
    Mock::given(method("GET"))
        .and(path("/me/mailboxSettings/timeZone"))
        .respond_with(ResponseTemplate::new(500))
        .expect(2)
        .mount(&server)
        .await;
    let updates = Arc::new(AtomicUsize::new(0));
    mount_update(&server, "Tokyo Standard Time", updates.clone()).await;
    update_twice(&provider).await;
}

#[tokio::test]
async fn an_empty_mailbox_zone_uses_the_computers() {
    let (server, mut provider) = setup().await;
    provider.computer_time_zone = || "Tokyo Standard Time".into();
    Mock::given(method("GET"))
        .and(path("/me/mailboxSettings/timeZone"))
        .respond_with(ok_json(json!({ "value": "" })))
        .expect(1)
        .mount(&server)
        .await;
    let updates = Arc::new(AtomicUsize::new(0));
    mount_update(&server, "Tokyo Standard Time", updates.clone()).await;
    update_twice(&provider).await;
}
