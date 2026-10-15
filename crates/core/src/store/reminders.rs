//! What became of each reminder (shown, snoozed, dismissed), the reminders
//! Microsoft 365 reports, and when the reminders were last looked at.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};

use super::{Store, from_ts, ts};
use crate::error::Result;
use crate::model::ReminderKind;
use crate::providers::ServerReminder;
use crate::reminders::{KnownEvent, ReminderState, ReminderStatus, ServerAction, ServerReminders};

/// When the reminders were last looked at (see `reminders::plan`).
pub(super) const CHECKED_UNTIL_KEY: &str = "reminders_checked_until";

const STATE_COLUMNS: &str =
    "key, account_id, item_kind, item_id, fire_at, state, snooze_until, pending, updated_at";

fn row_to_state(row: &Row<'_>) -> rusqlite::Result<Option<ReminderState>> {
    let kind: String = row.get(2)?;
    let status: String = row.get(5)?;
    let pending: Option<String> = row.get(7)?;
    let (Some(kind), Some(status)) = (ReminderKind::parse(&kind), ReminderStatus::parse(&status))
    else {
        // Written by a newer version.
        return Ok(None);
    };
    Ok(Some(ReminderState {
        key: row.get(0)?,
        account_id: row.get(1)?,
        kind,
        item_id: row.get(3)?,
        fire_at: from_ts(row.get(4)?),
        status,
        snooze_until: row.get::<_, Option<i64>>(6)?.map(from_ts),
        pending: pending.as_deref().and_then(ServerAction::parse),
        updated_at: from_ts(row.get(8)?),
    }))
}

fn put_state_in(conn: &rusqlite::Connection, s: &ReminderState) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO reminder_state ({STATE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(key) DO UPDATE SET state = excluded.state,
               snooze_until = excluded.snooze_until, pending = excluded.pending,
               updated_at = excluded.updated_at"
        ),
        params![
            s.key,
            s.account_id,
            s.kind.as_str(),
            s.item_id,
            ts(&s.fire_at),
            s.status.as_str(),
            s.snooze_until.as_ref().map(ts),
            s.pending.map(ServerAction::as_str),
            ts(&s.updated_at),
        ],
    )?;
    Ok(())
}

