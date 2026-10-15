use std::collections::HashSet;

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};

use super::{Store, date_str, to_json, ts};
use crate::error::{Error, Result};
use crate::model::{Task, TaskList};

const LIST_COLUMNS: &str = "id, account_id, remote_id, name, is_default";

fn row_to_list(row: &Row<'_>) -> rusqlite::Result<TaskList> {
    Ok(TaskList {
        id: row.get(0)?,
        account_id: row.get(1)?,
        remote_id: row.get(2)?,
        name: row.get(3)?,
        is_default: row.get(4)?,
    })
}

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    let json: String = row.get(0)?;
    serde_json::from_str(&json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn upsert_task_in(conn: &rusqlite::Connection, t: &Task) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO tasks (id, account_id, list_id, remote_id, status, due_date, created_at,
           data_json, remind_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET list_id = excluded.list_id, remote_id = excluded.remote_id,
           status = excluded.status, due_date = excluded.due_date, data_json = excluded.data_json,
           remind_at = excluded.remind_at",
        params![
            t.id,
            t.account_id,
            t.list_id,
            t.remote_id,
            t.status.as_str(),
            t.due_date.as_ref().map(date_str),
            ts(&t.created_at),
            to_json(t),
            t.reminder_at.as_ref().map(ts),
        ],
    )?;
    Ok(())
}

fn upsert_list_in(conn: &rusqlite::Connection, l: &TaskList) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO task_lists ({LIST_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, is_default = excluded.is_default"
        ),
        params![l.id, l.account_id, l.remote_id, l.name, l.is_default],
    )?;
    Ok(())
}

