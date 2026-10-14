//! Builds protocol providers for accounts. Abstracted behind [`Connector`] so
//! the service can be tested without network access.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::auth::microsoft::{self, MicrosoftAuthConfig, MicrosoftTokenSource, SignInResult};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{Account, AccountSettings};
use crate::providers::graph::GraphProvider;
use crate::providers::imap::{self, ImapCredentials, ImapProvider};
use crate::providers::{CalendarProvider, ContactsProvider, MailProvider, TasksProvider};
use crate::secrets::{SecretStore, keys};
use crate::store::Store;

/// Client id baked in at build time (`CUMULAS_MS_CLIENT_ID=... cargo build`).
/// Can be overridden in Settings.
pub const BUILTIN_MICROSOFT_CLIENT_ID: Option<&str> = option_env!("CUMULAS_MS_CLIENT_ID");

/// The providers serving one account. IMAP accounts have no remote calendar,
/// contacts or tasks; those are kept in the local store.
#[derive(Clone)]
pub struct ProviderSet {
    pub mail: Arc<dyn MailProvider>,
    pub calendar: Option<Arc<dyn CalendarProvider>>,
    pub contacts: Option<Arc<dyn ContactsProvider>>,
    pub tasks: Option<Arc<dyn TasksProvider>>,
}

pub type BrowserOpener = dyn Fn(&str) -> Result<()> + Send + Sync;

#[async_trait]
pub trait Connector: Send + Sync {
    async fn providers(&self, account: &Account) -> Result<ProviderSet>;

    /// Checks IMAP and SMTP settings and credentials.
    async fn verify_imap(&self, credentials: &ImapCredentials) -> Result<()>;

    /// Interactive Microsoft sign-in in the user's browser.
    async fn microsoft_sign_in(
        &self,
        open_browser: &BrowserOpener,
        login_hint: Option<&str>,
    ) -> Result<SignInResult>;
}

/// The real connector: Microsoft Graph and IMAP/SMTP over the network.
pub struct LiveConnector {
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
}

impl LiveConnector {
    pub fn new(store: Arc<Store>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { store, secrets }
    }

    fn microsoft_config(&self) -> Result<MicrosoftAuthConfig> {
        let settings = self.store.get_settings()?;
        let client_id = settings
            .microsoft_client_id
            .filter(|id| !id.trim().is_empty())
            .or_else(|| BUILTIN_MICROSOFT_CLIENT_ID.map(str::to_owned))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::NotConfigured,
                    "No Microsoft application (client) ID is configured. Add one under \
                     Settings → Accounts → Microsoft app registration.",
                )
            })?;
        Ok(MicrosoftAuthConfig::new(
            client_id.trim(),
            settings.microsoft_tenant.filter(|t| !t.trim().is_empty()),
        ))
    }
}

#[async_trait]
impl Connector for LiveConnector {
    async fn providers(&self, account: &Account) -> Result<ProviderSet> {
        match &account.settings {
            AccountSettings::Microsoft(_) => {
                let tokens = Arc::new(MicrosoftTokenSource::new(
                    self.microsoft_config()?,
                    account.id.clone(),
                    Arc::clone(&self.secrets),
                ));
                let graph = Arc::new(GraphProvider::new(account.id.clone(), tokens));
                Ok(ProviderSet {
                    mail: graph.clone(),
                    calendar: Some(graph.clone()),
                    contacts: Some(graph.clone()),
                    tasks: Some(graph),
                })
            }
            AccountSettings::Imap(settings) => {
                let password = self
                    .secrets
                    .get(&keys::imap_password(&account.id))
                    .await?
                    .ok_or_else(|| {
                        Error::auth_required(format!(
                            "The password for {} is missing from the keyring.",
                            account.email
                        ))
                    })?;
                let credentials = ImapCredentials {
                    settings: settings.clone(),
                    password,
                    display_name: account.display_name.clone(),
                    email: account.email.clone(),
                };
                Ok(ProviderSet {
                    mail: Arc::new(ImapProvider::new(account.id.clone(), credentials)),
                    calendar: None,
                    contacts: None,
                    tasks: None,
                })
            }
        }
    }

    async fn verify_imap(&self, credentials: &ImapCredentials) -> Result<()> {
        imap::test_connection(credentials).await
    }

    async fn microsoft_sign_in(
        &self,
        open_browser: &BrowserOpener,
        login_hint: Option<&str>,
    ) -> Result<SignInResult> {
        let config = self.microsoft_config()?;
        microsoft::sign_in(&config, open_browser, login_hint, Duration::from_secs(300)).await
    }
}
