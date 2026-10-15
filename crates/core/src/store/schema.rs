//! Schema migrations, tracked with `PRAGMA user_version`.

use rusqlite::Connection;

use crate::error::Result;

const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    r#"
    CREATE TABLE accounts (
        id              TEXT PRIMARY KEY,
        kind            TEXT NOT NULL,
        display_name    TEXT NOT NULL,
        email           TEXT NOT NULL,
        color           TEXT NOT NULL,
        settings_json   TEXT NOT NULL,
        created_at      INTEGER NOT NULL,
        last_synced_at  INTEGER,
        sync_error      TEXT
    );

    CREATE TABLE folders (
        id            TEXT PRIMARY KEY,
        account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        remote_id     TEXT NOT NULL,
        parent_id     TEXT,
        name          TEXT NOT NULL,
        role          TEXT NOT NULL,
        unread_count  INTEGER NOT NULL DEFAULT 0,
        total_count   INTEGER NOT NULL DEFAULT 0,
        sync_state    TEXT
    );
    CREATE INDEX folders_account ON folders(account_id);

    CREATE TABLE messages (
        id               TEXT PRIMARY KEY,
        account_id       TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        folder_id        TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
        remote_id        TEXT NOT NULL,
        conversation_id  TEXT,
        subject          TEXT NOT NULL,
        from_name        TEXT,
        from_address     TEXT,
        to_json          TEXT NOT NULL,
        cc_json          TEXT NOT NULL,
        preview          TEXT NOT NULL,
        received_at      INTEGER NOT NULL,
        is_read          INTEGER NOT NULL,
        is_flagged       INTEGER NOT NULL,
        is_draft         INTEGER NOT NULL,
        has_attachments  INTEGER NOT NULL,
        importance       TEXT NOT NULL,
        -- Filled once the body has been fetched.
        content_json     TEXT,
        body_html        TEXT,
        body_text        TEXT
    );
    CREATE INDEX messages_folder_received ON messages(folder_id, received_at DESC);
    CREATE INDEX messages_account ON messages(account_id);
    CREATE INDEX messages_from ON messages(from_address);

    CREATE VIRTUAL TABLE messages_fts USING fts5(
        message_id UNINDEXED,
        subject,
        sender,
        recipients,
        body,
        tokenize = 'unicode61 remove_diacritics 2'
    );

    CREATE TABLE calendars (
        id          TEXT PRIMARY KEY,
        account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        remote_id   TEXT NOT NULL,
        name        TEXT NOT NULL,
        color       TEXT NOT NULL,
        can_edit    INTEGER NOT NULL,
        is_default  INTEGER NOT NULL
    );

    CREATE TABLE events (
        id           TEXT PRIMARY KEY,
        account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        calendar_id  TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
        remote_id    TEXT NOT NULL,
        start_at     INTEGER NOT NULL,
        end_at       INTEGER NOT NULL,
        data_json    TEXT NOT NULL
    );
    CREATE INDEX events_range ON events(calendar_id, start_at, end_at);

    CREATE TABLE contacts (
        id            TEXT PRIMARY KEY,
        account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        remote_id     TEXT NOT NULL,
        display_name  TEXT NOT NULL,
        -- Lower-cased names and addresses, for search and autocomplete.
        search_text   TEXT NOT NULL,
        data_json     TEXT NOT NULL
    );
    CREATE INDEX contacts_account ON contacts(account_id);

    CREATE TABLE task_lists (
        id          TEXT PRIMARY KEY,
        account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        remote_id   TEXT NOT NULL,
        name        TEXT NOT NULL,
        is_default  INTEGER NOT NULL
    );

    CREATE TABLE tasks (
        id          TEXT PRIMARY KEY,
        account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        list_id     TEXT NOT NULL REFERENCES task_lists(id) ON DELETE CASCADE,
        remote_id   TEXT NOT NULL,
        status      TEXT NOT NULL,
        due_date    TEXT,
        created_at  INTEGER NOT NULL,
        data_json   TEXT NOT NULL
    );
    CREATE INDEX tasks_list ON tasks(list_id);

    CREATE TABLE app_settings (
        key    TEXT PRIMARY KEY,
        value  TEXT NOT NULL
    );
    "#,
    // 2: Microsoft Graph subscriptions push keeps (no clientState: the push
    // service gives it each time)
    r#"
    CREATE TABLE push_subscriptions (
        account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        id          TEXT NOT NULL,
        resource    TEXT NOT NULL,
        expires_at  INTEGER NOT NULL,
        PRIMARY KEY (account_id, id)
    );
    "#,
    // 3: reminders. Task reminder times, for finding them; what became of
    // each reminder (keyed by its item and fire time, so a moved event's
    // reminder starts afresh); and the reminders Microsoft 365 reports, with
    // the window each account's report covers.
    r#"
    ALTER TABLE tasks ADD COLUMN remind_at INTEGER;
    CREATE INDEX tasks_remind_at ON tasks(remind_at) WHERE remind_at IS NOT NULL;

    CREATE TABLE reminder_state (
        key           TEXT PRIMARY KEY,   -- '{event|task}:{local id}:{fire_at}'
        account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        item_kind     TEXT NOT NULL,      -- event | task
        item_id       TEXT NOT NULL,
        fire_at       INTEGER NOT NULL,
        state         TEXT NOT NULL,      -- shown | missed | snoozed | dismissed
        snooze_until  INTEGER,
        -- A dismissal or snooze Microsoft 365 hasn't heard of yet.
        pending       TEXT,               -- dismiss | snooze
        updated_at    INTEGER NOT NULL
    );
    CREATE INDEX reminder_state_item ON reminder_state(item_id);

    CREATE TABLE server_reminders (
        account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        event_id     TEXT NOT NULL,       -- the event's remote id
        fire_at      INTEGER NOT NULL,
        change_key   TEXT,
        -- To match occurrences whose ids differ from the calendar's.
        event_start  INTEGER NOT NULL,
        event_end    INTEGER NOT NULL,
        subject      TEXT NOT NULL,
        PRIMARY KEY (account_id, event_id)
    );

    CREATE TABLE server_reminder_windows (
        account_id    TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
        window_start  INTEGER NOT NULL,
        window_end    INTEGER NOT NULL,
        fetched_at    INTEGER NOT NULL
    );
    "#,
];

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
    }

    #[test]
    fn the_reminders_migration_upgrades_a_database_from_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        {
            // A database as the previous version left it, with a task.
            let conn = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..2] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 2).unwrap();
            conn.execute_batch(
                "INSERT INTO accounts VALUES ('a', 'imap', 'A', 'a@x', '#000', '{}', 0, NULL, NULL);
                 INSERT INTO task_lists VALUES ('l', 'a', 'l', 'Tasks', 1);
                 INSERT INTO tasks VALUES ('t', 'a', 'l', 't', 'notStarted', NULL, 0, '{}');",
            )
            .unwrap();
        }
        let conn = Connection::open(&path).unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 3);
        let remind_at: Option<i64> = conn
            .query_row("SELECT remind_at FROM tasks WHERE id = 't'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(remind_at, None);
        for table in [
            "reminder_state",
            "server_reminders",
            "server_reminder_windows",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
    }
}
