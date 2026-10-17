use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Store, from_ts, ts};
use crate::auth::microsoft::{self, legacy_graph_scopes};
use crate::error::{Error, Result};
use crate::model::{Account, AccountFeature, AccountKind, AccountSettings, MicrosoftGrant};

const COLUMNS: &str = "id, kind, display_name, email, color, settings_json, created_at, \
                       last_synced_at, sync_error, microsoft_grant";

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
    let kind = if kind == "microsoft" {
        AccountKind::Microsoft
    } else {
        AccountKind::Imap
    };
    let id: String = row.get(0)?;
    let (needs_permission, refused_features) = match kind {
        AccountKind::Microsoft => {
            let grant = parse_grant(&id, row.get(9)?);
            (
                microsoft::needs_permission(grant.as_ref()),
                microsoft::refused_features(grant.as_ref()),
            )
        }
        AccountKind::Imap => (Vec::new(), Vec::new()),
    };
    Ok(Account {
        id,
        kind,
        display_name: row.get(2)?,
        email: row.get(3)?,
        color: row.get(4)?,
        settings,
        created_at: from_ts(row.get(6)?),
        last_synced_at: row.get::<_, Option<i64>>(7)?.map(from_ts),
        sync_error: row.get(8)?,
        needs_permission,
        refused_features,
    })
}

/// A stored grant. One that can't be read counts as not recorded (every
/// permission), never as nothing granted, which would gate every feature.
fn parse_grant(account_id: &str, json: Option<String>) -> Option<MicrosoftGrant> {
    let json = json?;
    match serde_json::from_str(&json) {
        Ok(grant) => Some(grant),
        Err(e) => {
            tracing::warn!("ignoring the unreadable grant of account {account_id}: {e}");
            None
        }
    }
}

/// The account's grant, read on `conn`; `Ok(None)` when it has none or is
/// gone (`exists` tells which).
fn read_grant(conn: &Connection, id: &str) -> Result<(bool, Option<MicrosoftGrant>)> {
    let json: Option<Option<String>> = conn
        .query_row(
            "SELECT microsoft_grant FROM accounts WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match json {
        Some(json) => (true, parse_grant(id, json)),
        None => (false, None),
    })
}

fn write_grant(conn: &Connection, id: &str, grant: &MicrosoftGrant) -> Result<()> {
    conn.execute(
        "UPDATE accounts SET microsoft_grant = ?2 WHERE id = ?1",
        params![id, super::to_json(grant)],
    )?;
    Ok(())
}

/// What the UI shows of a grant: the features needing permission and those
/// refused.
fn shown(grant: Option<&MicrosoftGrant>) -> (Vec<AccountFeature>, Vec<AccountFeature>) {
    (
        microsoft::needs_permission(grant),
        microsoft::refused_features(grant),
    )
}

