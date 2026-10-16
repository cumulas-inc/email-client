//! Calendars and events.

use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use super::{ACCOUNT, fixture, ok_json, prefers, setup};
use crate::error::ErrorCode;
use crate::model::{
    AttendeeKind, Calendar, EmailAddress, EntityKind, EventDraft, EventResponse, ResponseStatus,
    ShowAs, local_id,
};
use crate::providers::graph::client::{PREFER_IMMUTABLE_ID, PREFER_UTC};
use crate::providers::{CalendarProvider, ServerReminder};

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

#[tokio::test]
async fn calendars_map_colours_and_permissions() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/calendars"))
        .and(query_param("$select", "id,name,color,hexColor,canEdit,isDefaultCalendar"))
        .respond_with(ok_json(json!({
            "value": [
                { "id": "AAMkCal", "name": "Calendar", "color": "auto", "hexColor": "", "isDefaultCalendar": true, "canEdit": true },
                { "id": "AAMkHolidays", "name": "United States holidays", "color": "lightGreen", "hexColor": null, "isDefaultCalendar": false, "canEdit": false },
                { "id": "AAMkFamily", "name": "Family", "color": "lightBlue", "hexColor": "#E3008C", "isDefaultCalendar": false, "canEdit": true }
            ]
        })))
        .mount(&server)
        .await;
    let calendars = provider.list_calendars().await.unwrap();
    assert_eq!(calendars.len(), 3);
    assert_eq!(calendars[0], calendar());
    assert_eq!(calendars[1].color, "#4d7a2e");
    assert!(!calendars[1].can_edit);
    assert!(!calendars[1].is_default);
    assert_eq!(calendars[2].color, "#e3008c");
}

#[tokio::test]
async fn events_are_read_from_the_calendar_view_in_utc() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/calendars/AAMkCal/calendarView"))
        .and(query_param("startDateTime", "2026-10-01T00:00:00Z"))
        .and(query_param("endDateTime", "2026-11-01T00:00:00Z"))
        .and(prefers(PREFER_UTC))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(fixture("calendar_view.json", &server)))
        .expect(1)
        .mount(&server)
        .await;
    let events = provider
        .list_events(
            &calendar(),
            Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap(),
        )
        .await
        .unwrap();
    // The event without times is skipped.
    assert_eq!(events.len(), 4);

    let standup = &events[0];
    assert_eq!(
        standup.id,
        local_id(ACCOUNT, EntityKind::Event, "AAMkEvent-standup")
    );
    assert_eq!(standup.calendar_id, calendar().id);
    assert_eq!(standup.subject, "Team stand-up");
    assert_eq!(standup.location.as_deref(), Some("Room 4.12"));
    assert_eq!(
        standup.body_html.as_deref(),
        Some("<html><body><p>Daily sync.</p></body></html>")
    );
    assert_eq!(standup.body_preview.as_deref(), Some("Daily sync."));
    assert_eq!(
        standup.start,
        Utc.with_ymd_and_hms(2026, 10, 15, 8, 30, 0).unwrap()
    );
    assert_eq!(
        standup.end,
        Utc.with_ymd_and_hms(2026, 10, 15, 8, 45, 0).unwrap()
    );
    assert!(!standup.is_all_day);
    assert_eq!(
        standup.organizer,
        Some(EmailAddress::new(Some("Megan Bowen"), "MeganB@contoso.com"))
    );
    let kinds: Vec<_> = standup
        .attendees
        .iter()
        .map(|a| (a.kind, a.response))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (AttendeeKind::Required, ResponseStatus::Accepted),
            (AttendeeKind::Optional, ResponseStatus::Tentative),
            (AttendeeKind::Resource, ResponseStatus::None),
        ]
    );
    assert_eq!(standup.show_as, ShowAs::Busy);
    assert_eq!(standup.response_status, ResponseStatus::Accepted);
    assert!(standup.is_online_meeting);
    assert!(
        standup
            .online_meeting_url
            .as_deref()
            .unwrap()
            .starts_with("https://teams.microsoft.com/")
    );
    assert_eq!(standup.reminder_minutes, Some(10));
    assert!(standup.is_recurring);
    assert!(!standup.is_cancelled);

    // All-day, returned shifted by the organizer's offset: dates recovered.
    let offsite = &events[1];
    assert!(offsite.is_all_day);
    assert_eq!(
        offsite.start,
        Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap()
    );
    assert_eq!(
        offsite.end,
        Utc.with_ymd_and_hms(2026, 10, 22, 0, 0, 0).unwrap()
    );
    assert_eq!(offsite.location, None);
    assert_eq!(offsite.body_html, None);
    assert_eq!(offsite.show_as, ShowAs::WorkingElsewhere);
    assert_eq!(offsite.response_status, ResponseStatus::Organizer);
    assert_eq!(offsite.reminder_minutes, None, "reminder is off");
    assert!(!offsite.is_recurring);

    let holiday = &events[2];
    assert_eq!(
        holiday.start,
        Utc.with_ymd_and_hms(2026, 10, 24, 0, 0, 0).unwrap()
    );
    assert_eq!(
        holiday.end,
        Utc.with_ymd_and_hms(2026, 10, 25, 0, 0, 0).unwrap()
    );
    assert_eq!(holiday.show_as, ShowAs::OutOfOffice);
    assert!(holiday.attendees.is_empty());

    let canceled = &events[3];
    assert!(canceled.is_cancelled);
    assert_eq!(canceled.show_as, ShowAs::Busy, "unknown shows as busy");
    assert_eq!(canceled.response_status, ResponseStatus::NotResponded);
    assert!(canceled.is_recurring, "exceptions belong to a series");
    assert_eq!(
        canceled.body_html.as_deref(),
        Some("<div style=\"white-space: pre-wrap\">Moved to next week &lt;sorry&gt;</div>")
    );
    assert_eq!(
        canceled.online_meeting_url.as_deref(),
        Some("https://meet.example.com/legacy")
    );
}

