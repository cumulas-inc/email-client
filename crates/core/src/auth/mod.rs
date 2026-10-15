//! Authentication: OAuth 2.0 for Microsoft accounts. IMAP accounts use a
//! password (or app password) kept in the [`crate::secrets::SecretStore`].

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
