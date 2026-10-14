//! Calendars and events on Microsoft Graph.

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use super::GraphProvider;
use super::client::{PREFER_HTML_BODY, PREFER_IMMUTABLE_ID, PREFER_UTC, segment};
use super::types::{
    DateTimeTimeZone, GraphEmailAddress, ItemBody, Recipient, date_time_json, email_address_json,
    midnight, nearest_date, non_empty, null_default,
};
use crate::error::{Error, Result};
use crate::html::text_to_html;
use crate::model::{
    Attendee, AttendeeKind, Calendar, EntityKind, Event, EventDraft, EventResponse, ResponseStatus,
    ShowAs, local_id,
};
use crate::providers::CalendarProvider;

const CALENDAR_FIELDS: &str = "id,name,color,hexColor,canEdit,isDefaultCalendar";
const EVENT_FIELDS: &str = "id,subject,location,body,bodyPreview,start,end,isAllDay,organizer,\
     attendees,showAs,responseStatus,isOnlineMeeting,onlineMeeting,onlineMeetingUrl,isReminderOn,\
     reminderMinutesBeforeStart,type,isCancelled";
/// Immutable ids, times in UTC and HTML bodies for every calendar request.
const PREFER: &[&str] = &[PREFER_IMMUTABLE_ID, PREFER_UTC, PREFER_HTML_BODY];
/// Outlook's default calendar blue.
const DEFAULT_COLOR: &str = "#0078d4";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphCalendar {
    id: String,
    name: Option<String>,
    color: Option<String>,
    hex_color: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    can_edit: bool,
    #[serde(default, deserialize_with = "null_default")]
    is_default_calendar: bool,
}

impl GraphCalendar {
    /// `hexColor` when the user picked a color, else the `color` preset.
    fn color(&self) -> String {
        self.hex_color
            .as_deref()
            .and_then(normalize_hex)
            .unwrap_or_else(|| preset_color(self.color.as_deref().unwrap_or("auto")).into())
    }
}

fn normalize_hex(hex: &str) -> Option<String> {
    let digits = hex.trim().trim_start_matches('#');
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", digits.to_ascii_lowercase()))
}

