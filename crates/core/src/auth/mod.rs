//! Authentication: OAuth 2.0 for Microsoft accounts. IMAP accounts use a
//! password (or app password) kept in the [`crate::secrets::SecretStore`].

pub mod microsoft;

use async_trait::async_trait;

use crate::error::Result;

/// Supplies bearer tokens to HTTP providers, refreshing them as needed.
#[async_trait]
pub trait TokenSource: Send + Sync {
    async fn access_token(&self) -> Result<String>;
}

/// A fixed token, for tests.
pub struct StaticToken(pub String);

#[async_trait]
impl TokenSource for StaticToken {
    async fn access_token(&self) -> Result<String> {
        Ok(self.0.clone())
    }
}
