//! The InvokeCommand client: runs Exchange Online and Security & Compliance
//! cmdlets over the REST interface the ExchangeOnlineManagement module uses
//! (`POST /adminapi/beta/{tenant}/InvokeCommand`), which Microsoft doesn't
//! document for other apps.
//!
//! Requests go only where [`HostRules`] allow, redirects are followed by
//! hand (the same POST, with the token, to an allowed origin), throttling is
//! retried with Graph's policy, and every call goes to the run log. Tokens,
//! cookies, parameter values and response bodies never go to `tracing`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, LOCATION, RETRY_AFTER, SET_COOKIE};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use url::{Origin, Url};

use super::runlog::{CallRecord, MAX_EXCERPT, MAX_MESSAGE, RunLog};
use super::script::render_value;
use crate::auth::admin_setup::{AdminTokens, Resource};
use crate::error::{Error, ErrorCode, Result};
use crate::protection::clean_value;
use crate::providers::graph::{MAX_RETRIES, is_retryable, retry_delay};

/// Microsoft's hosts InvokeCommand requests go to, and their subdomains
/// (regional Security & Compliance hosts, redirects).
const MICROSOFT_HOSTS: &[&str] = &[
    "outlook.office365.com",
    "ps.compliance.protection.outlook.com",
];

/// Who's calling, in `X-ClientApplication` and the user agent.
pub const CLIENT_APPLICATION: &str = concat!("CumulasMail/", env!("CARGO_PKG_VERSION"));
/// The mailbox every organization has, for anchoring an administrator who
/// has none of their own.
pub const SYSTEM_MAILBOX: &str = "SystemMailbox{bb558c35-97f1-4cb9-8ff7-d53741dc928c}";
/// Most pages of one listing.
const MAX_PAGES: usize = 20;
/// Most redirects one request follows.
const MAX_HOPS: usize = 3;
/// How long a read waits before trying again after a network error.
const NETWORK_RETRY: Duration = Duration::from_secs(2);
/// Microsoft's REST cmdlets may run 15 minutes; a slower answer than this
/// is treated as a failure (and, for writes, followed by a re-list).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Where InvokeCommand requests, their redirects and their next pages may
/// go, which receive the administrator's token. Decided on the origin
/// (scheme, host, port), like `GraphClient::is_own_link`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRules {
    /// Another origin allowed besides Microsoft's (a mock server's, in
    /// tests).
    also: Option<Origin>,
}

impl HostRules {
    /// Microsoft's only: https on port 443, to Exchange Online and Security
    /// & Compliance hosts.
    pub fn production() -> Self {
        Self { also: None }
    }

    /// Microsoft's, and `url`'s origin (a mock server's, in tests).
    pub fn also(url: &Url) -> Self {
        Self {
            also: Some(url.origin()),
        }
    }

    /// Whether a request may go to `url`.
    pub fn allows(&self, url: &Url) -> bool {
        if !url.username().is_empty() || url.password().is_some() {
            return false;
        }
        if self.also.as_ref().is_some_and(|o| *o == url.origin()) {
            return true;
        }
        if url.scheme() != "https" || url.port_or_known_default() != Some(443) {
            return false;
        }
        let Some(url::Host::Domain(host)) = url.host() else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        MICROSOFT_HOSTS.iter().any(|base| {
            host == *base
                || host
                    .strip_suffix(base)
                    .is_some_and(|sub| sub.len() > 1 && sub.ends_with('.'))
        })
    }
}

/// The client InvokeCommand requests go out on: its own, so redirects are
/// never followed for us (they would drop the token and turn the POST into
/// a GET), with Microsoft's timeouts. reqwest decompresses gzip itself; we
/// never set `Accept-Encoding`.
pub fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .user_agent(CLIENT_APPLICATION)
        .build()
        .map_err(|e| Error::internal(format!("could not set up the setup client: {e}")))
}

/// Which mailbox a request is routed by (`X-AnchorMailbox`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// `UPN:<sign-in name>`: the administrator's own mailbox.
    Upn(String),
    /// Exchange: `APP:SystemMailbox{…}@<tenant id>`. Security & Compliance:
    /// `UPN:SystemMailbox{…}@<initial domain>`. For administrators without
    /// a mailbox, as CIPP does.
    SystemMailbox(String),
}

