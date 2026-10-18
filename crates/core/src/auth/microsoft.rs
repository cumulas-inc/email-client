//! Microsoft identity platform sign-in (authorization code + PKCE through a
//! loopback redirect) and token refresh.
//!
//! The app is a public client: it has no secret. PKCE (RFC 7636, `S256`)
//! binds the authorization code to this process, and the browser hands the
//! code back to a listener on the loopback interface (RFC 8252). Entra app
//! registrations with a "Mobile and desktop applications" redirect URI of
//! `http://localhost` accept any port.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};
use url::Url;

use super::{ApiTokenError, ApiTokens, GrantRecord, SyncTokenError, SyncTokens, TokenSource};
use crate::error::{Error, ErrorCode, Result};
use crate::html::escape_html;
use crate::model::{AccountFeature, AdminConsentLink, MicrosoftGrant, PermissionRequest};
use crate::secrets::{self, SecretStore, keys};
use crate::signatures::CONSUMER_TENANT;

/// Graph permissions every Microsoft account has: without them it can't be
/// added. Frozen: a new permission is always optional (OPTIONAL_SCOPES).
/// Under Microsoft's default consent policy since late 2025, users can't
/// grant Mail.ReadWrite (nor the optional permissions) to apps outside
/// Microsoft's own list of mail clients, so an administrator often must
/// approve the app (docs/microsoft-365.md).
pub const CORE_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "offline_access",
    "User.Read",
    "Mail.ReadWrite",
    "Mail.Send",
];

/// Optional Graph permissions, one per feature.
pub const OPTIONAL_SCOPES: &[(AccountFeature, &str)] = &[
    (AccountFeature::MailboxSettings, "MailboxSettings.Read"),
    (AccountFeature::Calendar, "Calendars.ReadWrite"),
    (AccountFeature::Contacts, "Contacts.ReadWrite"),
    (AccountFeature::Tasks, "Tasks.ReadWrite"),
];

/// What every account added before grants were recorded consented to (each
/// of its sign-ins and refreshes asked for exactly this). Frozen.
const LEGACY_SCOPES: &[&str] = &[
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

/// Where Microsoft sends an administrator after approving the published registration.
pub const ADMIN_CONSENT_LANDING_URI: &str = "https://flatpak.cumulas.co/admin-consent/";

/// Whether the published registration's approval link also covers the push
/// service's scope. Off only if Microsoft refuses it there (docs, release steps).
pub const ADMIN_LINK_INCLUDES_PUSH: bool = true;

/// Every Graph permission the account was granted, without a prompt: work
/// and school accounts' refreshes ask for it.
const GRAPH_DEFAULT_SCOPE: &str = "https://graph.microsoft.com/.default";

/// The Graph permissions a sign-in can ask for: the core ones, then the
/// optional ones in `OPTIONAL_SCOPES` order (OpenID Connect scopes first).
pub fn all_scopes() -> Vec<&'static str> {
    CORE_SCOPES
        .iter()
        .copied()
        .chain(OPTIONAL_SCOPES.iter().map(|(_, scope)| *scope))
        .collect()
}

/// The Graph permission a feature needs.
pub fn feature_scope(feature: AccountFeature) -> &'static str {
    match feature {
        AccountFeature::Calendar => "Calendars.ReadWrite",
        AccountFeature::Contacts => "Contacts.ReadWrite",
        AccountFeature::Tasks => "Tasks.ReadWrite",
        AccountFeature::MailboxSettings => "MailboxSettings.Read",
    }
}

/// What an account added before grants were recorded has: the legacy
/// scopes without the OpenID Connect ones.
pub fn legacy_graph_scopes() -> Vec<String> {
    LEGACY_SCOPES
        .iter()
        .filter(|s| !is_oidc_scope(s))
        .map(|s| (*s).to_owned())
        .collect()
}

/// Whether a Microsoft account's tenant is the one of personal accounts
/// (Outlook.com, Hotmail): its id or `consumers`.
pub fn is_personal_tenant(tenant_id: &str) -> bool {
    tenant_id.eq_ignore_ascii_case(CONSUMER_TENANT) || tenant_id.eq_ignore_ascii_case("consumers")
}

/// Whether an email domain is one of Microsoft's personal accounts
/// (outlook.com, hotmail.co.uk, live.com.au, ...), whose users have no
/// administrator. Keep in step with `isConsumerDomain` in
/// src/modules/settings/microsoft.ts.
pub fn is_consumer_domain(domain: &str) -> bool {
    let domain = domain.trim().to_ascii_lowercase();
    if matches!(
        domain.as_str(),
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" | "passport.com" | "windowslive.com"
    ) {
        return true;
    }
    // ^(hotmail|live|outlook)\.((co|com)\.)?[a-z]{2}$
    let country = |label: &str| label.len() == 2 && label.bytes().all(|b| b.is_ascii_lowercase());
    match domain.split('.').collect::<Vec<_>>()[..] {
        [brand, cc] => is_consumer_brand(brand) && country(cc),
        [brand, "co" | "com", cc] => is_consumer_brand(brand) && country(cc),
        _ => false,
    }
}

fn is_consumer_brand(label: &str) -> bool {
    matches!(label, "hotmail" | "live" | "outlook")
}

/// Whether `scope` (`Calendars.ReadWrite`) is among `scopes`, ignoring
/// ASCII case.
fn has_scope(scopes: &[String], scope: &str) -> bool {
    scopes.iter().any(|s| s.eq_ignore_ascii_case(scope))
}

/// The optional features whose permission a recorded grant lacks, in enum
/// order; none when nothing was recorded yet (an account from before grants
/// were recorded has every permission).
pub fn needs_permission(grant: Option<&MicrosoftGrant>) -> Vec<AccountFeature> {
    let Some(grant) = grant else {
        return Vec::new();
    };
    AccountFeature::ALL
        .into_iter()
        .filter(|f| !has_scope(&grant.scopes, feature_scope(*f)))
        .collect()
}

/// The features Microsoft 365 refused although their permission is
/// granted, in enum order; never `mailboxSettings`.
pub fn refused_features(grant: Option<&MicrosoftGrant>) -> Vec<AccountFeature> {
    let Some(grant) = grant else {
        return Vec::new();
    };
    let missing = needs_permission(Some(grant));
    AccountFeature::ALL
        .into_iter()
        .filter(|f| {
            *f != AccountFeature::MailboxSettings
                && grant.refused.contains(f)
                && !missing.contains(f)
        })
        .collect()
}

/// The scopes a sign-in asks for (OpenID Connect first, then the core Graph
/// permissions, then optional ones in `OPTIONAL_SCOPES` order). `recorded`
/// is what the account signing in has (`None` when no account matches the
/// login hint). Asking again for what an account has shows no prompt, and
/// keeps the newest authorization covering what its refreshes ask for.
pub fn sign_in_scopes(request: PermissionRequest, recorded: Option<&[String]>) -> Vec<String> {
    let recorded_has = |scope: &str| recorded.is_none_or(|r| has_scope(r, scope));
    let wanted = |feature: AccountFeature, scope: &str| match request {
        PermissionRequest::All => true,
        PermissionRequest::MailOnly => false,
        PermissionRequest::Current => recorded_has(scope),
        PermissionRequest::Feature { feature: asked } => {
            feature == asked || recorded.is_some_and(|r| has_scope(r, scope))
        }
    };
    CORE_SCOPES
        .iter()
        .copied()
        .chain(
            OPTIONAL_SCOPES
                .iter()
                .filter(|(feature, scope)| wanted(*feature, scope))
                .map(|(_, scope)| *scope),
        )
        .map(str::to_owned)
        .collect()
}

/// The Graph permissions a token response's `scope` names: unqualified,
/// spelled as here, without the OpenID Connect scopes, `.default` or other
/// resources' scopes; known ones in [`all_scopes`] order, then unknown ones
/// sorted.
pub fn parse_granted_scopes(scope: &str) -> Vec<String> {
    let known = all_scopes();
    let mut found: Vec<&'static str> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    let graph = format!("{GRAPH_RESOURCE}/");
    for raw in scope.split_whitespace() {
        let name = strip_prefix_ignore_case(raw, &graph).unwrap_or(raw);
        if name.is_empty()
            || is_oidc_scope(name)
            || name.eq_ignore_ascii_case(".default")
            || name.contains("://")
            || strip_prefix_ignore_case(name, "api:").is_some()
        {
            continue;
        }
        match known.iter().find(|k| k.eq_ignore_ascii_case(name)) {
            Some(k) => {
                if !found.contains(k) {
                    found.push(k);
                }
            }
            None => {
                if !unknown.iter().any(|u| u.eq_ignore_ascii_case(name)) {
                    unknown.push(name.to_owned());
                }
            }
        }
    }
    unknown.sort();
    known
        .into_iter()
        .filter(|k| found.contains(k))
        .map(str::to_owned)
        .chain(unknown)
        .collect()
}

/// `text` without `prefix`, compared ignoring ASCII case.
fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

fn is_oidc_scope(scope: &str) -> bool {
    OIDC_SCOPES.iter().any(|s| s.eq_ignore_ascii_case(scope))
}

/// What a Graph token refresh asks for. Work and school accounts ask for
/// `.default`, every permission granted (a later approval included);
/// `explicit` (personal accounts, and a work account whose `.default` was
/// refused) names the core permissions and the optional ones `recorded`,
/// the legacy set without a record, never more than the last authorization
/// covered.
fn graph_refresh_scope(explicit: bool, recorded: Option<&[String]>) -> String {
    if !explicit {
        return format!("{GRAPH_DEFAULT_SCOPE} offline_access");
    }
    let legacy = legacy_graph_scopes();
    let recorded = recorded.unwrap_or(&legacy);
    sign_in_scopes(PermissionRequest::Current, Some(recorded)).join(" ")
}

/// The scope the Cumulas push service (Cumulas Sync) accepts tokens for, on
/// the app's own registration: `api://<client id>/Sync.Receive`.
pub const SYNC_SCOPE: &str = "Sync.Receive";

/// The push service's scope for the app registration `client_id`.
pub fn sync_scope(client_id: &str) -> String {
    format!("api://{client_id}/{SYNC_SCOPE}")
}

/// Microsoft Entra ID's public cloud authority.
pub const DEFAULT_AUTHORITY_HOST: &str = "https://login.microsoftonline.com";

/// Redirect URI of the admin consent link for app registrations other than
/// the published one, which sends administrators to
/// [`ADMIN_CONSENT_LANDING_URI`]. It must be registered on the app (it is
/// the same `http://localhost` entry used for sign-in). Nothing listens
/// there once the administrator has granted consent, so their browser then
/// ends on a "can't connect" page; consent has been recorded by then.
pub const ADMIN_CONSENT_REDIRECT_URI: &str = "http://localhost";

/// Resource prefix for fully qualified Graph scopes.
const GRAPH_RESOURCE: &str = "https://graph.microsoft.com";

/// OpenID Connect scopes; they are not Graph permissions and are never
/// prefixed with the resource.
const OIDC_SCOPES: &[&str] = &["openid", "profile", "email", "offline_access"];

/// Access tokens are refreshed when less than this much lifetime remains.
const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);

/// AADSTS codes meaning the user may not consent and an administrator must:
/// 65001 DelegationDoesNotExist, 90093 (user may not grant the permissions),
/// 90094 AdminConsentRequired, 90095 AdminConsentRequiredRequestAccess (admin
/// consent workflow), 900941 AdminConsentRequiredDueToRiskyApp and 900981
/// AdminConsentRequestRiskyAppWarning. See
/// <https://learn.microsoft.com/entra/identity-platform/reference-error-codes> and
/// <https://learn.microsoft.com/entra/identity/enterprise-apps/application-sign-in-unexpected-user-consent-error>.
const ADMIN_CONSENT_CODES: &[u32] = &[65001, 90093, 90094, 90095, 900941, 900981];

/// AADSTS90095 AdminConsentRequiredRequestAccess: the tenant has the admin
/// consent workflow enabled, so the user can request approval.
const ADMIN_CONSENT_WORKFLOW_CODE: u32 = 90095;

/// AADSTS codes meaning the app registration doesn't offer the scope asked
/// for: 70011 InvalidScope, 500011 InvalidResourceServicePrincipalNotFound
/// (no Application ID URI), 650053 (the scope doesn't exist on the resource)
/// and 650057 InvalidResource (not among the app's permissions).
const SCOPE_NOT_OFFERED_CODES: &[u32] = &[70011, 500011, 650053, 650057];

/// AADSTS65004 UserDeclinedConsent. Also sent when the user picks "Back to
/// app" after submitting an approval request in the admin consent workflow.
const USER_DECLINED_CODE: u32 = 65004;

/// AADSTS codes meaning an organization's API (or the app registration it
/// belongs to) isn't in the account's tenant: 500011
/// InvalidResourceServicePrincipalNotFound, 650052 (the organization hasn't
/// subscribed to or enabled a service the app needs), 650057
/// InvalidResource and 700016 UnauthorizedClient_DoesNotMatchRequest (the
/// application wasn't found in the directory).
const API_NOT_IN_TENANT_CODES: &[u32] = &[500011, 650052, 650057, 700016];

/// Upper bound for the browser's request head on the loopback listener.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// How long a browser connection may stay silent before it is dropped.
const CONNECTION_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Which Entra app registration and authority to sign in with.
#[derive(Debug, Clone)]
pub struct MicrosoftAuthConfig {
    /// Application (client) id of the Entra app registration.
    pub client_id: String,
    /// `common`, `organizations`, `consumers`, a tenant id or a domain.
    pub tenant: String,
    /// Defaults to `https://login.microsoftonline.com`; overridable for tests.
    pub authority_host: String,
    /// Scopes of other resources the sign-in also asks consent for (the
    /// push service's), beyond the Graph scopes it redeems the code for.
    pub consent_scopes: Vec<String>,
    /// What a sign-in asks for and redeems the code for: OpenID Connect
    /// scopes and Graph permissions (see [`sign_in_scopes`]).
    pub graph_scopes: Vec<String>,
    /// Whether this is Cumulas Mail's published app registration, whose
    /// approval link sends administrators to [`ADMIN_CONSENT_LANDING_URI`].
    pub published: bool,
}

impl MicrosoftAuthConfig {
    /// Config for the public cloud; `tenant` defaults to `common` (work,
    /// school and personal accounts).
    pub fn new(client_id: impl Into<String>, tenant: Option<String>) -> Self {
        Self {
            client_id: client_id.into(),
            tenant: tenant.unwrap_or_else(|| "common".into()),
            authority_host: DEFAULT_AUTHORITY_HOST.into(),
            consent_scopes: Vec::new(),
            graph_scopes: all_scopes().into_iter().map(str::to_owned).collect(),
            published: false,
        }
    }

    fn endpoint(&self, name: &str) -> String {
        format!(
            "{}/{}/oauth2/v2.0/{name}",
            self.authority_host.trim_end_matches('/'),
            self.tenant
        )
    }

