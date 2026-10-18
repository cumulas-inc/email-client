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
    // each reminder (keyed by its item and when it goes off by the item's
    // own reminder time, so a moved event's, or a changed reminder, starts
    // afresh); and the reminders Microsoft 365 reports, with the window
    // each account's report covers and the events it knew of.
    r#"
    ALTER TABLE tasks ADD COLUMN remind_at INTEGER;
    CREATE INDEX tasks_remind_at ON tasks(remind_at) WHERE remind_at IS NOT NULL;

    CREATE TABLE reminder_state (
        key           TEXT PRIMARY KEY,   -- '{event|task}:{local id}:{own reminder time}'
        account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        item_kind     TEXT NOT NULL,      -- event | task
        item_id       TEXT NOT NULL,
        fire_at       INTEGER NOT NULL,   -- when it last went off
        state         TEXT NOT NULL,      -- shown | missed | snoozed | dismissed
        snooze_until  INTEGER,
        -- A dismissal or snooze Microsoft 365 hasn't heard of yet.
        pending       TEXT,               -- dismiss | snooze
        updated_at    INTEGER NOT NULL
    );
    CREATE INDEX reminder_state_account ON reminder_state(account_id);

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

    -- The events in the window as the app had them when the report was
    -- fetched: only these, as they were, can be missing from it for being
    -- dismissed.
    CREATE TABLE server_reminder_known (
        account_id        TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        event_id          TEXT NOT NULL,       -- the event's remote id
        event_start       INTEGER NOT NULL,
        event_end         INTEGER NOT NULL,
        reminder_minutes  INTEGER,
        PRIMARY KEY (account_id, event_id)
    );
    "#,
    // 4: signatures. The person's own for each account, and what the
    // account's organization last said about its signature (Cumulas
    // Signatures), with when the app last asked and when to ask again.
    r#"
    CREATE TABLE account_signatures (
        account_id        TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
        own_html          TEXT NOT NULL DEFAULT '',
        own_on_replies    INTEGER NOT NULL DEFAULT 1,
        -- The API scope the answer came for (it names the service and its
        -- app registration): an answer from another isn't used.
        org_scope         TEXT,
        org_state         TEXT NOT NULL DEFAULT 'unchecked',  -- unchecked | notUsed | ready | addedOnSend | notAllowed
        org_html          TEXT,
        org_on_replies    INTEGER NOT NULL DEFAULT 0,
        org_etag          TEXT,
        org_checked_at    INTEGER,            -- the last answer
        org_attempted_at  INTEGER,            -- the last try, answered or not
        org_stale         INTEGER NOT NULL DEFAULT 0,  -- the last try failed
        org_retry_at      INTEGER             -- not before (the service's Retry-After)
    );
    "#,
    // 5: search. The full-text index keyed by a rowid the message row keeps,
    // so a message's index rows are found without scanning the index (they
    // were found by an unindexed column); To and Cc apart, for to: and cc:;
    // a trigram index of subjects and senders, for matches inside words; and
    // the rows server search fetched from outside the synced window.
    r#"
    ALTER TABLE messages ADD COLUMN fts_rowid INTEGER;
    ALTER TABLE messages ADD COLUMN search_only INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE messages ADD COLUMN search_seen_at INTEGER;

    -- The old index's body text (HTML already converted), latest row per message.
    CREATE TEMP TABLE fts_v4 AS
      SELECT message_id, body FROM messages_fts
      WHERE rowid IN (SELECT MAX(rowid) FROM messages_fts GROUP BY message_id);
    CREATE INDEX temp.fts_v4_id ON fts_v4(message_id);
    DROP TABLE messages_fts;

    -- A starting value, copied: stable from here on, unlike the rowid of a
    -- table with a TEXT primary key, which VACUUM may renumber.
    UPDATE messages SET fts_rowid = rowid;
    CREATE UNIQUE INDEX messages_fts_rowid ON messages(fts_rowid);
    CREATE INDEX messages_search_only ON messages(account_id, search_seen_at)
      WHERE search_only = 1;

    CREATE VIRTUAL TABLE messages_fts USING fts5(
        subject, sender, to_recipients, cc_recipients, body,
        tokenize = 'unicode61 remove_diacritics 2'
    );
    CREATE VIRTUAL TABLE messages_infix USING fts5(
        subject, sender,
        tokenize = 'trigram remove_diacritics 1'
    );

    INSERT INTO messages_fts (rowid, subject, sender, to_recipients, cc_recipients, body)
    SELECT m.fts_rowid, m.subject,
           TRIM(COALESCE(m.from_name, '') || ' ' || COALESCE(m.from_address, '')),
           (SELECT COALESCE(group_concat(TRIM(COALESCE(json_extract(r.value, '$.name'), '')
                     || ' ' || json_extract(r.value, '$.address')), ' '), '')
              FROM json_each(CASE WHEN json_valid(m.to_json) THEN m.to_json ELSE '[]' END) r),
           (SELECT COALESCE(group_concat(TRIM(COALESCE(json_extract(r.value, '$.name'), '')
                     || ' ' || json_extract(r.value, '$.address')), ' '), '')
              FROM json_each(CASE WHEN json_valid(m.cc_json) THEN m.cc_json ELSE '[]' END) r),
           COALESCE((SELECT o.body FROM fts_v4 o WHERE o.message_id = m.id),
                    m.body_text, m.preview)
    FROM messages m;

    INSERT INTO messages_infix (rowid, subject, sender)
    SELECT fts_rowid, subject, TRIM(COALESCE(from_name, '') || ' ' || COALESCE(from_address, ''))
    FROM messages;

    DROP TABLE fts_v4;

    -- FK cascades (account removed, folder gone) fire this too.
    CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages
    WHEN old.fts_rowid IS NOT NULL BEGIN
        DELETE FROM messages_fts WHERE rowid = old.fts_rowid;
        DELETE FROM messages_infix WHERE rowid = old.fts_rowid;
    END;
    "#,
    // 6: what each Microsoft account was granted (JSON, see MicrosoftGrant); NULL until first recorded.
    r#"
    ALTER TABLE accounts ADD COLUMN microsoft_grant TEXT;
    "#,
    // 7: protected mail. Per account: what its setup checks found (JSON, see
    // store::protection::StoredChecks), and the sensitivity labels Microsoft
    // 365 last listed (JSON, see model::LabelCatalog).
    r#"
    CREATE TABLE account_protection (
        account_id         TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
        checks_json        TEXT NOT NULL DEFAULT '{}',
        catalog_json       TEXT,              -- NULL until first listed, or after the permission went
        catalog_listed_at  INTEGER,           -- the last successful listing
        catalog_error      TEXT,              -- the last listing's failure; NULL after a success
        catalog_failures   INTEGER NOT NULL DEFAULT 0,  -- failures in a row (backoff); 0 after a success
        catalog_failed_at  INTEGER            -- the last failure
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
        assert_eq!(version as usize, MIGRATIONS.len());
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
            "server_reminder_known",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
        // Reminder states are looked up by key, and by account (an account
        // removed, its dismissals the server hasn't heard of).
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT key FROM reminder_state
                 WHERE account_id = 'a' AND pending IS NOT NULL",
                [],
                |r| r.get(3),
            )
            .unwrap();
        assert!(plan.contains("reminder_state_account"), "{plan}");
    }

    #[test]
    fn the_signatures_migration_upgrades_a_database_from_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        {
            // A database as the previous version left it, with an account.
            let conn = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..3] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 3).unwrap();
            conn.execute_batch(
                "INSERT INTO accounts VALUES ('a', 'imap', 'A', 'a@x', '#000', '{}', 0, NULL, NULL);",
            )
            .unwrap();
        }
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let count: i64 = conn
            .query_row("SELECT count(*) FROM account_signatures", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        // A row takes the defaults, and goes with its account.
        conn.execute(
            "INSERT INTO account_signatures (account_id) VALUES ('a')",
            [],
        )
        .unwrap();
        let (own, on_replies, state, stale): (String, bool, String, bool) = conn
            .query_row(
                "SELECT own_html, own_on_replies, org_state, org_stale FROM account_signatures",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (own.as_str(), on_replies, state.as_str(), stale),
            ("", true, "unchecked", false)
        );
        conn.execute("DELETE FROM accounts WHERE id = 'a'", [])
            .unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM account_signatures", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        // Only for accounts that exist.
        assert!(
            conn.execute(
                "INSERT INTO account_signatures (account_id) VALUES ('gone')",
                []
            )
            .is_err()
        );
    }

    #[test]
    fn the_grant_migration_keeps_accounts_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        {
            // A database as the previous version left it, with a Microsoft
            // account.
            let conn = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..5] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 5).unwrap();
            conn.execute_batch(
                r#"INSERT INTO accounts VALUES ('a', 'microsoft', 'A', 'a@contoso.com', '#000',
                     '{"kind":"microsoft","tenantId":"t","userId":"u"}', 0, NULL, NULL);"#,
            )
            .unwrap();
        }
        let store = crate::store::Store::open(&path).unwrap();
        let account = store.get_account("a").unwrap();
        // Everything it had, which never included sensitivity labels.
        assert_eq!(
            account.needs_permission,
            [crate::model::AccountFeature::SensitivityLabels]
        );
        assert!(account.refused_features.is_empty());
        assert_eq!(store.microsoft_grant("a").unwrap(), None);
        drop(store);
        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let grant: Option<String> = conn
            .query_row("SELECT microsoft_grant FROM accounts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(grant, None);
    }

    #[test]
    fn the_protection_migration_adds_a_table_that_goes_with_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        {
            // A database as the previous version left it, with an account.
            let conn = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..6] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 6).unwrap();
            conn.execute_batch(
                r#"INSERT INTO accounts VALUES ('a', 'microsoft', 'A', 'a@contoso.com', '#000',
                     '{"kind":"microsoft","tenantId":"t","userId":"u"}', 0, NULL, NULL, NULL);"#,
            )
            .unwrap();
        }
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        // A row takes the defaults, and goes with its account.
        conn.execute(
            "INSERT INTO account_protection (account_id) VALUES ('a')",
            [],
        )
        .unwrap();
        let (checks, catalog, failures): (String, Option<String>, i64) = conn
            .query_row(
                "SELECT checks_json, catalog_json, catalog_failures FROM account_protection",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((checks.as_str(), catalog, failures), ("{}", None, 0));
        conn.execute("DELETE FROM accounts WHERE id = 'a'", [])
            .unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM account_protection", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        // Only for accounts that exist.
        assert!(
            conn.execute(
                "INSERT INTO account_protection (account_id) VALUES ('gone')",
                []
            )
            .is_err()
        );
    }

    /// Ids of the messages whose rows in `table` match `query`.
    fn matching(conn: &Connection, table: &str, query: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT id FROM messages WHERE fts_rowid IN
                   (SELECT rowid FROM {table} WHERE {table} MATCH ?1)
                 ORDER BY id"
            ))
            .unwrap();
        stmt.query_map([query], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn the_search_migration_upgrades_a_database_from_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        {
            // A database as the previous version left it: messages indexed
            // by the old code (m1 twice, the second time with its fetched
            // body), one never indexed, and an orphaned index row.
            let conn = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..4] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 4).unwrap();
            conn.execute_batch(
                r#"
                INSERT INTO accounts VALUES ('a', 'imap', 'A', 'a@x', '#000', '{}', 0, NULL, NULL);
                INSERT INTO folders (id, account_id, remote_id, name, role)
                  VALUES ('f', 'a', 'INBOX', 'Inbox', 'inbox');
                INSERT INTO messages (id, account_id, folder_id, remote_id, subject, from_name,
                    from_address, to_json, cc_json, preview, received_at, is_read, is_flagged,
                    is_draft, has_attachments, importance, content_json, body_html)
                VALUES
                  ('m1', 'a', 'f', '1', 'Quarterly report', 'Ada Lovelace', 'ada@example.com',
                   '[{"name":"Adele Vance","address":"adele@contoso.com"}]', '[]',
                   'numbers inside', 4, 0, 0, 0, 0, 'normal', '{}', '<p>Pineapple pizza</p>'),
                  ('m2', 'a', 'f', '2', 'Your invoice', NULL, 'billing@example.com',
                   '[{"name":null,"address":"me@example.com"}]', 'not json',
                   'pay now', 3, 1, 0, 0, 0, 'normal', NULL, NULL),
                  ('m3', 'a', 'f', '3', 'Lunch?', 'Bob', 'bob@example.com', '[]',
                   '[{"name":"Dana Scully","address":"dana@example.com"}]',
                   'tacos', 2, 1, 0, 0, 0, 'normal', NULL, NULL),
                  ('m4', 'a', 'f', '4', 'Fruit', NULL, NULL, '[]', '[]',
                   'kiwi season', 1, 1, 0, 0, 0, 'normal', NULL, NULL);
                INSERT INTO messages_fts (message_id, subject, sender, recipients, body) VALUES
                  ('m1', 'Quarterly report', 'Ada Lovelace ada@example.com',
                   'Adele Vance adele@contoso.com ', 'numbers inside'),
                  ('m1', 'Quarterly report', 'Ada Lovelace ada@example.com',
                   'Adele Vance adele@contoso.com ', 'Pineapple pizza'),
                  ('m2', 'Your invoice', 'billing@example.com', 'me@example.com ', 'pay now'),
                  ('m3', 'Lunch?', 'Bob bob@example.com', ' Dana Scully dana@example.com', 'tacos'),
                  ('gone', 'Orphan', 'x', 'y', 'orphan text');
                "#,
            )
            .unwrap();
        }
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());

        // The fetched body was kept (the latest row), else the preview.
        assert_eq!(matching(&conn, "messages_fts", "body : pineapple"), ["m1"]);
        assert!(matching(&conn, "messages_fts", "body : numbers").is_empty());
        assert_eq!(matching(&conn, "messages_fts", "body : kiwi"), ["m4"]);
        assert!(matching(&conn, "messages_fts", "orphan").is_empty());
        // To and Cc apart, from the stored recipients.
        assert_eq!(
            matching(&conn, "messages_fts", "to_recipients : adele"),
            ["m1"]
        );
        assert_eq!(
            matching(&conn, "messages_fts", "to_recipients : me"),
            ["m2"]
        );
        assert_eq!(
            matching(&conn, "messages_fts", "cc_recipients : scully"),
            ["m3"]
        );
        assert!(matching(&conn, "messages_fts", "to_recipients : dana").is_empty());
        assert_eq!(matching(&conn, "messages_fts", "sender : lovelace"), ["m1"]);
        // Inside words.
        assert_eq!(
            matching(&conn, "messages_infix", "subject : \"voic\""),
            ["m2"]
        );
        assert_eq!(matching(&conn, "messages_infix", "\"illing@\""), ["m2"]);

        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(DISTINCT fts_rowid) FROM messages WHERE fts_rowid IS NOT NULL"
            ),
            4
        );
        for table in ["messages_fts", "messages_infix"] {
            assert_eq!(count(&conn, &format!("SELECT COUNT(*) FROM {table}")), 4);
            assert_eq!(
                count(
                    &conn,
                    &format!(
                        "SELECT COUNT(*) FROM {table}
                         WHERE rowid NOT IN (SELECT fts_rowid FROM messages)"
                    )
                ),
                0,
                "{table}"
            );
        }
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM messages WHERE search_only = 1"),
            0
        );

        // A folder removed takes its messages' index rows (cascade, trigger).
        conn.execute("DELETE FROM folders WHERE id = 'f'", [])
            .unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM messages"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM messages_fts"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM messages_infix"), 0);
    }
}