impl Anchor {
    /// Exchange Online's anchors, in order of preference.
    pub fn exchange(username: &str, tenant_id: &str) -> Vec<Anchor> {
        vec![
            Anchor::Upn(format!("UPN:{username}")),
            Anchor::SystemMailbox(format!("APP:{SYSTEM_MAILBOX}@{tenant_id}")),
        ]
    }

    /// Security & Compliance's anchors, in order of preference: the system
    /// mailbox only once the organization's initial domain is known.
    pub fn compliance(username: &str, initial_domain: Option<&str>) -> Vec<Anchor> {
        let mut anchors = vec![Anchor::Upn(format!("UPN:{username}"))];
        if let Some(domain) = initial_domain {
            anchors.push(Anchor::SystemMailbox(format!(
                "UPN:{SYSTEM_MAILBOX}@{domain}"
            )));
        }
        anchors
    }

    /// For the run log and `tracing`: "UPN" or "SystemMailbox".
    pub fn kind(&self) -> &'static str {
        match self {
            Anchor::Upn(_) => "UPN",
            Anchor::SystemMailbox(_) => "SystemMailbox",
        }
    }

    pub fn value(&self) -> &str {
        match self {
            Anchor::Upn(value) | Anchor::SystemMailbox(value) => value,
        }
    }
}

/// Whether a call reads or writes; only reads are retried after a server
/// error, or raced against the session's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    Read,
    Write,
}

/// One cmdlet to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Cmdlet {
    /// "Get-TransportRule".
    pub name: &'static str,
    /// Only the parameters we set.
    pub params: Map<String, Value>,
    pub kind: CallKind,
    /// "Read mail flow rules", "Create “Cumulas Mail: Encrypt”".
    pub step: String,
    /// Whether the parameter values may go into the run log: true for calls
    /// on Cumulas Mail's own objects; false for reads that name nothing of
    /// ours.
    pub log_values: bool,
}

impl Cmdlet {
    /// A read that names nothing of ours.
    pub fn read(name: &'static str, step: impl Into<String>) -> Self {
        Self {
            name,
            params: Map::new(),
            kind: CallKind::Read,
            step: step.into(),
            log_values: false,
        }
    }

    /// A write (on ours).
    pub fn write(name: &'static str, step: impl Into<String>) -> Self {
        Self {
            name,
            params: Map::new(),
            kind: CallKind::Write,
            step: step.into(),
            log_values: true,
        }
    }

    /// The same with `key` set to `value`.
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.params.insert(key.to_owned(), value.into());
        self
    }

    /// The `Identity` it addresses, if any.
    pub fn identity(&self) -> Option<&str> {
        self.params.get("Identity").and_then(Value::as_str)
    }

    /// The `Name` it creates, if any.
    pub fn new_name(&self) -> Option<&str> {
        self.params.get("Name").and_then(Value::as_str)
    }
}

/// What a cmdlet returned.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InvokeOutput {
    /// `value`, or the single object returned, or nothing.
    pub values: Vec<Value>,
    /// `@adminapi.warnings`.
    pub warnings: Vec<String>,
}

/// What a failed response said, read defensively: the error format isn't
/// documented.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ErrorDetail {
    pub status: Option<u16>,
    /// `error.code`.
    pub code: Option<String>,
    /// From `details[].message` or `X-ExceptionType`: a hint only.
    pub exception_type: Option<String>,
    /// The best message, one line, at most 500 characters.
    pub message: String,
    /// The body, at most 2,000 characters, control characters removed.
    pub excerpt: String,
}

impl ErrorDetail {
    fn text(message: &str) -> Self {
        Self {
            message: one_line(message, MAX_MESSAGE),
            ..Self::default()
        }
    }
}