    /// True for a specific tenant (id or domain) rather than one of the
    /// multi-tenant aliases.
    fn has_specific_tenant(&self) -> bool {
        !matches!(
            self.tenant.to_ascii_lowercase().as_str(),
            "common" | "organizations" | "consumers"
        )
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
    /// The Graph permissions the sign-in asked for (the config's
    /// `graph_scopes` without the OpenID Connect ones).
    pub requested_scopes: Vec<String>,
    /// The Graph permissions the token response said the account has
    /// (see [`parse_granted_scopes`]); empty when it didn't say.
    pub response_scopes: Vec<String>,
    /// Set when consent to the config's `consent_scopes` (the push
    /// service's) was refused, and the sign-in went ahead with Graph's
    /// scopes alone: `AdminConsentRequired`, whose approval link covers
    /// them, or `ConsentDeclined`.
    pub consent_refused: Option<Error>,
}

/// Runs the interactive sign-in: starts a loopback listener on 127.0.0.1
/// (and ::1 when available), calls `open_browser` with the authorization
/// URL, waits for the redirect (up to `timeout`) and redeems the code.
///
/// Consent to `config.consent_scopes` never costs the sign-in: when it is
/// refused, the browser is opened again for Graph's scopes alone.
///
/// Errors: `AdminConsentRequired` (with `details.adminConsentUrl`) when the
/// tenant blocks user consent, `ConsentDeclined`, `SignInBlocked` when the
/// organization's settings keep the person out, `SignInCancelled`.
pub async fn sign_in(
    config: &MicrosoftAuthConfig,
    open_browser: &(dyn Fn(&str) -> Result<()> + Send + Sync),
    login_hint: Option<&str>,
    timeout: Duration,
) -> Result<SignInResult> {
    let (scope, graph) = (authorize_scope(config), config.graph_scopes.join(" "));
    let continues_if_refused = !config.consent_scopes.is_empty();
    let listener = LoopbackListener::bind().await?;
    let first = authorize(
        config,
        &scope,
        &listener,
        continues_if_refused,
        open_browser,
        login_hint,
        timeout,
    )
    .await;
    let (authorization, consent_refused) = match first {
        Err(e) if continues_if_refused && is_consent_refusal(e.code) => {
            // Asked again on a listener of its own, on another port. The
            // browser may keep connections it opened to the first one, and
            // whatever arrives over them is checked against the first
            // authorization's state; on the same port it could send this
            // one's redirect over them. Binding the new listener while the
            // first is still bound guarantees the ports differ.
            let fresh = LoopbackListener::bind().await?;
            drop(listener);
            let authorization = authorize(
                config,
                &graph,
                &fresh,
                false,
                open_browser,
                login_hint,
                timeout,
            )
            .await?;
            (authorization, Some(push_consent_refused(e)))
        }
        first => (first?, None),
    };

    let http = http_client();
    let tokens = request_token(
        &http,
        config,
        &[
            ("client_id", config.client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", authorization.code.as_str()),
            ("redirect_uri", authorization.redirect_uri.as_str()),
            ("code_verifier", authorization.code_verifier.as_str()),
            ("scope", graph.as_str()),
        ],
    )
    .await
    .map_err(|e| match e {
        TokenFailure::OAuth(e) => sign_in_error(config, login_hint, &e),
        TokenFailure::Other(e) => e,
    })?;

    let claims = tokens
        .id_token
        .as_deref()
        .ok_or_else(|| Error::provider("Microsoft did not return an ID token."))
        .and_then(decode_id_token)?;
    let refresh_token = tokens.refresh_token.ok_or_else(|| {
        Error::provider("Microsoft did not return a refresh token (offline_access missing).")
    })?;
    let (Some(tenant_id), Some(user_id)) = (claims.tid, claims.oid) else {
        return Err(Error::provider(
            "Microsoft's response did not identify the account (tid/oid missing).",
        ));
    };
    let email = claims
        .email
        .filter(|e| !e.trim().is_empty())
        .or(claims.preferred_username)
        .filter(|e| !e.trim().is_empty())
        .ok_or_else(|| Error::provider("Microsoft's response did not include an email address."))?;
    let display_name = claims
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| email.clone());
    let requested_scopes = config
        .graph_scopes
        .iter()
        .filter(|s| !is_oidc_scope(s))
        .cloned()
        .collect();
    let response_scopes = tokens
        .scope
        .as_deref()
        .map(parse_granted_scopes)
        .unwrap_or_default();
    Ok(SignInResult {
        email,
        display_name,
        tenant_id,
        user_id,
        access_token: tokens.access_token,
        refresh_token,
        expires_in: Duration::from_secs(tokens.expires_in),
        requested_scopes,
        response_scopes,
        consent_refused,
    })
}

/// An authorization code and what redeeming it takes.
struct Authorization {
    code: String,
    /// The PKCE verifier of the request the code was issued for.
    code_verifier: String,
    /// The redirect URI of that request, which the redemption repeats.
    redirect_uri: String,
}

/// Opens the authorization request for `scope` in the browser and waits for
/// its code on `listener`. `continues_if_refused` tells the browser page
/// that the sign-in goes on if consent is refused.
async fn authorize(
    config: &MicrosoftAuthConfig,
    scope: &str,
    listener: &LoopbackListener,
    continues_if_refused: bool,
    open_browser: &(dyn Fn(&str) -> Result<()> + Send + Sync),
    login_hint: Option<&str>,
    timeout: Duration,
) -> Result<Authorization> {
    let redirect_uri = listener.redirect_uri();
    let pkce = Pkce::generate();
    let state = random_token(16);
    let url = authorize_url(
        config,
        scope,
        &redirect_uri,
        &state,
        &pkce.challenge,
        login_hint,
    )?;
    open_browser(&url)?;

    let response = listener.wait_for_response(&state, continues_if_refused);
    let response = tokio::time::timeout(timeout, response).await.map_err(|_| {
        Error::new(
            ErrorCode::SignInCancelled,
            "Sign-in was not completed in time. Try again.",
        )
        .with_details(json!({ "reason": "timeout" }))
    })?;
    match response.outcome() {
        Ok(code) => Ok(Authorization {
            code: code.to_owned(),
            code_verifier: pkce.verifier,
            redirect_uri,
        }),
        Err(error) => Err(sign_in_error(config, login_hint, &error)),
    }
}

/// URL an administrator can open to grant tenant-wide consent: to every
/// Graph permission (the optional ones too, whatever the sign-in asked
/// for), and to the other resources' scopes: the push service's for the
/// published registration (see [`ADMIN_LINK_INCLUDES_PUSH`]), and those
/// the sign-in asks consent for.
///
/// `tenant_hint` is a tenant id or domain (e.g. the domain of the user's email
/// address); without one the configured tenant is used when it is specific,
/// else `organizations` (the admin then signs in to their own tenant).
pub fn admin_consent_url(config: &MicrosoftAuthConfig, tenant_hint: Option<&str>) -> String {
    let tenant = admin_consent_tenant(config, tenant_hint);
    // OIDC scopes stay unqualified; Microsoft recommends including them so the
    // grant also covers the ID and refresh tokens the app always asks for
    // (learn.microsoft.com/entra/identity-platform/v2-admin-consent and
    // learn.microsoft.com/office/dev/add-ins/publish/publish-nested-app-auth-add-in).
    // The other resources' scopes are fully qualified already.
    let mut scopes: Vec<String> = all_scopes()
        .into_iter()
        .map(|s| {
            if is_oidc_scope(s) {
                s.to_owned()
            } else {
                format!("{GRAPH_RESOURCE}/{s}")
            }
        })
        .collect();
    let push =
        (config.published && ADMIN_LINK_INCLUDES_PUSH).then(|| sync_scope(&config.client_id));
    for other in push.iter().chain(&config.consent_scopes) {
        if !scopes.contains(other) {
            scopes.push(other.clone());
        }
    }
    let redirect_uri = if config.published {
        ADMIN_CONSENT_LANDING_URI
    } else {
        ADMIN_CONSENT_REDIRECT_URI
    };
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &config.client_id)
        .append_pair("scope", &scopes.join(" "))
        .append_pair("redirect_uri", redirect_uri)
        .finish();
    format!(
        "{}/{tenant}/v2.0/adminconsent?{query}",
        config.authority_host.trim_end_matches('/')
    )
}

/// The approval link for `tenant_hint`'s organization (see
/// [`admin_consent_url`]), the tenant it names and, for the published
/// registration, the page for IT administrators.
pub fn admin_consent_link(
    config: &MicrosoftAuthConfig,
    tenant_hint: Option<&str>,
) -> AdminConsentLink {
    let tenant = admin_consent_tenant(config, tenant_hint);
    AdminConsentLink {
        admin_consent_url: admin_consent_url(config, Some(&tenant)),
        tenant,
        admin_page_url: config
            .published
            .then(|| ADMIN_CONSENT_LANDING_URI.to_owned()),
    }
}

/// Tenant for the admin consent link: the hint if it looks like a tenant id
/// or domain, else the configured tenant if specific, else `organizations`
/// (`common` is not accepted by the admin consent endpoint).
fn admin_consent_tenant(config: &MicrosoftAuthConfig, hint: Option<&str>) -> String {
    let plausible = |t: &str| {
        !t.is_empty()
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    };
    match hint.map(str::trim) {
        Some(hint) if plausible(hint) => hint.to_owned(),
        _ if config.has_specific_tenant() && plausible(&config.tenant) => config.tenant.clone(),
        _ => "organizations".into(),
    }
}

/// Token source for a signed-in account: keeps the access token in memory and
/// refreshes it with the refresh token stored in the secret store.
pub struct MicrosoftTokenSource {
    config: MicrosoftAuthConfig,
    account_id: String,
    secrets: Arc<dyn SecretStore>,
    http: reqwest::Client,
    /// What the tokens are for: Graph, or another resource (the push
    /// service, an organization's API).
    scope: RefreshScope,
    /// Held across the refresh so concurrent callers wait for one refresh
    /// instead of each redeeming the refresh token.
    cache: Mutex<Option<CachedToken>>,
    wall_clock: WallClock,
}

/// What a token source's refreshes ask for.
enum RefreshScope {
    /// Always this scope; nothing is recorded.
    Fixed(String),
    /// Graph's permissions: `.default` for work and school accounts, those
    /// recorded for personal ones (see [`graph_refresh_scope`]). What
    /// Microsoft says each refresh covers is recorded in `grant`.
    Graph {
        personal: bool,
        grant: Arc<dyn GrantRecord>,
    },
}

/// Reads the wall clock, which (unlike `Instant`) counts the time the
/// computer spends suspended. Tests move it.
type WallClock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

/// An access token and when it expires by each clock. `Instant` stops while
/// the computer is suspended, so after a resume only the wall clock knows a
/// token has expired; the wall clock can be set back, which `Instant`
/// ignores. A token lasts only as long as both say.
struct CachedToken {
    access_token: String,
    expires_at: Instant,
    expires_wall: SystemTime,
}

impl CachedToken {
    fn new(access_token: &str, expires_in: Duration, wall_now: SystemTime) -> Self {
        let now = Instant::now();
        Self {
            access_token: access_token.to_owned(),
            expires_at: now.checked_add(expires_in).unwrap_or(now),
            expires_wall: wall_now.checked_add(expires_in).unwrap_or(wall_now),
        }
    }

    /// Whether it has more than `margin` left by both clocks.
    fn lasts(&self, margin: Duration, wall_now: SystemTime) -> bool {
        self.expires_at
            .checked_duration_since(Instant::now())
            .is_some_and(|left| left > margin)
            && self
                .expires_wall
                .duration_since(wall_now)
                .is_ok_and(|left| left > margin)
    }
}

impl MicrosoftTokenSource {
    /// Graph token source for `account_id`, whose refresh token is stored
    /// under [`keys::refresh_token`]. A `personal` account's refreshes name
    /// the permissions `grant` recorded; a work or school account's ask for
    /// `.default`. What Microsoft says each refresh covers goes to `grant`.
    pub fn new(
        config: MicrosoftAuthConfig,
        account_id: impl Into<String>,
        secrets: Arc<dyn SecretStore>,
        personal: bool,
        grant: Arc<dyn GrantRecord>,
    ) -> Self {
        Self::build(
            config,
            account_id,
            secrets,
            RefreshScope::Graph { personal, grant },
        )
    }

    fn with_scope(
        config: MicrosoftAuthConfig,
        account_id: impl Into<String>,
        secrets: Arc<dyn SecretStore>,
        scope: String,
    ) -> Self {
        Self::build(config, account_id, secrets, RefreshScope::Fixed(scope))
    }

    fn build(
        config: MicrosoftAuthConfig,
        account_id: impl Into<String>,
        secrets: Arc<dyn SecretStore>,
        scope: RefreshScope,
    ) -> Self {
        Self {
            config,
            account_id: account_id.into(),
            secrets,
            http: http_client(),
            scope,
            cache: Mutex::new(None),
            wall_clock: Arc::new(SystemTime::now),
        }
    }

    /// Seeds the in-memory cache right after sign-in.
    pub async fn prime(&self, access_token: &str, expires_in: Duration) {
        let token = CachedToken::new(access_token, expires_in, (self.wall_clock)());
        *self.cache.lock().await = Some(token);
    }

    /// The cached token if it has more than `margin` left (and `fresh`
    /// isn't asked for), else a new one from the refresh token.
    async fn token(
        &self,
        margin: Duration,
        fresh: bool,
    ) -> std::result::Result<String, TokenFailure> {
        let mut cache = self.cache.lock().await;
        if !fresh
            && let Some(token) = cache.as_ref()
            && token.lasts(margin, (self.wall_clock)())
        {
            return Ok(token.access_token.clone());
        }

        let key = keys::refresh_token(&self.account_id);
        // The refresh token and the grant are read together, under the
        // account's lock, which a sign-in takes to save both: what a
        // personal account's refresh names is what that token was granted.
        let (refresh_token, recorded) = {
            let _sign_in = secrets::lock_account(&self.account_id).await;
            let refresh_token = self
                .secrets
                .get(&key)
                .await
                .map_err(TokenFailure::Other)?
                .ok_or_else(|| {
                    TokenFailure::Other(Error::auth_required(
                        "The Microsoft sign-in for this account is missing. Sign in again.",
                    ))
                })?;
            let recorded = match &self.scope {
                RefreshScope::Fixed(_) => None,
                RefreshScope::Graph { grant, .. } => {
                    grant.granted().map_err(TokenFailure::Other)?
                }
            };
            (refresh_token, recorded)
        };
        let (tokens, granted) = match &self.scope {
            RefreshScope::Fixed(scope) => (self.redeem(&refresh_token, scope).await?, None),
            RefreshScope::Graph { personal, grant } => {
                let (tokens, sent) = self
                    .redeem_graph(&refresh_token, *personal, recorded.as_deref())
                    .await?;
                let granted = granted_by_refresh(tokens.scope.as_deref(), *personal, &sent);
                (tokens, Some((grant, granted)))
            }
        };

        // Microsoft may rotate the refresh token; the new one replaces the old.
        // The old one keeps working until it expires, so failing to store the
        // new one is not fatal. It replaces only the one redeemed: when that's
        // gone meanwhile, the account was removed while this request ran and
        // its sign-in mustn't come back; when another took its place, the
        // person signed in again (or another refresh rotated it), and that
        // one stays. Under the account's lock, which removing it and signing
        // in also take, so neither comes between the comparison and the save.
        // What a Graph refresh says the account was granted is recorded
        // likewise, and only over the grant it read: a sign-in (or another
        // refresh) meanwhile recorded a newer one, which stays.
        let rotated = tokens
            .refresh_token
            .as_deref()
            .filter(|rotated| *rotated != refresh_token);
        if rotated.is_some() || granted.is_some() {
            let _sign_in = secrets::lock_account(&self.account_id).await;
            match self.secrets.get(&key).await {
                Ok(Some(current)) if current == refresh_token => {
                    if let Some(rotated) = rotated
                        && let Err(e) = self.secrets.set(&key, rotated).await
                    {
                        tracing::warn!("could not store the rotated refresh token: {e}");
                    }
                    if let Some((grant, Some(granted))) = &granted {
                        match grant.granted() {
                            Ok(current) if current == recorded => grant.record(granted),
                            Ok(_) => tracing::info!(
                                "not recording what a token refresh brought: account {} has a \
                                 newer grant",
                                self.account_id
                            ),
                            Err(e) => tracing::warn!("could not check the recorded grant: {e}"),
                        }
                    }
                }
                Ok(Some(_)) => tracing::info!(
                    "not keeping what a token refresh brought: account {} has a newer sign-in",
                    self.account_id
                ),
                Ok(None) => tracing::info!(
                    "not keeping what a token refresh brought: account {} was removed",
                    self.account_id
                ),
                Err(e) => tracing::warn!("could not check the stored refresh token: {e}"),
            }
        }
        let token = CachedToken::new(
            &tokens.access_token,
            Duration::from_secs(tokens.expires_in),
            (self.wall_clock)(),
        );
        let access_token = token.access_token.clone();
        *cache = Some(token);
        Ok(access_token)
    }

    /// Redeems the refresh token for `scope`.
    async fn redeem(
        &self,
        refresh_token: &str,
        scope: &str,
    ) -> std::result::Result<TokenResponse, TokenFailure> {
        request_token(
            &self.http,
            &self.config,
            &[
                ("client_id", self.config.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("scope", scope),
            ],
        )
        .await
    }

    /// Redeems the refresh token for Graph; returns the tokens and the
    /// scope asked for. A personal account's refresh names the permissions
    /// `recorded` (the grant read with the token). Should Microsoft refuse
    /// a work account's `.default`, it asks once more naming them too.
    async fn redeem_graph(
        &self,
        refresh_token: &str,
        personal: bool,
        recorded: Option<&[String]>,
    ) -> std::result::Result<(TokenResponse, String), TokenFailure> {
        let explicit = || graph_refresh_scope(true, recorded);
        if personal {
            let scope = explicit();
            return Ok((self.redeem(refresh_token, &scope).await?, scope));
        }
        let scope = graph_refresh_scope(false, None);
        match self.redeem(refresh_token, &scope).await {
            Err(TokenFailure::OAuth(e)) if e.error == "invalid_scope" || e.has_code(70011) => {
                tracing::warn!(
                    "Microsoft refused `.default` for account {}; naming the permissions instead",
                    self.account_id
                );
                let scope = explicit();
                Ok((self.redeem(refresh_token, &scope).await?, scope))
            }
            result => Ok((result?, scope)),
        }
    }
}

/// What a Graph refresh says the account was granted, to record: the
/// response's `scope`, for a personal account only what was asked for
/// (`sent`; Microsoft names nothing beyond the original authorization).
/// `None` when it says nothing usable: no `scope`, or none of the core
/// Graph permissions.
fn granted_by_refresh(scope: Option<&str>, personal: bool, sent: &str) -> Option<Vec<String>> {
    let granted = parse_granted_scopes(scope?);
    let core = |scope: &String| {
        CORE_SCOPES
            .iter()
            .any(|c| !is_oidc_scope(c) && c.eq_ignore_ascii_case(scope))
    };
    if !granted.iter().any(core) {
        return None;
    }
    if !personal {
        return Some(granted);
    }
    let sent = parse_granted_scopes(sent);
    Some(
        granted
            .into_iter()
            .filter(|s| has_scope(&sent, s))
            .collect(),
    )
}

fn graph_token_error(e: TokenFailure) -> Error {
    match e {
        TokenFailure::OAuth(e) => refresh_error(&e),
        TokenFailure::Other(e) => e,
    }
}

#[async_trait]
impl TokenSource for MicrosoftTokenSource {
    async fn access_token(&self) -> Result<String> {
        self.token(REFRESH_MARGIN, false)
            .await
            .map_err(graph_token_error)
    }