impl Store {
    /// What became of every reminder, by key.
    pub fn reminder_states(&self) -> Result<HashMap<String, ReminderState>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {STATE_COLUMNS} FROM reminder_state"))?;
        let rows = stmt.query_map([], row_to_state)?;
        let mut out = HashMap::new();
        for row in rows {
            if let Some(state) = row? {
                out.insert(state.key.clone(), state);
            }
        }
        Ok(out)
    }

    pub fn reminder_state(&self, key: &str) -> Result<Option<ReminderState>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {STATE_COLUMNS} FROM reminder_state WHERE key = ?1"),
                [key],
                row_to_state,
            )
            .optional()?
            .flatten())
    }

    /// Saves these states, each replacing the one with its key. Skips any
    /// whose account is gone meanwhile.
    pub fn put_reminder_states(&self, states: &[ReminderState]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for state in states {
            match put_state_in(&tx, state) {
                Ok(()) => {}
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::ConstraintViolation => {}
                Err(e) => return Err(e.into()),
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The account's dismissals and snoozes its server hasn't heard of yet.
    pub fn pending_reminder_actions(&self, account_id: &str) -> Result<Vec<ReminderState>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {STATE_COLUMNS} FROM reminder_state
             WHERE account_id = ?1 AND pending IS NOT NULL ORDER BY updated_at"
        ))?;
        let rows = stmt.query_map([account_id], row_to_state)?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect())
    }

    /// The server heard of what was pending for `key` (or never will).
    /// Leaves a newer action alone: only `action` is cleared.
    pub fn clear_reminder_pending(&self, key: &str, action: ServerAction) -> Result<()> {
        self.conn().execute(
            "UPDATE reminder_state SET pending = NULL WHERE key = ?1 AND pending = ?2",
            params![key, action.as_str()],
        )?;
        Ok(())
    }

    /// Forgets what became of reminders that went off before `cutoff` and
    /// haven't changed since, and of reminders whose event or task is
    /// gone. Returns how many were forgotten.
    pub fn prune_reminder_states(&self, cutoff: DateTime<Utc>) -> Result<usize> {
        let cutoff = ts(&cutoff);
        Ok(self.conn().execute(
            "DELETE FROM reminder_state
             WHERE (fire_at < ?1 AND updated_at < ?1 AND coalesce(snooze_until, 0) < ?1)
                OR (item_kind = 'event' AND item_id NOT IN (SELECT id FROM events))
                OR (item_kind = 'task' AND item_id NOT IN (SELECT id FROM tasks))",
            [cutoff],
        )?)
    }

    /// Replaces what the account's server reported about the reminders of
    /// events starting within `[window_start, window_end)`.
    pub fn replace_server_reminders(&self, report: &ServerReminders) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM server_reminders WHERE account_id = ?1",
            [&report.account_id],
        )?;
        for r in &report.reminders {
            tx.execute(
                "INSERT OR REPLACE INTO server_reminders
                   (account_id, event_id, fire_at, change_key, event_start, event_end, subject)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    report.account_id,
                    r.event_id,
                    ts(&r.fire_at),
                    r.change_key,
                    ts(&r.event_start),
                    ts(&r.event_end),
                    r.subject,
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO server_reminder_windows (account_id, window_start, window_end, fetched_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id) DO UPDATE SET window_start = excluded.window_start,
               window_end = excluded.window_end, fetched_at = excluded.fetched_at",
            params![
                report.account_id,
                ts(&report.window_start),
                ts(&report.window_end),
                ts(&report.fetched_at),
            ],
        )?;
        tx.execute(
            "DELETE FROM server_reminder_known WHERE account_id = ?1",
            [&report.account_id],
        )?;
        for (event_id, known) in &report.known_events {
            tx.execute(
                "INSERT OR REPLACE INTO server_reminder_known
                   (account_id, event_id, event_start, event_end, reminder_minutes)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    report.account_id,
                    event_id,
                    ts(&known.start),
                    ts(&known.end),
                    known.reminder_minutes,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every account's last server report on its reminders.
    pub fn server_reminders(&self) -> Result<Vec<ServerReminders>> {
        let conn = self.conn();
        let mut reports: Vec<ServerReminders> = {
            let mut stmt = conn.prepare(
                "SELECT account_id, window_start, window_end, fetched_at
                 FROM server_reminder_windows ORDER BY account_id",
            )?;
            stmt.query_map([], |r| {
                Ok(ServerReminders {
                    account_id: r.get(0)?,
                    window_start: from_ts(r.get(1)?),
                    window_end: from_ts(r.get(2)?),
                    fetched_at: from_ts(r.get(3)?),
                    reminders: Vec::new(),
                    known_events: HashMap::new(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        let mut stmt = conn.prepare(
            "SELECT event_id, fire_at, change_key, event_start, event_end, subject
             FROM server_reminders WHERE account_id = ?1 ORDER BY fire_at, event_id",
        )?;
        let mut known = conn.prepare(
            "SELECT event_id, event_start, event_end, reminder_minutes
             FROM server_reminder_known WHERE account_id = ?1",
        )?;
        for report in &mut reports {
            report.reminders = stmt
                .query_map([&report.account_id], |r| {
                    Ok(ServerReminder {
                        event_id: r.get(0)?,
                        fire_at: from_ts(r.get(1)?),
                        change_key: r.get(2)?,
                        event_start: from_ts(r.get(3)?),
                        event_end: from_ts(r.get(4)?),
                        subject: r.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            report.known_events = known
                .query_map([&report.account_id], |r| {
                    Ok((
                        r.get(0)?,
                        KnownEvent {
                            start: from_ts(r.get(1)?),
                            end: from_ts(r.get(2)?),
                            reminder_minutes: r.get(3)?,
                        },
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
        }
        Ok(reports)
    }

    /// Forgets an account's server report, so that its events' own
    /// reminder times count again.
    pub fn forget_server_reminders(&self, account_id: &str) -> Result<()> {
        let conn = self.conn();
        for table in [
            "server_reminders",
            "server_reminder_windows",
            "server_reminder_known",
        ] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE account_id = ?1"),
                [account_id],
            )?;
        }
        Ok(())
    }

    /// When the reminders were last looked at.
    pub fn reminders_checked_until(&self) -> Result<Option<DateTime<Utc>>> {
        let value: Option<String> = self
            .conn()
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [CHECKED_UNTIL_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.and_then(|v| v.parse::<i64>().ok()).map(from_ts))
    }

    pub fn set_reminders_checked_until(&self, at: DateTime<Utc>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![CHECKED_UNTIL_KEY, ts(&at).to_string()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::store::test_support::imap_account;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 15, hour, 0, 0).unwrap()
    }

    fn state(key: &str, item: &str, status: ReminderStatus) -> ReminderState {
        ReminderState {
            key: key.into(),
            account_id: "a".into(),
            kind: ReminderKind::Event,
            item_id: item.into(),
            fire_at: at(9),
            status,
            snooze_until: None,
            pending: None,
            updated_at: at(9),
        }
    }

    /// A store with account "a" and an event "e1" in its calendar.
    fn store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&imap_account("a")).unwrap();
        store
            .conn()
            .execute_batch(
                "INSERT INTO calendars VALUES ('c', 'a', 'c', 'Calendar', '#000', 1, 1);
                 INSERT INTO events VALUES ('e1', 'a', 'c', 'e1', 0, 0, '{}');
                 INSERT INTO events VALUES ('e2', 'a', 'c', 'e2', 0, 0, '{}');",
            )
            .unwrap();
        store
    }

    #[test]
    fn states_are_saved_replaced_and_read_back() {
        let store = store();
        assert!(store.reminder_states().unwrap().is_empty());
        let shown = state("event:e1:1", "e1", ReminderStatus::Shown);
        store
            .put_reminder_states(std::slice::from_ref(&shown))
            .unwrap();
        assert_eq!(store.reminder_state("event:e1:1").unwrap(), Some(shown));

        let snoozed = ReminderState {
            status: ReminderStatus::Snoozed,
            snooze_until: Some(at(10)),
            pending: Some(ServerAction::Snooze),
            updated_at: at(9) + Duration::minutes(1),
            ..state("event:e1:1", "e1", ReminderStatus::Shown)
        };
        let dismissed = state("event:e2:1", "e2", ReminderStatus::Dismissed);
        store
            .put_reminder_states(&[snoozed.clone(), dismissed.clone()])
            .unwrap();
        let states = store.reminder_states().unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states["event:e1:1"], snoozed);
        assert_eq!(states["event:e2:1"], dismissed);

        assert_eq!(store.pending_reminder_actions("a").unwrap(), [snoozed]);
        // Only the action named is cleared.
        store
            .clear_reminder_pending("event:e1:1", ServerAction::Dismiss)
            .unwrap();
        assert_eq!(store.pending_reminder_actions("a").unwrap().len(), 1);
        store
            .clear_reminder_pending("event:e1:1", ServerAction::Snooze)
            .unwrap();
        assert!(store.pending_reminder_actions("a").unwrap().is_empty());
        assert_eq!(
            store.reminder_state("event:e1:1").unwrap().unwrap().status,
            ReminderStatus::Snoozed
        );
    }

    #[test]
    fn old_states_and_those_of_gone_items_are_pruned() {
        let store = store();
        let old = state("event:e1:1", "e1", ReminderStatus::Dismissed);
        let snoozed_long = ReminderState {
            status: ReminderStatus::Snoozed,
            snooze_until: Some(at(9) + Duration::days(40)),
            ..state("event:e1:2", "e1", ReminderStatus::Shown)
        };
        let recent = ReminderState {
            fire_at: at(9) + Duration::days(2),
            updated_at: at(9) + Duration::days(2),
            ..state("event:e2:1", "e2", ReminderStatus::Shown)
        };
        let gone = state("event:gone:1", "gone", ReminderStatus::Shown);
        let gone_task = ReminderState {
            kind: ReminderKind::Task,
            ..state("task:t:1", "t", ReminderStatus::Shown)
        };
        store
            .put_reminder_states(&[old, snoozed_long, recent, gone, gone_task])
            .unwrap();
        // Thirty days after the day after.
        let pruned = store
            .prune_reminder_states(at(9) + Duration::days(1))
            .unwrap();
        assert_eq!(pruned, 3);
        let mut left: Vec<String> = store.reminder_states().unwrap().into_keys().collect();
        left.sort();
        assert_eq!(left, ["event:e1:2", "event:e2:1"]);
    }

    #[test]
    fn states_go_with_their_account() {
        let store = store();
        store
            .put_reminder_states(&[state("event:e1:1", "e1", ReminderStatus::Shown)])
            .unwrap();
        // An account removed meanwhile: skipped, not an error.
        let orphan = ReminderState {
            account_id: "gone".into(),
            ..state("event:e2:1", "e2", ReminderStatus::Shown)
        };
        store.put_reminder_states(&[orphan]).unwrap();
        store.delete_account("a").unwrap();
        assert!(store.reminder_states().unwrap().is_empty());
    }

    #[test]
    fn server_reports_replace_each_other_per_account() {
        let store = store();
        store.insert_account(&imap_account("b")).unwrap();
        let reminder = |id: &str, hour| ServerReminder {
            event_id: id.into(),
            fire_at: at(hour),
            change_key: Some(format!("ck-{id}")),
            event_start: at(hour) + Duration::minutes(15),
            event_end: at(hour) + Duration::minutes(45),
            subject: format!("Meeting {id}"),
        };
        // With the events the app had then, reminded of or not, as they
        // were.
        let known_event = |hour, reminder_minutes| KnownEvent {
            start: at(hour),
            end: at(hour) + Duration::minutes(30),
            reminder_minutes,
        };
        let report =
            |account: &str, reminders: Vec<ServerReminder>, known: &[&str]| ServerReminders {
                account_id: account.into(),
                window_start: at(0),
                window_end: at(23),
                fetched_at: at(9),
                reminders,
                known_events: known
                    .iter()
                    .enumerate()
                    .map(|(n, id)| {
                        let reminder = (n > 0).then_some(15 * n as u32);
                        (id.to_string(), known_event(10 + n as u32, reminder))
                    })
                    .collect(),
            };
        store
            .replace_server_reminders(&report(
                "a",
                vec![reminder("r2", 11), reminder("r1", 10)],
                &["r1", "r2", "dismissed"],
            ))
            .unwrap();
        store
            .replace_server_reminders(&report("b", vec![reminder("r3", 12)], &["r3"]))
            .unwrap();
        let reports = store.server_reminders().unwrap();
        assert_eq!(
            reports,
            [
                report(
                    "a",
                    vec![reminder("r1", 10), reminder("r2", 11)],
                    &["r1", "r2", "dismissed"]
                ),
                report("b", vec![reminder("r3", 12)], &["r3"]),
            ]
        );
        // An empty report is a report: every reminder was dismissed.
        store
            .replace_server_reminders(&report("a", vec![], &["r1"]))
            .unwrap();
        assert_eq!(
            store.server_reminders().unwrap()[0],
            report("a", vec![], &["r1"])
        );
        store.forget_server_reminders("a").unwrap();
        assert_eq!(store.server_reminders().unwrap().len(), 1);
        store.delete_account("b").unwrap();
        assert!(store.server_reminders().unwrap().is_empty());
        let known: i64 = store
            .conn()
            .query_row("SELECT count(*) FROM server_reminder_known", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(known, 0);
    }

    #[test]
    fn the_last_check_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        let store = Store::open(&path).unwrap();
        assert_eq!(store.reminders_checked_until().unwrap(), None);
        store.set_reminders_checked_until(at(9)).unwrap();
        store.set_reminders_checked_until(at(10)).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.reminders_checked_until().unwrap(), Some(at(10)));
        assert!(store.set_app_value(CHECKED_UNTIL_KEY, "0").is_err());
    }
}