fn draft() -> EventDraft {
    EventDraft {
        calendar_id: calendar().id,
        subject: "Planning".into(),
        location: Some("Room 1".into()),
        body_html: Some("<p>Agenda</p>".into()),
        start: Utc.with_ymd_and_hms(2026, 10, 20, 9, 0, 0).unwrap(),
        end: Utc.with_ymd_and_hms(2026, 10, 20, 10, 30, 0).unwrap(),
        is_all_day: false,
        attendees: vec![EmailAddress::new(Some("Pat Kim"), "pat@fabrikam.com")],
        reminder_minutes: Some(15),
        show_as: ShowAs::Tentative,
        is_online_meeting: true,
    }
}

fn created_event(body: &Value) -> Value {
    json!({
        "id": "AAMkEvent-new",
        "subject": body["subject"],
        "start": { "dateTime": "2026-10-20T09:00:00.0000000", "timeZone": "UTC" },
        "end": { "dateTime": "2026-10-20T10:30:00.0000000", "timeZone": "UTC" },
        "isAllDay": body["isAllDay"],
        "showAs": body["showAs"],
        "isOnlineMeeting": true,
        "onlineMeeting": { "joinUrl": "https://teams.microsoft.com/l/meetup-join/new" },
        "responseStatus": { "response": "organizer" },
        "type": "singleInstance"
    })
}