/// Why a cmdlet didn't run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvokeError {
    /// 401 after one refresh (and, for reads, every anchor); or no token.
    Unauthorized(ErrorDetail),
    /// 403, or wording about roles.
    Forbidden(ErrorDetail),
    /// Security & Compliance: wording about licenses.
    LicenseRequired(ErrorDetail),
    /// 429, or Exchange's concurrency text, after retries.
    Throttled(ErrorDetail),
    AlreadyExists(ErrorDetail),
    NotFound(ErrorDetail),
    /// Any other failure response.
    Rejected(ErrorDetail),
    /// The request didn't get an answer (reqwest's error, without the URL).
    Network(String),
    /// To an origin not allowed (its host), or more than three hops.
    Redirect(String),
    /// A success response that isn't the envelope.
    Unexpected(String),
    /// The session ended.
    Cancelled,
}

impl InvokeError {
    /// What the response said, when there was one.
    pub fn detail(&self) -> Option<&ErrorDetail> {
        match self {
            InvokeError::Unauthorized(d)
            | InvokeError::Forbidden(d)
            | InvokeError::LicenseRequired(d)
            | InvokeError::Throttled(d)
            | InvokeError::AlreadyExists(d)
            | InvokeError::NotFound(d)
            | InvokeError::Rejected(d) => Some(d),
            _ => None,
        }
    }

    /// One line about it, for the run log and the admin.
    pub fn message(&self) -> String {
        match self {
            InvokeError::Network(m) | InvokeError::Redirect(m) | InvokeError::Unexpected(m) => {
                m.clone()
            }
            InvokeError::Cancelled => "The setup window was closed.".into(),
            other => other
                .detail()
                .map(|d| d.message.clone())
                .unwrap_or_default(),
        }
    }

    pub fn status(&self) -> Option<u16> {
        self.detail().and_then(|d| d.status)
    }
}

/// Where a client sends its cmdlets.
#[derive(Debug, Clone)]
pub struct InvokeTarget {
    pub resource: Resource,
    /// `https://outlook.office365.com`, or the Security & Compliance
    /// regional host.
    pub base: Url,
    pub tenant_id: String,
    /// In order of preference.
    pub anchors: Vec<Anchor>,
}

/// Runs cmdlets for one resource of a setup session, one at a time.
pub struct InvokeClient {
    http: reqwest::Client,
    target: InvokeTarget,
    tokens: Arc<AdminTokens>,
    /// The anchor that first got an answer; used from then on.
    pinned: Mutex<Option<usize>>,
    /// Affinity cookies by origin: (origin, name, value), this session only.
    cookies: Mutex<Vec<(String, String, String)>>,
    hosts: HostRules,
    log: RunLog,
    cancel: CancellationToken,
}

/// One call's progress through its attempts.
struct Call<'c> {
    cmdlet: &'c Cmdlet,
    body: Vec<u8>,
    /// The token was refreshed after a 401 once already.
    refreshed: bool,
    record: CallRecord,
}

