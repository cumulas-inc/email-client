use std::collections::HashSet;

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};

use super::{Store, to_json, ts};
use crate::error::{Error, Result};
use crate::model::{Calendar, Event};

const CALENDAR_COLUMNS: &str = "id, account_id, remote_id, name, color, can_edit, is_default";

fn row_to_calendar(row: &Row<'_>) -> rusqlite::Result<Calendar> {
    Ok(Calendar {
        id: row.get(0)?,
        account_id: row.get(1)?,
        remote_id: row.get(2)?,
        name: row.get(3)?,
        color: row.get(4)?,
        can_edit: row.get(5)?,
        is_default: row.get(6)?,
    })
}

fn row_to_event(row: &Row<'_>) -> rusqlite::Result<Event> {
    let json: String = row.get(0)?;
    serde_json::from_str(&json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn insert_event(conn: &rusqlite::Connection, event: &Event) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO events (id, account_id, calendar_id, remote_id, start_at, end_at, data_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
           calendar_id = excluded.calendar_id, remote_id = excluded.remote_id,
           start_at = excluded.start_at, end_at = excluded.end_at, data_json = excluded.data_json",
        params![
            event.id,
            event.account_id,
            event.calendar_id,
            event.remote_id,
            ts(&event.start),
            ts(&event.end),
            to_json(event),
        ],
    )?;
    Ok(())
}

impl Store {
    pub fn replace_calendars(&self, account_id: &str, calendars: &[Calendar]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let keep: HashSet<&str> = calendars.iter().map(|c| c.id.as_str()).collect();
        let existing: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM calendars WHERE account_id = ?1")?;
            stmt.query_map([account_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for id in existing.iter().filter(|id| !keep.contains(id.as_str())) {
            tx.execute("DELETE FROM calendars WHERE id = ?1", [id])?;
        }
        for c in calendars {
            Self::upsert_calendar_in(&tx, c)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn upsert_calendar_in(conn: &rusqlite::Connection, c: &Calendar) -> rusqlite::Result<()> {
        conn.execute(
            &format!(
                "INSERT INTO calendars ({CALENDAR_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, color = excluded.color,
                   can_edit = excluded.can_edit, is_default = excluded.is_default"
            ),
            params![
                c.id,
                c.account_id,
                c.remote_id,
                c.name,
                c.color,
                c.can_edit,
                c.is_default
            ],
        )?;
        Ok(())
    }

    pub fn upsert_calendar(&self, calendar: &Calendar) -> Result<()> {
        Self::upsert_calendar_in(&self.conn(), calendar)?;
        Ok(())
    }

    pub fn list_calendars(&self) -> Result<Vec<Calendar>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CALENDAR_COLUMNS} FROM calendars c
             ORDER BY (SELECT created_at FROM accounts a WHERE a.id = c.account_id),
                      is_default DESC, name COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([], row_to_calendar)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_calendar(&self, id: &str) -> Result<Calendar> {
        self.conn()
            .query_row(
                &format!("SELECT {CALENDAR_COLUMNS} FROM calendars WHERE id = ?1"),
                [id],
                row_to_calendar,
            )
            .optional()?
            .ok_or_else(|| Error::not_found("calendar"))
    }

    /// Replaces every event of `calendar_id` overlapping `[start, end)` with
    /// `events` (the server's view of that window).
    pub fn replace_events_in_range(
        &self,
        calendar_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        events: &[Event],
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM events WHERE calendar_id = ?1 AND start_at < ?3 AND end_at > ?2",
            params![calendar_id, ts(&start), ts(&end)],
        )?;
        for e in events {
            insert_event(&tx, e)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Events overlapping `[start, end)`, optionally restricted to some calendars.
    pub fn list_events(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        calendar_ids: Option<&[String]>,
    ) -> Result<Vec<Event>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT data_json, calendar_id FROM events WHERE start_at < ?2 AND end_at > ?1
             ORDER BY start_at, end_at DESC",
        )?;
        let rows = stmt.query_map(params![ts(&start), ts(&end)], |r| {
            Ok((row_to_event(r)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (event, calendar_id) = row?;
            if calendar_ids.is_none_or(|ids| ids.contains(&calendar_id)) {
                out.push(event);
            }
        }
        Ok(out)
    }

    pub fn get_event(&self, id: &str) -> Result<Event> {
        self.conn()
            .query_row(
                "SELECT data_json FROM events WHERE id = ?1",
                [id],
                row_to_event,
            )
            .optional()?
            .ok_or_else(|| Error::not_found("event"))
    }

    pub fn upsert_event(&self, event: &Event) -> Result<()> {
        insert_event(&self.conn(), event)?;
        Ok(())
    }

    pub fn delete_event(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM events WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::model::{EntityKind, ResponseStatus, ShowAs, local_id};
    use crate::store::test_support::imap_account;

    fn event(cal: &Calendar, remote: &str, start: DateTime<Utc>, hours: i64) -> Event {
        Event {
            id: local_id(&cal.account_id, EntityKind::Event, remote),
            account_id: cal.account_id.clone(),
            calendar_id: cal.id.clone(),
            remote_id: remote.into(),
            subject: remote.into(),
            location: None,
            body_html: None,
            body_preview: None,
            start,
            end: start + Duration::hours(hours),
            is_all_day: false,
            organizer: None,
            attendees: vec![],
            show_as: ShowAs::Busy,
            response_status: ResponseStatus::Organizer,
            is_online_meeting: false,
            online_meeting_url: None,
            reminder_minutes: Some(15),
            is_recurring: false,
            is_cancelled: false,
        }
    }

    #[test]
    fn events_by_range() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&imap_account("a")).unwrap();
        let cal = Calendar {
            id: local_id("a", EntityKind::Calendar, "c"),
            account_id: "a".into(),
            remote_id: "c".into(),
            name: "Calendar".into(),
            color: "#0f6cbd".into(),
            can_edit: true,
            is_default: true,
        };
        store
            .replace_calendars("a", std::slice::from_ref(&cal))
            .unwrap();
        let day = Utc.with_ymd_and_hms(2026, 10, 14, 0, 0, 0).unwrap();
        let e1 = event(&cal, "standup", day + Duration::hours(9), 1);
        let e2 = event(&cal, "tomorrow", day + Duration::hours(33), 1);
        store
            .replace_events_in_range(&cal.id, day, day + Duration::days(7), &[e1.clone(), e2])
            .unwrap();

        let today = store
            .list_events(day, day + Duration::days(1), None)
            .unwrap();
        assert_eq!(today, vec![e1.clone()]);
        assert!(
            store
                .list_events(day, day + Duration::days(1), Some(&["other".into()]))
                .unwrap()
                .is_empty()
        );

        // Replacing a window drops events the server no longer reports.
        store
            .replace_events_in_range(&cal.id, day, day + Duration::days(1), &[])
            .unwrap();
        assert!(store.get_event(&e1.id).is_err());
        assert_eq!(
            store
                .list_events(day, day + Duration::days(7), None)
                .unwrap()
                .len(),
            1
        );
    }
}