impl Store {
    pub fn insert_account(&self, account: &Account) -> Result<()> {
        self.conn().execute(
            &format!(
                "INSERT INTO accounts ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)"
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

    /// Deletes the account and, through foreign keys, everything cached for
    /// it (the messages' index rows through a trigger).
    pub fn delete_account(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM accounts WHERE id = ?1", [id])?;
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

    // --- what Microsoft accounts were granted ----------------------------------
    //
    // Each read-modify-write holds the connection throughout, so two never
    // interleave. An account that is gone is left alone.

    /// What a Microsoft account was granted; `None` when nothing was
    /// recorded yet (or the account is gone).
    pub fn microsoft_grant(&self, id: &str) -> Result<Option<MicrosoftGrant>> {
        Ok(read_grant(&self.conn(), id)?.1)
    }

    /// Records what a sign-in was granted. Nothing is refused any more: a
    /// sign-in checks every feature again.
    pub fn set_sign_in_grant(&self, id: &str, scopes: &[String]) -> Result<()> {
        let conn = self.conn();
        if !read_grant(&conn, id)?.0 {
            return Ok(());
        }
        let grant = MicrosoftGrant {
            scopes: scopes.to_vec(),
            refused: Vec::new(),
            refused_at: None,
        };
        write_grant(&conn, id, &grant)
    }

    /// Records the Graph permissions a token refresh says the account has.
    /// A refused feature whose permission newly appears is checked again.
    /// Returns whether what the UI shows changed.
    pub fn record_granted_scopes(&self, id: &str, scopes: &[String]) -> Result<bool> {
        let conn = self.conn();
        let (exists, old) = read_grant(&conn, id)?;
        if !exists {
            return Ok(false);
        }
        let refused = match &old {
            None => Vec::new(),
            Some(old) => old
                .refused
                .iter()
                .copied()
                .filter(|f| {
                    let scope = microsoft::feature_scope(*f);
                    let had = old.scopes.iter().any(|s| s.eq_ignore_ascii_case(scope));
                    let has = scopes.iter().any(|s| s.eq_ignore_ascii_case(scope));
                    had || !has
                })
                .collect(),
        };
        let grant = MicrosoftGrant {
            scopes: scopes.to_vec(),
            refused_at: old
                .as_ref()
                .and_then(|o| o.refused_at)
                .filter(|_| !refused.is_empty()),
            refused,
        };
        write_grant(&conn, id, &grant)?;
        Ok(shown(old.as_ref()) != shown(Some(&grant)))
    }

    /// Marks a feature refused by Microsoft 365, as of now. Without a
    /// record, the account has what every account had before grants were
    /// recorded. Returns whether what the UI shows changed.
    pub fn refuse_feature(&self, id: &str, feature: AccountFeature) -> Result<bool> {
        let conn = self.conn();
        let (exists, old) = read_grant(&conn, id)?;
        if !exists {
            return Ok(false);
        }
        let mut grant = old.clone().unwrap_or_else(|| MicrosoftGrant {
            scopes: legacy_graph_scopes(),
            ..MicrosoftGrant::default()
        });
        if !grant.refused.contains(&feature) {
            grant.refused.push(feature);
            grant.refused.sort();
        }
        // Refused again when tried again: not tried for another while.
        grant.refused_at = Some(Utc::now());
        write_grant(&conn, id, &grant)?;
        Ok(shown(old.as_ref()) != shown(Some(&grant)))
    }

    /// Checks every refused feature again. Returns whether what the UI
    /// shows changed.
    pub fn clear_refused(&self, id: &str) -> Result<bool> {
        let conn = self.conn();
        let Some(old) = read_grant(&conn, id)?.1 else {
            return Ok(false);
        };
        if old.refused.is_empty() {
            return Ok(false);
        }
        let grant = MicrosoftGrant {
            refused: Vec::new(),
            refused_at: None,
            ..old.clone()
        };
        write_grant(&conn, id, &grant)?;
        Ok(shown(Some(&old)) != shown(Some(&grant)))
    }

    /// The refused features due to be tried again: refused more than
    /// `max_age` ago. They stay refused until [`Self::lift_refusal`].
    pub fn due_refused(&self, id: &str, max_age: chrono::Duration) -> Result<Vec<AccountFeature>> {
        let Some(grant) = self.microsoft_grant(id)? else {
            return Ok(Vec::new());
        };
        if grant
            .refused_at
            .is_some_and(|at| Utc::now() - at <= max_age)
        {
            return Ok(Vec::new());
        }
        Ok(microsoft::refused_features(Some(&grant)))
    }

    /// A refused feature tried again answered: it is refused no more.
    /// Returns whether what the UI shows changed.
    pub fn lift_refusal(&self, id: &str, feature: AccountFeature) -> Result<bool> {
        let conn = self.conn();
        let Some(old) = read_grant(&conn, id)?.1 else {
            return Ok(false);
        };
        if !old.refused.contains(&feature) {
            return Ok(false);
        }
        let refused: Vec<AccountFeature> = old
            .refused
            .iter()
            .copied()
            .filter(|f| *f != feature)
            .collect();
        let grant = MicrosoftGrant {
            refused_at: old.refused_at.filter(|_| !refused.is_empty()),
            refused,
            ..old.clone()
        };
        write_grant(&conn, id, &grant)?;
        Ok(shown(Some(&old)) != shown(Some(&grant)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{imap_account, microsoft_account};

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

    fn scopes(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    fn mail_only() -> Vec<String> {
        scopes(&["User.Read", "Mail.ReadWrite", "Mail.Send"])
    }

    fn shown_of(store: &Store, id: &str) -> (Vec<AccountFeature>, Vec<AccountFeature>) {
        let account = store.get_account(id).unwrap();
        (account.needs_permission, account.refused_features)
    }

    #[test]
    fn grants_start_unrecorded_and_follow_sign_ins() {
        use AccountFeature::*;
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&microsoft_account("m")).unwrap();
        assert_eq!(store.microsoft_grant("m").unwrap(), None);
        assert_eq!(shown_of(&store, "m"), (vec![], vec![]));

        store.set_sign_in_grant("m", &mail_only()).unwrap();
        assert_eq!(
            shown_of(&store, "m"),
            (AccountFeature::ALL.to_vec(), vec![])
        );
        let with_tasks = [mail_only(), scopes(&["Tasks.ReadWrite"])].concat();
        store.set_sign_in_grant("m", &with_tasks).unwrap();
        assert!(store.refuse_feature("m", Tasks).unwrap());
        assert_eq!(
            shown_of(&store, "m"),
            (vec![Calendar, Contacts, MailboxSettings], vec![Tasks])
        );
        // A sign-in checks everything again.
        store.set_sign_in_grant("m", &with_tasks).unwrap();
        let grant = store.microsoft_grant("m").unwrap().unwrap();
        assert_eq!(grant.scopes, with_tasks);
        assert!(grant.refused.is_empty());
        assert_eq!(grant.refused_at, None);

        // Renaming or recoloring keeps it.
        let mut account = store.get_account("m").unwrap();
        account.display_name = "Renamed".into();
        account.color = "#123456".into();
        store.update_account(&account).unwrap();
        assert_eq!(
            store.microsoft_grant("m").unwrap().unwrap().scopes,
            with_tasks
        );

        // IMAP accounts never show any.
        store.insert_account(&imap_account("i")).unwrap();
        store.execute_for_tests(
            r#"UPDATE accounts SET microsoft_grant = '{"scopes":[]}' WHERE id = 'i'"#,
        );
        assert_eq!(shown_of(&store, "i"), (vec![], vec![]));

        // A gone account is left alone.
        store.set_sign_in_grant("gone", &mail_only()).unwrap();
        assert!(!store.record_granted_scopes("gone", &mail_only()).unwrap());
        assert!(!store.refuse_feature("gone", Tasks).unwrap());
        assert!(!store.clear_refused("gone").unwrap());
        assert_eq!(store.microsoft_grant("gone").unwrap(), None);
    }

    #[test]
    fn recorded_grants_check_a_refused_feature_again_only_when_its_permission_appears() {
        use AccountFeature::*;
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&microsoft_account("m")).unwrap();
        let everything = legacy_graph_scopes();
        // From nothing recorded to everything: nothing the UI shows changes.
        assert!(!store.record_granted_scopes("m", &everything).unwrap());
        assert!(store.refuse_feature("m", Calendar).unwrap());
        assert!(store.refuse_feature("m", Contacts).unwrap());
        let refused_at = store.microsoft_grant("m").unwrap().unwrap().refused_at;
        assert!(refused_at.is_some());
        // The same again: unchanged, and still refused.
        assert!(!store.record_granted_scopes("m", &everything).unwrap());
        assert_eq!(shown_of(&store, "m"), (vec![], vec![Calendar, Contacts]));
        // Calendars taken away: needs permission now, rather than refused.
        let without_calendar: Vec<String> = everything
            .iter()
            .filter(|s| *s != "Calendars.ReadWrite")
            .cloned()
            .collect();
        assert!(store.record_granted_scopes("m", &without_calendar).unwrap());
        assert_eq!(shown_of(&store, "m"), (vec![Calendar], vec![Contacts]));
        // Given back: checked again; Contacts, granted all along, stays refused.
        assert!(store.record_granted_scopes("m", &everything).unwrap());
        let grant = store.microsoft_grant("m").unwrap().unwrap();
        assert_eq!(grant.refused, [Contacts]);
        assert_eq!(grant.refused_at, refused_at);
        assert_eq!(shown_of(&store, "m"), (vec![], vec![Contacts]));
        // Spelled another way, it is the same permission.
        let lower: Vec<String> = everything.iter().map(|s| s.to_ascii_lowercase()).collect();
        assert!(!store.record_granted_scopes("m", &lower).unwrap());
        // The last refused feature checked again clears when.
        store.record_granted_scopes("m", &mail_only()).unwrap();
        store.record_granted_scopes("m", &everything).unwrap();
        let grant = store.microsoft_grant("m").unwrap().unwrap();
        assert!(grant.refused.is_empty());
        assert_eq!(grant.refused_at, None);
    }

    #[test]
    fn refusing_without_a_record_starts_from_the_legacy_grant() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&microsoft_account("m")).unwrap();
        let before = Utc::now();
        assert!(store.refuse_feature("m", AccountFeature::Tasks).unwrap());
        let grant = store.microsoft_grant("m").unwrap().unwrap();
        assert_eq!(grant.scopes, legacy_graph_scopes());
        assert_eq!(grant.refused, [AccountFeature::Tasks]);
        assert!(grant.refused_at.unwrap() >= before - chrono::Duration::seconds(1));
        assert_eq!(shown_of(&store, "m"), (vec![], vec![AccountFeature::Tasks]));
        // Again: nothing changes.
        assert!(!store.refuse_feature("m", AccountFeature::Tasks).unwrap());
    }

    #[test]
    fn refused_features_clear_on_request_or_come_due_once_old() {
        use AccountFeature::*;
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&microsoft_account("m")).unwrap();
        let day = chrono::Duration::hours(24);
        assert!(!store.clear_refused("m").unwrap());
        assert!(store.due_refused("m", day).unwrap().is_empty());

        store.refuse_feature("m", Calendar).unwrap();
        // Refused just now: not due.
        assert!(store.due_refused("m", day).unwrap().is_empty());
        assert_eq!(shown_of(&store, "m").1, [Calendar]);
        assert!(store.clear_refused("m").unwrap());
        assert_eq!(shown_of(&store, "m"), (vec![], vec![]));
        assert!(!store.clear_refused("m").unwrap());

        // Refused more than a day ago: due, and still refused.
        store.refuse_feature("m", Calendar).unwrap();
        store.refuse_feature("m", Tasks).unwrap();
        let refused_a_day_ago = || {
            let old = (Utc::now() - chrono::Duration::hours(25)).to_rfc3339();
            store.execute_for_tests(&format!(
                "UPDATE accounts SET microsoft_grant = \
                 json_set(microsoft_grant, '$.refusedAt', '{old}')"
            ));
        };
        refused_a_day_ago();
        assert_eq!(store.due_refused("m", day).unwrap(), [Calendar, Tasks]);
        assert_eq!(shown_of(&store, "m").1, [Calendar, Tasks]);
        // Refused again: as of now, so not due for another day.
        let before = Utc::now();
        assert!(!store.refuse_feature("m", Calendar).unwrap());
        let grant = store.microsoft_grant("m").unwrap().unwrap();
        assert!(grant.refused_at.unwrap() >= before - chrono::Duration::seconds(1));
        assert!(store.due_refused("m", day).unwrap().is_empty());

        // One answers: only it is lifted, and when stays.
        refused_a_day_ago();
        let refused_at = store.microsoft_grant("m").unwrap().unwrap().refused_at;
        assert!(store.lift_refusal("m", Tasks).unwrap());
        assert!(!store.lift_refusal("m", Tasks).unwrap());
        let grant = store.microsoft_grant("m").unwrap().unwrap();
        assert_eq!(grant.refused, [Calendar]);
        assert_eq!(grant.refused_at, refused_at);
        // The last one: when clears.
        assert!(store.lift_refusal("m", Calendar).unwrap());
        let grant = store.microsoft_grant("m").unwrap().unwrap();
        assert!(grant.refused.is_empty());
        assert_eq!(grant.refused_at, None);
        assert_eq!(grant.scopes, legacy_graph_scopes());
        assert!(!store.lift_refusal("gone", Calendar).unwrap());
    }

    #[test]
    fn an_unreadable_grant_counts_as_not_recorded() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&microsoft_account("m")).unwrap();
        store.execute_for_tests("UPDATE accounts SET microsoft_grant = 'not json' WHERE id = 'm'");
        assert_eq!(store.microsoft_grant("m").unwrap(), None);
        // Every permission, not none.
        assert_eq!(shown_of(&store, "m"), (vec![], vec![]));
        // What's recorded next replaces it.
        store.record_granted_scopes("m", &mail_only()).unwrap();
        assert_eq!(
            store.microsoft_grant("m").unwrap().unwrap().scopes,
            mail_only()
        );
    }

    #[test]
    fn a_grant_from_a_newer_version_keeps_its_scopes() {
        use AccountFeature::*;
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&microsoft_account("m")).unwrap();
        let with_tasks = [mail_only(), scopes(&["Tasks.ReadWrite"])].concat();
        // A newer version refused a feature this one doesn't know.
        let json = serde_json::json!({
            "scopes": with_tasks,
            "refused": ["notes", "tasks"],
            "refusedAt": "2026-10-01T08:00:00Z",
        });
        store.execute_for_tests(&format!(
            "UPDATE accounts SET microsoft_grant = '{json}' WHERE id = 'm'"
        ));

        // Recorded, so a personal account's refresh names what it was given,
        // not every permission.
        let grant = store.microsoft_grant("m").unwrap().unwrap();
        assert_eq!(grant.scopes, with_tasks);
        assert_eq!(grant.refused, vec![Tasks]);
        assert!(grant.refused_at.is_some());
        assert_eq!(
            shown_of(&store, "m"),
            (vec![Calendar, Contacts, MailboxSettings], vec![Tasks])
        );
    }
}