#[tokio::test]
async fn events_are_created_with_a_teams_meeting() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/calendars/AAMkCal"))
        .and(query_param(
            "$select",
            "allowedOnlineMeetingProviders,defaultOnlineMeetingProvider",
        ))
        .respond_with(ok_json(json!({
            "allowedOnlineMeetingProviders": ["teamsForBusiness"],
            "defaultOnlineMeetingProvider": "teamsForBusiness"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let expected = json!({
        "subject": "Planning",
        "body": { "contentType": "HTML", "content": "<p>Agenda</p>" },
        "start": { "dateTime": "2026-10-20T09:00:00", "timeZone": "UTC" },
        "end": { "dateTime": "2026-10-20T10:30:00", "timeZone": "UTC" },
        "isAllDay": false,
        "location": { "displayName": "Room 1" },
        "attendees": [{ "emailAddress": { "name": "Pat Kim", "address": "pat@fabrikam.com" }, "type": "required" }],
        "showAs": "tentative",
        "isReminderOn": true,
        "reminderMinutesBeforeStart": 15,
        "isOnlineMeeting": true,
        "onlineMeetingProvider": "teamsForBusiness",
    });
    Mock::given(method("POST"))
        .and(path("/me/calendars/AAMkCal/events"))
        .and(body_json(expected.clone()))
        .and(prefers(PREFER_UTC))
        .respond_with(ResponseTemplate::new(201).set_body_json(created_event(&expected)))
        .expect(1)
        .mount(&server)
        .await;
    let event = provider.create_event(&calendar(), &draft()).await.unwrap();
    assert_eq!(event.remote_id, "AAMkEvent-new");
    assert_eq!(event.show_as, ShowAs::Tentative);
    assert_eq!(
        event.online_meeting_url.as_deref(),
        Some("https://teams.microsoft.com/l/meetup-join/new")
    );
}

#[tokio::test]
async fn personal_calendars_use_their_own_meeting_provider() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/calendars/AAMkCal"))
        .respond_with(ok_json(json!({
            "allowedOnlineMeetingProviders": ["skypeForConsumer"],
            "defaultOnlineMeetingProvider": "skypeForConsumer"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/calendars/AAMkCal/events"))
        .respond_with(|r: &wiremock::Request| {
            let body: Value = r.body_json().unwrap();
            assert_eq!(body["onlineMeetingProvider"], "skypeForConsumer");
            ResponseTemplate::new(201).set_body_json(created_event(&body))
        })
        .expect(1)
        .mount(&server)
        .await;
    provider.create_event(&calendar(), &draft()).await.unwrap();

    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/calendars/AAMkCal"))
        .respond_with(ok_json(json!({
            "allowedOnlineMeetingProviders": [],
            "defaultOnlineMeetingProvider": "unknown"
        })))
        .mount(&server)
        .await;
    let error = provider
        .create_event(&calendar(), &draft())
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
}

#[tokio::test]
async fn all_day_events_are_anchored_in_the_mailbox_time_zone() {
    let (server, provider) = setup().await;
    Mock::given(method("GET"))
        .and(path("/me/mailboxSettings/timeZone"))
        .respond_with(ok_json(json!({
            "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#users('x')/mailboxSettings/timeZone",
            "value": "W. Europe Standard Time"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/me/calendars/AAMkCal/events/AAMkEvent-offsite"))
        .respond_with(|r: &wiremock::Request| {
            let body: Value = r.body_json().unwrap();
            assert_eq!(
                body["start"],
                json!({ "dateTime": "2026-10-20T00:00:00", "timeZone": "W. Europe Standard Time" })
            );
            assert_eq!(
                body["end"],
                json!({ "dateTime": "2026-10-22T00:00:00", "timeZone": "W. Europe Standard Time" })
            );
            assert_eq!(body["isAllDay"], true);
            assert_eq!(body["isReminderOn"], false);
            assert!(body.get("reminderMinutesBeforeStart").is_none());
            assert!(body.get("isOnlineMeeting").is_none());
            ok_json(json!({
                "id": "AAMkEvent-offsite",
                "subject": body["subject"],
                "start": { "dateTime": "2026-10-19T22:00:00.0000000", "timeZone": "UTC" },
                "end": { "dateTime": "2026-10-21T22:00:00.0000000", "timeZone": "UTC" },
                "isAllDay": true
            }))
        })
        .expect(2)
        .mount(&server)
        .await;
    let all_day = EventDraft {
        start: Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap(),
        end: Utc.with_ymd_and_hms(2026, 10, 22, 0, 0, 0).unwrap(),
        is_all_day: true,
        reminder_minutes: None,
        is_online_meeting: false,
        ..draft()
    };
    let event = provider
        .update_event(&calendar(), "AAMkEvent-offsite", &all_day)
        .await
        .unwrap();
    assert_eq!(
        event.start,
        Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap()
    );
    assert_eq!(
        event.end,
        Utc.with_ymd_and_hms(2026, 10, 22, 0, 0, 0).unwrap()
    );
    // The time zone is read once.
    provider
        .update_event(&calendar(), "AAMkEvent-offsite", &all_day)
        .await
        .unwrap();
}

#[tokio::test]
async fn events_are_deleted_and_answered() {
    let (server, provider) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/me/calendars/AAMkCal/events/AAMkEvent-1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(
            "/me/calendars/AAMkCal/events/AAMkEvent-2/tentativelyAccept",
        ))
        .and(body_json(
            json!({ "comment": "Might be late", "sendResponse": true }),
        ))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/calendars/AAMkCal/events/AAMkEvent-3/decline"))
        .and(body_json(json!({ "sendResponse": false })))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/calendars/AAMkCal/events/AAMkEvent-4/accept"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    let cal = calendar();
    provider.delete_event(&cal, "AAMkEvent-1").await.unwrap();
    provider
        .respond_to_event(
            &cal,
            "AAMkEvent-2",
            EventResponse::Tentative,
            Some("Might be late"),
            true,
        )
        .await
        .unwrap();
    provider
        .respond_to_event(
            &cal,
            "AAMkEvent-3",
            EventResponse::Decline,
            Some("  "),
            false,
        )
        .await
        .unwrap();
    provider
        .respond_to_event(&cal, "AAMkEvent-4", EventResponse::Accept, None, true)
        .await
        .unwrap();
}

#[tokio::test]
async fn events_must_end_after_they_start() {
    let (_server, provider) = setup().await;
    let backwards = EventDraft {
        end: Utc.with_ymd_and_hms(2026, 10, 20, 8, 0, 0).unwrap(),
        ..draft()
    };
    let error = provider
        .create_event(&calendar(), &backwards)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn reminders_are_read_from_the_reminder_view_in_utc() {
    let (server, provider) = setup().await;
    let view =
        "/me/reminderView(startDateTime='2026-10-14T09:00:00',endDateTime='2026-10-30T09:00:00')";
    Mock::given(method("GET"))
        .and(path(view))
        .and(query_param("$skip", "3"))
        .and(prefers(PREFER_UTC))
        .respond_with(ok_json(json!({
            "value": [{
                "eventId": "AAMkEvent-birthday",
                "eventSubject": "Pat's birthday",
                "eventStartTime": { "dateTime": "2026-10-20T07:00:00.0000000", "timeZone": "UTC" },
                "eventEndTime": { "dateTime": "2026-10-21T07:00:00.0000000", "timeZone": "UTC" },
                "reminderFireTime": { "dateTime": "2026-10-19T16:00:00.0000000", "timeZone": "UTC" }
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(view))
        .and(prefers(PREFER_UTC))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .respond_with(ok_json(fixture("reminder_view.json", &server)))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let start = Utc.with_ymd_and_hms(2026, 10, 14, 9, 0, 0).unwrap();
    let reminders = provider
        .reminders(start, start + chrono::Duration::days(16))
        .await
        .unwrap()
        .unwrap();
    let at = |d, h, m, s| Utc.with_ymd_and_hms(2026, 10, d, h, m, s).unwrap();
    assert_eq!(
        reminders,
        [
            ServerReminder {
                event_id: "AAMkEvent-standup".into(),
                fire_at: at(15, 8, 45, 0),
                change_key: Some("SuFHwDRP1EeXJUopWbSLlgAAmBvk2g==".into()),
                event_start: at(15, 9, 0, 0),
                event_end: at(15, 9, 15, 0),
                subject: "Team standup".into(),
            },
            ServerReminder {
                event_id: "AAMkEvent-review".into(),
                fire_at: at(15, 13, 52, 30),
                change_key: Some("SuFHwDRP1EeXJUopWbSLlgAAmBvk3g==".into()),
                event_start: at(15, 14, 0, 0),
                event_end: at(15, 15, 0, 0),
                subject: "Design review".into(),
            },
            ServerReminder {
                event_id: "AAMkEvent-birthday".into(),
                fire_at: at(19, 16, 0, 0),
                change_key: None,
                event_start: at(20, 7, 0, 0),
                event_end: at(21, 7, 0, 0),
                subject: "Pat's birthday".into(),
            },
        ]
    );
}

#[tokio::test]
async fn reminders_are_dismissed_and_snoozed_on_the_event() {
    let (server, provider) = setup().await;
    Mock::given(method("POST"))
        .and(path("/me/events/AAMk%2FEvent==/dismissReminder"))
        .and(prefers(PREFER_IMMUTABLE_ID))
        .and(|r: &wiremock::Request| r.body.is_empty())
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/me/events/AAMkEvent/snoozeReminder"))
        .and(body_json(json!({
            "newReminderTime": { "dateTime": "2026-10-15T09:05:00", "timeZone": "UTC" }
        })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    provider.dismiss_reminder("AAMk/Event==").await.unwrap();
    provider
        .snooze_reminder(
            "AAMkEvent",
            Utc.with_ymd_and_hms(2026, 10, 15, 9, 5, 0).unwrap(),
        )
        .await
        .unwrap();

    // An event that's gone.
    Mock::given(method("POST"))
        .and(path("/me/events/AAMkGone/dismissReminder"))
        .respond_with(super::graph_error(
            404,
            "ErrorItemNotFound",
            "The specified object was not found in the store.",
        ))
        .mount(&server)
        .await;
    let gone = provider.dismiss_reminder("AAMkGone").await.unwrap_err();
    assert_eq!(gone.code, ErrorCode::NotFound);
}
