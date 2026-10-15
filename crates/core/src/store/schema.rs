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
}
