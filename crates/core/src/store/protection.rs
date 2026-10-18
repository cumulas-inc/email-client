//! Each account's protection setup: what its checks found (whether the
//! organization's mail server encrypts or labels mail that asks for it),
//! the check waiting for its test messages, and the sensitivity labels
//! Microsoft 365 last listed, kept so that the composer works offline.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{Store, from_json, from_ts, to_json, ts};
use crate::error::Result;
use crate::model::{CheckItem, CheckNote, CheckState, LabelCatalog, ProtectAction};

/// What an account's checks found, as kept (`account_protection.checks_json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoredChecks {
    pub encrypt: CheckRecord,
    pub do_not_forward: CheckRecord,
    /// By label id. Kept when a label leaves the list, so its check comes
    /// back with it; only listed labels are offered.
    pub labels: BTreeMap<String, CheckRecord>,
    /// The check waiting for its messages, if any.
    pub run: Option<CheckRun>,
}

impl StoredChecks {
    /// What the item's last check found.
    pub fn record(&self, item: &CheckItem) -> CheckRecord {
        match item {
            CheckItem::Action {
                action: ProtectAction::Encrypt,
            } => self.encrypt.clone(),
            CheckItem::Action {
                action: ProtectAction::DoNotForward,
            } => self.do_not_forward.clone(),
            CheckItem::Label { label_id } => self.labels.get(label_id).cloned().unwrap_or_default(),
        }
    }

    pub fn record_mut(&mut self, item: &CheckItem) -> &mut CheckRecord {
        match item {
            CheckItem::Action {
                action: ProtectAction::Encrypt,
            } => &mut self.encrypt,
            CheckItem::Action {
                action: ProtectAction::DoNotForward,
            } => &mut self.do_not_forward,
            CheckItem::Label { label_id } => self.labels.entry(label_id.clone()).or_default(),
        }
    }
}

/// One option's last check.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CheckRecord {
    /// Never `Expired` here: that's worked out when the setup is read. Nor
    /// `Waiting`: a check on its way is the run's, and this stays what the
    /// last one decided until it's decided.
    pub state: CheckState,
    pub at: Option<DateTime<Utc>>,
    pub note: Option<CheckNote>,
    /// The arrived test message's Graph id, to open in Outlook on the web.
    pub test_message_id: Option<String>,
}

/// A check waiting for its messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckRun {
    pub control_token: String,
    pub control_sent_at: DateTime<Utc>,
    /// What the control looked like when it arrived; `None` until then.
    pub control: Option<ControlOutcome>,
    /// Only items sent and still waiting.
    pub items: Vec<RunItem>,
}

/// How the control message, sent without any request, arrived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlOutcome {
    /// Encrypted with Microsoft Purview.
    pub protected: bool,
    /// The account's organization's label on it, if any.
    pub label_id: Option<String>,
}

/// A test message on its way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunItem {
    pub item: CheckItem,
    pub token: String,
    pub sent_at: DateTime<Utc>,
}

/// The labels Microsoft 365 last listed, and how listing them went since.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredCatalog {
    /// `None` until first listed, or after the permission went.
    pub catalog: Option<LabelCatalog>,
    /// The last successful listing.
    pub listed_at: Option<DateTime<Utc>>,
    /// Why the last listing failed; `None` after a success.
    pub error: Option<String>,
    /// Failures in a row (the backoff); 0 after a success.
    pub failures: u32,
    pub failed_at: Option<DateTime<Utc>>,
}

/// Whether the account still exists, on `conn`.
fn account_exists(conn: &Connection, account_id: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM accounts WHERE id = ?1", [account_id], |_| {
            Ok(())
        })
        .optional()?
        .is_some())
}