/// Approximations of Outlook's swatches for the `calendarColor` presets.
fn preset_color(color: &str) -> &'static str {
    match color {
        "lightBlue" => "#3a96dd",
        "lightGreen" => "#4da64d",
        "lightOrange" => "#f7894a",
        "lightGray" => "#8a8886",
        "lightYellow" => "#d9b300",
        "lightTeal" => "#00a5af",
        "lightPink" => "#e3008c",
        "lightBrown" => "#a86c4b",
        "lightRed" => "#e74856",
        _ => DEFAULT_COLOR,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphEvent {
    id: String,
    subject: Option<String>,
    location: Option<GraphLocation>,
    body: Option<ItemBody>,
    body_preview: Option<String>,
    start: Option<DateTimeTimeZone>,
    end: Option<DateTimeTimeZone>,
    #[serde(default, deserialize_with = "null_default")]
    is_all_day: bool,
    organizer: Option<Recipient>,
    #[serde(default, deserialize_with = "null_default")]
    attendees: Vec<GraphAttendee>,
    show_as: Option<String>,
    response_status: Option<GraphResponseStatus>,
    #[serde(default, deserialize_with = "null_default")]
    is_online_meeting: bool,
    online_meeting: Option<OnlineMeetingInfo>,
    online_meeting_url: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    is_reminder_on: bool,
    reminder_minutes_before_start: Option<i64>,
    /// `singleInstance`, `occurrence`, `exception` or `seriesMaster`.
    #[serde(rename = "type")]
    event_type: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    is_cancelled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphLocation {
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphAttendee {
    #[serde(rename = "type")]
    kind: Option<String>,
    status: Option<GraphResponseStatus>,
    email_address: Option<GraphEmailAddress>,
}

#[derive(Debug, Deserialize)]
struct GraphResponseStatus {
    response: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnlineMeetingInfo {
    join_url: Option<String>,
}

fn response_status(status: Option<&GraphResponseStatus>) -> ResponseStatus {
    match status.and_then(|s| s.response.as_deref()) {
        Some("organizer") => ResponseStatus::Organizer,
        Some("accepted") => ResponseStatus::Accepted,
        Some("tentativelyAccepted") => ResponseStatus::Tentative,
        Some("declined") => ResponseStatus::Declined,
        Some("notResponded") => ResponseStatus::NotResponded,
        _ => ResponseStatus::None,
    }
}

fn show_as(value: Option<&str>) -> ShowAs {
    match value {
        Some("free") => ShowAs::Free,
        Some("tentative") => ShowAs::Tentative,
        Some("oof") => ShowAs::OutOfOffice,
        Some("workingElsewhere") => ShowAs::WorkingElsewhere,
        // `busy`, `unknown` and anything new.
        _ => ShowAs::Busy,
    }
}

fn show_as_value(show_as: ShowAs) -> &'static str {
    match show_as {
        ShowAs::Free => "free",
        ShowAs::Tentative => "tentative",
        ShowAs::Busy => "busy",
        ShowAs::OutOfOffice => "oof",
        ShowAs::WorkingElsewhere => "workingElsewhere",
    }
}

impl GraphEvent {
    /// `None` when the times cannot be read.
    fn into_event(self, account_id: &str, calendar_id: &str) -> Option<Event> {
        let start = self.start.as_ref()?.naive()?;
        let end = self.end.as_ref()?.naive()?;
        let (start, end) = if self.is_all_day {
            // All-day events are midnight to midnight in some zone; keep the
            // dates and store them as midnight UTC (see `model::Event`).
            let first = nearest_date(start);
            let after_last = nearest_date(end).max(first.succ_opt()?);
            (midnight(first).and_utc(), midnight(after_last).and_utc())
        } else {
            (start.and_utc(), end.and_utc())
        };
        let body_html = self.body.and_then(|body| {
            let is_html = body.is_html();
            non_empty(body.content).map(|c| if is_html { c } else { text_to_html(&c) })
        });
        let attendees = self
            .attendees
            .iter()
            .filter_map(|a| {
                Some(Attendee {
                    email: a.email_address.as_ref()?.to_model()?,
                    kind: match a.kind.as_deref() {
                        Some("optional") => AttendeeKind::Optional,
                        Some("resource") => AttendeeKind::Resource,
                        _ => AttendeeKind::Required,
                    },
                    response: response_status(a.status.as_ref()),
                })
            })
            .collect();
        Some(Event {
            id: local_id(account_id, EntityKind::Event, &self.id),
            account_id: account_id.to_owned(),
            calendar_id: calendar_id.to_owned(),
            subject: self.subject.unwrap_or_default(),
            location: self.location.and_then(|l| non_empty(l.display_name)),
            body_html,
            body_preview: non_empty(self.body_preview),
            start,
            end,
            is_all_day: self.is_all_day,
            organizer: self.organizer.as_ref().and_then(Recipient::to_model),
            attendees,
            show_as: show_as(self.show_as.as_deref()),
            response_status: response_status(self.response_status.as_ref()),
            online_meeting_url: self
                .online_meeting
                .and_then(|m| non_empty(m.join_url))
                .or_else(|| non_empty(self.online_meeting_url)),
            is_online_meeting: self.is_online_meeting,
            reminder_minutes: self
                .is_reminder_on
                .then(|| {
                    self.reminder_minutes_before_start
                        .and_then(|m| u32::try_from(m).ok())
                })
                .flatten(),
            is_recurring: self
                .event_type
                .as_deref()
                .is_some_and(|t| t != "singleInstance"),
            is_cancelled: self.is_cancelled,
            remote_id: self.id,
        })
    }
}

/// Request body for creating or updating an event. Timed events are sent in
/// UTC; all-day events as midnight in the mailbox time zone so Outlook shows
/// them on the intended days. `online_provider` asks for an online meeting.
fn event_json(draft: &EventDraft, time_zone: &str, online_provider: Option<&str>) -> Value {
    let (start, end) = if draft.is_all_day {
        let first = nearest_date(draft.start.naive_utc());
        let after_last = nearest_date(draft.end.naive_utc()).max(first.succ_opt().unwrap_or(first));
        (
            date_time_json(midnight(first), time_zone),
            date_time_json(midnight(after_last), time_zone),
        )
    } else {
        (
            date_time_json(draft.start.naive_utc(), "UTC"),
            date_time_json(draft.end.naive_utc(), "UTC"),
        )
    };
    let mut json = json!({
        "subject": draft.subject,
        "body": { "contentType": "HTML", "content": draft.body_html.clone().unwrap_or_default() },
        "start": start,
        "end": end,
        "isAllDay": draft.is_all_day,
        "location": { "displayName": draft.location.clone().unwrap_or_default() },
        "attendees": draft
            .attendees
            .iter()
            .map(|a| json!({ "emailAddress": email_address_json(a), "type": "required" }))
            .collect::<Vec<_>>(),
        "showAs": show_as_value(draft.show_as),
        "isReminderOn": draft.reminder_minutes.is_some(),
    });
    if let Some(minutes) = draft.reminder_minutes {
        json["reminderMinutesBeforeStart"] = minutes.into();
    }
    // Graph cannot remove an online meeting once added, so `false` is not
    // sent; it would be ignored or rejected on updates.
    if let Some(provider) = online_provider {
        json["isOnlineMeeting"] = true.into();
        json["onlineMeetingProvider"] = provider.into();
    }
    json
}

impl GraphProvider {
    fn event_url(&self, calendar: &Calendar, remote_id: &str, suffix: &str) -> String {
        self.client.url(&format!(
            "/me/calendars/{}/events/{}{suffix}",
            segment(&calendar.remote_id),
            segment(remote_id)
        ))
    }

    /// Teams for work accounts; otherwise the calendar's default provider
    /// (personal accounts cannot create Teams meetings).
    async fn online_meeting_provider(&self, calendar: &Calendar) -> Result<String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Providers {
            #[serde(default, deserialize_with = "null_default")]
            allowed_online_meeting_providers: Vec<String>,
            default_online_meeting_provider: Option<String>,
        }
        let url = self.client.url_with_query(
            &format!("/me/calendars/{}", segment(&calendar.remote_id)),
            &[(
                "$select",
                "allowedOnlineMeetingProviders,defaultOnlineMeetingProvider",
            )],
        );
        let providers: Providers = self.client.get_json(&url, PREFER).await?;
        let allowed = providers.allowed_online_meeting_providers;
        let usable = |p: &String| !matches!(p.as_str(), "unknown" | "unknownFutureValue");
        if allowed.iter().any(|p| p == "teamsForBusiness") {
            return Ok("teamsForBusiness".into());
        }
        providers
            .default_online_meeting_provider
            .filter(|p| usable(p) && allowed.contains(p))
            .or_else(|| allowed.iter().find(|p| usable(p)).cloned())
            .ok_or_else(|| Error::unsupported("This calendar cannot add online meetings."))
    }

    async fn event_body(&self, calendar: &Calendar, draft: &EventDraft) -> Result<Value> {
        if draft.end < draft.start {
            return Err(Error::invalid_input("The event must end after it starts."));
        }
        let time_zone = if draft.is_all_day {
            self.mailbox_time_zone().await
        } else {
            "UTC".into()
        };
        let provider = if draft.is_online_meeting {
            Some(self.online_meeting_provider(calendar).await?)
        } else {
            None
        };
        Ok(event_json(draft, &time_zone, provider.as_deref()))
    }

    fn to_event(&self, calendar: &Calendar, event: GraphEvent) -> Result<Event> {
        event
            .into_event(&self.account_id, &calendar.id)
            .ok_or_else(|| {
                Error::provider("Microsoft Graph returned an event without valid times.")
            })
    }
}

#[async_trait]
impl CalendarProvider for GraphProvider {
    async fn list_calendars(&self) -> Result<Vec<Calendar>> {
        let url = self.client.url_with_query(
            "/me/calendars",
            &[("$select", CALENDAR_FIELDS), ("$top", "100")],
        );
        let calendars: Vec<GraphCalendar> = self.client.get_all(&url, PREFER).await?;
        Ok(calendars
            .into_iter()
            .map(|c| Calendar {
                id: local_id(&self.account_id, EntityKind::Calendar, &c.id),
                account_id: self.account_id.clone(),
                name: c.name.clone().unwrap_or_default(),
                color: c.color(),
                can_edit: c.can_edit,
                is_default: c.is_default_calendar,
                remote_id: c.id,
            })
            .collect())
    }

    async fn list_events(
        &self,
        calendar: &Calendar,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Event>> {
        // calendarView expands recurring series into occurrences.
        let url = self.client.url_with_query(
            &format!(
                "/me/calendars/{}/calendarView",
                segment(&calendar.remote_id)
            ),
            &[
                (
                    "startDateTime",
                    &start.to_rfc3339_opts(SecondsFormat::Secs, true),
                ),
                (
                    "endDateTime",
                    &end.to_rfc3339_opts(SecondsFormat::Secs, true),
                ),
                ("$select", EVENT_FIELDS),
                ("$top", "100"),
            ],
        );
        let events: Vec<GraphEvent> = self.client.get_all(&url, PREFER).await?;
        Ok(events
            .into_iter()
            .filter_map(|event| {
                let id = event.id.clone();
                let mapped = event.into_event(&self.account_id, &calendar.id);
                if mapped.is_none() {
                    tracing::warn!("skipping event {id}: unreadable start or end");
                }
                mapped
            })
            .collect())
    }

    async fn create_event(&self, calendar: &Calendar, draft: &EventDraft) -> Result<Event> {
        let body = self.event_body(calendar, draft).await?;
        let url = self.client.url(&format!(
            "/me/calendars/{}/events",
            segment(&calendar.remote_id)
        ));
        let created: GraphEvent = self
            .client
            .send_json(Method::POST, &url, &body, PREFER)
            .await?;
        self.to_event(calendar, created)
    }

    async fn update_event(
        &self,
        calendar: &Calendar,
        remote_id: &str,
        draft: &EventDraft,
    ) -> Result<Event> {
        let body = self.event_body(calendar, draft).await?;
        let updated: GraphEvent = self
            .client
            .send_json(
                Method::PATCH,
                &self.event_url(calendar, remote_id, ""),
                &body,
                PREFER,
            )
            .await?;
        self.to_event(calendar, updated)
    }

    async fn delete_event(&self, calendar: &Calendar, remote_id: &str) -> Result<()> {
        // Deleting a meeting the user organizes sends the cancellations.
        self.client
            .send_no_content(
                Method::DELETE,
                &self.event_url(calendar, remote_id, ""),
                None,
                PREFER,
            )
            .await?;
        Ok(())
    }

    async fn respond_to_event(
        &self,
        calendar: &Calendar,
        remote_id: &str,
        response: EventResponse,
        comment: Option<&str>,
        send_response: bool,
    ) -> Result<()> {
        let action = match response {
            EventResponse::Accept => "/accept",
            EventResponse::Tentative => "/tentativelyAccept",
            EventResponse::Decline => "/decline",
        };
        let mut body = json!({ "sendResponse": send_response });
        if let Some(comment) = comment.map(str::trim).filter(|c| !c.is_empty()) {
            body["comment"] = comment.into();
        }
        self.client
            .send_no_content(
                Method::POST,
                &self.event_url(calendar, remote_id, action),
                Some(&body),
                PREFER,
            )
            .await?;
        Ok(())
    }
}
