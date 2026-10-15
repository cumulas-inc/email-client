//! Builds protocol providers for accounts. Abstracted behind [`Connector`] so
//! the service can be tested without network access.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;

use crate::auth::microsoft::{
    self, MicrosoftAuthConfig, MicrosoftTokenSource, SignInResult, SyncTokenSource, sync_scope,
};
use crate::error::{Error, ErrorCode, Result};
use crate::model::{Account, AccountKind, AccountSettings, ImapSettings};
use crate::providers::graph::GraphProvider;
use crate::providers::imap::{self, IdleConfig, ImapCredentials, ImapIdleWatcher, ImapProvider};
use crate::providers::{
    CalendarProvider, ChangeSubscriptions, ContactsProvider, MailProvider, TasksProvider,
};
use crate::push::MailWatcher;
use crate::push::graph::RelaySource;
use crate::push::relay::{self, HttpRelay, Relay};
use crate::push::supervisor::{MICROSOFT_POLL, UNSUPPORTED_POLL};
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
    /// Graph change subscriptions, for the push service (Microsoft only).
    pub subscriptions: Option<Arc<dyn ChangeSubscriptions>>,
}

pub type BrowserOpener = dyn Fn(&str) -> Result<()> + Send + Sync;

/// How an account hears about changes between full syncs.
#[derive(Clone)]
pub enum PushSource {
    /// A watcher keeps a connection open (IMAP IDLE).
    Watch(Arc<dyn MailWatcher>),
    /// The Inbox is checked on a timer.
    Poll(Duration),
    /// Graph change subscriptions through the Cumulas push service
    /// (Microsoft accounts, while the service is turned on).
    Relay(RelaySource),
}

#[async_trait]
pub trait Connector: Send + Sync {
    async fn providers(&self, account: &Account) -> Result<ProviderSet>;

    /// How `account` gets push. By default the Inbox is polled.
    async fn push_source(&self, account: &Account) -> Result<PushSource> {
        Ok(PushSource::Poll(match account.kind {
            AccountKind::Microsoft => MICROSOFT_POLL,
            AccountKind::Imap => UNSUPPORTED_POLL,
        }))
    }

    /// Checks IMAP and SMTP settings and credentials.
    async fn verify_imap(&self, credentials: &ImapCredentials) -> Result<()>;

    /// Interactive Microsoft sign-in in the user's browser.
    /// `push_consent` also asks consent for the push service's scope.
    async fn microsoft_sign_in(
        &self,
        open_browser: &BrowserOpener,
        login_hint: Option<&str>,
        push_consent: bool,
    ) -> Result<SignInResult>;
}

/// The real connector: Microsoft Graph and IMAP/SMTP over the network.
pub struct LiveConnector {
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
    /// The push service, made on first use: one connection for every
    /// account.
    relay: OnceLock<Arc<dyn Relay>>,
}

impl LiveConnector {
    pub fn new(store: Arc<Store>, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            store,
            secrets,
            relay: OnceLock::new(),
        }
    }

    fn relay(&self) -> Arc<dyn Relay> {
        Arc::clone(
            self.relay
                .get_or_init(|| Arc::new(HttpRelay::new(relay::sync_url()))),
        )
    }

    async fn imap_credentials(
        &self,
        account: &Account,
        settings: &ImapSettings,
    ) -> Result<ImapCredentials> {
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
        Ok(ImapCredentials {
            settings: settings.clone(),
            password,
            display_name: account.display_name.clone(),
            email: account.email.clone(),
        })
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
                    tasks: Some(graph.clone()),
                    subscriptions: Some(graph),
                })
            }
            AccountSettings::Imap(settings) => {
                let credentials = self.imap_credentials(account, settings).await?;
                Ok(ProviderSet {
                    mail: Arc::new(ImapProvider::new(account.id.clone(), credentials)),
                    calendar: None,
                    contacts: None,
                    tasks: None,
                    subscriptions: None,
                })
            }
        }
    }

    async fn push_source(&self, account: &Account) -> Result<PushSource> {
        match &account.settings {
            AccountSettings::Microsoft(_) => Ok(PushSource::Relay(RelaySource {
                relay: self.relay(),
                tokens: Arc::new(SyncTokenSource::new(
                    self.microsoft_config()?,
                    account.id.clone(),
                    Arc::clone(&self.secrets),
                )),
            })),
            AccountSettings::Imap(settings) => {
                let credentials = self.imap_credentials(account, settings).await?;
                Ok(PushSource::Watch(Arc::new(ImapIdleWatcher::new(
                    credentials,
                    IdleConfig::default(),
                ))))
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
        push_consent: bool,
    ) -> Result<SignInResult> {
        let mut config = self.microsoft_config()?;
        if push_consent {
            config.consent_scopes.push(sync_scope(&config.client_id));
        }
        microsoft::sign_in(&config, open_browser, login_hint, Duration::from_secs(300)).await
    }
}