impl InvokeClient {
    pub fn new(
        http: reqwest::Client,
        target: InvokeTarget,
        tokens: Arc<AdminTokens>,
        hosts: HostRules,
        log: RunLog,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            http,
            target,
            tokens,
            pinned: Mutex::new(None),
            cookies: Mutex::default(),
            hosts,
            log,
            cancel,
        }
    }

    pub fn resource(&self) -> Resource {
        self.target.resource
    }

    /// The host requests go to, for the run log.
    pub fn host(&self) -> String {
        host_of(&self.target.base)
    }

    /// The anchor that got the first answer, once one did.
    pub fn pinned_anchor(&self) -> Option<&'static str> {
        let pinned = *self.pinned.lock().unwrap();
        pinned.map(|i| self.target.anchors[i].kind())
    }

    fn endpoint(&self) -> Url {
        let mut url = self.target.base.clone();
        url.set_query(None);
        url.set_fragment(None);
        if let Ok(mut segments) = url.path_segments_mut() {
            segments
                .clear()
                .push("adminapi")
                .push("beta")
                .push(&self.target.tenant_id)
                .push("InvokeCommand");
        }
        url
    }

    /// Runs `cmdlet`, following its pages, and logs it.
    pub async fn invoke(&self, cmdlet: &Cmdlet) -> std::result::Result<InvokeOutput, InvokeError> {
        let started = Instant::now();
        let mut record = CallRecord::new(&cmdlet.step, &self.host(), cmdlet.name);
        record.parameters = ordered_params(&cmdlet.params)
            .map(|(name, value)| {
                let value = cmdlet
                    .log_values
                    .then(|| clean_value(&render_value(value), MAX_MESSAGE).unwrap_or_default());
                (name.clone(), value)
            })
            .collect();
        let body = json!({
            "CmdletInput": { "CmdletName": cmdlet.name, "Parameters": cmdlet.params }
        });
        let mut call = Call {
            cmdlet,
            body: serde_json::to_vec(&body).unwrap_or_default(),
            refreshed: false,
            record,
        };
        let result = self.pages(&mut call).await;
        let mut record = call.record;
        record.ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match &result {
            Ok(output) => {
                record.results = Some(output.values.len());
                record.warnings = output
                    .warnings
                    .iter()
                    .filter_map(|w| clean_value(w, MAX_MESSAGE))
                    .collect();
            }
            Err(e) => {
                record.error = Some(e.detail().cloned().unwrap_or_else(|| ErrorDetail {
                    message: one_line(&e.message(), MAX_MESSAGE),
                    ..ErrorDetail::default()
                }));
            }
        }
        tracing::info!(
            target: "orgsetup",
            cmdlet = cmdlet.name,
            host = %record.host,
            status = ?record.status,
            ms = record.ms,
            anchor = record.anchor.unwrap_or("none"),
            "InvokeCommand"
        );
        tracing::debug!(
            target: "orgsetup",
            cmdlet = cmdlet.name,
            parameters = ?cmdlet.params.keys().collect::<Vec<_>>(),
            "InvokeCommand parameters"
        );
        self.log.call(record);
        result
    }

    async fn pages(&self, call: &mut Call<'_>) -> std::result::Result<InvokeOutput, InvokeError> {
        let mut output = InvokeOutput::default();
        let mut url = self.endpoint();
        for _ in 0..MAX_PAGES {
            let body = self.anchored(call, &url).await?;
            let next = read_envelope(body, &mut output)?;
            let Some(next) = next else {
                return Ok(output);
            };
            url = match Url::parse(&next).or_else(|_| url.join(&next)) {
                Ok(next) if self.hosts.allows(&next) && next.path().starts_with("/adminapi/") => {
                    next
                }
                Ok(next) => return Err(InvokeError::Redirect(host_of(&next))),
                Err(_) => {
                    return Err(InvokeError::Unexpected(
                        "Microsoft 365 sent a next page link that couldn't be read.".into(),
                    ));
                }
            };
        }
        Err(InvokeError::Unexpected("too many pages".into()))
    }

    /// Sends the call with the pinned anchor, or until one is pinned, each
    /// anchor in turn while the answer suggests the anchor is at fault.
    /// Writes never switch anchors: they run after a read pinned one.
    async fn anchored(
        &self,
        call: &mut Call<'_>,
        url: &Url,
    ) -> std::result::Result<Value, InvokeError> {
        let pinned = *self.pinned.lock().unwrap();
        if let Some(i) = pinned {
            return self.send(call, url, &self.target.anchors[i]).await;
        }
        if call.cmdlet.kind == CallKind::Write {
            return self.send(call, url, &self.target.anchors[0]).await;
        }
        let mut first: Option<InvokeError> = None;
        for (i, anchor) in self.target.anchors.iter().enumerate() {
            match self.send(call, url, anchor).await {
                Ok(body) => {
                    *self.pinned.lock().unwrap() = Some(i);
                    return Ok(body);
                }
                Err(e) => {
                    let last = i + 1 == self.target.anchors.len();
                    if last || !anchor_at_fault(&e, anchor) {
                        let denied = |e: &InvokeError| {
                            matches!(e, InvokeError::Unauthorized(_) | InvokeError::Forbidden(_))
                        };
                        return Err(match first {
                            Some(first) if denied(&first) && denied(&e) => first,
                            _ => e,
                        });
                    }
                    first.get_or_insert(e);
                }
            }
        }
        Err(first.unwrap_or(InvokeError::Unexpected("no anchor to send with".into())))
    }

    /// One anchor's attempt: retries, the token's one refresh, redirects.
    async fn send(
        &self,
        call: &mut Call<'_>,
        start: &Url,
        anchor: &Anchor,
    ) -> std::result::Result<Value, InvokeError> {
        let read = call.cmdlet.kind == CallKind::Read;
        let mut url = start.clone();
        let mut hops = 0;
        let mut retries = 0;
        let mut network_retried = false;
        loop {
            if !self.hosts.allows(&url) {
                return Err(InvokeError::Redirect(host_of(&url)));
            }
            let token = self
                .race(read, self.tokens.token(self.target.resource))
                .await?;
            let token = token.map_err(token_error)?;
            let mut request = self
                .http
                .post(url.clone())
                .bearer_auth(token)
                .header(CONTENT_TYPE, "application/json; charset=utf-8")
                .header(ACCEPT, "application/json")
                .header("Prefer", "odata.maxpagesize=1000")
                .header("X-AnchorMailbox", anchor.value())
                .header("X-CmdletName", call.cmdlet.name)
                .header("X-ClientApplication", CLIENT_APPLICATION)
                .header("client-request-id", uuid::Uuid::new_v4().to_string())
                .body(call.body.clone());
            if let Some(cookies) = self.cookie_header(&url) {
                request = request.header(COOKIE, cookies);
            }
            let sent = self.race(read, request.send()).await?;
            let response = match sent {
                Ok(response) => response,
                Err(e) => {
                    call.record.attempts.push((Some(anchor.kind()), None));
                    if read && !network_retried {
                        network_retried = true;
                        self.race(read, tokio::time::sleep(NETWORK_RETRY)).await?;
                        continue;
                    }
                    return Err(InvokeError::Network(e.without_url().to_string()));
                }
            };
            let status = response.status();
            call.record
                .attempts
                .push((Some(anchor.kind()), Some(status.as_u16())));
            call.record.status = Some(status.as_u16());
            call.record.anchor = Some(anchor.kind());
            self.keep_cookies(&url, &response);

            if status.is_redirection() {
                hops += 1;
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|l| url.join(l).ok());
                let Some(mut next) = location else {
                    return Err(InvokeError::Redirect(
                        "a redirect without a location".into(),
                    ));
                };
                rewrite_location(&mut next);
                if hops > MAX_HOPS {
                    return Err(InvokeError::Redirect(format!(
                        "{} (more than {MAX_HOPS} redirects)",
                        host_of(&next)
                    )));
                }
                if !self.hosts.allows(&next) {
                    return Err(InvokeError::Redirect(host_of(&next)));
                }
                call.record.redirects.push(host_of(&next));
                url = next;
                continue;
            }

            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let exception_type = response
                .headers()
                .get("X-ExceptionType")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let bytes = self.race(read, response.bytes()).await?;
            let bytes = match bytes {
                Ok(bytes) => bytes,
                Err(e) => {
                    return Err(InvokeError::Network(e.without_url().to_string()));
                }
            };
            if status.is_success() {
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    return Ok(Value::Null);
                }
                return serde_json::from_slice(&bytes).map_err(|_| {
                    InvokeError::Unexpected(
                        "Microsoft 365 answered with something other than JSON.".into(),
                    )
                });
            }

            let detail = error_detail(status.as_u16(), &bytes, exception_type.as_deref());
            if status == StatusCode::UNAUTHORIZED && !call.refreshed {
                call.refreshed = true;
                self.tokens.invalidate(self.target.resource).await;
                continue;
            }
            let busy = is_concurrency_limit(&detail);
            let retryable =
                busy || (is_retryable(status) && (read || status == StatusCode::TOO_MANY_REQUESTS));
            if retryable && retries < MAX_RETRIES {
                let delay = retry_delay(retry_after.as_deref(), retries);
                retries += 1;
                self.race(read, tokio::time::sleep(delay)).await?;
                continue;
            }
            return Err(classify(status.as_u16(), detail, self.target.resource));
        }
    }

    /// `future`, or `Cancelled` when the session ends first (reads only:
    /// a write is never left half-way).
    async fn race<T>(
        &self,
        read: bool,
        future: impl Future<Output = T>,
    ) -> std::result::Result<T, InvokeError> {
        if !read {
            return Ok(future.await);
        }
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(InvokeError::Cancelled),
            output = future => Ok(output),
        }
    }

    fn cookie_header(&self, url: &Url) -> Option<String> {
        let origin = url.origin().ascii_serialization();
        let cookies = self.cookies.lock().unwrap();
        let pairs: Vec<String> = cookies
            .iter()
            .filter(|(o, _, _)| *o == origin)
            .map(|(_, name, value)| format!("{name}={value}"))
            .collect();
        (!pairs.is_empty()).then(|| pairs.join("; "))
    }

    fn keep_cookies(&self, url: &Url, response: &reqwest::Response) {
        let origin = url.origin().ascii_serialization();
        let mut cookies = self.cookies.lock().unwrap();
        for header in response.headers().get_all(SET_COOKIE) {
            let Ok(header) = header.to_str() else {
                continue;
            };
            let pair = header.split(';').next().unwrap_or_default();
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            if name.is_empty() {
                continue;
            }
            cookies.retain(|(o, n, _)| !(*o == origin && n == name));
            cookies.push((origin.clone(), name.to_owned(), value.to_owned()));
        }
    }
}

