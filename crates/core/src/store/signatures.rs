//! Each account's signatures: the person's own, and what the account's
//! organization last said about its signature, with when the app last
//! asked. The organization's answer is kept so that its signature works
//! offline and after a restart.

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};

use super::{Store, from_ts, ts};
use crate::error::Result;
use crate::model::OrganizationSignatureState;

/// What the organization last said, as kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOrganizationSignature {
    /// The API scope the answer came for; `None` before any.
    pub scope: Option<String>,
    /// Never `Unsupported`: that depends on the account and the build.
    pub state: OrganizationSignatureState,
    /// Sanitized; only while `Ready`.
    pub html: Option<String>,
    pub on_replies: bool,
    /// The service's ETag for its last answer.
    pub etag: Option<String>,
    /// The last answer.
    pub checked_at: Option<DateTime<Utc>>,
    /// The last try, answered or not.
    pub attempted_at: Option<DateTime<Utc>>,
    /// The last try failed; `state` is what was known before.
    pub stale: bool,
    /// Not to be asked before then (the service's `Retry-After`).
    pub retry_at: Option<DateTime<Utc>>,
}

impl Default for StoredOrganizationSignature {
    fn default() -> Self {
        Self {
            scope: None,
            state: OrganizationSignatureState::Unchecked,
            html: None,
            on_replies: false,
            etag: None,
            checked_at: None,
            attempted_at: None,
            stale: false,
            retry_at: None,
        }
    }
}

/// An account's signatures, as kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSignatures {
    /// Sanitized; empty for none.
    pub own_html: String,
    pub own_on_replies: bool,
    pub organization: StoredOrganizationSignature,
}

impl Default for StoredSignatures {
    fn default() -> Self {
        Self {
            own_html: String::new(),
            own_on_replies: true,
            organization: StoredOrganizationSignature::default(),
        }
    }
}

fn state_name(state: OrganizationSignatureState) -> &'static str {
    match state {
        // Never kept; read back as unchecked.
        OrganizationSignatureState::Unsupported | OrganizationSignatureState::Unchecked => {
            "unchecked"
        }
        OrganizationSignatureState::NotUsed => "notUsed",
        OrganizationSignatureState::Ready => "ready",
        OrganizationSignatureState::AddedOnSend => "addedOnSend",
        OrganizationSignatureState::NotAllowed => "notAllowed",
    }
}

fn parse_state(name: &str) -> OrganizationSignatureState {
    match name {
        "notUsed" => OrganizationSignatureState::NotUsed,
        "ready" => OrganizationSignatureState::Ready,
        "addedOnSend" => OrganizationSignatureState::AddedOnSend,
        "notAllowed" => OrganizationSignatureState::NotAllowed,
        // Unchecked, or a state from a newer version: ask again.
        _ => OrganizationSignatureState::Unchecked,
    }
}

impl Store {
    /// The account's signatures; the defaults (none of its own, nothing
    /// heard from its organization) when nothing was kept.
    pub fn account_signatures(&self, account_id: &str) -> Result<StoredSignatures> {
        let stored = self
            .conn()
            .query_row(
                "SELECT own_html, own_on_replies, org_scope, org_state, org_html,
                        org_on_replies, org_etag, org_checked_at, org_attempted_at,
                        org_stale, org_retry_at
                 FROM account_signatures WHERE account_id = ?1",
                [account_id],
                |r| {
                    let state: String = r.get(3)?;
                    Ok(StoredSignatures {
                        own_html: r.get(0)?,
                        own_on_replies: r.get(1)?,
                        organization: StoredOrganizationSignature {
                            scope: r.get(2)?,
                            state: parse_state(&state),
                            html: r.get(4)?,
                            on_replies: r.get(5)?,
                            etag: r.get(6)?,
                            checked_at: r.get::<_, Option<i64>>(7)?.map(from_ts),
                            attempted_at: r.get::<_, Option<i64>>(8)?.map(from_ts),
                            stale: r.get(9)?,
                            retry_at: r.get::<_, Option<i64>>(10)?.map(from_ts),
                        },
                    })
                },
            )
            .optional()?;
        Ok(stored.unwrap_or_default())
    }

