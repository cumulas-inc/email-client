//! The organization setup app: a separate Microsoft Entra app registration
//! an administrator signs in with to read, create and repair the
//! organization's mail flow rules and auto-labeling policies
//! (`crate::orgsetup`).
//!
//! It is never the mail app's registration, and the mail app never asks
//! for what it asks for: its Exchange.Manage permission would let anyone
//! holding a token change the organization's mail flow. Nothing here builds
//! a link or a scope with the mail app's helpers (a test reads this file to
//! make sure), and the mail app's scopes, refresh scopes and approval links
//! never name a [`SETUP_ONLY_MARKERS`] string (tests in `microsoft.rs`).

use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime};

use serde_json::json;
use url::Url;

use super::TokenSource;
use super::microsoft::{
    ADMIN_CONSENT_CODES, ADMIN_CONSENT_WORKFLOW_CODE, AuthorizeFailure, BlockedReason,
    DEFAULT_AUTHORITY_HOST, LoopbackListener, MicrosoftAuthConfig, MicrosoftTokenSource,
    OAuthError, REFRESH_MARGIN, TokenFailure, USER_DECLINED_CODE, WallClock, aadsts_codes,
    authorize, decode_id_token, first_line, http_client, request_token,
};
use crate::error::{Error, ErrorCode, Result};
use crate::orgsetup::runlog::{RunLog, SignInRecord};
use crate::secrets::{MemorySecretStore, keys};
use crate::service::{PUBLISHED_MICROSOFT_CLIENT_ID, builtin_microsoft_client_id};
use crate::signatures::config::{NO_CLIENT_ID, is_guid};

/// What people see: the registration's display name, and every mention of
/// the setup app in Cumulas Mail. `SETUP_APP_NAME` in
/// `src/modules/settings/orgsetup/copy.ts` must match.
pub const SETUP_APP_NAME: &str = "Cumulas Mail - Admin Setup";
/// "Cumulas Mail - Admin Setup": Cumulas Inc's multi-tenant public client
/// (organizations only, http://localhost, publisher-verified) that an
/// administrator signs in with to set up and repair the organization's
/// rules and auto-labeling policies. Never the mail app's registration.
/// Lowercase GUID. Released only once the registration is finished (owner
/// steps A, README "Before release").
pub const PUBLISHED_SETUP_CLIENT_ID: Option<&str> = Some("b4f17c55-b9f6-4a84-958f-afe3842da327");
/// The build variable naming another setup registration, or `none` for
/// none (then only the PowerShell script sets things up).
pub const SETUP_CLIENT_ID_VAR: &str = "CUMULAS_MS_SETUP_CLIENT_ID";
const BUILD_SETUP_CLIENT_ID: Option<&str> = option_env!("CUMULAS_MS_SETUP_CLIENT_ID");

/// Exchange Online, the resource setup's mail flow rule calls go to.
pub const EXCHANGE_RESOURCE: &str = "https://outlook.office365.com";
pub const EXCHANGE_MANAGE_SCOPE: &str = "https://outlook.office365.com/Exchange.Manage";
/// Where the regional Security & Compliance host is found.
pub const COMPLIANCE_DISCOVERY_HOST: &str = "https://ps.compliance.protection.outlook.com";
/// Tried in order for the Security & Compliance token.
pub const COMPLIANCE_SCOPES: [&str; 2] = [
    "https://ps.compliance.protection.outlook.com/.default",
    "https://ps.compliance.protection.outlook.com/Exchange.Manage",
];
/// What the setup sign-in asks consent for, in this order. String literals,
/// so check-admin-consent.sh can read them; a test asserts the last equals
/// [`EXCHANGE_MANAGE_SCOPE`].
pub const SETUP_CONSENT_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "offline_access",
    "https://outlook.office365.com/Exchange.Manage",
];
/// Where the approval link and the sign-in return: the registration's only
/// redirect URI. Nothing listens there once an administrator has approved,
/// so their browser then shows a page that can't connect; the approval is
/// recorded by then.
pub const SETUP_REDIRECT_URI: &str = "http://localhost";
/// Strings that never appear in the mail app's scopes, refresh scopes,
/// approval links or the registration's documented permission list.
pub const SETUP_ONLY_MARKERS: &[&str] = &[
    "outlook.office365.com",
    "Exchange.Manage",
    "ps.compliance.protection.outlook.com",
    "00000002-0000-0ff1-ce00-000000000000",
    "00000007-0000-0ff1-ce00-000000000000",
];

/// The setup client ID this build uses: the build variable (a GUID, or
/// `none`), else [`PUBLISHED_SETUP_CLIENT_ID`]. A value equal to the mail
/// app's client ID (published, built in, or `mail_client_id`, the one
/// entered in Settings) is refused, with a warning: the two registrations
/// must never be one.
pub fn setup_client_id(mail_client_id: Option<&str>) -> Option<String> {
    // Worked out once, so that a bad value is warned about once.
    static BUILT: LazyLock<Option<String>> =
        LazyLock::new(|| resolve_setup_client_id(BUILD_SETUP_CLIENT_ID));
    apart_from_mail(BUILT.as_deref()?, mail_client_id)
}

/// The build variable's setup client ID, else the published one; `None`
/// when the build said `none`. A value that isn't a GUID is ignored, with a
/// warning.
fn resolve_setup_client_id(built: Option<&str>) -> Option<String> {
    let Some(value) = built.map(str::trim).filter(|v| !v.is_empty()) else {
        return PUBLISHED_SETUP_CLIENT_ID.map(str::to_owned);
    };
    if value.eq_ignore_ascii_case(NO_CLIENT_ID) {
        return None;
    }
    if is_guid(value) {
        return Some(value.to_ascii_lowercase());
    }
    tracing::warn!(
        "ignoring the setup client ID from the build ({SETUP_CLIENT_ID_VAR}): it must be a GUID \
         (or `{NO_CLIENT_ID}`)"
    );
    PUBLISHED_SETUP_CLIENT_ID.map(str::to_owned)
}