/// A cmdlet's parameters as people read them: `Identity` and `Name` first,
/// then the others.
pub fn ordered_params(params: &Map<String, Value>) -> impl Iterator<Item = (&String, &Value)> {
    const FIRST: [&str; 2] = ["Identity", "Name"];
    FIRST
        .iter()
        .filter_map(|k| params.get_key_value(*k))
        .chain(params.iter().filter(|(k, _)| !FIRST.contains(&k.as_str())))
}

/// Whether an unpinned read should try the next anchor after `error` with
/// `anchor`: answers that suggest the mailbox it names is the trouble.
fn anchor_at_fault(error: &InvokeError, anchor: &Anchor) -> bool {
    let mentions = error.detail().is_some_and(|d| {
        let text = format!("{} {}", d.message, d.excerpt).to_ascii_lowercase();
        text.contains("anchor") || text.contains("mailbox")
    });
    match error {
        InvokeError::Rejected(d) => mentions || matches!(d.status, Some(400 | 404 | 500 | 502)),
        InvokeError::NotFound(_) => true,
        InvokeError::Unauthorized(_) | InvokeError::Forbidden(_) => {
            mentions || matches!(anchor, Anchor::Upn(_))
        }
        _ => mentions,
    }
}

/// A token that couldn't be had, as InvokeCommand's answer would be.
fn token_error(error: Error) -> InvokeError {
    match error.code {
        ErrorCode::Network => InvokeError::Network(error.message),
        _ => InvokeError::Unauthorized(ErrorDetail::text(&error.message)),
    }
}