    async fn access_token_lasting(&self, at_least: Duration) -> Result<String> {
        self.token(at_least.max(REFRESH_MARGIN), false)
            .await
            .map_err(graph_token_error)
    }

    async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }
}

/// Tokens for the Cumulas push service, from the account's refresh token
/// (shared with Graph) with a cache of their own. A refusal never becomes
/// an account error: the account keeps syncing through Graph, and push
/// falls back to checking the Inbox.
pub struct SyncTokenSource(MicrosoftTokenSource);

impl SyncTokenSource {
    pub fn new(
        config: MicrosoftAuthConfig,
        account_id: impl Into<String>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        let scope = format!("{} offline_access", sync_scope(&config.client_id));
        Self(MicrosoftTokenSource::with_scope(
            config, account_id, secrets, scope,
        ))
    }
}

#[async_trait]
impl SyncTokens for SyncTokenSource {
    async fn sync_token(
        &self,
        min_left: Duration,
        fresh: bool,
    ) -> std::result::Result<String, SyncTokenError> {
        self.0
            .token(min_left.max(REFRESH_MARGIN), fresh)
            .await
            .map_err(|e| match e {
                TokenFailure::OAuth(e) => sync_token_error(&e),
                TokenFailure::Other(e) => SyncTokenError::Failed(e),
            })
    }
}

/// Tokens for an API in the account's own organization (Cumulas
/// Signatures'), from the account's refresh token (shared with Graph, and
/// saved when Microsoft rotates it) with a cache of their own. `config`'s
/// tenant should be the account's own, where the API is looked for. No
/// refusal ever becomes an account error, or asks the person to sign in
/// again: the API simply isn't used meanwhile.
pub struct ApiTokenSource(MicrosoftTokenSource);

impl ApiTokenSource {
    /// `scope` is the API's delegated scope, such as
    /// `api://signature.cumulas.co/<client id>/access_as_user`.
    pub fn new(
        config: MicrosoftAuthConfig,
        account_id: impl Into<String>,
        secrets: Arc<dyn SecretStore>,
        scope: &str,
    ) -> Self {
        Self(MicrosoftTokenSource::with_scope(
            config,
            account_id,
            secrets,
            format!("{scope} offline_access"),
        ))
    }
}

#[async_trait]
impl ApiTokens for ApiTokenSource {
    async fn api_token(&self, fresh: bool) -> std::result::Result<String, ApiTokenError> {
        self.0
            .token(REFRESH_MARGIN, fresh)
            .await
            .map_err(|e| match e {
                TokenFailure::OAuth(e) => api_token_error(&e),
                TokenFailure::Other(e) => ApiTokenError::Failed(e),
            })
    }
}

// ---------------------------------------------------------------------------
// Authorization request
// ---------------------------------------------------------------------------

/// PKCE verifier and its `S256` challenge (RFC 7636).
struct Pkce {
    verifier: String,
    challenge: String,
}

impl Pkce {
    fn generate() -> Self {
        // 32 random bytes give a 43-character verifier, the minimum length.
        Self::from_verifier(random_token(32))
    }

    fn from_verifier(verifier: String) -> Self {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
        }
    }
}

/// `bytes` random bytes, base64url-encoded.
fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// The Graph scopes, then any other resource's the sign-in asks consent for.
/// Microsoft lets the authorization request cover several resources; the
/// code is then redeemed for Graph's alone, as a token is for one resource.
fn authorize_scope(config: &MicrosoftAuthConfig) -> String {
    config
        .graph_scopes
        .iter()
        .chain(&config.consent_scopes)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

fn authorize_url(
    config: &MicrosoftAuthConfig,
    scope: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
    login_hint: Option<&str>,
) -> Result<String> {
    let mut url = Url::parse(&config.endpoint("authorize")).map_err(|e| {
        Error::new(
            ErrorCode::NotConfigured,
            format!("Invalid Microsoft authority: {e}"),
        )
    })?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", &config.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_mode", "query")
            .append_pair("scope", scope)
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("prompt", "select_account");
        if let Some(hint) = login_hint.map(str::trim).filter(|h| !h.is_empty()) {
            query.append_pair("login_hint", hint);
        }
    }
    Ok(url.into())
}

// ---------------------------------------------------------------------------
// Loopback redirect listener
// ---------------------------------------------------------------------------

/// Parameters of the authorization response delivered to the redirect URI.
#[derive(Debug, Default, PartialEq)]
struct AuthorizationResponse {
    code: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    error_subcode: Option<String>,
}

impl AuthorizationResponse {
    /// The code, or the error the redirect carried instead.
    fn outcome(&self) -> std::result::Result<&str, OAuthError> {
        match (&self.code, &self.error) {
            (Some(code), None) => Ok(code),
            (_, error) => Err(OAuthError {
                error: error.clone().unwrap_or_else(|| "invalid_request".into()),
                error_description: self.error_description.clone().unwrap_or_default(),
                error_codes: Vec::new(),
                error_subcode: self.error_subcode.clone(),
            }),
        }
    }
}

/// What a request to the loopback listener turned out to be.
#[derive(Debug, PartialEq)]
enum Incoming {
    /// Not an authorization response (favicon, probes, ...).
    Unrelated,
    /// An authorization response for some other request; ignored.
    StateMismatch,
    Response(AuthorizationResponse),
}

struct LoopbackListener {
    v4: TcpListener,
    v6: Option<TcpListener>,
    port: u16,
}