/// `setup`, unless it is one of the mail app's client IDs.
fn apart_from_mail(setup: &str, mail_client_id: Option<&str>) -> Option<String> {
    let mail = [
        Some(PUBLISHED_MICROSOFT_CLIENT_ID),
        builtin_microsoft_client_id(),
        mail_client_id,
    ];
    if mail
        .into_iter()
        .flatten()
        .any(|id| id.trim().eq_ignore_ascii_case(setup))
    {
        tracing::warn!(
            "not using the setup client ID {setup}: it is the mail app's own, and the two \
             registrations must never be one"
        );
        return None;
    }
    Some(setup.to_owned())
}

/// The setup app's approval link for `tenant` (a tenant ID; `organizations`
/// without one):
/// `https://login.microsoftonline.com/<tenant>/v2.0/adminconsent?client_id=…&scope=…&redirect_uri=http://localhost`,
/// asking for [`SETUP_CONSENT_SCOPES`], the OpenID Connect ones unqualified.
pub fn setup_admin_consent_url(client_id: &str, tenant: Option<&str>) -> String {
    let plausible = |t: &str| {
        !t.is_empty()
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    };
    let tenant = tenant
        .map(str::trim)
        .filter(|t| plausible(t))
        .unwrap_or("organizations");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", client_id)
        .append_pair("scope", &SETUP_CONSENT_SCOPES.join(" "))
        .append_pair("redirect_uri", SETUP_REDIRECT_URI)
        .finish();
    format!("{DEFAULT_AUTHORITY_HOST}/{tenant}/v2.0/adminconsent?{query}")
}

/// Why setup can't sign in from this build (yet): the PowerShell script
/// does the same.
pub(crate) fn setup_not_available() -> Error {
    Error::new(
        ErrorCode::NotConfigured,
        "Automatic setup isn't available in this version of Cumulas Mail. Use the PowerShell \
         script instead.",
    )
}

/// Where an administrator signs in with the setup app: its client ID and the
/// work account's organization.
#[derive(Debug, Clone)]
pub struct AdminSetupConfig {
    /// The setup app's client ID ([`setup_client_id`]): never the mail
    /// app's.
    pub client_id: String,
    /// The work account's tenant ID: the sign-in is for that organization
    /// only.
    pub tenant_id: String,
    /// `https://login.microsoftonline.com`; a mock server's in tests.
    pub authority_host: String,
}

impl AdminSetupConfig {
    /// For `authorize` and `request_token` only: the work account's tenant,
    /// no Graph or consent scopes, never the published mail registration.
    /// Nothing else may take it.
    pub(super) fn auth_config(&self) -> MicrosoftAuthConfig {
        MicrosoftAuthConfig {
            client_id: self.client_id.clone(),
            tenant: self.tenant_id.clone(),
            authority_host: self.authority_host.clone(),
            consent_scopes: Vec::new(),
            graph_scopes: Vec::new(),
            published: false,
        }
    }

    /// Where the sign-in happens, for the run log: the authority's host and
    /// the tenant.
    fn authority(&self) -> String {
        let host = Url::parse(&self.authority_host)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| self.authority_host.clone());
        format!("{host}/{}", self.tenant_id)
    }
}

/// An administrator's setup sign-in.
pub struct AdminSignIn {
    /// The ID token's preferred_username (else email): the UPN anchor.
    pub username: String,
    /// Its name, else the username.
    pub display_name: String,
    /// Its tid, checked against the work account's organization.
    pub tenant_id: String,
    /// Its oid: for the command that removes this administrator's approval.
    pub object_id: String,
    pub tokens: AdminTokens,
    /// The redemption's `scope`, for the run log.
    pub granted: String,
}

/// What the setup sign-in asks for and redeems its code for.
fn setup_scope() -> String {
    SETUP_CONSENT_SCOPES.join(" ")
}

/// Opens the browser at Microsoft's sign-in for the setup app and waits (up
/// to `timeout`) for the code on a fresh loopback listener, then redeems it.
/// Every attempt, failed or not, goes to `log`: where, what was asked for,
/// and what Microsoft answered (never a token).
///
/// Errors: see [`admin_sign_in_error`]; `InvalidInput` (details `reason:
/// "otherTenant"`) for an administrator of another organization.
pub async fn admin_sign_in(
    config: &AdminSetupConfig,
    open_browser: &(dyn Fn(&str) -> Result<()> + Send + Sync),
    timeout: Duration,
    log: &RunLog,
) -> Result<AdminSignIn> {
    let auth = config.auth_config();
    let scope = setup_scope();
    let mut record = SignInRecord::new(config.authority(), &scope);
    let listener = match LoopbackListener::bind().await {
        Ok(listener) => listener,
        Err(e) => {
            log.sign_in(record.failed(&e));
            return Err(e);
        }
    };
    record.port = Url::parse(&listener.redirect_uri())
        .ok()
        .and_then(|u| u.port());
    let authorized = authorize(&auth, &scope, &listener, false, open_browser, None, timeout).await;
    let authorization = match authorized {
        Ok(authorization) => authorization,
        Err(AuthorizeFailure::OAuth(e)) => {
            log.sign_in(refused(record, &e));
            return Err(admin_sign_in_error(config, &e));
        }
        Err(AuthorizeFailure::Other(e)) => {
            log.sign_in(record.failed(&e));
            return Err(e);
        }
    };
    drop(listener);

    let redeemed = request_token(
        &http_client(),
        &auth,
        &[
            ("client_id", config.client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", authorization.code.as_str()),
            ("redirect_uri", authorization.redirect_uri.as_str()),
            ("code_verifier", authorization.code_verifier.as_str()),
            ("scope", scope.as_str()),
        ],
    )
    .await;
    let tokens = match redeemed {
        Ok(tokens) => tokens,
        Err(TokenFailure::OAuth(e)) => {
            log.sign_in(refused(record, &e));
            return Err(admin_sign_in_error(config, &e));
        }
        Err(TokenFailure::Other(e)) => {
            log.sign_in(record.failed(&e));
            return Err(e);
        }
    };
    log.sign_in(record.redeemed(
        tokens.scope.as_deref(),
        tokens.id_token.is_some(),
        tokens.refresh_token.is_some(),
    ));

    let claims = tokens
        .id_token
        .as_deref()
        .ok_or_else(|| Error::provider("Microsoft did not return an ID token."))
        .and_then(decode_id_token)?;
    let refresh_token = tokens.refresh_token.as_deref().ok_or_else(|| {
        Error::provider("Microsoft did not return a refresh token (offline_access missing).")
    })?;
    let (Some(tenant_id), Some(object_id)) = (claims.tid, claims.oid) else {
        return Err(Error::provider(
            "Microsoft's response did not identify the account (tid/oid missing).",
        ));
    };
    if !tenant_id.trim().eq_ignore_ascii_case(&config.tenant_id) {
        log.note(format!(
            "Signed in to another organization (tenant {}), not {}.",
            tenant_id.trim(),
            config.tenant_id
        ));
        return Err(other_tenant("your organization"));
    }
    let username = claims
        .preferred_username
        .filter(|u| !u.trim().is_empty())
        .or(claims.email)
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            Error::provider(
                "Microsoft's response did not include the administrator's sign-in name.",
            )
        })?;
    let display_name = claims
        .name
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| username.clone());
    let admin_tokens = AdminTokens::new(
        config,
        refresh_token,
        log.clone(),
        Arc::new(SystemTime::now),
    );
    admin_tokens
        .prime(
            Resource::Exchange,
            &tokens.access_token,
            Duration::from_secs(tokens.expires_in),
        )
        .await;
    Ok(AdminSignIn {
        username,
        display_name,
        tenant_id: config.tenant_id.clone(),
        object_id: object_id.trim().to_ascii_lowercase(),
        tokens: admin_tokens,
        granted: tokens.scope.unwrap_or_default(),
    })
}