/// Reads a success response into `output`; returns the next page's link.
fn read_envelope(
    body: Value,
    output: &mut InvokeOutput,
) -> std::result::Result<Option<String>, InvokeError> {
    let mut object = match body {
        Value::Null => return Ok(None),
        Value::Object(object) => object,
        _ => {
            return Err(InvokeError::Unexpected(
                "Microsoft 365 answered with something other than an object.".into(),
            ));
        }
    };
    if let Some(warnings) = object.remove("@adminapi.warnings") {
        output.warnings.extend(match warnings {
            Value::Array(items) => items.iter().filter_map(value_text).collect::<Vec<_>>(),
            other => value_text(&other).into_iter().collect(),
        });
    }
    let next = object
        .remove("@odata.nextLink")
        .and_then(|v| v.as_str().map(str::to_owned));
    match object.remove("value") {
        Some(Value::Array(values)) => output.values.extend(values),
        Some(Value::Null) | None => {
            object.retain(|key, _| !key.starts_with('@'));
            if !object.is_empty() {
                output.values.push(Value::Object(object));
            }
        }
        Some(single) => output.values.push(single),
    }
    Ok(next)
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// `text` on one line, control and bidirectional characters removed, at
/// most `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let joined: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    clean_value(&joined, max).unwrap_or_default()
}

