//! Local SQLite cache. The UI always reads from here; the sync engine keeps it
//! up to date with the servers. Secrets never live in this database.

mod accounts;
mod calendar;
mod contacts;
mod mail;
mod push;
mod reminders;
mod schema;
mod settings;
mod signatures;
mod tasks;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rusqlite::Connection;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Result;

pub use mail::{BatchOutcome, FolderUnreadDelta, SearchPageMark, SearchUpsert};
pub use push::PushSubscription;
pub use signatures::{StoredOrganizationSignature, StoredSignatures};

pub struct Store {
    conn: Mutex<Connection>,
    /// Messages changed while server search pages were on their way; see
    /// [`Store::search_page_started`].
    changes: Arc<Mutex<mail::LocalChanges>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        schema::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            changes: Arc::default(),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite itself consistent, so
        // recover from poisoning instead of propagating the panic.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// --- column helpers --------------------------------------------------------

pub(crate) fn ts(dt: &DateTime<Utc>) -> i64 {
    dt.timestamp()
}

pub(crate) fn from_ts(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0).single().unwrap_or_default()
}

pub(crate) fn date_str(d: &NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub(crate) fn to_json<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).expect("model types always serialize")
}

pub(crate) fn from_json<T: DeserializeOwned + Default>(s: &str) -> T {
    serde_json::from_str(s).unwrap_or_default()
}

/// Escapes user input for use in a `LIKE ... ESCAPE '\'` pattern.
pub(crate) fn like_pattern(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('%');
    for c in input.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

#[cfg(test)]
pub(crate) mod test_support {
    use chrono::Utc;

    use super::Store;
    use crate::model::{Account, AccountKind, AccountSettings, ImapSettings, TlsMode};

    impl Store {
        /// Runs `sql` on the database, for tests that need it in a state
        /// the app never puts it in.
        pub fn execute_for_tests(&self, sql: &str) {
            self.conn().execute_batch(sql).unwrap();
        }
    }

    pub fn imap_account(id: &str) -> Account {
        Account {
            id: id.to_owned(),
            kind: AccountKind::Imap,
            display_name: "Test".into(),
            email: format!("{id}@example.com"),
            color: "#0b7a75".into(),
            settings: AccountSettings::Imap(ImapSettings {
                imap_host: "imap.example.com".into(),
                imap_port: 993,
                imap_tls: TlsMode::Tls,
                smtp_host: "smtp.example.com".into(),
                smtp_port: 465,
                smtp_tls: TlsMode::Tls,
                username: format!("{id}@example.com"),
            }),
            created_at: Utc::now(),
            last_synced_at: None,
            sync_error: None,
        }
    }
}