/// The run log's record of a refusal: the error, its AADSTS codes and the
/// first line of Microsoft's description.
fn refused(record: SignInRecord, error: &OAuthError) -> SignInRecord {
    let mut codes = error.error_codes.clone();
    codes.extend(aadsts_codes(&error.error_description));
    codes.sort_unstable();
    codes.dedup();
    record.refused(&error.error, codes, first_line(&error.error_description))
}

/// The administrator signed in to another organization than the work
/// account's (`domain`, its email domain).
pub fn other_tenant(domain: &str) -> Error {
    Error::invalid_input(format!(
        "You signed in with an account from another organization. Sign in with an \
         administrator account of {domain}."
    ))
    .with_details(json!({ "reason": "otherTenant" }))
}

/// AADSTS codes meaning the setup app isn't fully registered: 650057
/// InvalidResource (Exchange.Manage isn't among its permissions), 70011
/// InvalidScope, 650053 (the scope doesn't exist) and 700016 (the app isn't
/// in the directory).
const NOT_REGISTERED_CODES: &[u32] = &[650057, 70011, 650053, 700016];
/// AADSTS500011: the resource (Exchange Online) isn't in the organization.
const NO_EXCHANGE_CODE: u32 = 500011;
/// AADSTS90002: the tenant isn't in Microsoft's public cloud.
const OTHER_CLOUD_CODE: u32 = 90002;

/// The first AADSTS code of `error`.
fn first_code(error: &OAuthError) -> Option<u32> {
    error
        .error_codes
        .first()
        .copied()
        .or_else(|| aadsts_codes(&error.error_description).first().copied())
}

/// Maps an error from the setup app's authorization redirect or code
/// redemption. Every link it gives is the setup app's own approval link.
pub(super) fn admin_sign_in_error(config: &AdminSetupConfig, error: &OAuthError) -> Error {
    let app = SETUP_APP_NAME;
    let code = first_code(error);
    let approval = || setup_admin_consent_url(&config.client_id, Some(&config.tenant_id));
    if let Some((reason, code)) = BlockedReason::find(error) {
        let mut details = json!({
            "reason": reason.as_str(),
            "aadsts": code,
            "app": "setup",
            "tenant": config.tenant_id,
        });
        if reason == BlockedReason::AppNotSetUp {
            details["adminConsentUrl"] = json!(approval());
        }
        return Error::new(ErrorCode::SignInBlocked, reason.setup_message()).with_details(details);
    }
    if ADMIN_CONSENT_CODES.iter().any(|c| error.has_code(*c))
        || (error.error == "consent_required" && !error.has_code(USER_DECLINED_CODE))
    {
        return Error::new(
            ErrorCode::AdminConsentRequired,
            format!("Microsoft 365 needs an administrator to approve {app} before you can use it."),
        )
        .with_details(json!({
            "reason": "setupConsent",
            "adminConsentUrl": approval(),
            "tenant": config.tenant_id,
            "canRequestApproval": error.has_code(ADMIN_CONSENT_WORKFLOW_CODE),
            "aadsts": code,
        }));
    }
    if error.has_code(USER_DECLINED_CODE) {
        return Error::new(
            ErrorCode::ConsentDeclined,
            format!(
                "You declined the permissions {app} asks for, so it can't read or change your \
                 organization's setup. Sign in again to approve them, or use the PowerShell \
                 script."
            ),
        );
    }
    if let Some(code) = NOT_REGISTERED_CODES
        .iter()
        .copied()
        .find(|c| error.has_code(*c))
    {
        return Error::provider(format!(
            "{app} isn't fully registered with Microsoft (AADSTS{code}). Use the PowerShell \
             script for now, and send Copy setup details to Cumulas."
        ))
        .with_details(json!({ "reason": "setupNotRegistered", "aadsts": code }));
    }
    if error.has_code(NO_EXCHANGE_CODE) {
        return Error::provider(
            "Microsoft says your organization doesn't have Exchange Online, which setting this \
             up needs (AADSTS500011).",
        )
        .with_details(json!({ "reason": "noExchange", "aadsts": NO_EXCHANGE_CODE }));
    }
    if error.has_code(OTHER_CLOUD_CODE) {
        return Error::provider(
            "Microsoft couldn't find your organization in its public cloud. Organizations in \
             government or other national clouds need to use the PowerShell script.",
        )
        .with_details(json!({ "reason": "otherCloud", "aadsts": OTHER_CLOUD_CODE }));
    }
    if error.error == "access_denied" {
        let canceled = error
            .error_subcode
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("cancel"))
            || error
                .error_description
                .to_ascii_lowercase()
                .contains("cancel");
        return if canceled {
            Error::new(ErrorCode::SignInCancelled, "Sign-in was canceled.")
        } else {
            Error::new(
                ErrorCode::ConsentDeclined,
                format!(
                    "You declined the permissions {app} asks for, so it can't read or change \
                     your organization's setup. Sign in again to approve them, or use the \
                     PowerShell script."
                ),
            )
        };
    }
    Error::provider(error.summary()).with_details(json!({ "error": error.error, "aadsts": code }))
}