/// What a failed response says ([FEAS] exo-rest "error format"): the
/// details' messages (JSON inside a string, or `_|type|message`), then
/// the inner exception's, then the error's own; the text of a body that
/// isn't JSON; `X-ExceptionType` when nothing else named a type.
pub(crate) fn error_detail(status: u16, body: &[u8], header_type: Option<&str>) -> ErrorDetail {
    let text = String::from_utf8_lossy(body);
    let mut detail = ErrorDetail {
        status: Some(status),
        excerpt: clean_value(&text, MAX_EXCERPT).unwrap_or_default(),
        ..ErrorDetail::default()
    };
    let mut message = None;
    match serde_json::from_slice::<Value>(body) {
        Ok(json) => {
            let error = field(&json, "error").cloned().unwrap_or(Value::Null);
            detail.code = field(&error, "code").and_then(value_text);
            if let Some(Value::Array(details)) = field(&error, "details") {
                for item in details {
                    let Some(text) = field(item, "message").and_then(Value::as_str) else {
                        continue;
                    };
                    let (kind, said) = detail_message(text);
                    if detail.exception_type.is_none() {
                        detail.exception_type = kind;
                    }
                    if let Some(said) = said {
                        message = Some(said);
                        break;
                    }
                }
            }
            if message.is_none() {
                let inner = field(&error, "innererror");
                message = inner
                    .and_then(|i| field(i, "internalException"))
                    .and_then(|i| field(i, "message"))
                    .or_else(|| inner.and_then(|i| field(i, "message")))
                    .or_else(|| field(&error, "message"))
                    .and_then(value_text)
                    .filter(|m| !m.trim().is_empty());
            }
            if message.is_none() && !json.is_object() {
                message = value_text(&json);
            }
        }
        Err(_) => message = Some(text.into_owned()),
    }
    if detail.exception_type.is_none() {
        detail.exception_type = header_type.and_then(|t| clean_value(t, 200));
    }
    detail.message = message
        .map(|m| one_line(&m, MAX_MESSAGE))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("HTTP {status}"));
    detail
}

/// A `details[].message`: JSON with a message and a type, or
/// `_|type|message`, or the message itself. Returns (type, message).
fn detail_message(text: &str) -> (Option<String>, Option<String>) {
    let nonempty = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
    if let Ok(json @ Value::Object(_)) = serde_json::from_str::<Value>(text) {
        let message = ["message", "Message"]
            .iter()
            .find_map(|k| json.get(*k).and_then(Value::as_str))
            .and_then(nonempty);
        let kind = ["exceptionType", "ExceptionType", "type"]
            .iter()
            .find_map(|k| json.get(*k).and_then(Value::as_str))
            .and_then(nonempty);
        return (kind, message);
    }
    let parts: Vec<&str> = text.splitn(3, '|').collect();
    if let [_, kind, message] = parts.as_slice() {
        return (nonempty(kind), nonempty(message));
    }
    (None, nonempty(text))
}

/// A field of a JSON object, its name's case ignored.
fn field<'v>(value: &'v Value, name: &str) -> Option<&'v Value> {
    let object = value.as_object()?;
    object.get(name).or_else(|| {
        object
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    })
}

/// Exchange's answer when a tenant runs too many requests at once (any
/// status).
fn is_concurrency_limit(detail: &ErrorDetail) -> bool {
    let text = format!("{} {}", detail.message, detail.excerpt).to_ascii_lowercase();
    text.contains("maximum number of concurrent requests")
}

