//! Graph JSON shapes shared by several domains, and their conversions.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use crate::model::EmailAddress;

/// Deserializes `null` like a missing field (Graph sends both).
pub(super) fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

/// Trimmed text, `None` when empty.
pub(super) fn non_empty(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct GraphEmailAddress {
    pub name: Option<String>,
    pub address: Option<String>,
}

impl GraphEmailAddress {
    /// `None` without an address (e.g. unresolved Exchange recipients).
    pub fn to_model(&self) -> Option<EmailAddress> {
        let address = self.address.as_deref()?.trim();
        (!address.is_empty()).then(|| EmailAddress::new(self.name.as_deref(), address))
    }
}

/// `recipient`, `attendee`-base and `organizer` shape: `{ emailAddress }`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Recipient {
    pub email_address: Option<GraphEmailAddress>,
}

impl Recipient {
    pub fn to_model(&self) -> Option<EmailAddress> {
        self.email_address.as_ref()?.to_model()
    }
}

pub(super) fn addresses(recipients: &[Recipient]) -> Vec<EmailAddress> {
    recipients.iter().filter_map(Recipient::to_model).collect()
}

pub(super) fn email_address_json(address: &EmailAddress) -> Value {
    match &address.name {
        Some(name) => json!({ "address": address.address, "name": name }),
        None => json!({ "address": address.address }),
    }
}

pub(super) fn recipients_json(addresses: &[EmailAddress]) -> Value {
    addresses
        .iter()
        .map(|a| json!({ "emailAddress": email_address_json(a) }))
        .collect()
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ItemBody {
    pub content_type: Option<String>,
    pub content: Option<String>,
}

impl ItemBody {
    pub fn is_html(&self) -> bool {
        self.content_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("html"))
    }
}

/// Graph `dateTimeTimeZone`: a wall-clock time plus the zone it is in.
/// Calendar reads ask for UTC, and the dates of To Do tasks are recovered
/// with [`nearest_date`]; the zone counts for To Do reminders
/// ([`DateTimeTimeZone::instant`]).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DateTimeTimeZone {
    pub date_time: String,
    #[serde(default)]
    pub time_zone: Option<String>,
}

impl DateTimeTimeZone {
    /// The wall-clock time as given (`2026-10-14T09:00:00.0000000`).
    pub fn naive(&self) -> Option<NaiveDateTime> {
        NaiveDateTime::parse_from_str(self.date_time.trim(), "%Y-%m-%dT%H:%M:%S%.f").ok()
    }

    /// The instant, reading the time as UTC. Calendar reads ask for UTC with
    /// `Prefer: outlook.timezone="UTC"`.
    pub fn utc(&self) -> Option<DateTime<Utc>> {
        self.naive().map(|n| n.and_utc())
    }

    /// The instant, reading the time in its zone. To Do keeps a reminder in
    /// the zone it was set in, UTC when this app set it. Other zones are
    /// read as the computer's own: the person set it where they are, and
    /// Windows zone names can't be converted here.
    pub fn instant(&self) -> Option<DateTime<Utc>> {
        self.instant_in(&chrono::Local)
    }

    /// [`Self::instant`], with `local` as the computer's zone.
    fn instant_in<Tz: TimeZone>(&self, local: &Tz) -> Option<DateTime<Utc>> {
        let naive = self.naive()?;
        let utc = self.time_zone.as_deref().is_none_or(|zone| {
            matches!(
                zone.trim(),
                "" | "UTC"
                    | "Etc/UTC"
                    | "Etc/GMT"
                    | "GMT"
                    | "Coordinated Universal Time"
                    | "tzone://Microsoft/Utc"
            )
        });
        if utc {
            return Some(naive.and_utc());
        }
        local
            .from_local_datetime(&naive)
            .earliest()
            .map(|t| t.with_timezone(&Utc))
            .or(Some(naive.and_utc()))
    }
}

pub(super) fn date_time_json(at: NaiveDateTime, time_zone: &str) -> Value {
    json!({
        "dateTime": at.format("%Y-%m-%dT%H:%M:%S").to_string(),
        "timeZone": time_zone,
    })
}

/// The calendar date nearest to `at`. Dates stored as "midnight in some time
/// zone" (all-day events, due dates, birthdays) come back shifted by that
/// zone's UTC offset; rounding to the nearest midnight recovers the intended
/// date for offsets of less than 12 hours.
pub(super) fn nearest_date(at: NaiveDateTime) -> NaiveDate {
    let date = at.date();
    if at.hour() >= 12 {
        date.succ_opt().unwrap_or(date)
    } else {
        date
    }
}

pub(super) fn midnight(date: NaiveDate) -> NaiveDateTime {
    date.and_time(NaiveTime::MIN)
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").unwrap()
    }

    #[test]
    fn nearest_date_rounds_to_the_closest_midnight() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        assert_eq!(nearest_date(at("2026-10-14T00:00:00")), day);
        assert_eq!(nearest_date(at("2026-10-13T22:00:00")), day);
        assert_eq!(nearest_date(at("2026-10-14T07:00:00")), day);
        assert_eq!(nearest_date(at("2026-10-14T11:59:59")), day);
    }

    #[test]
    fn date_times_parse_with_fractional_seconds() {
        let t = DateTimeTimeZone {
            date_time: "2026-10-14T09:30:00.0000000".into(),
            time_zone: None,
        };
        assert_eq!(t.utc().unwrap().to_rfc3339(), "2026-10-14T09:30:00+00:00");
    }

    #[test]
    fn instants_read_utc_as_utc_and_other_zones_as_local() {
        let at = |zone: &str| DateTimeTimeZone {
            date_time: "2026-10-14T09:30:00.0000000".into(),
            time_zone: Some(zone.into()),
        };
        // Whatever the zone the tests run in (UTC in CI).
        let west = FixedOffset::west_opt(7 * 3600).unwrap();
        let east = FixedOffset::east_opt(10 * 3600).unwrap();
        let utc = "2026-10-14T09:30:00+00:00";
        for zone in ["UTC", "Etc/UTC", " tzone://Microsoft/Utc "] {
            for local in [west, east] {
                let instant = at(zone).instant_in(&local).unwrap();
                assert_eq!(instant.to_rfc3339(), utc, "{zone}");
            }
        }
        let pacific = at("Pacific Standard Time");
        assert_eq!(
            pacific.instant_in(&west).unwrap().to_rfc3339(),
            "2026-10-14T16:30:00+00:00"
        );
        assert_eq!(
            pacific.instant_in(&east).unwrap().to_rfc3339(),
            "2026-10-13T23:30:00+00:00"
        );
        let local = chrono::Local
            .with_ymd_and_hms(2026, 10, 14, 9, 30, 0)
            .earliest()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(pacific.instant(), Some(local));
    }
}