impl BlockedReason {
    /// What the setup app's sign-in says for this reason: the mail app's
    /// words, about the setup app.
    pub(super) fn setup_message(self) -> String {
        let app = SETUP_APP_NAME;
        match self {
            Self::NotAssigned => format!(
                "Your organization requires assignment to {app}. An administrator assigns you \
                 under Enterprise apps → {app} → Users and groups."
            ),
            Self::AppDisabled => format!(
                "Your organization has blocked {app}. Ask your IT team whether they can allow it."
            ),
            Self::AppNotSetUp => format!(
                "Microsoft 365 in your organization isn't set up for {app} yet. Ask your IT team \
                 to approve it."
            ),
            Self::ServiceNotEnabled => format!(
                "Your organization's Microsoft 365 doesn't include a service {app} needs, usually \
                 Exchange Online mail. Ask your IT team to check your subscription."
            ),
            Self::SecurityInfo => "Your organization needs you to finish setting up your sign-in \
                                   security info, such as multifactor authentication. Ask your IT \
                                   team for help."
                .to_owned(),
            Self::ConditionalAccess => "Your organization's sign-in rules didn't allow this \
                                        sign-in, for example because of the device, the app or \
                                        where you're signing in from. Ask your IT team what's \
                                        required."
                .to_owned(),
        }
    }
}

/// What a setup token is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resource {
    /// Exchange Online: mail flow rules.
    Exchange,
    /// Security & Compliance: labels and auto-labeling policies.
    Compliance,
}

/// What Exchange Online's tokens are asked for when refreshed.
fn exchange_refresh_scope() -> String {
    format!("{EXCHANGE_MANAGE_SCOPE} offline_access")
}

/// AADSTS codes meaning a Security & Compliance scope form isn't the one
/// this grant covers, so the next form is tried: 70011 InvalidScope, 650053,
/// 650057, 65001 and 500011.
const COMPLIANCE_RETRY_CODES: &[u32] = &[70011, 650053, 650057, 65001, 500011];

/// The setup sign-in's tokens, in memory only: never written to the keyring,
/// the database or the log. Built on the mail app's own token machinery: the
/// refresh token lives in a memory store of this session's own, under a
/// synthetic account ID, and each resource's tokens come from a
/// `MicrosoftTokenSource` on it, so rotation, the refresh margin and expiry
/// by both clocks are the mail app's. Dropping it drops the refresh token.
pub struct AdminTokens {
    secrets: Arc<MemorySecretStore>,
    config: MicrosoftAuthConfig,
    account: String,
    exchange: MicrosoftTokenSource,
    /// The Security & Compliance tokens, once a scope form worked, and that
    /// form.
    compliance: tokio::sync::Mutex<Option<(MicrosoftTokenSource, &'static str)>>,
    clock: WallClock,
    /// What each token response covered, and refusals' AADSTS codes.
    pub(crate) notes: RunLog,
}

impl AdminTokens {
    /// Tokens from the setup sign-in's `refresh_token`, read by the wall
    /// clock `clock`.
    pub fn new(
        config: &AdminSetupConfig,
        refresh_token: &str,
        notes: RunLog,
        clock: WallClock,
    ) -> Self {
        let secrets = Arc::new(MemorySecretStore::default());
        let account = format!("orgsetup:{}", uuid::Uuid::new_v4());
        secrets.put(&keys::refresh_token(&account), refresh_token);
        let config = config.auth_config();
        let exchange = MicrosoftTokenSource::with_scope(
            config.clone(),
            account.clone(),
            secrets.clone(),
            exchange_refresh_scope(),
        )
        .clocked(clock.clone());
        Self {
            secrets,
            config,
            account,
            exchange,
            compliance: tokio::sync::Mutex::new(None),
            clock,
            notes,
        }
    }

    /// Seeds `resource`'s cache with a token Microsoft just issued (the
    /// sign-in's, for Exchange Online).
    pub async fn prime(&self, resource: Resource, access_token: &str, expires_in: Duration) {
        match resource {
            Resource::Exchange => self.exchange.prime(access_token, expires_in).await,
            Resource::Compliance => {
                if let Some((source, _)) = self.compliance.lock().await.as_ref() {
                    source.prime(access_token, expires_in).await;
                }
            }
        }
    }

    /// An access token for `resource`, refreshed with the in-memory refresh
    /// token when needed. Security & Compliance tries [`COMPLIANCE_SCOPES`]
    /// in order, and keeps the form that worked.
    pub async fn token(&self, resource: Resource) -> Result<String> {
        match resource {
            Resource::Exchange => self
                .exchange
                .token(REFRESH_MARGIN, false)
                .await
                .map_err(setup_token_error),
            Resource::Compliance => self.compliance_token().await,
        }
    }