    /// Keeps the person's own signature (already sanitized), leaving the
    /// organization's as it was.
    pub fn save_own_signature(&self, account_id: &str, html: &str, on_replies: bool) -> Result<()> {
        self.conn().execute(
            "INSERT INTO account_signatures (account_id, own_html, own_on_replies)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id) DO UPDATE SET
                own_html = excluded.own_html,
                own_on_replies = excluded.own_on_replies",
            params![account_id, html, on_replies],
        )?;
        Ok(())
    }

    /// Keeps what the organization said (or that it couldn't be asked),
    /// leaving the person's own signature as it was.
    pub fn save_organization_signature(
        &self,
        account_id: &str,
        organization: &StoredOrganizationSignature,
    ) -> Result<()> {
        let o = organization;
        self.conn().execute(
            "INSERT INTO account_signatures (account_id, org_scope, org_state, org_html,
                org_on_replies, org_etag, org_checked_at, org_attempted_at, org_stale,
                org_retry_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(account_id) DO UPDATE SET
                org_scope = excluded.org_scope,
                org_state = excluded.org_state,
                org_html = excluded.org_html,
                org_on_replies = excluded.org_on_replies,
                org_etag = excluded.org_etag,
                org_checked_at = excluded.org_checked_at,
                org_attempted_at = excluded.org_attempted_at,
                org_stale = excluded.org_stale,
                org_retry_at = excluded.org_retry_at",
            params![
                account_id,
                o.scope,
                state_name(o.state),
                o.html,
                o.on_replies,
                o.etag,
                o.checked_at.as_ref().map(ts),
                o.attempted_at.as_ref().map(ts),
                o.stale,
                o.retry_at.as_ref().map(ts),
            ],
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
    fn signatures_are_kept_apart_and_go_with_the_account() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&imap_account("a")).unwrap();
        assert_eq!(
            store.account_signatures("a").unwrap(),
            StoredSignatures::default()
        );
        assert!(store.account_signatures("a").unwrap().own_on_replies);

        store.save_own_signature("a", "<p>Ada</p>", false).unwrap();
        let at = |minute| Utc.with_ymd_and_hms(2026, 10, 15, 9, minute, 0).unwrap();
        let organization = StoredOrganizationSignature {
            scope: Some("api://signature.example.org/x/access_as_user".into()),
            state: OrganizationSignatureState::Ready,
            html: Some("<table><tr><td>Ada</td></tr></table>".into()),
            on_replies: true,
            etag: Some("\"abc\"".into()),
            checked_at: Some(at(0)),
            attempted_at: Some(at(5)),
            stale: true,
            retry_at: Some(at(35)),
        };
        store
            .save_organization_signature("a", &organization)
            .unwrap();
        // Neither save touched the other's half.
        store
            .save_own_signature("a", "<p>Ada L.</p>", false)
            .unwrap();
        let stored = store.account_signatures("a").unwrap();
        assert_eq!(stored.own_html, "<p>Ada L.</p>");
        assert!(!stored.own_on_replies);
        assert_eq!(stored.organization, organization);

        // Every state reads back as written; a newer version's as unchecked.
        for state in [
            OrganizationSignatureState::Unchecked,
            OrganizationSignatureState::NotUsed,
            OrganizationSignatureState::AddedOnSend,
            OrganizationSignatureState::NotAllowed,
        ] {
            let organization = StoredOrganizationSignature {
                state,
                ..Default::default()
            };
            store
                .save_organization_signature("a", &organization)
                .unwrap();
            assert_eq!(
                store.account_signatures("a").unwrap().organization.state,
                state
            );
        }
        store
            .conn()
            .execute("UPDATE account_signatures SET org_state = 'someday'", [])
            .unwrap();
        assert_eq!(
            store.account_signatures("a").unwrap().organization.state,
            OrganizationSignatureState::Unchecked
        );

        store.delete_account("a").unwrap();
        assert_eq!(
            store.account_signatures("a").unwrap(),
            StoredSignatures::default()
        );
        // Nothing is kept for an account that doesn't exist.
        assert!(store.save_own_signature("gone", "<p>x</p>", true).is_err());
    }
}