impl Store {
    pub fn replace_task_lists(&self, account_id: &str, lists: &[TaskList]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let keep: HashSet<&str> = lists.iter().map(|l| l.id.as_str()).collect();
        let existing: Vec<String> = {
            let mut stmt = tx.prepare("SELECT id FROM task_lists WHERE account_id = ?1")?;
            stmt.query_map([account_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for id in existing.iter().filter(|id| !keep.contains(id.as_str())) {
            tx.execute("DELETE FROM task_lists WHERE id = ?1", [id])?;
        }
        for l in lists {
            upsert_list_in(&tx, l)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_task_list(&self, list: &TaskList) -> Result<()> {
        upsert_list_in(&self.conn(), list)?;
        Ok(())
    }

    pub fn list_task_lists(&self) -> Result<Vec<TaskList>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {LIST_COLUMNS} FROM task_lists l
             ORDER BY (SELECT created_at FROM accounts a WHERE a.id = l.account_id),
                      is_default DESC, name COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([], row_to_list)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_task_list(&self, id: &str) -> Result<TaskList> {
        self.conn()
            .query_row(
                &format!("SELECT {LIST_COLUMNS} FROM task_lists WHERE id = ?1"),
                [id],
                row_to_list,
            )
            .optional()?
            .ok_or_else(|| Error::not_found("task list"))
    }

    pub fn replace_tasks(&self, list_id: &str, tasks: &[Task]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM tasks WHERE list_id = ?1", [list_id])?;
        for t in tasks {
            upsert_task_in(&tx, t)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Tasks of one list (or all lists), open tasks first, then by due date
    /// and creation time.
    pub fn list_tasks(&self, list_id: Option<&str>, include_completed: bool) -> Result<Vec<Task>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT data_json FROM tasks
             WHERE (?1 IS NULL OR list_id = ?1) AND (?2 = 1 OR status != 'completed')
             ORDER BY status = 'completed', due_date IS NULL, due_date, created_at DESC",
        )?;
        let rows = stmt.query_map(params![list_id, include_completed], row_to_task)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Open tasks with a reminder at or after `since`.
    pub fn tasks_with_reminders(&self, since: DateTime<Utc>) -> Result<Vec<Task>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT data_json FROM tasks
             WHERE remind_at IS NOT NULL AND remind_at >= ?1 AND status != 'completed'
             ORDER BY remind_at",
        )?;
        let rows = stmt.query_map([ts(&since)], row_to_task)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_task(&self, id: &str) -> Result<Task> {
        self.conn()
            .query_row(
                "SELECT data_json FROM tasks WHERE id = ?1",
                [id],
                row_to_task,
            )
            .optional()?
            .ok_or_else(|| Error::not_found("task"))
    }

    pub fn upsert_task(&self, task: &Task) -> Result<()> {
        upsert_task_in(&self.conn(), task)?;
        Ok(())
    }

    pub fn delete_task(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, Utc};

    use super::*;
    use crate::model::{EntityKind, Importance, TaskStatus, local_id};
    use crate::store::test_support::imap_account;

    fn task(list: &TaskList, remote: &str, status: TaskStatus, due: Option<NaiveDate>) -> Task {
        Task {
            id: local_id(&list.account_id, EntityKind::Task, remote),
            account_id: list.account_id.clone(),
            list_id: list.id.clone(),
            remote_id: remote.into(),
            title: remote.into(),
            notes: None,
            status,
            importance: Importance::Normal,
            due_date: due,
            reminder_at: None,
            completed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn tasks_ordering_and_filtering() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&imap_account("a")).unwrap();
        let list = TaskList {
            id: local_id("a", EntityKind::TaskList, "l"),
            account_id: "a".into(),
            remote_id: "l".into(),
            name: "Tasks".into(),
            is_default: true,
        };
        store
            .replace_task_lists("a", std::slice::from_ref(&list))
            .unwrap();
        let d = |day| NaiveDate::from_ymd_opt(2026, 10, day);
        store
            .replace_tasks(
                &list.id,
                &[
                    task(&list, "done", TaskStatus::Completed, d(1)),
                    task(&list, "later", TaskStatus::NotStarted, d(20)),
                    task(&list, "undated", TaskStatus::NotStarted, None),
                    task(&list, "soon", TaskStatus::InProgress, d(15)),
                ],
            )
            .unwrap();
        let open: Vec<String> = store
            .list_tasks(Some(&list.id), false)
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert_eq!(open, vec!["soon", "later", "undated"]);
        let all = store.list_tasks(None, true).unwrap();
        assert_eq!(all.last().unwrap().title, "done");
    }

    #[test]
    fn tasks_with_reminders_are_found_by_their_time() {
        use chrono::TimeZone;

        let store = Store::open_in_memory().unwrap();
        store.insert_account(&imap_account("a")).unwrap();
        let list = TaskList {
            id: local_id("a", EntityKind::TaskList, "l"),
            account_id: "a".into(),
            remote_id: "l".into(),
            name: "Tasks".into(),
            is_default: true,
        };
        store.upsert_task_list(&list).unwrap();
        let at = |h| Utc.with_ymd_and_hms(2026, 10, 15, h, 0, 0).unwrap();
        let with = |remote: &str, status, hour: Option<u32>| Task {
            reminder_at: hour.map(at),
            ..task(&list, remote, status, None)
        };
        store
            .replace_tasks(
                &list.id,
                &[
                    with("later", TaskStatus::NotStarted, Some(14)),
                    with("early", TaskStatus::InProgress, Some(6)),
                    with("soon", TaskStatus::NotStarted, Some(10)),
                    with("done", TaskStatus::Completed, Some(11)),
                    with("none", TaskStatus::NotStarted, None),
                ],
            )
            .unwrap();
        let titles = |since| -> Vec<String> {
            store
                .tasks_with_reminders(since)
                .unwrap()
                .into_iter()
                .map(|t| t.title)
                .collect()
        };
        assert_eq!(titles(at(8)), ["soon", "later"]);
        assert_eq!(titles(at(0)), ["early", "soon", "later"]);

        // Clearing the reminder clears the column too.
        let mut soon = store
            .get_task(&local_id("a", EntityKind::Task, "soon"))
            .unwrap();
        assert_eq!(soon.reminder_at, Some(at(10)));
        soon.reminder_at = None;
        store.upsert_task(&soon).unwrap();
        assert_eq!(titles(at(8)), ["later"]);
    }

    #[test]
    fn tasks_saved_before_reminders_read_without_one() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&imap_account("a")).unwrap();
        let list = TaskList {
            id: "l".into(),
            account_id: "a".into(),
            remote_id: "l".into(),
            name: "Tasks".into(),
            is_default: true,
        };
        store.upsert_task_list(&list).unwrap();
        let mut json =
            serde_json::to_value(task(&list, "old", TaskStatus::NotStarted, None)).unwrap();
        json.as_object_mut().unwrap().remove("reminderAt");
        store
            .conn()
            .execute(
                "INSERT INTO tasks (id, account_id, list_id, remote_id, status, created_at, data_json)
                 VALUES ('t', 'a', 'l', 'old', 'notStarted', 0, ?1)",
                [json.to_string()],
            )
            .unwrap();
        assert_eq!(store.get_task("t").unwrap().reminder_at, None);
    }
}