    async fn compliance_token(&self) -> Result<String> {
        let mut compliance = self.compliance.lock().await;
        if let Some((source, _)) = compliance.as_ref() {
            return source
                .token(REFRESH_MARGIN, false)
                .await
                .map_err(setup_token_error);
        }
        let mut refusals = Vec::new();
        for scope in COMPLIANCE_SCOPES {
            let source = MicrosoftTokenSource::with_scope(
                self.config.clone(),
                self.account.clone(),
                self.secrets.clone(),
                format!("{scope} offline_access"),
            )
            .clocked(self.clock.clone());
            match source.token(REFRESH_MARGIN, false).await {
                Ok(token) => {
                    self.notes
                        .note(format!("Security & Compliance token: {scope} issued"));
                    *compliance = Some((source, scope));
                    return Ok(token);
                }
                Err(TokenFailure::OAuth(e))
                    if e.error == "invalid_scope"
                        || COMPLIANCE_RETRY_CODES.iter().any(|c| e.has_code(*c)) =>
                {
                    let refusal = refusal(&e);
                    self.notes.note(format!(
                        "Security & Compliance token: {scope} refused ({refusal})"
                    ));
                    refusals.push(refusal);
                }
                Err(e) => {
                    let e = setup_token_error(e);
                    self.notes.note(format!(
                        "Security & Compliance token: {scope} failed ({})",
                        e.message
                    ));
                    return Err(e);
                }
            }
        }
        Err(Error::provider(format!(
            "Microsoft didn't issue a token for Security & Compliance ({}).",
            refusals.join("; ")
        )))
    }

    /// Forgets `resource`'s cached access token after a server rejected it.
    pub async fn invalidate(&self, resource: Resource) {
        match resource {
            Resource::Exchange => self.exchange.invalidate().await,
            Resource::Compliance => {
                if let Some((source, _)) = self.compliance.lock().await.as_ref() {
                    source.invalidate().await;
                }
            }
        }
    }