/// What a failed response means, first match wins. Exception type names are
/// hints only.
pub(crate) fn classify(status: u16, detail: ErrorDetail, resource: Resource) -> InvokeError {
    let text = format!(
        "{} {} {}",
        detail.message,
        detail.exception_type.as_deref().unwrap_or_default(),
        detail.code.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase();
    let has = |needle: &str| text.contains(needle);
    if status == 401 {
        return InvokeError::Unauthorized(detail);
    }
    // In Exchange Online, licensing wording is about IRM (Message
    // Encryption), which stays a refusal of its own.
    if resource == Resource::Compliance && has("licens") {
        return InvokeError::LicenseRequired(detail);
    }
    if status == 403
        || (has("is not recognized as") && has("cmdlet"))
        || has("not allowed to call")
        || has("access denied")
        || has("access is denied")
        || has("accessdenied")
        || has("isn't within your current write scope")
    {
        return InvokeError::Forbidden(detail);
    }
    if has("already exists") || has("alreadyexists") {
        return InvokeError::AlreadyExists(detail);
    }
    if has("couldn't be found") || has("could not be found") || has("managementobjectnotfound") {
        return InvokeError::NotFound(detail);
    }
    if status == 429 || is_concurrency_limit(&detail) {
        return InvokeError::Throttled(detail);
    }
    InvokeError::Rejected(detail)
}

/// Security & Compliance's redirects name `<x>.admin.protection.outlook.com`
/// on port 446; requests go to `<x>.ps.compliance.protection.outlook.com`
/// on 443 instead, as the module does.
pub(crate) fn rewrite_location(url: &mut Url) {
    let host = url.host_str().map(str::to_ascii_lowercase);
    if let Some(prefix) = host
        .as_deref()
        .and_then(|h| h.strip_suffix(".admin.protection.outlook.com"))
        .filter(|p| !p.is_empty())
    {
        let _ = url.set_host(Some(&format!(
            "{prefix}.ps.compliance.protection.outlook.com"
        )));
    }
    if url.port() == Some(446) {
        let _ = url.set_port(None);
    }
}

/// A URL's host, for the run log (never its path or query).
pub(crate) fn host_of(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_owned(),
        _ => "(no host)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn only_microsofts_https_hosts_get_requests() {
        let rules = HostRules::production();
        for allowed in [
            "https://outlook.office365.com/adminapi/beta/t/InvokeCommand",
            "https://OUTLOOK.office365.com:443/adminapi/beta/t/InvokeCommand",
            "https://eur03b.outlook.office365.com/adminapi/beta/t/InvokeCommand",
            "https://ps.compliance.protection.outlook.com/adminapi/beta/t/InvokeCommand",
            "https://eur03b.ps.compliance.protection.outlook.com/adminapi/beta/t/InvokeCommand",
        ] {
            assert!(rules.allows(&url(allowed)), "{allowed}");
        }
        for refused in [
            "http://outlook.office365.com/adminapi/beta/t/InvokeCommand",
            "https://outlook.office365.com:446/adminapi/beta/t/InvokeCommand",
            "https://evil.example/adminapi/beta/t/InvokeCommand",
            "https://outlook.office365.com.evil.example/x",
            "https://evilоutlook.office365.com/x",
            "https://notoutlook.office365.com/x",
            "https://.outlook.office365.com/x",
            "https://eur03b.admin.protection.outlook.com/x",
            "https://user:pass@outlook.office365.com/x",
            "https://52.96.0.1/x",
        ] {
            assert!(!rules.allows(&url(refused)), "{refused}");
        }
    }

    #[test]
    fn tests_may_also_allow_a_mock_server() {
        let mock = url("http://127.0.0.1:41234/adminapi/beta/t/InvokeCommand");
        let rules = HostRules::also(&mock);
        assert!(rules.allows(&url("http://127.0.0.1:41234/next?page=2")));
        assert!(!rules.allows(&url("http://127.0.0.1:41235/next")));
        assert!(rules.allows(&url("https://outlook.office365.com/x")));
        // Production keeps refusing an http Location.
        assert!(!HostRules::production().allows(&mock));
    }

    #[test]
    fn security_and_compliance_redirects_are_rewritten_to_their_rest_host() {
        let mut location = url("https://eur03b.admin.protection.outlook.com:446/adminapi/beta/t/x");
        rewrite_location(&mut location);
        assert_eq!(
            location.as_str(),
            "https://eur03b.ps.compliance.protection.outlook.com/adminapi/beta/t/x"
        );
        assert!(HostRules::production().allows(&location));
        // Others keep their host; only port 446 is dropped.
        let mut other = url("https://evil.example:446/x");
        rewrite_location(&mut other);
        assert_eq!(other.as_str(), "https://evil.example/x");
        assert!(!HostRules::production().allows(&other));
        let mut bare = url("https://admin.protection.outlook.com/x");
        rewrite_location(&mut bare);
        assert_eq!(bare.host_str(), Some("admin.protection.outlook.com"));
    }
}
