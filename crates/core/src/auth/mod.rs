//! Authentication: OAuth 2.0 for Microsoft accounts. IMAP accounts use a
//! password (or app password) kept in the [`crate::secrets::SecretStore`].

pub mod admin_setup;
pub mod microsoft;

use std::time::Duration;

use async_trait::async_trait;

use crate::error::{Error, Result};

/// Supplies bearer tokens to HTTP providers, refreshing them as needed.
#[async_trait]
pub trait TokenSource: Send + Sync {
    async fn access_token(&self) -> Result<String>;

    /// A token that stays valid for at least `at_least` more, refreshed
    /// early if need be. Graph subscriptions pause when the token they were
    /// last authorized with expires, so renewals ask for one that outlives
    /// the next renewal.
    async fn access_token_lasting(&self, _at_least: Duration) -> Result<String> {
        self.access_token().await
    }

    /// Forgets the cached access token after the server rejected it, so the
    /// next [`access_token`](Self::access_token) call obtains a fresh one.
    async fn invalidate(&self) {}
}

/// A fixed token, for tests.
pub struct StaticToken(pub String);

/// Keeps what a Microsoft account was granted: the Graph permissions
/// Microsoft last said it has.
pub trait GrantRecord: Send + Sync {
    /// The recorded Graph permissions; `None` when nothing was recorded yet.
    fn granted(&self) -> Result<Option<Vec<String>>>;
    /// Records the Graph permissions a token refresh says the account has.
    /// Called with the account's lock held; never fails the refresh.
    fn record(&self, granted: &[String]);
}

/// Records nothing (tests).
pub struct NoGrantRecord;

impl GrantRecord for NoGrantRecord {
    fn granted(&self) -> Result<Option<Vec<String>>> {
        Ok(None)
    }

    fn record(&self, _: &[String]) {}
}

#[async_trait]
impl TokenSource for StaticToken {
    async fn access_token(&self) -> Result<String> {
        Ok(self.0.clone())
    }
}

/// Why no token for the push service could be had.
#[derive(Debug, Clone)]
pub enum SyncTokenError {
    /// The account hasn't consented to the push service's scope. Signing in
    /// again (or an administrator's consent) fixes it; the Graph sign-in is
    /// unaffected.
    ConsentMissing,
    /// The app registration doesn't offer the push service's scope (it
    /// isn't set up for it).
    NotOffered,
    /// Anything else (no network, Microsoft unavailable, the keyring): try
    /// again later.
    Failed(Error),
}

/// Supplies tokens for the Cumulas push service (`api://<client
/// id>/Sync.Receive`). They are never sent to Microsoft Graph, and Graph
/// tokens are never sent to the push service.
#[async_trait]
pub trait SyncTokens: Send + Sync {
    /// A token with at least `min_left` to go; `fresh` gets a new one
    /// whatever the cache holds.
    async fn sync_token(
        &self,
        min_left: Duration,
        fresh: bool,
    ) -> std::result::Result<String, SyncTokenError>;
}

/// Why no token for an organization's API (Cumulas Signatures') could be
/// had. None of these is the account's problem: its sign-in and sync are
/// unaffected.
#[derive(Debug, Clone)]
pub enum ApiTokenError {
    /// The API, or the app it belongs to, isn't in the account's
    /// organization: it doesn't use the service.
    NotInTenant,
    /// The organization has the API but hasn't let this app use it (no
    /// consent, or it was declined).
    NotAllowed,
    /// Anything else (no network, Microsoft unavailable, the keyring, a
    /// sign-in that needs the person, such as MFA): try again later.
    Failed(Error),
}

/// Supplies tokens for an API in the account's own organization, from the
/// account's sign-in. They go to that API only.
#[async_trait]
pub trait ApiTokens: Send + Sync {
    /// A token; `fresh` gets a new one whatever the cache holds (the API
    /// refused the last one).
    async fn api_token(&self, fresh: bool) -> std::result::Result<String, ApiTokenError>;
}