    /// The memory store holding the refresh token, to see that it goes with
    /// the tokens.
    pub(crate) fn store(&self) -> std::sync::Weak<MemorySecretStore> {
        Arc::downgrade(&self.secrets)
    }
}

/// An OAuth refusal for the run log: the error and its AADSTS codes, never
/// the description (it can name the person).
fn refusal(error: &OAuthError) -> String {
    let mut codes = error.error_codes.clone();
    codes.extend(aadsts_codes(&error.error_description));
    codes.sort_unstable();
    codes.dedup();
    let codes: String = codes.iter().map(|c| format!(", AADSTS{c}")).collect();
    format!("{}{codes}", error.error)
}

/// A setup token that couldn't be had: a refusal ends the sign-in (sign in
/// again), anything else is what it was.
fn setup_token_error(failure: TokenFailure) -> Error {
    match failure {
        TokenFailure::OAuth(e) => Error::auth_required(format!(
            "Your setup sign-in ended. Sign in again to continue. ({})",
            refusal(&e)
        ))
        .with_details(json!({ "reason": "setupSignedOut", "error": e.error })),
        TokenFailure::Other(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The approval link the IT page publishes for the setup app.
    const PUBLISHED_SETUP_LINK: &str = "https://login.microsoftonline.com/organizations/v2.0/adminconsent?client_id=b4f17c55-b9f6-4a84-958f-afe3842da327&scope=openid+profile+offline_access+https%3A%2F%2Foutlook.office365.com%2FExchange.Manage&redirect_uri=http%3A%2F%2Flocalhost";
    const ID: &str = "99999999-8888-7777-6666-555555555555";

    #[test]
    fn the_setup_app_is_named_and_registered_as_the_owner_made_it() {
        assert_eq!(SETUP_APP_NAME, "Cumulas Mail - Admin Setup");
        let id = PUBLISHED_SETUP_CLIENT_ID.unwrap();
        assert_eq!(id, "b4f17c55-b9f6-4a84-958f-afe3842da327");
        assert!(is_guid(id) && id == id.to_ascii_lowercase());
        assert_ne!(id, PUBLISHED_MICROSOFT_CLIENT_ID);
        assert_eq!(
            SETUP_CONSENT_SCOPES.last().copied(),
            Some(EXCHANGE_MANAGE_SCOPE)
        );
        assert_eq!(SETUP_CONSENT_SCOPES[3], EXCHANGE_MANAGE_SCOPE);
        assert_eq!(
            EXCHANGE_MANAGE_SCOPE,
            format!("{EXCHANGE_RESOURCE}/Exchange.Manage")
        );
        assert!(
            COMPLIANCE_SCOPES
                .iter()
                .all(|s| s.starts_with(COMPLIANCE_DISCOVERY_HOST))
        );
    }

    #[test]
    fn setup_client_id_is_never_the_mail_client_id() {
        let setup = PUBLISHED_SETUP_CLIENT_ID.unwrap();
        // Not set, or set empty: the published one.
        for unset in [None, Some(""), Some("  ")] {
            assert_eq!(
                resolve_setup_client_id(unset).as_deref(),
                Some(setup),
                "{unset:?}"
            );
        }
        assert_eq!(
            resolve_setup_client_id(Some(&format!(" {} ", ID.to_uppercase()))).as_deref(),
            Some(ID)
        );
        for none in ["none", " NONE "] {
            assert_eq!(resolve_setup_client_id(Some(none)), None, "{none}");
        }
        assert_eq!(
            resolve_setup_client_id(Some("not-a-guid")).as_deref(),
            Some(setup)
        );

        // The published setup app, next to the published mail app or an
        // organization's own: kept.
        assert_eq!(
            apart_from_mail(setup, Some(PUBLISHED_MICROSOFT_CLIENT_ID)).as_deref(),
            Some(setup)
        );
        assert_eq!(apart_from_mail(setup, Some(ID)).as_deref(), Some(setup));
        // The setup ID entered in Settings as the mail app's: refused.
        assert_eq!(apart_from_mail(setup, Some(setup)), None);
        assert_eq!(
            apart_from_mail(setup, Some(&format!(" {} ", setup.to_uppercase()))),
            None
        );
        // A build naming the published mail app as the setup app: refused,
        // whatever Settings says.
        let built = resolve_setup_client_id(Some(PUBLISHED_MICROSOFT_CLIENT_ID)).unwrap();
        assert_eq!(apart_from_mail(&built, None), None);
        assert_eq!(apart_from_mail(&built, Some(ID)), None);
        // A build value equal to the mail app's own client ID: refused.
        let built = resolve_setup_client_id(Some(ID)).unwrap();
        assert_eq!(apart_from_mail(&built, Some(ID)), None);

        // What this build does.
        if BUILD_SETUP_CLIENT_ID.is_none_or(|v| v.trim().is_empty()) {
            assert_eq!(
                setup_client_id(Some(PUBLISHED_MICROSOFT_CLIENT_ID)).as_deref(),
                Some(setup)
            );
            assert_eq!(setup_client_id(Some(setup)), None);
        }
    }

    #[test]
    fn setup_admin_consent_url_for_the_published_registration() {
        let setup = PUBLISHED_SETUP_CLIENT_ID.unwrap();
        assert_eq!(setup_admin_consent_url(setup, None), PUBLISHED_SETUP_LINK);
        // Not a tenant: organizations.
        for junk in [Some(""), Some(" "), Some("contoso.com/../x"), Some("a b")] {
            assert_eq!(
                setup_admin_consent_url(setup, junk),
                PUBLISHED_SETUP_LINK,
                "{junk:?}"
            );
        }
    }

    #[test]
    fn setup_admin_consent_url_names_the_tenant_and_the_scopes_in_order() {
        let tenant = "85f67998-e79a-4173-95c2-90b0e6dc231d";
        let url = setup_admin_consent_url(ID, Some(tenant));
        assert!(
            url.starts_with(&format!(
                "https://login.microsoftonline.com/{tenant}/v2.0/adminconsent?client_id={ID}&"
            )),
            "{url}"
        );
        assert!(
            url.ends_with("&redirect_uri=http%3A%2F%2Flocalhost"),
            "{url}"
        );
        let parsed = url::Url::parse(&url).unwrap();
        let scope = parsed
            .query_pairs()
            .find(|(k, _)| k == "scope")
            .map(|(_, v)| v.into_owned())
            .unwrap();
        assert_eq!(
            scope,
            "openid profile offline_access https://outlook.office365.com/Exchange.Manage"
        );
        assert_eq!(scope.split(' ').collect::<Vec<_>>(), SETUP_CONSENT_SCOPES);
        // Never the mail app's: no Graph scope, no push service.
        assert!(!url.contains("graph.microsoft.com"));
        assert!(!url.contains("Sync.Receive"));
        assert!(!url.contains("email"));
    }

    /// Whether `source` calls the function `name` (a whole word, not part
    /// of a longer name such as `setup_admin_consent_url`).
    fn calls(source: &str, name: &str) -> bool {
        source.match_indices(&format!("{name}(")).any(|(at, _)| {
            source[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
        })
    }

    #[test]
    fn never_builds_a_link_with_the_mail_apps_helpers() {
        let source = include_str!("admin_setup.rs");
        let (code, _tests) = source.split_once("#[cfg(test)]").unwrap();
        for helper in ["admin_consent_link", "admin_consent_url", "sign_in_error"] {
            assert!(!calls(code, helper), "admin_setup.rs calls {helper}");
        }
        // The check itself finds calls, and only whole names.
        assert!(calls(
            "x = admin_consent_url(&c, None);",
            "admin_consent_url"
        ));
        assert!(calls("microsoft::sign_in_error(c, e)", "sign_in_error"));
        assert!(!calls(
            "setup_admin_consent_url(id, None)",
            "admin_consent_url"
        ));
        assert!(!calls("admin_sign_in_error(c, e)", "sign_in_error"));
    }

    // --- Signing in with the setup app ---------------------------------------

    mod signin {
        use std::collections::HashMap;

        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::auth::microsoft::tests::{fake_browser, id_token, redirect_with, token_error};
        use crate::orgsetup::runlog::Entry;

        const TENANT: &str = "85f67998-e79a-4173-95c2-90b0e6dc231d";
        const OTHER_TENANT: &str = "11111111-2222-3333-4444-555555555555";
        const SCOPE: &str =
            "openid profile offline_access https://outlook.office365.com/Exchange.Manage";

        fn config(server: &MockServer) -> AdminSetupConfig {
            AdminSetupConfig {
                client_id: ID.into(),
                tenant_id: TENANT.into(),
                authority_host: server.uri(),
            }
        }

        fn token_path() -> String {
            format!("/{TENANT}/oauth2/v2.0/token")
        }

        fn redeemed(tenant: &str) -> ResponseTemplate {
            ResponseTemplate::new(200).set_body_json(json!({
                "token_type": "Bearer",
                "scope": "https://outlook.office365.com/Exchange.Manage https://outlook.office365.com/.default",
                "expires_in": 3599,
                "access_token": "exchange-1",
                "refresh_token": "refresh-1",
                "id_token": id_token(json!({
                    "tid": tenant,
                    "oid": "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE",
                    "name": "Megan Bowen",
                    "preferred_username": "MeganB@contoso.com",
                })),
            }))
        }

        fn sign_ins(log: &RunLog) -> Vec<SignInRecord> {
            log.entries()
                .into_iter()
                .filter_map(|e| match e {
                    Entry::SignIn(record) => Some(record),
                    _ => None,
                })
                .collect()
        }

        #[tokio::test]
        async fn the_setup_app_signs_in_to_the_work_accounts_organization_only() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path(token_path()))
                .and(body_string_contains("grant_type=authorization_code"))
                .respond_with(redeemed(TENANT))
                .expect(1)
                .mount(&server)
                .await;
            let browser = fake_browser(vec![redirect_with("code=the-code")]);
            let log = RunLog::default();
            let signed_in = admin_sign_in(
                &config(&server),
                &*browser.open,
                Duration::from_secs(10),
                &log,
            )
            .await
            .unwrap();
            assert_eq!(signed_in.username, "MeganB@contoso.com");
            assert_eq!(signed_in.display_name, "Megan Bowen");
            assert_eq!(signed_in.tenant_id, TENANT);
            assert_eq!(signed_in.object_id, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
            assert!(signed_in.granted.contains("Exchange.Manage"));
            // The token the sign-in brought is used: no refresh yet.
            assert_eq!(
                signed_in.tokens.token(Resource::Exchange).await.unwrap(),
                "exchange-1"
            );

            // The authorization request: the setup app, the organization,
            // the scope exactly, PKCE, an account picker and no hint.
            let authorize = browser.authorize_url.lock().unwrap().clone().unwrap();
            assert_eq!(authorize.path(), format!("/{TENANT}/oauth2/v2.0/authorize"));
            let query: HashMap<String, String> = authorize.query_pairs().into_owned().collect();
            assert_eq!(query["client_id"], ID);
            assert_eq!(query["scope"], SCOPE);
            assert_eq!(query["response_type"], "code");
            assert_eq!(query["response_mode"], "query");
            assert_eq!(query["code_challenge_method"], "S256");
            assert_eq!(query["prompt"], "select_account");
            assert!(!query.contains_key("login_hint"));
            assert!(query["redirect_uri"].starts_with("http://localhost:"));
            assert!(!authorize.as_str().contains("graph.microsoft.com"));

            // The redemption asks for the same scope, with the verifier.
            let requests = server.received_requests().await.unwrap();
            let form: HashMap<String, String> = url::form_urlencoded::parse(&requests[0].body)
                .into_owned()
                .collect();
            assert_eq!(form["client_id"], ID);
            assert_eq!(form["scope"], SCOPE);
            assert_eq!(form["redirect_uri"], query["redirect_uri"]);
            assert!(!form["code_verifier"].is_empty());
            assert!(!form.contains_key("client_secret"));

            // The attempt is in the run log, without a token.
            let records = sign_ins(&log);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.outcome, "code redeemed");
            assert_eq!(
                record.authority,
                format!(
                    "{}/{TENANT}",
                    url::Url::parse(&server.uri()).unwrap().host_str().unwrap()
                )
            );
            assert_eq!(record.scope, SCOPE);
            assert!(record.port.is_some());
            assert!(record.id_token && record.refresh_token);
            let text = crate::orgsetup::details_text(&log);
            assert!(
                !text.contains("exchange-1") && !text.contains("refresh-1"),
                "{text}"
            );
        }

        #[tokio::test]
        async fn an_administrator_of_another_organization_is_refused() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path(token_path()))
                .respond_with(redeemed(OTHER_TENANT))
                .mount(&server)
                .await;
            let browser = fake_browser(vec![redirect_with("code=c")]);
            let log = RunLog::default();
            let error = admin_sign_in(
                &config(&server),
                &*browser.open,
                Duration::from_secs(10),
                &log,
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error.code, ErrorCode::InvalidInput);
            assert_eq!(error.details.unwrap()["reason"], "otherTenant");
            assert_eq!(sign_ins(&log)[0].outcome, "code redeemed");
            assert!(crate::orgsetup::details_text(&log).contains(OTHER_TENANT));
        }

        /// One refusal at the redirect: what the admin is told, and its
        /// details.
        struct Refusal {
            query: &'static str,
            code: ErrorCode,
            message: String,
            reason: Option<&'static str>,
            aadsts: Vec<u32>,
            link: bool,
        }

        fn refusals() -> Vec<Refusal> {
            let app = SETUP_APP_NAME;
            let declined = format!(
                "You declined the permissions {app} asks for, so it can't read or change your \
                 organization's setup. Sign in again to approve them, or use the PowerShell \
                 script."
            );
            vec![
                Refusal {
                    query: "error=consent_required&error_description=AADSTS65001%3A+The+user+or+administrator+has+not+consented.",
                    code: ErrorCode::AdminConsentRequired,
                    message: format!(
                        "Microsoft 365 needs an administrator to approve {app} before you can use it."
                    ),
                    reason: Some("setupConsent"),
                    aadsts: vec![65001],
                    link: true,
                },
                Refusal {
                    query: "error=access_denied&error_description=AADSTS90095%3A+Admin+consent+is+required.",
                    code: ErrorCode::AdminConsentRequired,
                    message: format!(
                        "Microsoft 365 needs an administrator to approve {app} before you can use it."
                    ),
                    reason: Some("setupConsent"),
                    aadsts: vec![90095],
                    link: true,
                },
                Refusal {
                    query: "error=access_denied&error_description=AADSTS65004%3A+User+declined+to+consent.",
                    code: ErrorCode::ConsentDeclined,
                    message: declined.clone(),
                    reason: None,
                    aadsts: vec![65004],
                    link: false,
                },
                Refusal {
                    query: "error=invalid_request&error_description=AADSTS650057%3A+Invalid+resource.",
                    code: ErrorCode::Provider,
                    message: format!(
                        "{app} isn't fully registered with Microsoft (AADSTS650057). Use the \
                         PowerShell script for now, and send Copy setup details to Cumulas."
                    ),
                    reason: Some("setupNotRegistered"),
                    aadsts: vec![650057],
                    link: false,
                },
                Refusal {
                    query: "error=unauthorized_client&error_description=AADSTS700016%3A+Application+not+found.",
                    code: ErrorCode::Provider,
                    message: format!(
                        "{app} isn't fully registered with Microsoft (AADSTS700016). Use the \
                         PowerShell script for now, and send Copy setup details to Cumulas."
                    ),
                    reason: Some("setupNotRegistered"),
                    aadsts: vec![700016],
                    link: false,
                },
                Refusal {
                    query: "error=invalid_request&error_description=AADSTS500011%3A+The+resource+principal+was+not+found.",
                    code: ErrorCode::Provider,
                    message: "Microsoft says your organization doesn't have Exchange Online, \
                              which setting this up needs (AADSTS500011)."
                        .into(),
                    reason: Some("noExchange"),
                    aadsts: vec![500011],
                    link: false,
                },
                Refusal {
                    query: "error=invalid_request&error_description=AADSTS90002%3A+Tenant+not+found.",
                    code: ErrorCode::Provider,
                    message: "Microsoft couldn't find your organization in its public cloud. \
                              Organizations in government or other national clouds need to use \
                              the PowerShell script."
                        .into(),
                    reason: Some("otherCloud"),
                    aadsts: vec![90002],
                    link: false,
                },
                Refusal {
                    query: "error=access_denied&error_description=AADSTS50105%3A+Not+assigned.",
                    code: ErrorCode::SignInBlocked,
                    message: format!(
                        "Your organization requires assignment to {app}. An administrator \
                         assigns you under Enterprise apps → {app} → Users and groups."
                    ),
                    reason: Some("notAssigned"),
                    aadsts: vec![50105],
                    link: false,
                },
                Refusal {
                    query: "error=access_denied&error_description=AADSTS65005%3A+Misconfigured.",
                    code: ErrorCode::SignInBlocked,
                    message: format!(
                        "Microsoft 365 in your organization isn't set up for {app} yet. Ask your \
                         IT team to approve it."
                    ),
                    reason: Some("appNotSetUp"),
                    aadsts: vec![65005],
                    link: true,
                },
                Refusal {
                    query: "error=access_denied&error_description=AADSTS53003%3A+Blocked+by+Conditional+Access.",
                    code: ErrorCode::SignInBlocked,
                    message: "Your organization's sign-in rules didn't allow this sign-in, for \
                              example because of the device, the app or where you're signing in \
                              from. Ask your IT team what's required."
                        .into(),
                    reason: Some("conditionalAccess"),
                    aadsts: vec![53003],
                    link: false,
                },
                Refusal {
                    query: "error=access_denied&error_subcode=cancel&error_description=the+user+canceled",
                    code: ErrorCode::SignInCancelled,
                    message: "Sign-in was canceled.".into(),
                    reason: None,
                    aadsts: Vec::new(),
                    link: false,
                },
                Refusal {
                    query: "error=server_error&error_description=AADSTS90033%3A+A+transient+error.%0D%0ATrace+ID%3A+x",
                    code: ErrorCode::Provider,
                    message: "AADSTS90033: A transient error.".into(),
                    reason: None,
                    aadsts: vec![90033],
                    link: false,
                },
            ]
        }

        #[tokio::test]
        async fn each_refusal_says_what_to_do_and_links_only_the_setup_app() {
            let server = MockServer::start().await;
            let approval = setup_admin_consent_url(ID, Some(TENANT));
            for refusal in refusals() {
                let browser = fake_browser(vec![redirect_with(refusal.query)]);
                let log = RunLog::default();
                let error = admin_sign_in(
                    &config(&server),
                    &*browser.open,
                    Duration::from_secs(10),
                    &log,
                )
                .await
                .err()
                .unwrap();
                let case = refusal.query;
                assert_eq!(error.code, refusal.code, "{case}");
                assert_eq!(error.message, refusal.message, "{case}");
                let details = error.details.clone().unwrap_or_default();
                assert_eq!(details["reason"].as_str(), refusal.reason, "{case}");
                if let Some(&code) = refusal.aadsts.first()
                    && refusal.code != ErrorCode::ConsentDeclined
                {
                    assert_eq!(details["aadsts"], json!(code), "{case}");
                }
                if refusal.code == ErrorCode::SignInBlocked {
                    assert_eq!(details["app"], "setup", "{case}");
                }
                // Every link is the setup app's approval link.
                match details.get("adminConsentUrl") {
                    Some(url) => {
                        assert!(refusal.link, "{case}");
                        assert_eq!(url, &json!(approval), "{case}");
                    }
                    None => assert!(!refusal.link, "{case}"),
                }
                assert!(
                    !details
                        .to_string()
                        .contains(crate::service::PUBLISHED_MICROSOFT_CLIENT_ID),
                    "{case}"
                );
                // Logged before the error came back, with its codes.
                let records = sign_ins(&log);
                assert_eq!(records.len(), 1, "{case}");
                assert_eq!(records[0].aadsts, refusal.aadsts, "{case}");
                if refusal.code != ErrorCode::SignInCancelled {
                    assert_eq!(records[0].outcome, "error", "{case}");
                }
            }
            assert_eq!(
                refusals()
                    .iter()
                    .find(|r| r.aadsts == [90095])
                    .map(|r| r.code),
                Some(ErrorCode::AdminConsentRequired)
            );
        }

        #[tokio::test]
        async fn approval_needed_at_redemption_offers_the_request_when_it_can() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path(token_path()))
                .respond_with(token_error(
                    "invalid_grant",
                    "AADSTS90095: Admin consent is required for the permissions requested.",
                    &[90095],
                ))
                .mount(&server)
                .await;
            let browser = fake_browser(vec![redirect_with("code=c")]);
            let log = RunLog::default();
            let error = admin_sign_in(
                &config(&server),
                &*browser.open,
                Duration::from_secs(10),
                &log,
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error.code, ErrorCode::AdminConsentRequired);
            let details = error.details.unwrap();
            assert_eq!(details["canRequestApproval"], true);
            assert_eq!(details["tenant"], TENANT);
            assert_eq!(
                details["adminConsentUrl"],
                json!(setup_admin_consent_url(ID, Some(TENANT)))
            );
            let records = sign_ins(&log);
            assert_eq!(records[0].error.as_deref(), Some("invalid_grant"));
            assert_eq!(records[0].aadsts, [90095]);
            assert_eq!(
                records[0].description.as_deref(),
                Some("AADSTS90095: Admin consent is required for the permissions requested.")
            );
        }

        #[tokio::test]
        async fn a_sign_in_that_runs_out_of_time_is_logged_as_such() {
            let server = MockServer::start().await;
            let browser = fake_browser(Vec::new());
            let log = RunLog::default();
            let error = admin_sign_in(
                &config(&server),
                &*browser.open,
                Duration::from_millis(200),
                &log,
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error.code, ErrorCode::SignInCancelled);
            assert_eq!(sign_ins(&log)[0].outcome, "timed out");
        }
    }
}
