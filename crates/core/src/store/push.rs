//! The Microsoft Graph subscriptions push keeps for each account: which
//! exist, what they watch and when they expire. Reconciling with Graph on
//! every connection is what counts; this is what the app knew last, so a
//! restart doesn't lose track of them and removing an account can delete
//! them.

use chrono::{DateTime, Utc};
use rusqlite::params;

use super::{Store, from_ts, ts};
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSubscription {
    pub id: String,
    pub resource: String,
    pub expires_at: DateTime<Utc>,
}

impl Store {
    pub fn push_subscriptions(&self, account_id: &str) -> Result<Vec<PushSubscription>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, resource, expires_at FROM push_subscriptions
             WHERE account_id = ?1 ORDER BY resource, id",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            Ok(PushSubscription {
                id: r.get(0)?,
                resource: r.get(1)?,
                expires_at: from_ts(r.get(2)?),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replaces the account's subscriptions with `subscriptions`.
    pub fn replace_push_subscriptions(
        &self,
        account_id: &str,
        subscriptions: &[PushSubscription],
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM push_subscriptions WHERE account_id = ?1",
            [account_id],
        )?;
        for s in subscriptions {
            tx.execute(
                "INSERT OR REPLACE INTO push_subscriptions (account_id, id, resource, expires_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![account_id, s.id, s.resource, ts(&s.expires_at)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_push_subscription(&self, account_id: &str, id: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM push_subscriptions WHERE account_id = ?1 AND id = ?2",
            params![account_id, id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::store::test_support::imap_account;

    #[test]
    fn subscriptions_are_replaced_removed_and_go_with_the_account() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&imap_account("a")).unwrap();
        store.insert_account(&imap_account("b")).unwrap();
        let sub = |id: &str, resource: &str| PushSubscription {
            id: id.into(),
            resource: resource.into(),
            expires_at: Utc.with_ymd_and_hms(2026, 10, 18, 9, 30, 0).unwrap(),
        };
        store
            .replace_push_subscriptions("a", &[sub("1", "me/events"), sub("2", "inbox")])
            .unwrap();
        store
            .replace_push_subscriptions("b", &[sub("3", "inbox")])
            .unwrap();
        assert_eq!(
            store.push_subscriptions("a").unwrap(),
            [sub("2", "inbox"), sub("1", "me/events")]
        );
        store
            .replace_push_subscriptions("a", &[sub("4", "inbox")])
            .unwrap();
        assert_eq!(store.push_subscriptions("a").unwrap(), [sub("4", "inbox")]);
        store.remove_push_subscription("a", "4").unwrap();
        assert!(store.push_subscriptions("a").unwrap().is_empty());
        store.delete_account("b").unwrap();
        assert!(store.push_subscriptions("b").unwrap().is_empty());
    }
}