fn read_catalog(conn: &Connection, account_id: &str) -> Result<StoredCatalog> {
    let stored = conn
        .query_row(
            "SELECT catalog_json, catalog_listed_at, catalog_error, catalog_failures,
                    catalog_failed_at
             FROM account_protection WHERE account_id = ?1",
            [account_id],
            |r| {
                let json: Option<String> = r.get(0)?;
                Ok(StoredCatalog {
                    // Unreadable: as if never listed, so it's listed again.
                    catalog: json.and_then(|json| serde_json::from_str(&json).ok()),
                    listed_at: r.get::<_, Option<i64>>(1)?.map(from_ts),
                    error: r.get(2)?,
                    failures: r.get(3)?,
                    failed_at: r.get::<_, Option<i64>>(4)?.map(from_ts),
                })
            },
        )
        .optional()?;
    Ok(stored.unwrap_or_default())
}

/// Keeps `catalog` as the account's, creating its row; a gone account is
/// left alone (`false`).
fn write_catalog(conn: &Connection, account_id: &str, catalog: &StoredCatalog) -> Result<bool> {
    if !account_exists(conn, account_id)? {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO account_protection (account_id, catalog_json, catalog_listed_at,
            catalog_error, catalog_failures, catalog_failed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(account_id) DO UPDATE SET
            catalog_json = excluded.catalog_json,
            catalog_listed_at = excluded.catalog_listed_at,
            catalog_error = excluded.catalog_error,
            catalog_failures = excluded.catalog_failures,
            catalog_failed_at = excluded.catalog_failed_at",
        params![
            account_id,
            catalog.catalog.as_ref().map(to_json),
            catalog.listed_at.as_ref().map(ts),
            catalog.error,
            catalog.failures,
            catalog.failed_at.as_ref().map(ts),
        ],
    )?;
    Ok(true)
}

/// What the UI shows of a stored catalog.
fn shown(catalog: &StoredCatalog) -> (Option<&LabelCatalog>, Option<i64>, Option<&str>) {
    (
        catalog.catalog.as_ref(),
        catalog.listed_at.as_ref().map(ts),
        catalog.error.as_deref(),
    )
}

impl Store {
    /// The labels Microsoft 365 last listed for the account, and how
    /// listing them went since; the defaults when nothing was kept.
    pub fn label_catalog(&self, account_id: &str) -> Result<StoredCatalog> {
        read_catalog(&self.conn(), account_id)
    }

    /// Keeps a successful listing, which ends the backoff. Returns whether
    /// anything the UI shows changed.
    pub fn save_label_catalog(
        &self,
        account_id: &str,
        catalog: &LabelCatalog,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let conn = self.conn();
        let old = read_catalog(&conn, account_id)?;
        let new = StoredCatalog {
            catalog: Some(catalog.clone()),
            listed_at: Some(at),
            error: None,
            failures: 0,
            failed_at: None,
        };
        Ok(write_catalog(&conn, account_id, &new)? && shown(&old) != shown(&new))
    }

    /// Records a failed listing: the labels listed before stay, with the
    /// error; one more failure in a row, as of `at`. Returns whether
    /// anything the UI shows changed.
    pub fn label_catalog_failed(
        &self,
        account_id: &str,
        error: &str,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let conn = self.conn();
        let old = read_catalog(&conn, account_id)?;
        let new = StoredCatalog {
            error: Some(error.to_owned()),
            failures: old.failures.saturating_add(1),
            failed_at: Some(at),
            ..old.clone()
        };
        Ok(write_catalog(&conn, account_id, &new)? && shown(&old) != shown(&new))
    }

    /// Forgets the labels, the error and the backoff, so the next sync
    /// lists them at once. Returns whether anything the UI shows changed.
    pub fn clear_label_catalog(&self, account_id: &str) -> Result<bool> {
        let conn = self.conn();
        let old = read_catalog(&conn, account_id)?;
        if old == StoredCatalog::default() {
            return Ok(false);
        }
        let new = StoredCatalog::default();
        Ok(write_catalog(&conn, account_id, &new)? && shown(&old) != shown(&new))
    }

    /// What the account's checks found; the defaults (nothing checked)
    /// when nothing was kept or it can't be read.
    pub fn protection_checks(&self, account_id: &str) -> Result<StoredChecks> {
        let json: Option<String> = self
            .conn()
            .query_row(
                "SELECT checks_json FROM account_protection WHERE account_id = ?1",
                [account_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json.map(|json| from_json(&json)).unwrap_or_default())
    }

    /// Changes what the account's checks found with `f`, read and written
    /// under one hold of the database, so nothing else writes them in
    /// between. A gone account is left alone: `None`, and `f` isn't run.
    pub fn update_protection_checks<T>(
        &self,
        account_id: &str,
        f: impl FnOnce(&mut StoredChecks) -> T,
    ) -> Result<Option<T>> {
        let conn = self.conn();
        if !account_exists(&conn, account_id)? {
            return Ok(None);
        }
        let json: Option<String> = conn
            .query_row(
                "SELECT checks_json FROM account_protection WHERE account_id = ?1",
                [account_id],
                |r| r.get(0),
            )
            .optional()?;
        let mut checks: StoredChecks = json.map(|json| from_json(&json)).unwrap_or_default();
        let result = f(&mut checks);
        conn.execute(
            "INSERT INTO account_protection (account_id, checks_json) VALUES (?1, ?2)
             ON CONFLICT(account_id) DO UPDATE SET checks_json = excluded.checks_json",
            params![account_id, to_json(&checks)],
        )?;
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::model::CatalogLabel;
    use crate::store::test_support::microsoft_account;

    const LABEL: &str = "6c1b3b5e-0d4f-4b1e-9d3a-2f0e6a7b8c9d";

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 16, hour, 0, 0).unwrap()
    }

    fn catalog(name: &str) -> LabelCatalog {
        LabelCatalog {
            labels: vec![CatalogLabel {
                id: LABEL.into(),
                name: name.into(),
                parent_name: None,
                tooltip: None,
                color: Some("#b35f26".into()),
                has_protection: false,
                sensitivity: Some(2),
            }],
            ..LabelCatalog::default()
        }
    }

    #[test]
    fn catalogs_round_trip_and_count_failures() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&microsoft_account("m")).unwrap();
        assert_eq!(store.label_catalog("m").unwrap(), StoredCatalog::default());

        assert!(
            store
                .save_label_catalog("m", &catalog("General"), at(9))
                .unwrap()
        );
        // The same again, at the same time: nothing shown changes.
        assert!(
            !store
                .save_label_catalog("m", &catalog("General"), at(9))
                .unwrap()
        );
        let stored = store.label_catalog("m").unwrap();
        assert_eq!(stored.catalog, Some(catalog("General")));
        assert_eq!(stored.listed_at, Some(at(9)));

        // Failures keep the list, with the error, and count up.
        assert!(store.label_catalog_failed("m", "offline", at(10)).unwrap());
        assert!(!store.label_catalog_failed("m", "offline", at(11)).unwrap());
        let stored = store.label_catalog("m").unwrap();
        assert_eq!(stored.catalog, Some(catalog("General")));
        assert_eq!(stored.error.as_deref(), Some("offline"));
        assert_eq!(stored.failures, 2);
        assert_eq!(stored.failed_at, Some(at(11)));
        // A success resets them.
        assert!(
            store
                .save_label_catalog("m", &catalog("General"), at(12))
                .unwrap()
        );
        let stored = store.label_catalog("m").unwrap();
        assert_eq!(
            (stored.error, stored.failures, stored.failed_at),
            (None, 0, None)
        );
        assert!(
            store
                .save_label_catalog("m", &catalog("Renamed"), at(12))
                .unwrap()
        );

        // Cleared: as if never listed.
        assert!(store.clear_label_catalog("m").unwrap());
        assert!(!store.clear_label_catalog("m").unwrap());
        assert_eq!(store.label_catalog("m").unwrap(), StoredCatalog::default());
        // A failure without a list is shown too.
        assert!(store.label_catalog_failed("m", "HTTP 500", at(13)).unwrap());
        assert_eq!(store.label_catalog("m").unwrap().catalog, None);
    }

    #[test]
    fn checks_round_trip_and_unreadable_ones_read_as_none() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&microsoft_account("m")).unwrap();
        assert_eq!(
            store.protection_checks("m").unwrap(),
            StoredChecks::default()
        );
        let label = CheckItem::Label {
            label_id: LABEL.into(),
        };
        let encrypt = CheckItem::Action {
            action: ProtectAction::Encrypt,
        };
        let token = store
            .update_protection_checks("m", |checks| {
                *checks.record_mut(&label) = CheckRecord {
                    state: CheckState::Applied,
                    at: Some(at(9)),
                    note: None,
                    test_message_id: Some("AAMk-1".into()),
                };
                checks.record_mut(&encrypt).state = CheckState::NotApplied;
                checks.run = Some(CheckRun {
                    control_token: "k7q2m9xa".into(),
                    control_sent_at: at(9),
                    control: Some(ControlOutcome {
                        protected: false,
                        label_id: None,
                    }),
                    items: vec![RunItem {
                        item: encrypt.clone(),
                        token: "a2b3c4d5".into(),
                        sent_at: at(9),
                    }],
                });
                "k7q2m9xa"
            })
            .unwrap();
        assert_eq!(token, Some("k7q2m9xa"));
        let checks = store.protection_checks("m").unwrap();
        assert_eq!(checks.record(&label).state, CheckState::Applied);
        assert_eq!(
            checks.record(&label).test_message_id.as_deref(),
            Some("AAMk-1")
        );
        assert_eq!(checks.record(&encrypt).state, CheckState::NotApplied);
        assert_eq!(
            checks
                .record(&CheckItem::Action {
                    action: ProtectAction::DoNotForward
                })
                .state,
            CheckState::NotChecked
        );
        assert_eq!(checks.run.as_ref().unwrap().items[0].item, encrypt);

        // The catalog and the checks are kept apart.
        store
            .save_label_catalog("m", &catalog("General"), at(10))
            .unwrap();
        store.clear_label_catalog("m").unwrap();
        assert_eq!(store.protection_checks("m").unwrap(), checks);

        store.execute_for_tests(
            "UPDATE account_protection SET checks_json = 'not json' WHERE account_id = 'm'",
        );
        assert_eq!(
            store.protection_checks("m").unwrap(),
            StoredChecks::default()
        );
        store.execute_for_tests(
            "UPDATE account_protection SET catalog_json = '[1]' WHERE account_id = 'm'",
        );
        assert_eq!(store.label_catalog("m").unwrap().catalog, None);
    }

    #[test]
    fn a_gone_account_is_left_alone() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.update_protection_checks("gone", |_| 1).unwrap(), None);
        assert!(
            !store
                .save_label_catalog("gone", &catalog("General"), at(9))
                .unwrap()
        );
        assert!(
            !store
                .label_catalog_failed("gone", "offline", at(9))
                .unwrap()
        );
        assert!(!store.clear_label_catalog("gone").unwrap());
        assert_eq!(
            store.label_catalog("gone").unwrap(),
            StoredCatalog::default()
        );

        // And what was kept goes with the account.
        store.insert_account(&microsoft_account("m")).unwrap();
        store
            .save_label_catalog("m", &catalog("General"), at(9))
            .unwrap();
        store.update_protection_checks("m", |_| ()).unwrap();
        store.delete_account("m").unwrap();
        assert_eq!(store.label_catalog("m").unwrap(), StoredCatalog::default());
        assert_eq!(
            store.protection_checks("m").unwrap(),
            StoredChecks::default()
        );
    }
}
