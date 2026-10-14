//! Microsoft identity platform sign-in (authorization code + PKCE through a
//! loopback redirect) and token refresh.
//!
//! STUB: the public API below is the contract used by `service.rs`; the
//! implementation is filled in separately.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::TokenSource;
use crate::error::{Error, Result};
use crate::secrets::SecretStore;

/// Delegated Graph permissions requested at sign-in. All of them can be
/// granted by users themselves (no admin consent needed by default).
pub const SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "offline_access",
    "User.Read",
    "Mail.ReadWrite",
    "Mail.Send",
    "MailboxSettings.Read",
    "Calendars.ReadWrite",
    "Contacts.ReadWrite",
    "Tasks.ReadWrite",
];

#[derive(Debug, Clone)]
pub struct MicrosoftAuthConfig {
    /// Application (client) id of the Entra app registration.
    pub client_id: String,
    /// `common`, `organizations`, `consumers`, a tenant id or a domain.
    pub tenant: String,
    /// Defaults to `https://login.microsoftonline.com`; overridable for tests.
    pub authority_host: String,
}

impl MicrosoftAuthConfig {
    pub fn new(client_id: impl Into<String>, tenant: Option<String>) -> Self {
        Self {
            client_id: client_id.into(),
            tenant: tenant.unwrap_or_else(|| "common".into()),
            authority_host: "https://login.microsoftonline.com".into(),
        }
    }
}

/// Identity and tokens obtained from a successful interactive sign-in.
#[derive(Debug, Clone)]
pub struct SignInResult {
    pub email: String,
    pub display_name: String,
    /// `tid` claim; `9188040d-6c67-4c5b-b112-36a304b66dad` for personal accounts.
    pub tenant_id: String,
    /// `oid` claim.
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: Duration,
}

/// Runs the interactive sign-in: starts a loopback listener on 127.0.0.1,
/// calls `open_browser` with the authorization URL, waits for the redirect
/// (up to `timeout`) and redeems the code.
///
/// Errors: `AdminConsentRequired` (with `details.adminConsentUrl`) when the
/// tenant blocks user consent, `ConsentDeclined`, `SignInCancelled`.
pub async fn sign_in(
    _config: &MicrosoftAuthConfig,
    _open_browser: &(dyn Fn(&str) -> Result<()> + Send + Sync),
    _login_hint: Option<&str>,
    _timeout: Duration,
) -> Result<SignInResult> {
    Err(Error::unsupported(
        "Microsoft sign-in is not implemented yet",
    ))
}

/// URL an administrator can open to grant tenant-wide consent.
pub fn admin_consent_url(config: &MicrosoftAuthConfig, tenant_hint: Option<&str>) -> String {
    let _ = tenant_hint;
    format!(
        "{}/organizations/v2.0/adminconsent?client_id={}",
        config.authority_host, config.client_id
    )
}

/// Token source for a signed-in account: keeps the access token in memory and
/// refreshes it with the refresh token stored in the secret store.
pub struct MicrosoftTokenSource {
    _config: MicrosoftAuthConfig,
    _account_id: String,
    _secrets: Arc<dyn SecretStore>,
}

impl MicrosoftTokenSource {
    pub fn new(
        config: MicrosoftAuthConfig,
        account_id: impl Into<String>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            _config: config,
            _account_id: account_id.into(),
            _secrets: secrets,
        }
    }

    /// Seeds the in-memory cache right after sign-in.
    pub async fn prime(&self, _access_token: &str, _expires_in: Duration) {}
}

#[async_trait]
impl TokenSource for MicrosoftTokenSource {
    async fn access_token(&self) -> Result<String> {
        Err(Error::unsupported(
            "Microsoft token refresh is not implemented yet",
        ))
    }
}
