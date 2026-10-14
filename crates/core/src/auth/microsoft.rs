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
use std::time::{Duration, Instant};

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

use super::TokenSource;
use crate::error::{Error, ErrorCode, Result};
use crate::html::escape_html;
use crate::secrets::{SecretStore, keys};

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

/// Microsoft Entra ID's public cloud authority.
pub const DEFAULT_AUTHORITY_HOST: &str = "https://login.microsoftonline.com";

/// Redirect URI of the admin consent link. It must be registered on the app
/// (it is the same `http://localhost` entry used for sign-in). Nothing listens
/// there once the administrator has granted consent, so their browser ends on
/// a "can't connect" page; consent has been recorded by then.
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

/// AADSTS65004 UserDeclinedConsent. Also sent when the user picks "Back to
/// app" after submitting an approval request in the admin consent workflow.
const USER_DECLINED_CODE: u32 = 65004;

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
}

impl MicrosoftAuthConfig {
    /// Config for the public cloud; `tenant` defaults to `common` (work,
    /// school and personal accounts).
    pub fn new(client_id: impl Into<String>, tenant: Option<String>) -> Self {
        Self {
            client_id: client_id.into(),
            tenant: tenant.unwrap_or_else(|| "common".into()),
            authority_host: DEFAULT_AUTHORITY_HOST.into(),
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
}

/// Runs the interactive sign-in: starts a loopback listener on 127.0.0.1
/// (and ::1 when available), calls `open_browser` with the authorization
/// URL, waits for the redirect (up to `timeout`) and redeems the code.
///
/// Errors: `AdminConsentRequired` (with `details.adminConsentUrl`) when the
/// tenant blocks user consent, `ConsentDeclined`, `SignInCancelled`.
pub async fn sign_in(
    config: &MicrosoftAuthConfig,
    open_browser: &(dyn Fn(&str) -> Result<()> + Send + Sync),
    login_hint: Option<&str>,
    timeout: Duration,
) -> Result<SignInResult> {
    let listener = LoopbackListener::bind().await?;
    let redirect_uri = format!("http://localhost:{}", listener.port);
    let pkce = Pkce::generate();
    let state = random_token(16);
    let url = authorize_url(config, &redirect_uri, &state, &pkce.challenge, login_hint)?;
    open_browser(&url)?;

    let response = tokio::time::timeout(timeout, listener.wait_for_response(&state))
        .await
        .map_err(|_| {
            Error::new(
                ErrorCode::SignInCancelled,
                "Sign-in was not completed in time. Try again.",
            )
        })?;
    let code = match response.code {
        Some(code) if response.error.is_none() => code,
        _ => {
            let error = response.error.unwrap_or_else(|| "invalid_request".into());
            let description = response.error_description.unwrap_or_default();
            return Err(sign_in_error(
                config,
                login_hint,
                &OAuthError {
                    error,
                    error_description: description,
                    error_codes: Vec::new(),
                    error_subcode: response.error_subcode,
                },
            ));
        }
    };

    let http = http_client();
    let scope = SCOPES.join(" ");
    let tokens = request_token(
        &http,
        config,
        &[
            ("client_id", config.client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_verifier", pkce.verifier.as_str()),
            ("scope", scope.as_str()),
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
    Ok(SignInResult {
        email,
        display_name,
        tenant_id,
        user_id,
        access_token: tokens.access_token,
        refresh_token,
        expires_in: Duration::from_secs(tokens.expires_in),
    })
}

/// URL an administrator can open to grant tenant-wide consent.
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
    let scope = SCOPES
        .iter()
        .map(|s| {
            if OIDC_SCOPES.contains(s) {
                (*s).to_owned()
            } else {
                format!("{GRAPH_RESOURCE}/{s}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &config.client_id)
        .append_pair("scope", &scope)
        .append_pair("redirect_uri", ADMIN_CONSENT_REDIRECT_URI)
        .finish();
    format!(
        "{}/{tenant}/v2.0/adminconsent?{query}",
        config.authority_host.trim_end_matches('/')
    )
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
    /// Held across the refresh so concurrent callers wait for one refresh
    /// instead of each redeeming the refresh token.
    cache: Mutex<Option<CachedToken>>,
}

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

impl CachedToken {
    fn new(access_token: &str, expires_in: Duration) -> Self {
        let now = Instant::now();
        Self {
            access_token: access_token.to_owned(),
            expires_at: now.checked_add(expires_in).unwrap_or(now),
        }
    }

    fn is_fresh(&self) -> bool {
        self.expires_at
            .checked_duration_since(Instant::now())
            .is_some_and(|left| left > REFRESH_MARGIN)
    }
}

impl MicrosoftTokenSource {
    /// Token source for `account_id`, whose refresh token is stored under
    /// [`keys::refresh_token`].
    pub fn new(
        config: MicrosoftAuthConfig,
        account_id: impl Into<String>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            config,
            account_id: account_id.into(),
            secrets,
            http: http_client(),
            cache: Mutex::new(None),
        }
    }

    /// Seeds the in-memory cache right after sign-in.
    pub async fn prime(&self, access_token: &str, expires_in: Duration) {
        *self.cache.lock().await = Some(CachedToken::new(access_token, expires_in));
    }
}

#[async_trait]
impl TokenSource for MicrosoftTokenSource {
    async fn access_token(&self) -> Result<String> {
        let mut cache = self.cache.lock().await;
        if let Some(token) = cache.as_ref()
            && token.is_fresh()
        {
            return Ok(token.access_token.clone());
        }

        let key = keys::refresh_token(&self.account_id);
        let refresh_token = self.secrets.get(&key).await?.ok_or_else(|| {
            Error::auth_required(
                "The Microsoft sign-in for this account is missing. Sign in again.",
            )
        })?;
        let scope = SCOPES.join(" ");
        let tokens = request_token(
            &self.http,
            &self.config,
            &[
                ("client_id", self.config.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("scope", scope.as_str()),
            ],
        )
        .await
        .map_err(|e| match e {
            TokenFailure::OAuth(e) => refresh_error(&e),
            TokenFailure::Other(e) => e,
        })?;

        // Microsoft may rotate the refresh token; the new one replaces the old.
        // The old one keeps working until it expires, so failing to store the
        // new one is not fatal.
        if let Some(rotated) = tokens.refresh_token.as_deref()
            && rotated != refresh_token
            && let Err(e) = self.secrets.set(&key, rotated).await
        {
            tracing::warn!("could not store the rotated refresh token: {e}");
        }
        let token = CachedToken::new(&tokens.access_token, Duration::from_secs(tokens.expires_in));
        let access_token = token.access_token.clone();
        *cache = Some(token);
        Ok(access_token)
    }

    async fn invalidate(&self) {
        *self.cache.lock().await = None;
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

fn authorize_url(
    config: &MicrosoftAuthConfig,
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
            .append_pair("scope", &SCOPES.join(" "))
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
    async fn wait_for_response(&self, state: &str) -> AuthorizationResponse {
        let (tx, mut rx) = mpsc::channel(1);
        loop {
            tokio::select! {
                accepted = self.accept() => match accepted {
                    Ok(stream) => {
                        let tx = tx.clone();
                        let state = state.to_owned();
                        tokio::spawn(async move {
                            if let Some(response) = handle_connection(stream, &state).await {
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

async fn handle_connection(mut stream: TcpStream, state: &str) -> Option<AuthorizationResponse> {
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
            let page = match &response.error {
                None => result_page(
                    "You're signed in",
                    "You can close this tab and return to Cumulas Mail.",
                ),
                Some(error) => result_page(
                    "Sign-in did not complete",
                    &format!(
                        "{} You can close this tab and return to Cumulas Mail.",
                        response
                            .error_description
                            .as_deref()
                            .map(first_line)
                            .filter(|d| !d.is_empty())
                            .unwrap_or(error)
                    ),
                ),
            };
            let _ = respond(&mut stream, "200 OK", &page).await;
            Some(response)
        }
    }
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

/// Maps an error from the authorization redirect or the code redemption.
fn sign_in_error(
    config: &MicrosoftAuthConfig,
    login_hint: Option<&str>,
    error: &OAuthError,
) -> Error {
    let canceled = error
        .error_subcode
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("cancel"));
    if ADMIN_CONSENT_CODES.iter().any(|c| error.has_code(*c))
        || (error.error == "consent_required" && !error.has_code(USER_DECLINED_CODE))
    {
        let hint = login_hint.and_then(|h| h.rsplit_once('@')).map(|(_, d)| d);
        let tenant = admin_consent_tenant(config, hint);
        // With the admin consent workflow enabled, Microsoft's page lets the
        // user send an approval request themselves.
        let can_request_approval = error.has_code(ADMIN_CONSENT_WORKFLOW_CODE);
        let message = if can_request_approval {
            "Your organization requires an administrator to approve Cumulas Mail before \
             you can sign in. You can request approval on Microsoft's sign-in page, or send \
             the approval link to your IT department."
        } else {
            "Your organization requires an administrator to approve Cumulas Mail before \
             you can sign in. Send the approval link to your IT department."
        };
        return Error::new(ErrorCode::AdminConsentRequired, message).with_details(json!({
            "adminConsentUrl": admin_consent_url(config, Some(&tenant)),
            "tenant": tenant,
            "canRequestApproval": can_request_approval,
        }));
    }
    if error.has_code(USER_DECLINED_CODE) {
        return consent_declined();
    }
    if error.error == "access_denied" {
        // Microsoft's own example of a canceled sign-in carries no subcode
        // but says so in the description.
        if canceled
            || error
                .error_description
                .to_ascii_lowercase()
                .contains("cancel")
        {
            return Error::new(ErrorCode::SignInCancelled, "Sign-in was canceled.");
        }
        return consent_declined();
    }
    Error::provider(error.summary()).with_details(json!({ "error": error.error }))
}

fn consent_declined() -> Error {
    Error::new(
        ErrorCode::ConsentDeclined,
        "Cumulas Mail needs the requested permissions to show your mail, calendar, contacts \
         and tasks. If you asked your administrator for approval, sign in again once it has \
         been granted.",
    )
}

/// Maps a failed refresh. `invalid_grant` covers expired, revoked or
/// password-invalidated refresh tokens and withdrawn consent;
/// `interaction_required` new MFA or Conditional Access requirements. Either
/// way only an interactive sign-in helps.
fn refresh_error(error: &OAuthError) -> Error {
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
    use crate::secrets::MemorySecretStore;

    const CLIENT_ID: &str = "test-client-id";

    fn config(server: &MockServer) -> MicrosoftAuthConfig {
        MicrosoftAuthConfig {
            client_id: CLIENT_ID.into(),
            tenant: "common".into(),
            authority_host: server.uri(),
        }
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
        assert_eq!(params["scope"], SCOPES.join(" "));
        assert_eq!(params["state"], "st");
        assert_eq!(params["code_challenge"], "ch");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["prompt"], "select_account");
        assert_eq!(params["login_hint"], "adele@contoso.com");

        let without_hint = authorize_url(&config, "http://localhost:1", "s", "c", None).unwrap();
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
        assert_eq!(scopes.len(), SCOPES.len());

        // Without a usable hint: the configured tenant if specific, else
        // `organizations` (`common` is not accepted by the endpoint).
        assert!(admin_consent_url(&config, None).contains("/organizations/v2.0/adminconsent?"));
        assert!(admin_consent_url(&config, Some("bad/tenant")).contains("/organizations/"));
        let specific = MicrosoftAuthConfig::new(CLIENT_ID, Some("fabrikam.onmicrosoft.com".into()));
        assert!(admin_consent_url(&specific, None).contains("/fabrikam.onmicrosoft.com/v2.0/"));
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
                let details = error.details.expect("admin consent details");
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
            if *expected == ErrorCode::Provider {
                assert!(
                    error.message.starts_with("AADSTS50105"),
                    "{}",
                    error.message
                );
            }
            // The browser is told how it went.
            assert_eq!(browser.wait_for(1).await, vec![200]);
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
        MicrosoftTokenSource::new(config(server), "acct", secrets)
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
