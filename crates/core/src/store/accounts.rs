use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};

use super::{Store, from_ts, ts};
use crate::error::{Error, Result};
use crate::model::{Account, AccountKind, AccountSettings};

const COLUMNS: &str =
    "id, kind, display_name, email, color, settings_json, created_at, last_synced_at, sync_error";

fn kind_str(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Microsoft => "microsoft",
        AccountKind::Imap => "imap",
    }
}

fn row_to_account(row: &Row<'_>) -> rusqlite::Result<Account> {
    let kind: String = row.get(1)?;
    let settings_json: String = row.get(5)?;
    let settings: AccountSettings = serde_json::from_str(&settings_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Account {
        id: row.get(0)?,
        kind: if kind == "microsoft" {
            AccountKind::Microsoft
        } else {
            AccountKind::Imap
        },
        display_name: row.get(2)?,
        email: row.get(3)?,
        color: row.get(4)?,
        settings,
        created_at: from_ts(row.get(6)?),
        last_synced_at: row.get::<_, Option<i64>>(7)?.map(from_ts),
        sync_error: row.get(8)?,
    })
}

impl Store {
    pub fn insert_account(&self, account: &Account) -> Result<()> {
        self.conn().execute(
            &format!(
                "INSERT INTO accounts ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ),
            params![
                account.id,
                kind_str(account.kind),
                account.display_name,
                account.email,
                account.color,
                super::to_json(&account.settings),
                ts(&account.created_at),
                account.last_synced_at.as_ref().map(ts),
                account.sync_error,
            ],
        )?;
        Ok(())
    }

    pub fn update_account(&self, account: &Account) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE accounts SET display_name = ?2, email = ?3, color = ?4, settings_json = ?5
             WHERE id = ?1",
            params![
                account.id,
                account.display_name,
                account.email,
                account.color,
                super::to_json(&account.settings),
            ],
        )?;
        if changed == 0 {
            return Err(Error::not_found("account"));
        }
        Ok(())
    }

    pub fn get_account(&self, id: &str) -> Result<Account> {
        self.conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM accounts WHERE id = ?1"),
                [id],
                row_to_account,
            )
            .optional()?
            .ok_or_else(|| Error::not_found("account"))
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM accounts ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map([], row_to_account)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Deletes the account and, through foreign keys, everything cached for it.
    pub fn delete_account(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM messages_fts WHERE message_id IN
               (SELECT id FROM messages WHERE account_id = ?1)",
            [id],
        )?;
        tx.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_account_sync_result(
        &self,
        id: &str,
        synced_at: Option<DateTime<Utc>>,
        error: Option<&str>,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE accounts SET last_synced_at = COALESCE(?2, last_synced_at), sync_error = ?3
             WHERE id = ?1",
            params![id, synced_at.as_ref().map(ts), error],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::imap_account;

    #[test]
    fn account_crud() {
        let store = Store::open_in_memory().unwrap();
        let mut account = imap_account("a1");
        store.insert_account(&account).unwrap();
        assert_eq!(store.get_account("a1").unwrap().email, account.email);

        account.display_name = "Renamed".into();
        store.update_account(&account).unwrap();
        assert_eq!(store.list_accounts().unwrap()[0].display_name, "Renamed");

        store
            .set_account_sync_result("a1", Some(Utc::now()), Some("boom"))
            .unwrap();
        let loaded = store.get_account("a1").unwrap();
        assert!(loaded.last_synced_at.is_some());
        assert_eq!(loaded.sync_error.as_deref(), Some("boom"));

        store.delete_account("a1").unwrap();
        assert!(store.list_accounts().unwrap().is_empty());
    }
}
