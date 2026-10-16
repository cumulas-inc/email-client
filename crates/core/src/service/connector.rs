//! Builds protocol providers for accounts. Abstracted behind [`Connector`] so
//! the service can be tested without network access.

use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;

use async_trait::async_trait;

use crate::auth::microsoft::{
    self, ApiTokenSource, MicrosoftAuthConfig, MicrosoftTokenSource, SignInResult, SyncTokenSource,
    sync_scope,
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
use crate::signatures::client::SignatureClient;
use crate::signatures::config::{NO_CLIENT_ID, SIGNATURE_CLIENT_ID_VAR, SignatureService, is_guid};
use crate::signatures::{HttpOrganizationSignatures, OrganizationSignatures};
use crate::store::Store;

/// The Application (client) ID of Cumulas Mail's published Microsoft Entra
/// app registration (any organization and personal accounts, from the
/// verified publisher Cumulas Inc), built in unless the build names another
/// or none (see [`builtin_microsoft_client_id`]). It isn't a secret: every
/// sign-in shows it to Microsoft.
pub const PUBLISHED_MICROSOFT_CLIENT_ID: &str = "10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad";
/// The variable that builds another registration's client ID in, or with
/// `none` no client ID at all. Read at build time only: at run time,
/// Settings → Microsoft app registration overrides it.
pub const MICROSOFT_CLIENT_ID_VAR: &str = "CUMULAS_MS_CLIENT_ID";
const BUILD_MICROSOFT_CLIENT_ID: Option<&str> = option_env!("CUMULAS_MS_CLIENT_ID");

/// The client ID Microsoft sign-in uses unless Settings names another:
/// `CUMULAS_MS_CLIENT_ID` at build time, else
/// [`PUBLISHED_MICROSOFT_CLIENT_ID`]; `None` when the build said `none`,
/// and people must then enter one in Settings before signing in. A value
/// that isn't a GUID is ignored, with a warning.
pub fn builtin_microsoft_client_id() -> Option<&'static str> {
    // Worked out once, so that a bad value is warned about once rather than
    // at every sign-in and token refresh.
    static RESOLVED: LazyLock<Option<String>> =
        LazyLock::new(|| resolve_builtin_microsoft_client_id(BUILD_MICROSOFT_CLIENT_ID));
    RESOLVED.as_deref()
}

fn resolve_builtin_microsoft_client_id(built_in: Option<&str>) -> Option<String> {
    let Some(value) = built_in.map(str::trim).filter(|v| !v.is_empty()) else {
        return Some(PUBLISHED_MICROSOFT_CLIENT_ID.to_owned());
    };
    if value.eq_ignore_ascii_case(NO_CLIENT_ID) {
        return None;
    }
    if is_guid(value) {
        return Some(value.to_ascii_lowercase());
    }
    tracing::warn!(
        "ignoring the Microsoft client ID from the build ({MICROSOFT_CLIENT_ID_VAR}): it must be \
         a GUID (or `{NO_CLIENT_ID}`)"
    );
    Some(PUBLISHED_MICROSOFT_CLIENT_ID.to_owned())
}

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

    /// Asks a Microsoft 365 work account's organization for its signature
    /// (Cumulas Signatures); `None` when the app can't (company signatures
    /// are off for this build or run, see [`SignatureService::configured`]).
    /// Only asked for work and school accounts.
    fn organization_signatures(
        &self,
        _account: &Account,
    ) -> Option<Arc<dyn OrganizationSignatures>> {
        None
    }
}

/// The real connector: Microsoft Graph and IMAP/SMTP over the network.
pub struct LiveConnector {
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
    /// The push service, made on first use: one connection for every
    /// account.
    relay: OnceLock<Arc<dyn Relay>>,
    /// The signature service and its API's scope, read on first use; `None`
    /// when company signatures are off.
    signatures: OnceLock<Option<(String, Arc<SignatureClient>)>>,
}

impl LiveConnector {
    pub fn new(store: Arc<Store>, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            store,
            secrets,
            relay: OnceLock::new(),
            signatures: OnceLock::new(),
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
            .or_else(|| builtin_microsoft_client_id().map(str::to_owned))
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

    fn organization_signatures(
        &self,
        account: &Account,
    ) -> Option<Arc<dyn OrganizationSignatures>> {
        let AccountSettings::Microsoft(settings) = &account.settings else {
            return None;
        };
        let (scope, client) = self
            .signatures
            .get_or_init(|| {
                let Some(service) = SignatureService::configured() else {
                    tracing::info!(
                        "company signatures are off: {SIGNATURE_CLIENT_ID_VAR} is `{NO_CLIENT_ID}`, \
                         or names no client ID for the signature service"
                    );
                    return None;
                };
                tracing::info!("company signatures come from {}", service.host());
                let client = SignatureClient::new(&service.url)?;
                Some((service.scope, Arc::new(client)))
            })
            .clone()?;
        // The API is looked for in the account's own organization.
        let config = MicrosoftAuthConfig {
            tenant: settings.tenant_id.clone(),
            ..self.microsoft_config().ok()?
        };
        let tokens = ApiTokenSource::new(
            config,
            account.id.clone(),
            Arc::clone(&self.secrets),
            &scope,
        );
        Some(Arc::new(HttpOrganizationSignatures::new(
            client,
            Arc::new(tokens),
            scope,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "99999999-8888-7777-6666-555555555555";

    #[test]
    fn the_published_client_id_is_built_in_unless_the_build_names_another() {
        let published = Some(PUBLISHED_MICROSOFT_CLIENT_ID);
        // Not set, or set empty (as the Flatpak manifest does).
        assert_eq!(
            resolve_builtin_microsoft_client_id(None).as_deref(),
            published
        );
        assert_eq!(
            resolve_builtin_microsoft_client_id(Some("")).as_deref(),
            published
        );
        assert_eq!(
            resolve_builtin_microsoft_client_id(Some("  ")).as_deref(),
            published
        );
        // Another registration's.
        assert_eq!(
            resolve_builtin_microsoft_client_id(Some(ID)).as_deref(),
            Some(ID)
        );
        assert_eq!(
            resolve_builtin_microsoft_client_id(Some(&format!(" {} ", ID.to_uppercase())))
                .as_deref(),
            Some(ID)
        );
        // None at all: people enter one in Settings.
        for none in ["none", "NONE", " None "] {
            assert_eq!(
                resolve_builtin_microsoft_client_id(Some(none)),
                None,
                "{none}"
            );
        }
        // Not a GUID: ignored, as if not set.
        for junk in [
            "not-a-guid",
            "{x}",
            "off",
            "99999999888877776666555555555555",
        ] {
            assert_eq!(
                resolve_builtin_microsoft_client_id(Some(junk)).as_deref(),
                published,
                "{junk}"
            );
        }
        assert!(is_guid(PUBLISHED_MICROSOFT_CLIENT_ID));
    }
}