impl LoopbackListener {
    async fn bind() -> Result<Self> {
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|e| Error::internal(format!("could not start the sign-in listener: {e}")))?;
        let port = v4.local_addr()?.port();
        // Browsers may resolve `localhost` to ::1 first. Listening there too
        // avoids a refused connection (and another program taking the port).
        let v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await.ok();
        Ok(Self { v4, v6, port })
    }

    fn redirect_uri(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    async fn accept(&self) -> std::io::Result<TcpStream> {
        let (stream, _) = match &self.v6 {
            Some(v6) => tokio::select! {
                r = self.v4.accept() => r?,
                r = v6.accept() => r?,
            },
            None => self.v4.accept().await?,
        };
        Ok(stream)
    }

    /// Serves browser connections until one carries the authorization
    /// response for `state`. Connections are handled concurrently because
    /// browsers open speculative connections that never send a request.
    /// Each is checked against `state` for as long as it stays open, so a
    /// listener serves one authorization only.
    async fn wait_for_response(
        &self,
        state: &str,
        continues_if_refused: bool,
    ) -> AuthorizationResponse {
        let (tx, mut rx) = mpsc::channel(1);
        loop {
            tokio::select! {
                accepted = self.accept() => match accepted {
                    Ok(stream) => {
                        let tx = tx.clone();
                        let state = state.to_owned();
                        tokio::spawn(async move {
                            let response =
                                handle_connection(stream, &state, continues_if_refused).await;
                            if let Some(response) = response {
                                let _ = tx.send(response).await;
                            }
                        });
                    }
                    Err(e) => {
                        // E.g. out of file descriptors; avoid spinning.
                        tracing::debug!("sign-in listener: accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
                Some(response) = rx.recv() => return response,
            }
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    state: &str,
    continues_if_refused: bool,
) -> Option<AuthorizationResponse> {
    let target = tokio::time::timeout(CONNECTION_READ_TIMEOUT, read_request_target(&mut stream))
        .await
        .ok()
        .flatten();
    let Some(target) = target else {
        let _ = respond(&mut stream, "400 Bad Request", "").await;
        return None;
    };
    match classify_request(&target, state) {
        Incoming::Unrelated => {
            let _ = respond(&mut stream, "404 Not Found", "").await;
            None
        }
        Incoming::StateMismatch => {
            let page = result_page(
                "Sign-in failed",
                "This response does not belong to the sign-in in progress. \
                 Return to Cumulas Mail and try again.",
            );
            let _ = respond(&mut stream, "400 Bad Request", &page).await;
            None
        }
        Incoming::Response(response) => {
            let page = response_page(&response, continues_if_refused);
            let _ = respond(&mut stream, "200 OK", &page).await;
            Some(response)
        }
    }
}

/// The page for an authorization response. When the sign-in goes on after
/// a refused consent, it says so: the browser is about to open again.
fn response_page(response: &AuthorizationResponse, continues_if_refused: bool) -> String {
    let error = match response.outcome() {
        Ok(_) => {
            return result_page(
                "You're signed in",
                "You can close this tab and return to Cumulas Mail.",
            );
        }
        Err(error) => error,
    };
    let code = sign_in_error_code(&error);
    if continues_if_refused && is_consent_refusal(code) {
        let why = if code == ErrorCode::AdminConsentRequired {
            "Your organization requires an administrator to approve instant updates."
        } else {
            "Instant updates weren't allowed."
        };
        return result_page(
            "Signing in without instant updates",
            &format!(
                "{why} Cumulas Mail is opening the sign-in again without them. Continue in \
                 the new tab."
            ),
        );
    }
    result_page(
        "Sign-in did not complete",
        &format!(
            "{} You can close this tab and return to Cumulas Mail.",
            error.summary()
        ),
    )
}

/// Reads the request head and returns the target of a `GET` request.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < MAX_REQUEST_BYTES {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&head);
    let mut request_line = head.lines().next()?.split_whitespace();
    let (method, target) = (request_line.next()?, request_line.next()?);
    (method == "GET").then(|| target.to_owned())
}

fn classify_request(target: &str, expected_state: &str) -> Incoming {
    if !target.starts_with('/') {
        return Incoming::Unrelated;
    }
    let Ok(url) = Url::parse(&format!("http://localhost{target}")) else {
        return Incoming::Unrelated;
    };
    if url.path() != "/" {
        return Incoming::Unrelated;
    }
    let mut params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    if !params.contains_key("code") && !params.contains_key("error") {
        return Incoming::Unrelated;
    }
    // The state ties the response to this sign-in (CSRF protection); responses
    // for anything else are ignored and we keep waiting.
    if params.get("state").map(String::as_str) != Some(expected_state) {
        return Incoming::StateMismatch;
    }
    Incoming::Response(AuthorizationResponse {
        code: params.remove("code"),
        error: params.remove("error"),
        error_description: params.remove("error_description"),
        error_subcode: params.remove("error_subcode"),
    })
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

/// Styles for the result page: the app's warm stone canvas, a white card and
/// the teal mark, with the app's dark palette when the browser is dark.
const RESULT_PAGE_STYLE: &str = r#"<style>
:root{color-scheme:light dark;--canvas:#efede8;--card:#fff;--ink:#1d1c1a;--muted:#57534c;
--front-top:#149189;--front-bottom:#0a6c67;--back:#93d9cf;--selection:#c9e5e0;
--shadow:0 0 0 1px rgba(60,50,35,.07),0 1px 2px rgba(60,50,35,.05),0 8px 24px -12px rgba(60,50,35,.14)}
@media (prefers-color-scheme:dark){:root{--canvas:#181715;--card:#22211e;--ink:#edebe6;--muted:#bbb6ac;
--front-top:#33b3a8;--front-bottom:#178a82;--back:#2f5f5a;--selection:#23524c;
--shadow:0 0 0 1px rgba(255,255,255,.055),0 1px 2px rgba(0,0,0,.3),0 10px 28px -12px rgba(0,0,0,.55)}}
*{box-sizing:border-box}
body{margin:0;min-height:100vh;display:grid;place-items:center;padding:24px 16px;
background:var(--canvas);color:var(--ink);-webkit-font-smoothing:antialiased;
font:15px/1.5 system-ui,-apple-system,"Noto Sans",Cantarell,Ubuntu,sans-serif}
main{width:100%;max-width:28rem;padding:32px 32px 30px;border-radius:12px;background:var(--card);
box-shadow:var(--shadow);text-align:center;animation:enter .2s cubic-bezier(.23,1,.32,1)}
svg{display:block;width:44px;height:auto;margin:0 auto 18px}
.back{fill:var(--back)}.front-top{stop-color:var(--front-top)}.front-bottom{stop-color:var(--front-bottom)}
h1{margin:0 0 6px;font-size:20px;line-height:1.3;font-weight:600;letter-spacing:-.01em;text-wrap:balance}
p{margin:0;color:var(--muted);overflow-wrap:anywhere;text-wrap:balance}
::selection{background:var(--selection)}
@keyframes enter{from{opacity:0;transform:scale(.97)}}
@media (prefers-reduced-motion:reduce){main{animation:none}}
</style>"#;

/// The Cumulas mark (src/app/LogoMark.tsx), colored by the page style.
const RESULT_PAGE_MARK: &str = r#"<svg viewBox="3.3 3.4 22.8 19.4" aria-hidden="true">
<defs><linearGradient id="front" gradientUnits="userSpaceOnUse" x1="0" y1="6.6" x2="0" y2="22.2">
<stop offset="0" class="front-top"/><stop offset="1" class="front-bottom"/></linearGradient></defs>
<g class="back"><circle cx="17.6" cy="8.6" r="4.4"/><circle cx="22.2" cy="11.6" r="3.3"/>
<rect x="13.2" y="9.5" width="12.3" height="5.6" rx="2.8"/></g>
<g fill="url(#front)"><circle cx="8.4" cy="15.6" r="4.4"/><circle cx="13.8" cy="12.6" r="6"/>
<circle cx="19.6" cy="16.4" r="3.9"/><rect x="4" y="15" width="19.5" height="7.2" rx="3.6"/></g>
</svg>"#;

/// Small self-contained page shown in the browser after the redirect.
fn result_page(title: &str, message: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title} · Cumulas Mail</title>{RESULT_PAGE_STYLE}</head>\
         <body><main>{RESULT_PAGE_MARK}<h1>{title}</h1><p>{message}</p></main></body></html>",
        title = escape_html(title),
        message = escape_html(message),
    )
}

// ---------------------------------------------------------------------------
// Token endpoint
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: u64,
    /// The scopes the access token is for, as Microsoft spells them.
    #[serde(default)]
    scope: Option<String>,
}

/// OAuth error from the authorization redirect or the token endpoint.
#[derive(Debug, Default, Deserialize)]
struct OAuthError {
    error: String,
    #[serde(default)]
    error_description: String,
    /// AADSTS codes (token endpoint only; parsed from the description for
    /// redirects).
    #[serde(default)]
    error_codes: Vec<u32>,
    #[serde(default)]
    error_subcode: Option<String>,
}

impl OAuthError {
    fn has_code(&self, code: u32) -> bool {
        self.error_codes.contains(&code) || aadsts_codes(&self.error_description).contains(&code)
    }

    /// The first line of Microsoft's description (the rest is trace and
    /// correlation ids), or the bare error code.
    fn summary(&self) -> String {
        match first_line(&self.error_description) {
            "" => self.error.clone(),
            line => line.to_owned(),
        }
    }
}

enum TokenFailure {
    OAuth(OAuthError),
    Other(Error),
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

async fn request_token(
    http: &reqwest::Client,
    config: &MicrosoftAuthConfig,
    form: &[(&str, &str)],
) -> std::result::Result<TokenResponse, TokenFailure> {
    let response = http
        .post(config.endpoint("token"))
        .form(form)
        .send()
        .await
        .map_err(|e| {
            TokenFailure::Other(Error::network(format!(
                "Could not reach Microsoft sign-in: {}",
                e.without_url()
            )))
        })?;
    let status = response.status();
    let body = response.bytes().await.map_err(|e| {
        TokenFailure::Other(Error::network(format!(
            "Could not read Microsoft's sign-in response: {}",
            e.without_url()
        )))
    })?;
    if status.is_success() {
        serde_json::from_slice(&body).map_err(|e| {
            TokenFailure::Other(Error::provider(format!(
                "Unexpected response from Microsoft sign-in: {e}"
            )))
        })
    } else {
        match serde_json::from_slice::<OAuthError>(&body) {
            Ok(error) => Err(TokenFailure::OAuth(error)),
            Err(_) => Err(TokenFailure::Other(Error::provider(format!(
                "Microsoft sign-in failed (HTTP {status})."
            )))),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why an organization's settings keep someone from signing in, in a way
/// approving the app doesn't fix (except `AppNotSetUp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockedReason {
    /// AADSTS50105: the app requires assignment and the user isn't assigned.
    NotAssigned,
    /// AADSTS7000112: the app is disabled in the organization.
    AppDisabled,
    /// AADSTS65005: the app (or a permission it asks for) isn't set up in
    /// the organization; approving it does.
    AppNotSetUp,
    /// AADSTS650052: the organization hasn't subscribed to or enabled a
    /// service the app needs (usually Exchange Online).
    ServiceNotEnabled,
    /// AADSTS53004 and 53010: the user must first register security info
    /// (multifactor authentication).
    SecurityInfo,
    /// AADSTS53000 to 53003, 53011 and 530xxx: Conditional Access.
    ConditionalAccess,
}

impl BlockedReason {
    /// Which of the reasons an AADSTS code is, if any.
    fn of(code: u32) -> Option<Self> {
        Some(match code {
            50105 => Self::NotAssigned,
            7000112 => Self::AppDisabled,
            65005 => Self::AppNotSetUp,
            650052 => Self::ServiceNotEnabled,
            53004 | 53010 => Self::SecurityInfo,
            53000..=53003 | 53011 | 530000..=530999 => Self::ConditionalAccess,
            _ => return None,
        })
    }

    /// The first code of `error` that is a reason, and the reason: from its
    /// `error_codes`, then from the codes its description names.
    fn find(error: &OAuthError) -> Option<(Self, u32)> {
        error
            .error_codes
            .iter()
            .copied()
            .chain(aadsts_codes(&error.error_description))
            .find_map(|code| Self::of(code).map(|reason| (reason, code)))
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::NotAssigned => "notAssigned",
            Self::AppDisabled => "appDisabled",
            Self::AppNotSetUp => "appNotSetUp",
            Self::ServiceNotEnabled => "serviceNotEnabled",
            Self::SecurityInfo => "securityInfo",
            Self::ConditionalAccess => "conditionalAccess",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::NotAssigned => {
                "Your organization hasn't given you access to Cumulas Mail. Ask your IT team to \
                 assign you to the app."
            }
            Self::AppDisabled => {
                "Your organization has blocked Cumulas Mail. Ask your IT team whether they can \
                 allow it."
            }
            Self::AppNotSetUp => {
                "Microsoft 365 in your organization isn't set up for Cumulas Mail yet. Ask your \
                 IT team to approve it."
            }
            Self::ServiceNotEnabled => {
                "Your organization's Microsoft 365 doesn't include a service Cumulas Mail needs, \
                 usually Exchange Online mail. Ask your IT team to check your subscription."
            }
            Self::SecurityInfo => {
                "Your organization needs you to finish setting up your sign-in security info, \
                 such as multifactor authentication. Ask your IT team for help."
            }
            Self::ConditionalAccess => {
                "Your organization's sign-in rules didn't allow this sign-in, for example \
                 because of the device, the app or where you're signing in from. Ask your IT \
                 team what's required."
            }
        }
    }
}

/// What an error from the authorization redirect or the code redemption
/// means: `SignInBlocked`, `AdminConsentRequired`, `ConsentDeclined`,
/// `SignInCancelled` or `Provider`.
fn sign_in_error_code(error: &OAuthError) -> ErrorCode {
    if BlockedReason::find(error).is_some() {
        return ErrorCode::SignInBlocked;
    }
    if ADMIN_CONSENT_CODES.iter().any(|c| error.has_code(*c))
        || (error.error == "consent_required" && !error.has_code(USER_DECLINED_CODE))
    {
        return ErrorCode::AdminConsentRequired;
    }
    if error.has_code(USER_DECLINED_CODE) {
        return ErrorCode::ConsentDeclined;
    }
    if error.error == "access_denied" {
        // Microsoft's own example of a canceled sign-in carries no subcode
        // but says so in the description.
        let canceled = error
            .error_subcode
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("cancel"))
            || error
                .error_description
                .to_ascii_lowercase()
                .contains("cancel");
        return if canceled {
            ErrorCode::SignInCancelled
        } else {
            ErrorCode::ConsentDeclined
        };
    }
    ErrorCode::Provider
}

/// Whether consent was refused, rather than the sign-in canceled or failed.
/// A sign-in that also asked consent for other resources' scopes goes on
/// with Graph's alone then.
fn is_consent_refusal(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::AdminConsentRequired | ErrorCode::ConsentDeclined
    )
}

/// Maps an error from the authorization redirect or the code redemption.
fn sign_in_error(
    config: &MicrosoftAuthConfig,
    login_hint: Option<&str>,
    error: &OAuthError,
) -> Error {
    let hint = login_hint.and_then(|h| h.rsplit_once('@')).map(|(_, d)| d);
    match sign_in_error_code(error) {
        ErrorCode::SignInBlocked => {
            let (reason, code) = BlockedReason::find(error).expect("classified as blocked");
            let link = admin_consent_link(config, hint);
            let mut details = json!({
                "reason": reason.as_str(),
                "aadsts": code,
                "tenant": link.tenant,
                "adminPageUrl": link.admin_page_url,
            });
            if reason == BlockedReason::AppNotSetUp {
                details["adminConsentUrl"] = json!(link.admin_consent_url);
            }
            Error::new(ErrorCode::SignInBlocked, reason.message()).with_details(details)
        }
        ErrorCode::AdminConsentRequired => {
            let link = admin_consent_link(config, hint);
            // With the admin consent workflow enabled, Microsoft's page lets
            // the user send an approval request themselves.
            let can_request_approval = error.has_code(ADMIN_CONSENT_WORKFLOW_CODE);
            let message = if can_request_approval {
                "Your organization requires an administrator to approve Cumulas Mail before \
                 you can sign in. You can request approval on Microsoft's sign-in page, or \
                 send the approval link to your IT department."
            } else {
                "Your organization requires an administrator to approve Cumulas Mail before \
                 you can sign in. Send the approval link to your IT department."
            };
            Error::new(ErrorCode::AdminConsentRequired, message).with_details(json!({
                "adminConsentUrl": link.admin_consent_url,
                "tenant": link.tenant,
                "canRequestApproval": can_request_approval,
                "adminPageUrl": link.admin_page_url,
            }))
        }
        ErrorCode::ConsentDeclined => consent_declined(),
        ErrorCode::SignInCancelled => {
            Error::new(ErrorCode::SignInCancelled, "Sign-in was canceled.")
        }
        _ => Error::provider(error.summary()).with_details(json!({ "error": error.error })),
    }
}

fn consent_declined() -> Error {
    Error::new(
        ErrorCode::ConsentDeclined,
        "Cumulas Mail needs the requested permissions to show your mail, calendar, contacts \
         and tasks. If you asked your administrator for approval, sign in again once it has \
         been granted.",
    )
}

/// Why the push service's scope wasn't granted, for its status: the account
/// itself signed in.
fn push_consent_refused(error: Error) -> Error {
    let message = if error.code == ErrorCode::AdminConsentRequired {
        "Your organization requires an administrator to approve instant updates. Send the \
         approval link to your IT department; until then, the Inbox is checked every 30 seconds."
    } else {
        "Instant updates weren't allowed, so the Inbox is checked every 30 seconds. If you \
         asked your administrator for approval, they turn on once it has been granted."
    };
    Error {
        message: message.into(),
        ..error
    }
}

/// Maps a failed refresh. `invalid_grant` covers expired, revoked or
/// password-invalidated refresh tokens and withdrawn consent;
/// `interaction_required` new MFA or Conditional Access requirements. Either
/// way only an interactive sign-in helps. When the organization's settings
/// keep the person out, the error says why (Microsoft often shows that on
/// its own page during a sign-in, which then never comes back), with
/// `details { reason, aadsts }`; it stays `AuthRequired` for everything else
/// that treats it as a sign-in that's gone.
fn refresh_error(error: &OAuthError) -> Error {
    if let Some((reason, code)) = BlockedReason::find(error) {
        return Error::auth_required(format!("{} (AADSTS{code})", reason.message()))
            .with_details(json!({ "reason": reason.as_str(), "aadsts": code }));
    }
    match error.error.as_str() {
        "invalid_grant" | "interaction_required" | "consent_required" | "login_required" => {
            Error::auth_required(format!(
                "Your Microsoft sign-in has expired or was revoked. Sign in again. ({})",
                error.summary()
            ))
        }
        _ => Error::provider(error.summary()).with_details(json!({ "error": error.error })),
    }
}

/// Maps a refused push-service token. The scope is the app's own, so
/// Microsoft refusing it says nothing about the Graph sign-in: a missing
/// consent (or a sign-in that needs the user) waits for the next sign-in,
/// and a scope the registration doesn't offer for it to be set up.
fn sync_token_error(error: &OAuthError) -> SyncTokenError {
    if SCOPE_NOT_OFFERED_CODES.iter().any(|c| error.has_code(*c))
        || matches!(
            error.error.as_str(),
            "invalid_scope" | "invalid_resource" | "unauthorized_client"
        )
    {
        return SyncTokenError::NotOffered;
    }
    if ADMIN_CONSENT_CODES.iter().any(|c| error.has_code(*c))
        || error.has_code(USER_DECLINED_CODE)
        || matches!(
            error.error.as_str(),
            "invalid_grant" | "consent_required" | "interaction_required" | "login_required"
        )
    {
        return SyncTokenError::ConsentMissing;
    }
    SyncTokenError::Failed(
        Error::provider(error.summary()).with_details(json!({ "error": error.error })),
    )
}

/// Maps a refused token for an organization's API. Unlike a refused Graph
/// token, nothing here means "sign in again": `invalid_grant` with
/// AADSTS65001 is consent the organization hasn't given, and an expired
/// sign-in or new MFA requirement is the Graph sync's to report. Everything
/// but the API missing from the tenant, or consent missing, is tried again
/// later.
fn api_token_error(error: &OAuthError) -> ApiTokenError {
    if API_NOT_IN_TENANT_CODES.iter().any(|c| error.has_code(*c)) {
        return ApiTokenError::NotInTenant;
    }
    // 65001 and the other codes asking for an administrator's consent, and
    // 65004 (declined).
    if ADMIN_CONSENT_CODES.iter().any(|c| error.has_code(*c))
        || error.has_code(USER_DECLINED_CODE)
        || error.error == "consent_required"
    {
        return ApiTokenError::NotAllowed;
    }
    // The error and its codes only: Microsoft's description can name the
    // person, and this goes to the log.
    let mut codes = error.error_codes.clone();
    codes.extend(aadsts_codes(&error.error_description));
    codes.sort_unstable();
    codes.dedup();
    let codes: String = codes.iter().map(|c| format!(", AADSTS{c}")).collect();
    ApiTokenError::Failed(
        Error::provider(format!(
            "Microsoft didn't issue a token for the organization's API ({}{codes}).",
            error.error
        ))
        .with_details(json!({ "error": error.error })),
    )
}

/// All `AADSTSnnnn` codes mentioned in a description.
fn aadsts_codes(description: &str) -> Vec<u32> {
    description
        .match_indices("AADSTS")
        .filter_map(|(at, prefix)| {
            let digits: String = description[at + prefix.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().ok()
        })
        .collect()
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default().trim()
}

// ---------------------------------------------------------------------------
// ID token
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct IdTokenClaims {
    tid: Option<String>,
    oid: Option<String>,
    name: Option<String>,
    email: Option<String>,
    preferred_username: Option<String>,
}

/// Reads the claims of an ID token without verifying its signature. That is
/// sound here because the token comes straight from the token endpoint over
/// TLS in answer to our own request (OpenID Connect Core 1.0 §3.1.3.7, step 6).
fn decode_id_token(token: &str) -> Result<IdTokenClaims> {
    let invalid = || Error::provider("Microsoft returned an ID token that could not be read.");
    let payload = token.split('.').nth(1).ok_or_else(invalid)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| invalid())?;
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::auth::NoGrantRecord;
    use crate::secrets::MemorySecretStore;
    use crate::service::PUBLISHED_MICROSOFT_CLIENT_ID;

    const CLIENT_ID: &str = "test-client-id";

    fn config(server: &MockServer) -> MicrosoftAuthConfig {
        MicrosoftAuthConfig {
            client_id: CLIENT_ID.into(),
            tenant: "common".into(),
            authority_host: server.uri(),
            consent_scopes: Vec::new(),
            graph_scopes: strings(&all_scopes()),
            published: false,
        }
    }

    fn strings(scopes: &[&str]) -> Vec<String> {
        scopes.iter().map(|s| (*s).to_owned()).collect()
    }

    fn id_token(claims: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("{header}.{payload}.c2lnbmF0dXJl")
    }

    fn token_success() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "token_type": "Bearer",
            "scope": "Mail.ReadWrite User.Read",
            "expires_in": 3599,
            "ext_expires_in": 3599,
            "access_token": "access-1",
            "refresh_token": "refresh-1",
            "id_token": id_token(json!({
                "tid": "11111111-2222-3333-4444-555555555555",
                "oid": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                "name": "Adele Vance",
                "preferred_username": "AdeleV@contoso.com",
            })),
        }))
    }

    fn token_error(error: &str, description: &str, codes: &[u32]) -> ResponseTemplate {
        ResponseTemplate::new(400).set_body_json(json!({
            "error": error,
            "error_description": description,
            "error_codes": codes,
            "timestamp": "2026-10-14 10:00:00Z",
            "trace_id": "trace",
            "correlation_id": "correlation",
        }))
    }

    /// Builds the URL the browser visits from the redirect URI and the state.
    type Visit = Box<dyn Fn(&str, &str) -> String + Send + Sync>;

    type Opener = Box<dyn Fn(&str) -> Result<()> + Send + Sync>;

    struct Browser {
        open: Opener,
        authorize_url: Arc<StdMutex<Option<Url>>>,
        statuses: Arc<StdMutex<Vec<u16>>>,
    }

    impl Browser {
        /// Waits until the browser has made `n` requests.
        async fn wait_for(&self, n: usize) -> Vec<u16> {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let statuses = self.statuses.lock().unwrap().clone();
                    if statuses.len() >= n {
                        return statuses;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap()
        }
    }

    /// Plays the browser: records the authorization URL and then visits each
    /// URL in turn on the loopback listener, recording the HTTP statuses.
    fn fake_browser(visits: Vec<Visit>) -> Browser {
        let authorize_url = Arc::new(StdMutex::new(None));
        let statuses = Arc::new(StdMutex::new(Vec::new()));
        let visits = Arc::new(visits);
        let (seen, recorded) = (authorize_url.clone(), statuses.clone());
        let open = Box::new(move |url: &str| -> Result<()> {
            let url = Url::parse(url).expect("authorize URL parses");
            let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
            let (redirect, state) = (params["redirect_uri"].clone(), params["state"].clone());
            *seen.lock().unwrap() = Some(url);
            let (visits, recorded) = (visits.clone(), recorded.clone());
            tokio::spawn(async move {
                let client = reqwest::Client::builder().no_proxy().build().unwrap();
                for visit in visits.iter() {
                    let response = client.get(visit(&redirect, &state)).send().await.unwrap();
                    recorded.lock().unwrap().push(response.status().as_u16());
                }
            });
            Ok(())
        });
        Browser {
            open,
            authorize_url,
            statuses,
        }
    }

    fn redirect_with(query: &'static str) -> Visit {
        Box::new(move |redirect, state| format!("{redirect}/?{query}&state={state}"))
    }

    async fn run_sign_in(
        server: &MockServer,
        browser: &Browser,
        login_hint: Option<&str>,
    ) -> Result<SignInResult> {
        sign_in(
            &config(server),
            &*browser.open,
            login_hint,
            Duration::from_secs(10),
        )
        .await
    }

    #[test]
    fn pkce_matches_rfc7636_test_vector() {
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let generated = Pkce::generate();
        assert_eq!(generated.verifier.len(), 43);
        assert_ne!(generated.verifier, Pkce::generate().verifier);
    }

    #[test]
    fn authorize_url_carries_all_parameters() {
        let config = MicrosoftAuthConfig::new(CLIENT_ID, None);
        let url = authorize_url(
            &config,
            &authorize_scope(&config),
            "http://localhost:4242",
            "st",
            "ch",
            Some("adele@contoso.com"),
        )
        .unwrap();
        let url = Url::parse(&url).unwrap();
        assert_eq!(url.host_str(), Some("login.microsoftonline.com"));
        assert_eq!(url.path(), "/common/oauth2/v2.0/authorize");
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(params["client_id"], CLIENT_ID);
        assert_eq!(params["response_type"], "code");
        assert_eq!(params["redirect_uri"], "http://localhost:4242");
        assert_eq!(params["response_mode"], "query");
        assert_eq!(params["scope"], all_scopes().join(" "));
        assert_eq!(params["state"], "st");
        assert_eq!(params["code_challenge"], "ch");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["prompt"], "select_account");
        assert_eq!(params["login_hint"], "adele@contoso.com");

        let without_hint =
            authorize_url(&config, "openid", "http://localhost:1", "s", "c", None).unwrap();
        assert!(!without_hint.contains("login_hint"));
    }

    #[test]
    fn admin_consent_url_uses_qualified_graph_scopes() {
        let config = MicrosoftAuthConfig::new(CLIENT_ID, None);
        let url = Url::parse(&admin_consent_url(&config, Some("contoso.com"))).unwrap();
        assert_eq!(url.host_str(), Some("login.microsoftonline.com"));
        assert_eq!(url.path(), "/contoso.com/v2.0/adminconsent");
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(params["client_id"], CLIENT_ID);
        assert_eq!(params["redirect_uri"], ADMIN_CONSENT_REDIRECT_URI);
        let scopes: Vec<&str> = params["scope"].split(' ').collect();
        assert!(scopes.contains(&"openid"));
        assert!(scopes.contains(&"offline_access"));
        assert!(scopes.contains(&"https://graph.microsoft.com/Mail.ReadWrite"));
        assert!(scopes.contains(&"https://graph.microsoft.com/Tasks.ReadWrite"));
        assert!(!scopes.contains(&"https://graph.microsoft.com/openid"));
        assert_eq!(scopes.len(), all_scopes().len());

        // Without a usable hint: the configured tenant if specific, else
        // `organizations` (`common` is not accepted by the endpoint).
        assert!(admin_consent_url(&config, None).contains("/organizations/v2.0/adminconsent?"));
        assert!(admin_consent_url(&config, Some("bad/tenant")).contains("/organizations/"));
        let specific = MicrosoftAuthConfig::new(CLIENT_ID, Some("fabrikam.onmicrosoft.com".into()));
        assert!(admin_consent_url(&specific, None).contains("/fabrikam.onmicrosoft.com/v2.0/"));

        // A sign-in that asks consent for the push service too gets a link
        // that covers it.
        let push = MicrosoftAuthConfig {
            consent_scopes: vec![sync_scope(CLIENT_ID)],
            ..config
        };
        let url = Url::parse(&admin_consent_url(&push, Some("contoso.com"))).unwrap();
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let scopes: Vec<&str> = params["scope"].split(' ').collect();
        assert_eq!(scopes.len(), all_scopes().len() + 1);
        assert_eq!(scopes.last(), Some(&"api://test-client-id/Sync.Receive"));

        // Every permission, whatever the sign-in asked for.
        let mail_only = MicrosoftAuthConfig {
            graph_scopes: sign_in_scopes(PermissionRequest::MailOnly, None),
            ..push
        };
        let url = Url::parse(&admin_consent_url(&mail_only, None)).unwrap();
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert!(params["scope"].contains("https://graph.microsoft.com/Calendars.ReadWrite"));
    }

    /// The link the landing page publishes (`packaging/flatpak/repo`), with
    /// the push service's scope and without.
    const PUBLISHED_LINK_WITH_PUSH: &str = "https://login.microsoftonline.com/organizations/v2.0/adminconsent?client_id=10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad&scope=openid+profile+email+offline_access+https%3A%2F%2Fgraph.microsoft.com%2FUser.Read+https%3A%2F%2Fgraph.microsoft.com%2FMail.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FMail.Send+https%3A%2F%2Fgraph.microsoft.com%2FMailboxSettings.Read+https%3A%2F%2Fgraph.microsoft.com%2FCalendars.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FContacts.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FTasks.ReadWrite+api%3A%2F%2F10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad%2FSync.Receive&redirect_uri=https%3A%2F%2Fflatpak.cumulas.co%2Fadmin-consent%2F";
    const PUBLISHED_LINK_WITHOUT_PUSH: &str = "https://login.microsoftonline.com/organizations/v2.0/adminconsent?client_id=10abb2b9-1a7a-4dbc-ba0f-c9a0de9088ad&scope=openid+profile+email+offline_access+https%3A%2F%2Fgraph.microsoft.com%2FUser.Read+https%3A%2F%2Fgraph.microsoft.com%2FMail.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FMail.Send+https%3A%2F%2Fgraph.microsoft.com%2FMailboxSettings.Read+https%3A%2F%2Fgraph.microsoft.com%2FCalendars.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FContacts.ReadWrite+https%3A%2F%2Fgraph.microsoft.com%2FTasks.ReadWrite&redirect_uri=https%3A%2F%2Fflatpak.cumulas.co%2Fadmin-consent%2F";

    fn published() -> MicrosoftAuthConfig {
        MicrosoftAuthConfig {
            published: true,
            ..MicrosoftAuthConfig::new(PUBLISHED_MICROSOFT_CLIENT_ID, None)
        }
    }

    #[test]
    fn admin_consent_url_for_the_published_registration() {
        let expected = if ADMIN_LINK_INCLUDES_PUSH {
            PUBLISHED_LINK_WITH_PUSH
        } else {
            PUBLISHED_LINK_WITHOUT_PUSH
        };
        assert_eq!(admin_consent_url(&published(), None), expected);
        let link = admin_consent_link(&published(), None);
        assert_eq!(link.admin_consent_url, expected);
        assert_eq!(link.tenant, "organizations");
        assert_eq!(
            link.admin_page_url.as_deref(),
            Some(ADMIN_CONSENT_LANDING_URI)
        );
        // A sign-in that asks consent for the push service names it once.
        let push = MicrosoftAuthConfig {
            consent_scopes: vec![sync_scope(PUBLISHED_MICROSOFT_CLIENT_ID)],
            ..published()
        };
        let pushes = admin_consent_url(&push, None)
            .matches("Sync.Receive")
            .count();
        assert_eq!(pushes, 1);

        // Another registration: back to localhost, and the push service's
        // scope only when the sign-in asked for it.
        let own = MicrosoftAuthConfig::new(PUBLISHED_MICROSOFT_CLIENT_ID, None);
        let url = admin_consent_url(&own, Some("contoso.com"));
        assert!(
            url.ends_with("&redirect_uri=http%3A%2F%2Flocalhost"),
            "{url}"
        );
        assert!(!url.contains("Sync.Receive"));
        let link = admin_consent_link(&own, Some("contoso.com"));
        assert_eq!(link.tenant, "contoso.com");
        assert_eq!(link.admin_page_url, None);
        let own_push = MicrosoftAuthConfig {
            consent_scopes: vec![sync_scope(PUBLISHED_MICROSOFT_CLIENT_ID)],
            ..own
        };
        assert!(admin_consent_url(&own_push, None).contains("Sync.Receive"));
    }

    #[test]
    fn core_and_legacy_scopes_are_frozen() {
        assert_eq!(
            CORE_SCOPES,
            [
                "openid",
                "profile",
                "email",
                "offline_access",
                "User.Read",
                "Mail.ReadWrite",
                "Mail.Send",
            ]
        );
        assert_eq!(
            LEGACY_SCOPES,
            [
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
            ]
        );
        assert_eq!(all_scopes(), LEGACY_SCOPES);
        assert_eq!(legacy_graph_scopes(), LEGACY_SCOPES[4..]);
        // One permission per feature, the same both ways.
        assert_eq!(OPTIONAL_SCOPES.len(), AccountFeature::ALL.len());
        for (feature, scope) in OPTIONAL_SCOPES {
            assert_eq!(feature_scope(*feature), *scope);
        }
    }

    #[test]
    fn sign_in_scopes_per_request() {
        let core = strings(CORE_SCOPES);
        let with = |optional: &[&str]| [core.clone(), strings(optional)].concat();
        let mail_only = strings(&["User.Read", "Mail.ReadWrite", "Mail.Send"]);
        let legacy = legacy_graph_scopes();
        let contacts = [mail_only.clone(), strings(&["Contacts.ReadWrite"])].concat();

        assert_eq!(sign_in_scopes(PermissionRequest::All, None), LEGACY_SCOPES);
        assert_eq!(
            sign_in_scopes(PermissionRequest::All, Some(&mail_only)),
            LEGACY_SCOPES
        );
        assert_eq!(sign_in_scopes(PermissionRequest::MailOnly, None), core);
        assert_eq!(
            sign_in_scopes(PermissionRequest::MailOnly, Some(&legacy)),
            core
        );

        let current = PermissionRequest::Current;
        assert_eq!(sign_in_scopes(current, Some(&mail_only)), core);
        assert_eq!(sign_in_scopes(current, Some(&legacy)), LEGACY_SCOPES);
        assert_eq!(sign_in_scopes(current, None), LEGACY_SCOPES);
        assert_eq!(
            sign_in_scopes(current, Some(&contacts)),
            with(&["Contacts.ReadWrite"])
        );

        let calendar = PermissionRequest::Feature {
            feature: AccountFeature::Calendar,
        };
        assert_eq!(
            sign_in_scopes(calendar, Some(&contacts)),
            with(&["Calendars.ReadWrite", "Contacts.ReadWrite"])
        );
        assert_eq!(
            sign_in_scopes(calendar, None),
            with(&["Calendars.ReadWrite"])
        );
        // Spelled any way.
        let lower = strings(&["user.read", "contacts.readwrite"]);
        assert_eq!(
            sign_in_scopes(calendar, Some(&lower)),
            with(&["Calendars.ReadWrite", "Contacts.ReadWrite"])
        );
    }

    #[test]
    fn parse_granted_scopes_normalizes() {
        assert_eq!(
            parse_granted_scopes(
                "https://graph.microsoft.com/Mail.ReadWrite https://graph.microsoft.com/User.Read \
                 openid profile email offline_access"
            ),
            ["User.Read", "Mail.ReadWrite"]
        );
        assert_eq!(
            parse_granted_scopes(
                "HTTPS://GRAPH.MICROSOFT.COM/tasks.readwrite mail.send Calendars.ReadWrite \
                 User.Read Mail.Send"
            ),
            [
                "User.Read",
                "Mail.Send",
                "Calendars.ReadWrite",
                "Tasks.ReadWrite"
            ]
        );
        // `.default`, other resources' scopes and the OpenID Connect ones go;
        // permissions this app doesn't know stay as given, sorted.
        assert_eq!(
            parse_granted_scopes(
                "https://graph.microsoft.com/.default .default api://x/Sync.Receive \
                 https://outlook.office.com/Mail.Read OFFLINE_ACCESS Notes.Read Mail.Read \
                 notes.read  User.Read\tPeople.Read"
            ),
            ["User.Read", "Mail.Read", "Notes.Read", "People.Read"]
        );
        assert!(parse_granted_scopes("").is_empty());
        assert!(parse_granted_scopes("openid profile").is_empty());
    }

    #[test]
    fn consumer_domains_and_personal_tenants() {
        for domain in [
            "outlook.com",
            "Hotmail.com",
            "hotmail.co.uk",
            "live.com.au",
            "outlook.de",
            "live.nl",
            "msn.com",
            "passport.com",
            " windowslive.com ",
        ] {
            assert!(is_consumer_domain(domain), "{domain}");
        }
        for domain in [
            "contoso.com",
            "live.unc.edu",
            "outlook.contoso.com",
            "hotmail.example",
            "mylive.com",
            "organizations",
            "",
            "live.co.u",
            "outlook.c0m.de",
        ] {
            assert!(!is_consumer_domain(domain), "{domain}");
        }
        assert!(is_personal_tenant(CONSUMER_TENANT));
        assert!(is_personal_tenant("9188040D-6C67-4C5B-B112-36A304B66DAD"));
        assert!(is_personal_tenant("consumers"));
        assert!(is_personal_tenant("Consumers"));
        assert!(!is_personal_tenant("11111111-2222-3333-4444-555555555555"));
        assert!(!is_personal_tenant("organizations"));
        assert!(!is_personal_tenant("common"));
    }

    #[test]
    fn what_a_grant_lacks_and_what_was_refused() {
        let grant = |scopes: &[&str], refused: &[AccountFeature]| MicrosoftGrant {
            scopes: strings(scopes),
            refused: refused.to_vec(),
            refused_at: None,
        };
        // Not recorded yet: everything, as every account had before.
        assert!(needs_permission(None).is_empty());
        assert!(refused_features(None).is_empty());
        let mail_only = grant(&["User.Read", "Mail.ReadWrite", "Mail.Send"], &[]);
        assert_eq!(needs_permission(Some(&mail_only)), AccountFeature::ALL);
        let some = grant(
            &["User.Read", "mailboxsettings.read", "Tasks.ReadWrite"],
            &[
                AccountFeature::Tasks,
                AccountFeature::MailboxSettings,
                AccountFeature::Calendar,
            ],
        );
        assert_eq!(
            needs_permission(Some(&some)),
            [AccountFeature::Calendar, AccountFeature::Contacts]
        );
        // Never a feature lacking its permission, nor the time zone.
        assert_eq!(refused_features(Some(&some)), [AccountFeature::Tasks]);
    }

    #[test]
    fn refusal_pages_say_whether_the_sign_in_goes_on() {
        let page = |query: &str, continues_if_refused: bool| {
            let Incoming::Response(response) = classify_request(&format!("/?{query}&state=s"), "s")
            else {
                panic!("not a response: {query}");
            };
            let page = response_page(&response, continues_if_refused);
            let title = page.split("<h1>").nth(1).unwrap().split("</h1>").next();
            title.unwrap().to_owned()
        };
        let admin = "error=access_denied&error_description=AADSTS90094%3a+Admin+consent+required";
        let declined = "error=access_denied&error_subcode=cancel&error_description=AADSTS65004";
        let canceled = "error=access_denied&error_subcode=cancel&error_description=AADSTS50058";
        let continues = "Signing in without instant updates";
        let failed = "Sign-in did not complete";
        assert_eq!(page(admin, true), continues);
        assert_eq!(page(declined, true), continues);
        assert_eq!(page(canceled, true), failed);
        assert_eq!(page("error=server_error", true), failed);
        assert_eq!(page(admin, false), failed);
        assert_eq!(page(declined, false), failed);
        assert_eq!(page("code=c", true), "You&#39;re signed in");
    }

    #[test]
    fn loopback_requests_are_classified() {
        assert_eq!(classify_request("/favicon.ico", "s"), Incoming::Unrelated);
        assert_eq!(classify_request("/", "s"), Incoming::Unrelated);
        assert_eq!(
            classify_request("/?code=c&state=other", "s"),
            Incoming::StateMismatch
        );
        assert_eq!(classify_request("/?code=c", "s"), Incoming::StateMismatch);
        assert_eq!(
            classify_request("/?code=c&state=s", "s"),
            Incoming::Response(AuthorizationResponse {
                code: Some("c".into()),
                ..Default::default()
            })
        );
        assert_eq!(
            classify_request(
                "/?error=access_denied&error_subcode=cancel&error_description=x&state=s",
                "s"
            ),
            Incoming::Response(AuthorizationResponse {
                error: Some("access_denied".into()),
                error_description: Some("x".into()),
                error_subcode: Some("cancel".into()),
                ..Default::default()
            })
        );
    }

    #[tokio::test]
    async fn sign_in_redeems_the_code_with_the_pkce_verifier() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/common/oauth2/v2.0/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .respond_with(token_success())
            .expect(1)
            .mount(&server)
            .await;
        let browser = fake_browser(vec![
            Box::new(|redirect, _| format!("{redirect}/favicon.ico")),
            redirect_with("code=the-code&session_state=xyz"),
        ]);

        let result = run_sign_in(&server, &browser, Some("adelev@contoso.com"))
            .await
            .unwrap();
        assert_eq!(result.email, "AdeleV@contoso.com");
        assert_eq!(result.display_name, "Adele Vance");
        assert_eq!(result.tenant_id, "11111111-2222-3333-4444-555555555555");
        assert_eq!(result.user_id, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        assert_eq!(result.access_token, "access-1");
        assert_eq!(result.refresh_token, "refresh-1");
        assert_eq!(result.expires_in, Duration::from_secs(3599));

        // The verifier sent to the token endpoint matches the challenge sent
        // to the authorize endpoint, and the redirect URIs agree.
        let authorize = browser.authorize_url.lock().unwrap().clone().unwrap();
        let authorize: HashMap<String, String> = authorize.query_pairs().into_owned().collect();
        assert!(authorize["redirect_uri"].starts_with("http://localhost:"));
        let requests = server.received_requests().await.unwrap();
        let form: HashMap<String, String> = url::form_urlencoded::parse(&requests[0].body)
            .into_owned()
            .collect();
        assert_eq!(
            Pkce::from_verifier(form["code_verifier"].clone()).challenge,
            authorize["code_challenge"]
        );
        assert_eq!(form["redirect_uri"], authorize["redirect_uri"]);
        assert_eq!(form["client_id"], CLIENT_ID);
        assert!(!form.contains_key("client_secret"));

        assert_eq!(browser.wait_for(2).await, vec![404, 200]);
    }

    #[tokio::test]
    async fn redirect_with_another_state_is_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("code=genuine"))
            .respond_with(token_success())
            .expect(1)
            .mount(&server)
            .await;
        let browser = fake_browser(vec![
            Box::new(|redirect, _| format!("{redirect}/?code=forged&state=attacker")),
            redirect_with("code=genuine"),
        ]);
        run_sign_in(&server, &browser, None).await.unwrap();
        assert_eq!(browser.wait_for(2).await, vec![400, 200]);
    }

    #[tokio::test]
    async fn sign_in_times_out_as_cancelled() {
        let server = MockServer::start().await;
        let browser = fake_browser(vec![Box::new(|redirect, _| {
            format!("{redirect}/?code=forged&state=attacker")
        })]);
        let error = sign_in(
            &config(&server),
            &*browser.open,
            None,
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::SignInCancelled);
        // timeout_says_so_in_details
        assert_eq!(
            error.message,
            "Sign-in was not completed in time. Try again."
        );
        assert_eq!(error.details.unwrap()["reason"], "timeout");
    }

    #[tokio::test]
    async fn sign_in_asks_for_the_configured_graph_scopes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(token_success())
            .expect(1)
            .mount(&server)
            .await;
        let browser = fake_browser(vec![redirect_with("code=the-code")]);
        let config = MicrosoftAuthConfig {
            graph_scopes: sign_in_scopes(PermissionRequest::MailOnly, None),
            ..config(&server)
        };
        let result = sign_in(&config, &*browser.open, None, Duration::from_secs(10))
            .await
            .unwrap();
        let core = CORE_SCOPES.join(" ");
        let authorize = browser.authorize_url.lock().unwrap().clone().unwrap();
        let authorize: HashMap<String, String> = authorize.query_pairs().into_owned().collect();
        assert_eq!(authorize["scope"], core);
        let requests = server.received_requests().await.unwrap();
        let form: HashMap<String, String> = url::form_urlencoded::parse(&requests[0].body)
            .into_owned()
            .collect();
        assert_eq!(form["scope"], core);
        // What was asked for, and what Microsoft said (`Mail.ReadWrite
        // User.Read`), as Graph permissions.
        assert_eq!(
            result.requested_scopes,
            ["User.Read", "Mail.ReadWrite", "Mail.Send"]
        );
        assert_eq!(result.response_scopes, ["User.Read", "Mail.ReadWrite"]);
    }

    #[tokio::test]
    async fn browser_errors_propagate() {
        let server = MockServer::start().await;
        let open = |_: &str| -> Result<()> { Err(Error::internal("no browser")) };
        let error = sign_in(&config(&server), &open, None, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(error.message, "no browser");
    }

    #[tokio::test]
    async fn blocked_sign_ins_say_why() {
        /// The redirect, the reason, its AADSTS code and whether the error
        /// carries an approval link.
        type Case = (&'static str, &'static str, u32, bool);
        let cases: &[Case] = &[
            (
                "error=invalid_request&error_description=AADSTS50105%3a+Your+administrator+has+configured+the+application+to+block+users",
                "notAssigned",
                50105,
                false,
            ),
            (
                "error=unauthorized_client&error_description=AADSTS7000112%3a+Application+is+disabled.",
                "appDisabled",
                7000112,
                false,
            ),
            (
                "error=invalid_client&error_description=AADSTS65005%3a+Misconfigured+application.",
                "appNotSetUp",
                65005,
                true,
            ),
            (
                "error=invalid_client&error_description=AADSTS650052%3a+The+app+needs+access+to+a+service+that+your+organization+has+not+subscribed+to.",
                "serviceNotEnabled",
                650052,
                false,
            ),
            (
                "error=interaction_required&error_description=AADSTS53004%3a+Proof+up+required.",
                "securityInfo",
                53004,
                false,
            ),
            (
                "error=access_denied&error_description=AADSTS53003%3a+Access+has+been+blocked+by+Conditional+Access+policies.",
                "conditionalAccess",
                53003,
                false,
            ),
            (
                "error=access_denied&error_description=AADSTS530032%3a+Blocked+by+security+defaults.",
                "conditionalAccess",
                530032,
                false,
            ),
            // The blocking code wins over one asking for approval.
            (
                "error=access_denied&error_description=AADSTS90094%3a+Admin+consent+required.+AADSTS53003%3a+Blocked.",
                "conditionalAccess",
                53003,
                false,
            ),
        ];
        for (query, reason, aadsts, link) in cases {
            let server = MockServer::start().await;
            let browser = fake_browser(vec![redirect_with(query)]);
            let error = run_sign_in(&server, &browser, Some("adele@contoso.com"))
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::SignInBlocked, "{query}");
            let details = error.details.expect("details");
            assert_eq!(details["reason"], *reason, "{query}");
            assert_eq!(details["aadsts"], *aadsts, "{query}");
            assert_eq!(details["tenant"], "contoso.com");
            assert_eq!(details["adminPageUrl"], serde_json::Value::Null);
            assert_eq!(details.get("adminConsentUrl").is_some(), *link, "{query}");
            assert!(!error.message.contains("AADSTS"), "{}", error.message);
            assert_eq!(browser.wait_for(1).await, vec![200]);
        }
    }

    #[tokio::test]
    async fn error_redirects_map_to_error_codes() {
        let cases: &[(&'static str, Option<&str>, ErrorCode)] = &[
            (
                "error=access_denied&error_subcode=cancel&error_description=AADSTS50058",
                None,
                ErrorCode::SignInCancelled,
            ),
            (
                "error=access_denied&error_description=the+user+canceled+the+authentication",
                None,
                ErrorCode::SignInCancelled,
            ),
            (
                "error=access_denied&error_description=AADSTS65004%3a+User+declined+to+consent+to+access+the+app.%0d%0aTrace+ID%3a+x",
                None,
                ErrorCode::ConsentDeclined,
            ),
            (
                "error=access_denied&error_subcode=cancel&error_description=AADSTS65004%3a+User+declined+to+consent",
                None,
                ErrorCode::ConsentDeclined,
            ),
            (
                "error=access_denied&error_description=AADSTS90094%3a+An+administrator+of+Contoso+has+set+a+policy",
                Some("contoso.com"),
                ErrorCode::AdminConsentRequired,
            ),
            (
                "error=access_denied&error_subcode=cancel&error_description=AADSTS90095%3a+Admin+consent+required",
                Some("contoso.com"),
                ErrorCode::AdminConsentRequired,
            ),
            (
                "error=access_denied&error_description=AADSTS900941%3a+Administrator+consent+is+required.+App+is+considered+risky.",
                Some("contoso.com"),
                ErrorCode::AdminConsentRequired,
            ),
            (
                "error=consent_required&error_description=AADSTS65001%3a+The+user+or+administrator+has+not+consented",
                Some("contoso.com"),
                ErrorCode::AdminConsentRequired,
            ),
            (
                "error=invalid_request&error_description=AADSTS50105%3a+Your+administrator+has+configured+the+application+to+block+users",
                None,
                ErrorCode::SignInBlocked,
            ),
            (
                "error=invalid_request&error_description=AADSTS50000%3a+There+was+an+error+issuing+a+token.",
                None,
                ErrorCode::Provider,
            ),
        ];
        for (query, tenant, expected) in cases {
            let server = MockServer::start().await;
            let browser = fake_browser(vec![redirect_with(query)]);
            let error = run_sign_in(&server, &browser, Some("adele@contoso.com"))
                .await
                .unwrap_err();
            assert_eq!(error.code, *expected, "{query}");
            if let Some(tenant) = tenant {
                let details = error.details.as_ref().expect("admin consent details");
                assert_eq!(details["tenant"], *tenant);
                assert_eq!(
                    details["canRequestApproval"],
                    query.contains("AADSTS90095"),
                    "{query}"
                );
                let url = details["adminConsentUrl"].as_str().unwrap();
                assert!(
                    url.starts_with(&format!("{}/{tenant}/v2.0/adminconsent?", server.uri())),
                    "{url}"
                );
            }
            if *expected == ErrorCode::SignInBlocked {
                // More in `blocked_sign_ins_say_why`.
                let details = error.details.as_ref().expect("blocked details");
                assert_eq!(details["reason"], "notAssigned");
                assert_eq!(details["aadsts"], 50105);
            }
            if *expected == ErrorCode::Provider {
                assert!(
                    error.message.starts_with("AADSTS50000"),
                    "{}",
                    error.message
                );
            }
            // The browser is told how it went.
            assert_eq!(browser.wait_for(1).await, vec![200]);
        }
    }

    #[tokio::test]
    async fn admin_consent_details_name_the_admin_page_for_the_published_registration() {
        const ADMIN: &str =
            "error=access_denied&error_description=AADSTS90094%3a+Admin+consent+required";
        for published in [true, false] {
            let server = MockServer::start().await;
            let browser = fake_browser(vec![redirect_with(ADMIN)]);
            let config = MicrosoftAuthConfig {
                published,
                ..config(&server)
            };
            let error = sign_in(
                &config,
                &*browser.open,
                Some("adele@contoso.com"),
                Duration::from_secs(10),
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::AdminConsentRequired);
            let details = error.details.unwrap();
            let url = Url::parse(details["adminConsentUrl"].as_str().unwrap()).unwrap();
            let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
            if published {
                assert_eq!(details["adminPageUrl"], ADMIN_CONSENT_LANDING_URI);
                assert_eq!(params["redirect_uri"], ADMIN_CONSENT_LANDING_URI);
                assert_eq!(
                    params["scope"].contains("Sync.Receive"),
                    ADMIN_LINK_INCLUDES_PUSH
                );
            } else {
                assert_eq!(details["adminPageUrl"], serde_json::Value::Null);
                assert_eq!(params["redirect_uri"], ADMIN_CONSENT_REDIRECT_URI);
                assert!(!params["scope"].contains("Sync.Receive"));
            }
            assert_eq!(details["canRequestApproval"], false);
            assert_eq!(details["tenant"], "contoso.com");
        }
    }

    #[tokio::test]
    async fn admin_consent_without_login_hint_uses_organizations() {
        let server = MockServer::start().await;
        let browser = fake_browser(vec![redirect_with(
            "error=access_denied&error_description=AADSTS90094%3a+The+grant+requires+admin+permission.",
        )]);
        let error = run_sign_in(&server, &browser, None).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::AdminConsentRequired);
        let details = error.details.unwrap();
        assert_eq!(details["tenant"], "organizations");
        assert_eq!(details["canRequestApproval"], false);
    }

    #[tokio::test]
    async fn admin_consent_workflow_lets_users_request_approval() {
        let server = MockServer::start().await;
        let browser = fake_browser(vec![redirect_with(
            "error=access_denied&error_description=AADSTS90095%3a+Admin+consent+is+required+for+the+permissions+requested+by+this+application.+An+admin+consent+request+may+be+sent+to+the+admin.",
        )]);
        let error = run_sign_in(&server, &browser, Some("adele@contoso.com"))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AdminConsentRequired);
        assert!(error.message.contains("request approval"));
        let details = error.details.unwrap();
        assert_eq!(details["canRequestApproval"], true);
        assert_eq!(details["tenant"], "contoso.com");
        assert!(details["adminConsentUrl"].as_str().is_some());
    }

    #[tokio::test]
    async fn token_endpoint_errors_are_mapped() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_error(
                "invalid_grant",
                "AADSTS65001: The user or administrator has not consented to use the application.\r\nTrace ID: x",
                &[65001],
            ))
            .mount(&server)
            .await;
        let browser = fake_browser(vec![redirect_with("code=c")]);
        let error = run_sign_in(&server, &browser, Some("adele@fabrikam.com"))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AdminConsentRequired);
        assert_eq!(error.details.unwrap()["tenant"], "fabrikam.com");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_error(
                "invalid_client",
                "AADSTS7000218: The request body must contain the following parameter: 'client_assertion' or 'client_secret'.\r\nTrace ID: x",
                &[7000218],
            ))
            .mount(&server)
            .await;
        let browser = fake_browser(vec![redirect_with("code=c")]);
        let error = run_sign_in(&server, &browser, None).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Provider);
        assert!(error.message.starts_with("AADSTS7000218:"));
        assert!(!error.message.contains("Trace ID"));
    }

    async fn token_source(server: &MockServer, refresh: Option<&str>) -> MicrosoftTokenSource {
        let secrets = Arc::new(MemorySecretStore::default());
        if let Some(refresh) = refresh {
            secrets
                .set(&keys::refresh_token("acct"), refresh)
                .await
                .unwrap();
        }
        MicrosoftTokenSource::new(
            config(server),
            "acct",
            secrets,
            false,
            Arc::new(NoGrantRecord),
        )
    }

    /// Keeps what it's told, and records what refreshes say.
    #[derive(Default)]
    struct FakeGrant {
        granted: StdMutex<Option<Vec<String>>>,
        recorded: StdMutex<Vec<Vec<String>>>,
    }

    impl FakeGrant {
        fn with(granted: Option<&[&str]>) -> Arc<Self> {
            Arc::new(Self {
                granted: StdMutex::new(granted.map(strings)),
                ..Self::default()
            })
        }

        fn recorded(&self) -> Vec<Vec<String>> {
            self.recorded.lock().unwrap().clone()
        }
    }

    impl GrantRecord for FakeGrant {
        fn granted(&self) -> Result<Option<Vec<String>>> {
            Ok(self.granted.lock().unwrap().clone())
        }

        fn record(&self, granted: &[String]) {
            self.recorded.lock().unwrap().push(granted.to_vec());
        }
    }

    /// A Graph token source for a work (or `personal`) account signed in
    /// as refresh-0, recording in `grant`.
    async fn graph_tokens(
        server: &MockServer,
        personal: bool,
        grant: &Arc<FakeGrant>,
    ) -> MicrosoftTokenSource {
        let secrets = Arc::new(MemorySecretStore::default());
        secrets
            .set(&keys::refresh_token("acct"), "refresh-0")
            .await
            .unwrap();
        MicrosoftTokenSource::new(config(server), "acct", secrets, personal, grant.clone())
    }

    /// A successful refresh whose `scope` is `scope`.
    fn token_with_scope(scope: Option<&str>) -> ResponseTemplate {
        let mut body = json!({
            "token_type": "Bearer",
            "expires_in": 3599,
            "access_token": "access-1",
            "refresh_token": "refresh-1",
        });
        if let Some(scope) = scope {
            body["scope"] = json!(scope);
        }
        ResponseTemplate::new(200).set_body_json(body)
    }

    /// The `scope` each token request asked for.
    async fn asked_scopes(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| {
                url::form_urlencoded::parse(&r.body)
                    .find(|(k, _)| k == "scope")
                    .map(|(_, v)| v.into_owned())
                    .unwrap_or_default()
            })
            .collect()
    }

    #[tokio::test]
    async fn work_refresh_uses_default_and_records_the_grant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains(
                "scope=https%3A%2F%2Fgraph.microsoft.com%2F.default+offline_access",
            ))
            .respond_with(token_with_scope(Some(
                "https://graph.microsoft.com/Mail.ReadWrite https://graph.microsoft.com/User.Read \
                 https://graph.microsoft.com/Calendars.ReadWrite \
                 https://graph.microsoft.com/Mail.Send openid profile email",
            )))
            .expect(1)
            .mount(&server)
            .await;
        // Whatever was recorded before: `.default` covers a later approval.
        let grant = FakeGrant::with(Some(&["User.Read", "Mail.ReadWrite", "Mail.Send"]));
        let source = graph_tokens(&server, false, &grant).await;
        assert_eq!(source.access_token().await.unwrap(), "access-1");
        assert_eq!(
            grant.recorded(),
            [strings(&[
                "User.Read",
                "Mail.ReadWrite",
                "Mail.Send",
                "Calendars.ReadWrite"
            ])]
        );
        // The rotated refresh token is kept too.
        assert_eq!(
            source
                .secrets
                .get(&keys::refresh_token("acct"))
                .await
                .unwrap()
                .as_deref(),
            Some("refresh-1")
        );
    }

    #[tokio::test]
    async fn refresh_without_scope_or_core_records_nothing() {
        for scope in [
            None,
            Some(""),
            Some("https://graph.microsoft.com/Calendars.ReadWrite openid profile"),
            Some("api://test-client-id/Sync.Receive"),
        ] {
            for personal in [false, true] {
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .respond_with(token_with_scope(scope))
                    .expect(1)
                    .mount(&server)
                    .await;
                let grant = FakeGrant::with(None);
                let source = graph_tokens(&server, personal, &grant).await;
                assert_eq!(source.access_token().await.unwrap(), "access-1");
                assert!(grant.recorded().is_empty(), "{scope:?}");
                // The refresh token is still kept.
                assert_eq!(
                    source
                        .secrets
                        .get(&keys::refresh_token("acct"))
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("refresh-1")
                );
            }
        }
    }

    #[tokio::test]
    async fn default_refused_as_invalid_scope_retries_once_with_the_explicit_list() {
        let explicit = "openid profile email offline_access User.Read Mail.ReadWrite Mail.Send \
                        Contacts.ReadWrite";
        let recorded: &[&str] = &[
            "User.Read",
            "Mail.ReadWrite",
            "Mail.Send",
            "Contacts.ReadWrite",
        ];
        let refused = || {
            token_error(
                "invalid_scope",
                "AADSTS70011: The provided request must include a 'scope' input parameter.",
                &[70011],
            )
        };

        // Named one by one, the permissions are granted.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains(".default"))
            .respond_with(refused())
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("scope=openid+profile"))
            .respond_with(token_success())
            .expect(1)
            .mount(&server)
            .await;
        let grant = FakeGrant::with(Some(recorded));
        let source = graph_tokens(&server, false, &grant).await;
        assert_eq!(source.access_token().await.unwrap(), "access-1");
        assert_eq!(
            asked_scopes(&server).await,
            [
                format!("{GRAPH_DEFAULT_SCOPE} offline_access"),
                explicit.to_owned()
            ]
        );
        // A work account records what Microsoft said.
        assert_eq!(
            grant.recorded(),
            [strings(&["User.Read", "Mail.ReadWrite"])]
        );

        // Refused that way too: no third try.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(refused())
            .expect(2)
            .mount(&server)
            .await;
        let grant = FakeGrant::with(Some(recorded));
        let error = graph_tokens(&server, false, &grant)
            .await
            .access_token()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Provider);
        assert!(grant.recorded().is_empty());

        // Other refusals aren't tried again.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_error(
                "invalid_grant",
                "AADSTS700082: Expired.",
                &[700082],
            ))
            .expect(1)
            .mount(&server)
            .await;
        let grant = FakeGrant::with(Some(recorded));
        let error = graph_tokens(&server, false, &grant)
            .await
            .access_token()
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthRequired);
    }

    #[tokio::test]
    async fn personal_refresh_names_only_recorded_permissions() {
        /// Refreshes a personal account whose grant is `recorded`, Microsoft
        /// answering that it has `answer`; returns the scope asked for and
        /// what was recorded.
        async fn refresh(recorded: Option<&[&str]>, answer: &str) -> (String, Vec<Vec<String>>) {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(token_with_scope(Some(answer)))
                .expect(1)
                .mount(&server)
                .await;
            let grant = FakeGrant::with(recorded);
            let source = graph_tokens(&server, true, &grant).await;
            source.access_token().await.unwrap();
            let asked = asked_scopes(&server).await.remove(0);
            (asked, grant.recorded())
        }
        let mail_only: &[&str] = &["User.Read", "Mail.ReadWrite", "Mail.Send"];
        let everything = LEGACY_SCOPES.join(" ");

        // Not recorded yet: exactly what every refresh asked for before.
        let (asked, _) = refresh(None, &everything).await;
        assert_eq!(asked, everything);
        // Mail only.
        let (asked, recorded) = refresh(Some(mail_only), &everything).await;
        assert_eq!(asked, CORE_SCOPES.join(" "));
        // Microsoft naming more than was asked for doesn't add it.
        assert_eq!(recorded, [strings(mail_only)]);
        // A feature turned on since.
        let with_calendar = [mail_only, &["Calendars.ReadWrite"]].concat();
        let (asked, recorded) = refresh(Some(&with_calendar), "Mail.ReadWrite User.Read").await;
        assert_eq!(
            asked,
            format!("{} Calendars.ReadWrite", CORE_SCOPES.join(" "))
        );
        assert_eq!(recorded, [strings(&["User.Read", "Mail.ReadWrite"])]);
    }

    #[tokio::test]
    async fn a_refresh_overtaken_by_a_sign_in_records_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_success().set_delay(Duration::from_millis(300)))
            .mount(&server)
            .await;
        let grant = FakeGrant::with(None);
        let source = Arc::new(graph_tokens(&server, false, &grant).await);
        let refreshing = tokio::spawn({
            let source = Arc::clone(&source);
            async move { source.access_token().await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        // A sign-in meanwhile stores its refresh token (and its grant).
        let key = keys::refresh_token("acct");
        source.secrets.set(&key, "refresh-new").await.unwrap();
        assert_eq!(refreshing.await.unwrap().unwrap(), "access-1");
        assert!(grant.recorded().is_empty());
        assert_eq!(
            source.secrets.get(&key).await.unwrap().as_deref(),
            Some("refresh-new")
        );
    }

    #[tokio::test]
    async fn a_refresh_reads_its_token_and_grant_after_a_sign_in_saving_both() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_with_scope(Some(
                "User.Read Mail.ReadWrite Mail.Send Calendars.ReadWrite",
            )))
            .mount(&server)
            .await;
        let mail_only: &[&str] = &["User.Read", "Mail.ReadWrite", "Mail.Send"];
        let with_calendar = strings(&[mail_only, &["Calendars.ReadWrite"]].concat());
        let grant = FakeGrant::with(Some(mail_only));
        let secrets = Arc::new(MemorySecretStore::default());
        let (id, key) = ("acct-turning-on", keys::refresh_token("acct-turning-on"));
        secrets.set(&key, "refresh-0").await.unwrap();
        let source = Arc::new(MicrosoftTokenSource::new(
            config(&server),
            id,
            secrets,
            true,
            grant.clone(),
        ));
        // A personal account's sign-in turning the calendar on saves its
        // refresh token, then its grant, as a refresh starts.
        let signing_in = secrets::lock_account(id).await;
        let refreshing = tokio::spawn({
            let source = Arc::clone(&source);
            async move { source.access_token().await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        source.secrets.set(&key, "refresh-new").await.unwrap();
        *grant.granted.lock().unwrap() = Some(with_calendar.clone());
        drop(signing_in);
        assert_eq!(refreshing.await.unwrap().unwrap(), "access-1");
        // The refresh asked with the new token for what it was granted.
        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(body.contains("refresh_token=refresh-new"), "{body}");
        assert_eq!(
            asked_scopes(&server).await,
            [format!("{} Calendars.ReadWrite", CORE_SCOPES.join(" "))]
        );
        assert_eq!(grant.recorded(), [with_calendar]);
    }

    #[tokio::test]
    async fn a_refresh_keeps_a_grant_recorded_meanwhile() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                token_with_scope(Some("User.Read Mail.ReadWrite Mail.Send"))
                    .set_delay(Duration::from_millis(300)),
            )
            .mount(&server)
            .await;
        let grant = FakeGrant::with(Some(&["User.Read", "Mail.ReadWrite", "Mail.Send"]));
        let source = Arc::new(graph_tokens(&server, true, &grant).await);
        let refreshing = tokio::spawn({
            let source = Arc::clone(&source);
            async move { source.access_token().await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Another refresh of the same token records a newer grant.
        *grant.granted.lock().unwrap() = Some(strings(&[
            "User.Read",
            "Mail.ReadWrite",
            "Mail.Send",
            "Calendars.ReadWrite",
        ]));
        assert_eq!(refreshing.await.unwrap().unwrap(), "access-1");
        assert!(grant.recorded().is_empty());
    }

    #[tokio::test]
    async fn refresh_blocked_codes_explain_themselves() {
        let cases: &[(&str, &str, u32, &str)] = &[
            (
                "invalid_grant",
                "AADSTS50105: Your administrator has configured the application to block users.",
                50105,
                "notAssigned",
            ),
            (
                "invalid_grant",
                "AADSTS53003: Access has been blocked by Conditional Access policies.",
                53003,
                "conditionalAccess",
            ),
            (
                "invalid_client",
                "AADSTS7000112: Application is disabled.",
                7000112,
                "appDisabled",
            ),
        ];
        for (error, description, code, reason) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(token_error(error, description, &[*code]))
                .mount(&server)
                .await;
            let source = token_source(&server, Some("refresh-0")).await;
            let failure = source.access_token().await.unwrap_err();
            assert_eq!(failure.code, ErrorCode::AuthRequired);
            assert!(
                failure.message.ends_with(&format!("(AADSTS{code})")),
                "{}",
                failure.message
            );
            assert!(
                failure.message.starts_with("Your organization"),
                "{}",
                failure.message
            );
            let details = failure.details.unwrap();
            assert_eq!(details["reason"], *reason);
            assert_eq!(details["aadsts"], *code);
        }
    }

    #[tokio::test]
    async fn valid_cached_token_is_reused() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_success())
            .expect(0)
            .mount(&server)
            .await;
        let source = token_source(&server, Some("refresh-0")).await;
        source.prime("primed", Duration::from_secs(3600)).await;
        assert_eq!(source.access_token().await.unwrap(), "primed");
        assert_eq!(source.access_token().await.unwrap(), "primed");
    }

    #[tokio::test]
    async fn invalidated_tokens_are_refreshed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(token_success())
            .expect(1)
            .mount(&server)
            .await;
        let source = token_source(&server, Some("refresh-0")).await;
        source.prime("rejected", Duration::from_secs(3600)).await;
        source.invalidate().await;
        assert_eq!(source.access_token().await.unwrap(), "access-1");
        // The refreshed token is cached again.
        assert_eq!(source.access_token().await.unwrap(), "access-1");
    }

    #[tokio::test]
    async fn expiring_token_is_refreshed_and_rotation_saved() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/common/oauth2/v2.0/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=refresh-0"))
            .and(body_string_contains("client_id=test-client-id"))
            .respond_with(token_success())
            .expect(1)
            .mount(&server)
            .await;
        let source = token_source(&server, Some("refresh-0")).await;
        source.prime("stale", Duration::from_secs(60)).await;

        // Concurrent callers share one refresh.
        let (a, b) = tokio::join!(source.access_token(), source.access_token());
        assert_eq!(a.unwrap(), "access-1");
        assert_eq!(b.unwrap(), "access-1");
        assert_eq!(
            source
                .secrets
                .get(&keys::refresh_token("acct"))
                .await
                .unwrap()
                .as_deref(),
            Some("refresh-1")
        );
    }

    /// Redeems refresh-0, which Microsoft rotates to refresh-1, while
    /// `meanwhile` changes the stored one; returns what's stored after.
    async fn rotate_while(meanwhile: Option<&str>) -> Option<String> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_success().set_delay(Duration::from_millis(300)))
            .mount(&server)
            .await;
        let source = Arc::new(token_source(&server, Some("refresh-0")).await);
        let refreshing = tokio::spawn({
            let source = Arc::clone(&source);
            async move { source.access_token().await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let key = keys::refresh_token("acct");
        match meanwhile {
            Some(refresh) => source.secrets.set(&key, refresh).await.unwrap(),
            None => source.secrets.delete(&key).await.unwrap(),
        }
        assert_eq!(refreshing.await.unwrap().unwrap(), "access-1");
        source.secrets.get(&key).await.unwrap()
    }

    #[tokio::test]
    async fn a_rotated_refresh_token_is_not_kept_for_a_removed_account() {
        // Removed while the refresh token is being redeemed.
        assert_eq!(rotate_while(None).await, None);
    }

    #[tokio::test]
    async fn a_rotated_refresh_token_does_not_replace_a_new_sign_in() {
        // Signed in again while the refresh token is being redeemed.
        assert_eq!(
            rotate_while(Some("refresh-new")).await.as_deref(),
            Some("refresh-new")
        );
    }

    #[tokio::test]
    async fn invalid_grant_requires_sign_in() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_error(
                "invalid_grant",
                "AADSTS700082: The refresh token has expired due to inactivity.\r\nTrace ID: x",
                &[700082],
            ))
            .mount(&server)
            .await;
        let source = token_source(&server, Some("refresh-0")).await;
        let error = source.access_token().await.unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthRequired);
        assert!(error.message.contains("AADSTS700082"));

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_error(
                "interaction_required",
                "AADSTS50076: Due to a configuration change made by your administrator, you must use multi-factor authentication.",
                &[50076],
            ))
            .mount(&server)
            .await;
        let source = token_source(&server, Some("refresh-0")).await;
        assert_eq!(
            source.access_token().await.unwrap_err().code,
            ErrorCode::AuthRequired
        );
    }

    #[tokio::test]
    async fn other_refresh_failures_are_provider_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_error(
                "temporarily_unavailable",
                "AADSTS90033: A transient error has occurred.",
                &[90033],
            ))
            .mount(&server)
            .await;
        let source = token_source(&server, Some("refresh-0")).await;
        assert_eq!(
            source.access_token().await.unwrap_err().code,
            ErrorCode::Provider
        );
    }

    #[tokio::test]
    async fn missing_refresh_token_requires_sign_in() {
        let server = MockServer::start().await;
        let source = token_source(&server, None).await;
        assert_eq!(
            source.access_token().await.unwrap_err().code,
            ErrorCode::AuthRequired
        );
    }

    #[tokio::test]
    async fn sign_in_can_ask_consent_for_the_push_service_too() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/common/oauth2/v2.0/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(token_success())
            .expect(1)
            .mount(&server)
            .await;
        let browser = fake_browser(vec![redirect_with("code=the-code")]);
        let config = MicrosoftAuthConfig {
            consent_scopes: vec![sync_scope(CLIENT_ID)],
            ..config(&server)
        };
        sign_in(&config, &*browser.open, None, Duration::from_secs(10))
            .await
            .unwrap();

        // Consent covers both resources...
        let authorize = browser.authorize_url.lock().unwrap().clone().unwrap();
        let authorize: HashMap<String, String> = authorize.query_pairs().into_owned().collect();
        let scopes: Vec<&str> = authorize["scope"].split(' ').collect();
        assert_eq!(scopes.len(), all_scopes().len() + 1);
        assert_eq!(scopes.last(), Some(&"api://test-client-id/Sync.Receive"));
        // ...but the code is redeemed for Graph alone.
        let requests = server.received_requests().await.unwrap();
        let form: HashMap<String, String> = url::form_urlencoded::parse(&requests[0].body)
            .into_owned()
            .collect();
        assert_eq!(form["scope"], all_scopes().join(" "));
    }

    /// Plays a browser that answers each authorization request in turn with
    /// the next of `answers`; returns the requests' scopes too.
    fn answering_browser(answers: &[&'static str]) -> (Opener, Arc<StdMutex<Vec<String>>>) {
        let scopes = Arc::new(StdMutex::new(Vec::new()));
        let (answers, seen) = (answers.to_vec(), scopes.clone());
        let open = Box::new(move |url: &str| -> Result<()> {
            let url = Url::parse(url).expect("authorize URL parses");
            let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
            let mut seen = seen.lock().unwrap();
            let visit = format!(
                "{}/?{}&state={}",
                params["redirect_uri"],
                answers[seen.len()],
                params["state"]
            );
            seen.push(params["scope"].clone());
            tokio::spawn(async move {
                let client = reqwest::Client::builder().no_proxy().build().unwrap();
                client.get(visit).send().await.unwrap();
            });
            Ok(())
        });
        (open, scopes)
    }

    /// The scopes of the admin consent link in `error`.
    fn approval_scopes(error: &Error) -> Vec<String> {
        let details = error.details.as_ref().expect("admin consent details");
        let url = Url::parse(details["adminConsentUrl"].as_str().unwrap()).unwrap();
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        params["scope"].split(' ').map(str::to_owned).collect()
    }

    #[tokio::test]
    async fn refused_push_consent_never_costs_the_sign_in() {
        const ADMIN: &str =
            "error=access_denied&error_description=AADSTS90094%3a+Admin+consent+required";
        const DECLINED: &str = "error=access_denied&error_subcode=cancel&error_description=AADSTS65004%3a+User+declined+to+consent";
        const CANCELED: &str =
            "error=access_denied&error_subcode=cancel&error_description=AADSTS50058";
        let sync_receive = sync_scope(CLIENT_ID);
        let sign_in_with_push = async |server: &MockServer, answers: &[&'static str]| {
            let config = MicrosoftAuthConfig {
                consent_scopes: vec![sync_receive.clone()],
                ..config(server)
            };
            let (open, scopes) = answering_browser(answers);
            let result = sign_in(
                &config,
                &*open,
                Some("adele@contoso.com"),
                Duration::from_secs(10),
            )
            .await;
            let scopes = scopes.lock().unwrap().clone();
            (result, scopes)
        };

        // The push service's scope needs an administrator, or was declined:
        // the user signs in with Graph's scopes alone.
        for (refusal, code) in [
            (ADMIN, ErrorCode::AdminConsentRequired),
            (DECLINED, ErrorCode::ConsentDeclined),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_string_contains("grant_type=authorization_code"))
                .respond_with(token_success())
                .expect(1)
                .mount(&server)
                .await;
            let (result, scopes) = sign_in_with_push(&server, &[refusal, "code=the-code"]).await;
            let result = result.unwrap();
            assert_eq!(result.access_token, "access-1");
            let graph = config(&server).graph_scopes.join(" ");
            assert_eq!(scopes, [format!("{graph} {sync_receive}"), graph]);
            let refused = result.consent_refused.expect("the refusal");
            assert_eq!(refused.code, code);
            assert!(refused.message.contains("Inbox is checked"), "{refused:?}");
            if code == ErrorCode::AdminConsentRequired {
                assert!(approval_scopes(&refused).contains(&sync_receive));
            }
        }

        // Graph's scopes need an administrator too: that fails the sign-in,
        // with a link that covers both.
        let server = MockServer::start().await;
        let (result, scopes) = sign_in_with_push(&server, &[ADMIN, ADMIN]).await;
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::AdminConsentRequired);
        assert!(approval_scopes(&error).contains(&sync_receive));
        assert_eq!(scopes.len(), 2);

        // A sign-in the user canceled isn't started again.
        let (result, scopes) = sign_in_with_push(&server, &[CANCELED]).await;
        assert_eq!(result.unwrap_err().code, ErrorCode::SignInCancelled);
        assert_eq!(scopes.len(), 1);
    }

    /// Sends `GET target` over `stream`; returns the response.
    async fn http_get(stream: &mut TcpStream, target: &str) -> String {
        let request = format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response).await;
        response
    }

    #[tokio::test]
    async fn connections_from_the_first_authorization_never_carry_the_second() {
        const ADMIN: &str =
            "error=access_denied&error_description=AADSTS90094%3a+Admin+consent+required";
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .respond_with(token_success())
            .expect(1)
            .mount(&server)
            .await;
        // Plays a browser that, like real ones, keeps idle connections for
        // reuse by origin: during the first authorization it opens one to
        // the redirect URI that it doesn't use (a speculative connection),
        // and it sends a later request to that origin over it.
        let idle: Arc<Mutex<HashMap<String, TcpStream>>> = Arc::default();
        let redirects = Arc::new(StdMutex::new(Vec::new()));
        let pages = Arc::new(StdMutex::new(Vec::new()));
        let open = {
            let (redirects, pages) = (redirects.clone(), pages.clone());
            move |url: &str| -> Result<()> {
                let url = Url::parse(url).expect("authorize URL parses");
                let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
                let (redirect, state) = (params["redirect_uri"].clone(), params["state"].clone());
                let first = {
                    let mut redirects = redirects.lock().unwrap();
                    redirects.push(redirect.clone());
                    redirects.len() == 1
                };
                let (idle, pages) = (idle.clone(), pages.clone());
                tokio::spawn(async move {
                    let port = Url::parse(&redirect).unwrap().port().unwrap();
                    let connect = async || TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await;
                    let (answer, stream) = if first {
                        let spare = connect().await.unwrap();
                        idle.lock().await.insert(redirect, spare);
                        // The listener takes it in meanwhile.
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        (ADMIN, connect().await.unwrap())
                    } else {
                        let pooled = idle.lock().await.remove(&redirect);
                        let stream = match pooled {
                            Some(stream) => stream,
                            None => connect().await.unwrap(),
                        };
                        ("code=the-code", stream)
                    };
                    let mut stream = stream;
                    let page = http_get(&mut stream, &format!("/?{answer}&state={state}")).await;
                    pages.lock().unwrap().push(page);
                });
                Ok(())
            }
        };
        let config = MicrosoftAuthConfig {
            consent_scopes: vec![sync_scope(CLIENT_ID)],
            ..config(&server)
        };

        let result = sign_in(&config, &open, None, Duration::from_secs(5))
            .await
            .expect("signed in");
        let refused = result.consent_refused.expect("the refusal");
        assert_eq!(refused.code, ErrorCode::AdminConsentRequired);
        let pages = pages.lock().unwrap().clone();
        assert!(pages[0].starts_with("HTTP/1.1 200 "), "{}", pages[0]);
        assert!(pages[0].contains("<h1>Signing in without instant updates</h1>"));
        assert!(pages[1].starts_with("HTTP/1.1 200 "), "{}", pages[1]);
        assert!(pages[1].contains("You&#39;re signed in"), "{}", pages[1]);
        // The code is redeemed with the redirect URI it was issued for.
        let redirects = redirects.lock().unwrap().clone();
        assert_ne!(redirects[0], redirects[1]);
        let requests = server.received_requests().await.unwrap();
        let form: HashMap<String, String> = url::form_urlencoded::parse(&requests[0].body)
            .into_owned()
            .collect();
        assert_eq!(form["redirect_uri"], redirects[1]);
    }

    async fn sync_tokens(server: &MockServer) -> SyncTokenSource {
        let secrets = Arc::new(MemorySecretStore::default());
        secrets
            .set(&keys::refresh_token("acct"), "refresh-0")
            .await
            .unwrap();
        SyncTokenSource::new(config(server), "acct", secrets)
    }

    #[tokio::test]
    async fn push_service_tokens_have_their_own_scope_and_cache() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/common/oauth2/v2.0/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains(
                "scope=api%3A%2F%2Ftest-client-id%2FSync.Receive+offline_access",
            ))
            .respond_with(token_success())
            .expect(3)
            .mount(&server)
            .await;
        let tokens = sync_tokens(&server).await;
        let ten_minutes = Duration::from_secs(600);
        assert_eq!(
            tokens.sync_token(ten_minutes, false).await.unwrap(),
            "access-1"
        );
        // Cached while it has the time asked for...
        assert_eq!(
            tokens.sync_token(ten_minutes, false).await.unwrap(),
            "access-1"
        );
        // ...not when it hasn't (the token lasts an hour)...
        tokens
            .sync_token(Duration::from_secs(2 * 3600), false)
            .await
            .unwrap();
        // ...nor when a fresh one is asked for (`renew`, `token_expiring`).
        tokens.sync_token(ten_minutes, true).await.unwrap();
        // Rotation is saved for Graph too: they share the refresh token.
        assert_eq!(
            tokens
                .0
                .secrets
                .get(&keys::refresh_token("acct"))
                .await
                .unwrap()
                .as_deref(),
            Some("refresh-1")
        );
    }

    #[tokio::test]
    async fn refused_push_service_tokens_are_not_account_errors() {
        /// Microsoft's error, its description and code, and what it means.
        type Case = (&'static str, &'static str, u32, fn(&SyncTokenError) -> bool);
        let cases: &[Case] = &[
            (
                "invalid_grant",
                "AADSTS65001: The user or administrator has not consented to use the application with ID 'x' named 'Cumulas Mail'.",
                65001,
                |e| matches!(e, SyncTokenError::ConsentMissing),
            ),
            (
                "consent_required",
                "AADSTS65001: consent required.",
                65001,
                |e| matches!(e, SyncTokenError::ConsentMissing),
            ),
            (
                "interaction_required",
                "AADSTS50076: multi-factor authentication required.",
                50076,
                |e| matches!(e, SyncTokenError::ConsentMissing),
            ),
            (
                "invalid_scope",
                "AADSTS70011: The provided value for the input parameter 'scope' isn't valid.",
                70011,
                |e| matches!(e, SyncTokenError::NotOffered),
            ),
            (
                "invalid_resource",
                "AADSTS500011: The resource principal named api://x was not found in the tenant.",
                500011,
                |e| matches!(e, SyncTokenError::NotOffered),
            ),
            (
                "temporarily_unavailable",
                "AADSTS90033: A transient error has occurred.",
                90033,
                |e| matches!(e, SyncTokenError::Failed(e) if e.code == ErrorCode::Provider),
            ),
        ];
        for (error, description, code, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(token_error(error, description, &[*code]))
                .mount(&server)
                .await;
            let failure = sync_tokens(&server)
                .await
                .sync_token(Duration::from_secs(600), false)
                .await
                .unwrap_err();
            assert!(expected(&failure), "{error}: {failure:?}");
        }
    }

    #[tokio::test]
    async fn graph_tokens_can_be_asked_to_last() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("scope=openid+profile"))
            .respond_with(token_success())
            .expect(1)
            .mount(&server)
            .await;
        // A personal account's, which name the permissions.
        let source = graph_tokens(&server, true, &FakeGrant::with(None)).await;
        source
            .prime("twenty-minutes", Duration::from_secs(20 * 60))
            .await;
        assert_eq!(source.access_token().await.unwrap(), "twenty-minutes");
        assert_eq!(
            source
                .access_token_lasting(Duration::from_secs(45 * 60))
                .await
                .unwrap(),
            "access-1"
        );
        assert_eq!(source.access_token().await.unwrap(), "access-1");
    }

    /// A wall clock the test moves, as a suspend does; `Instant` stands
    /// still meanwhile.
    fn wall_clock() -> (Arc<StdMutex<SystemTime>>, WallClock) {
        let now = Arc::new(StdMutex::new(SystemTime::now()));
        let read = now.clone();
        (now, Arc::new(move || *read.lock().unwrap()))
    }

    #[tokio::test]
    async fn cached_tokens_expire_while_the_computer_sleeps() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(token_success())
            .mount(&server)
            .await;
        let (wall, clock) = wall_clock();
        let sleep = |time: Duration| *wall.lock().unwrap() += time;
        let tokens = SyncTokenSource(MicrosoftTokenSource {
            wall_clock: clock.clone(),
            ..sync_tokens(&server).await.0
        });
        let ten_minutes = Duration::from_secs(600);
        tokens.sync_token(ten_minutes, false).await.unwrap();
        tokens.sync_token(ten_minutes, false).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        // 50 minutes asleep: the token (an hour's) has less than the ten
        // minutes a stream needs, however little `Instant` has moved.
        sleep(Duration::from_secs(50 * 60));
        tokens.sync_token(ten_minutes, false).await.unwrap();
        // A night asleep: it has expired.
        sleep(Duration::from_secs(8 * 3600));
        tokens.sync_token(ten_minutes, false).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 3);

        // Graph's tokens likewise.
        let source = MicrosoftTokenSource {
            wall_clock: clock,
            ..token_source(&server, Some("refresh-0")).await
        };
        source
            .prime("before-the-night", Duration::from_secs(3600))
            .await;
        sleep(Duration::from_secs(8 * 3600));
        assert_eq!(source.access_token().await.unwrap(), "access-1");

        // Setting the wall clock back doesn't make a token last longer.
        source
            .prime("twenty-minutes", Duration::from_secs(20 * 60))
            .await;
        *wall.lock().unwrap() -= Duration::from_secs(2 * 3600);
        assert_eq!(
            source
                .access_token_lasting(Duration::from_secs(45 * 60))
                .await
                .unwrap(),
            "access-1"
        );
    }

    const TENANT: &str = "11111111-2222-3333-4444-555555555555";
    const SIGNATURE_SCOPE: &str =
        "api://signature.example.org/99999999-8888-7777-6666-555555555555/access_as_user";

    async fn api_tokens(server: &MockServer) -> ApiTokenSource {
        let secrets = Arc::new(MemorySecretStore::default());
        secrets
            .set(&keys::refresh_token("acct"), "refresh-0")
            .await
            .unwrap();
        let config = MicrosoftAuthConfig {
            tenant: TENANT.into(),
            ..config(server)
        };
        ApiTokenSource::new(config, "acct", secrets, SIGNATURE_SCOPE)
    }

    #[tokio::test]
    async fn api_tokens_come_from_the_accounts_tenant_with_their_own_scope_and_cache() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/{TENANT}/oauth2/v2.0/token")))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains(
                "scope=api%3A%2F%2Fsignature.example.org%2F99999999-8888-7777-6666-555555555555\
                 %2Faccess_as_user+offline_access",
            ))
            .respond_with(token_success())
            .expect(2)
            .mount(&server)
            .await;
        let tokens = api_tokens(&server).await;
        assert_eq!(tokens.api_token(false).await.unwrap(), "access-1");
        // Cached...
        assert_eq!(tokens.api_token(false).await.unwrap(), "access-1");
        // ...unless the API refused it.
        assert_eq!(tokens.api_token(true).await.unwrap(), "access-1");
        // Rotation is saved for Graph too: they share the refresh token.
        assert_eq!(
            tokens
                .0
                .secrets
                .get(&keys::refresh_token("acct"))
                .await
                .unwrap()
                .as_deref(),
            Some("refresh-1")
        );
    }

    #[tokio::test]
    async fn api_token_refusals_never_ask_to_sign_in_again() {
        /// Microsoft's error, its description and code, and what it means.
        type Case = (&'static str, &'static str, u32, fn(&ApiTokenError) -> bool);
        let not_in_tenant: fn(&ApiTokenError) -> bool = |e| matches!(e, ApiTokenError::NotInTenant);
        let not_allowed: fn(&ApiTokenError) -> bool = |e| matches!(e, ApiTokenError::NotAllowed);
        let later: fn(&ApiTokenError) -> bool = |e| {
            matches!(e, ApiTokenError::Failed(e) if e.code == ErrorCode::Provider
                && !e.message.contains("adele"))
        };
        let cases: &[Case] = &[
            (
                "invalid_resource",
                "AADSTS500011: The resource principal named api://signature.example.org/x was not found in the tenant named Contoso.",
                500011,
                not_in_tenant,
            ),
            (
                "invalid_client",
                "AADSTS650052: The app is trying to access a service that your organization lacks a service principal for.",
                650052,
                not_in_tenant,
            ),
            (
                "invalid_request",
                "AADSTS650057: Invalid resource. The client has requested access to a resource which isn't listed in the requested permissions.",
                650057,
                not_in_tenant,
            ),
            (
                "unauthorized_client",
                "AADSTS700016: Application with identifier 'x' was not found in the directory 'Contoso'.",
                700016,
                not_in_tenant,
            ),
            // Not "sign in again", as it would be for Graph.
            (
                "invalid_grant",
                "AADSTS65001: The user or administrator has not consented to use the application with ID 'x' named 'Cumulas Mail'.",
                65001,
                not_allowed,
            ),
            (
                "consent_required",
                "AADSTS65001: consent required.",
                65001,
                not_allowed,
            ),
            (
                "access_denied",
                "AADSTS65004: User declined to consent to access the app.",
                65004,
                not_allowed,
            ),
            ("consent_required", "Consent is required.", 0, not_allowed),
            (
                "interaction_required",
                "AADSTS50076: Due to a configuration change made by your administrator, you must use multi-factor authentication.",
                50076,
                later,
            ),
            (
                "interaction_required",
                "AADSTS50079: The user is required to use multi-factor authentication.",
                50079,
                later,
            ),
            (
                "invalid_grant",
                "AADSTS53003: Access has been blocked by Conditional Access policies.",
                53003,
                later,
            ),
            (
                "invalid_grant",
                "AADSTS700082: The refresh token has expired due to inactivity. adele@contoso.com",
                700082,
                later,
            ),
            (
                "invalid_grant",
                "AADSTS50173: The provided grant has expired due to it being revoked.",
                50173,
                later,
            ),
            (
                "temporarily_unavailable",
                "AADSTS90033: A transient error has occurred.",
                90033,
                later,
            ),
        ];
        for (error, description, code, expected) in cases {
            let server = MockServer::start().await;
            let codes: &[u32] = if *code == 0 { &[] } else { &[*code] };
            Mock::given(method("POST"))
                .respond_with(token_error(error, description, codes))
                .mount(&server)
                .await;
            let failure = api_tokens(&server)
                .await
                .api_token(false)
                .await
                .unwrap_err();
            assert!(expected(&failure), "{error} {code}: {failure:?}");
        }

        // No network, or a sign-in that's gone from the keyring: later too.
        let server = MockServer::start().await;
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let tokens = ApiTokenSource(MicrosoftTokenSource {
            config: MicrosoftAuthConfig {
                authority_host: format!("http://{}", closed.local_addr().unwrap()),
                ..api_tokens(&server).await.0.config
            },
            ..api_tokens(&server).await.0
        });
        drop(closed);
        assert!(matches!(
            tokens.api_token(false).await,
            Err(ApiTokenError::Failed(e)) if e.code == ErrorCode::Network
        ));
        tokens
            .0
            .secrets
            .delete(&keys::refresh_token("acct"))
            .await
            .unwrap();
        assert!(matches!(
            tokens.api_token(false).await,
            Err(ApiTokenError::Failed(e)) if e.code == ErrorCode::AuthRequired
        ));
    }

    #[tokio::test]
    async fn api_tokens_expire_while_the_computer_sleeps() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(token_success())
            .mount(&server)
            .await;
        let (wall, clock) = wall_clock();
        let tokens = ApiTokenSource(MicrosoftTokenSource {
            wall_clock: clock,
            ..api_tokens(&server).await.0
        });
        tokens.api_token(false).await.unwrap();
        tokens.api_token(false).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        *wall.lock().unwrap() += Duration::from_secs(8 * 3600);
        tokens.api_token(false).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[test]
    fn aadsts_codes_are_extracted() {
        assert_eq!(
            aadsts_codes("AADSTS90094: x\r\nsee AADSTS65001 and AADSTS"),
            vec![90094, 65001]
        );
        assert!(aadsts_codes("nothing").is_empty());
    }

    #[test]
    fn id_token_claims_are_decoded() {
        let token = id_token(json!({"tid": "t", "oid": "o", "email": "a@b.c"}));
        let claims = decode_id_token(&token).unwrap();
        assert_eq!(claims.tid.as_deref(), Some("t"));
        assert_eq!(claims.email.as_deref(), Some("a@b.c"));
        assert!(decode_id_token("not-a-jwt").is_err());
    }
}
