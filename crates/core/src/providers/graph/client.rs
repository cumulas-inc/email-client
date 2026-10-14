//! HTTP plumbing for Microsoft Graph: bearer tokens, `Prefer` headers,
//! throttling retries, error mapping, paging and JSON batching.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use url::Url;

use crate::auth::TokenSource;
use crate::error::{Error, ErrorCode, Result};

/// Ids of Outlook items that do not change when the item moves.
pub(super) const PREFER_IMMUTABLE_ID: &str = r#"IdType="ImmutableId""#;
/// Message and event bodies as HTML (plain-text bodies are converted).
pub(super) const PREFER_HTML_BODY: &str = r#"outlook.body-content-type="html""#;
/// Event times in UTC rather than the event's own time zone.
pub(super) const PREFER_UTC: &str = r#"outlook.timezone="UTC""#;
/// Page size for delta queries, which ignore `$top`.
pub(super) const PREFER_PAGE_SIZE: &str = "odata.maxpagesize=100";

/// Retries of a throttled (429) or unavailable (503/504) request.
const MAX_RETRIES: u32 = 3;
/// Upper bound for one wait between retries, whatever `Retry-After` says.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
/// Guard against paging links that never end.
pub(super) const MAX_PAGES: usize = 1000;
/// Most requests Graph accepts in one JSON batch.
const BATCH_SIZE: usize = 20;

pub(super) type GraphResult<T> = std::result::Result<T, Failure>;

/// A failed call. Graph errors keep their HTTP status and Graph error code
/// for callers that react to specific failures (e.g. an expired delta token).
#[derive(Debug)]
pub(super) enum Failure {
    Graph {
        status: StatusCode,
        code: String,
        message: String,
    },
    Other(Error),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Other(e)
    }
}

impl From<Failure> for Error {
    fn from(f: Failure) -> Self {
        match f {
            Failure::Graph {
                status,
                code,
                message,
            } => graph_error(status, &code, &message),
            Failure::Other(e) => e,
        }
    }
}

/// Maps a Graph error response: 401 means the token was rejected and the
/// user must sign in again; 404 that the item is gone.
fn graph_error(status: StatusCode, code: &str, message: &str) -> Error {
    let message = match message.trim() {
        "" => format!("Microsoft Graph returned HTTP {}.", status.as_u16()),
        m => m.to_owned(),
    };
    match status {
        StatusCode::UNAUTHORIZED => Error::auth_required(format!(
            "Microsoft did not accept the sign-in ({}). Sign in again.",
            message.trim_end_matches('.')
        )),
        StatusCode::NOT_FOUND => Error::new(ErrorCode::NotFound, message),
        _ => Error::provider(message).with_details(json!({
            "status": status.as_u16(),
            "code": code,
        })),
    }
}

fn parse_error(body: Value) -> (String, String) {
    #[derive(Deserialize)]
    struct Envelope {
        error: ErrorBody,
    }
    #[derive(Deserialize)]
    struct ErrorBody {
        #[serde(default)]
        code: String,
        #[serde(default)]
        message: String,
    }
    serde_json::from_value::<Envelope>(body)
        .map(|e| (e.error.code, e.error.message))
        .unwrap_or_default()
}

fn is_retryable(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

/// `Retry-After` in seconds when given, else exponential backoff; capped.
fn retry_delay(retry_after: Option<&str>, attempt: u32) -> Duration {
    retry_after
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(1 << attempt))
        .min(MAX_RETRY_DELAY)
}

fn unexpected(e: impl std::fmt::Display) -> Failure {
    Failure::Other(Error::provider(format!(
        "Unexpected response from Microsoft Graph: {e}"
    )))
}

/// Percent-encodes an id for use as one URL path segment.
pub(super) fn segment(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'=') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// One page of a collection.
#[derive(Debug, Deserialize)]
pub(super) struct Page<T> {
    #[serde(default = "Vec::new")]
    pub value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

/// Any entity when only its id matters.
#[derive(Debug, Deserialize)]
pub(super) struct IdOnly {
    pub id: String,
}

enum Body<'a> {
    Empty,
    Json(&'a Value),
    Chunk { data: &'a [u8], range: &'a str },
}

struct Request<'a> {
    method: Method,
    url: &'a str,
    prefer: &'a [&'a str],
    body: Body<'a>,
    /// Upload-session URLs are pre-authenticated and must not get the token.
    authenticated: bool,
}

pub(super) struct GraphClient {
    http: reqwest::Client,
    base_url: String,
    tokens: Arc<dyn TokenSource>,
}

impl GraphClient {
    pub fn new(base_url: String, tokens: Arc<dyn TokenSource>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            // Per read rather than per request, so large downloads can finish.
            .read_timeout(Duration::from_secs(60))
            .user_agent(concat!("CumulasMail/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
            tokens,
        }
    }

    /// Absolute URL for a path such as `/me/messages`.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    pub fn url_with_query(&self, path: &str, query: &[(&str, &str)]) -> String {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(query)
            .finish();
        format!("{}{path}?{query}", self.base_url)
    }

    /// Whether a link (paging link, stored delta link) points at the Graph
    /// endpoint this client talks to. Others are never followed, since they
    /// would receive the access token.
    pub fn is_own_link(&self, link: &str) -> bool {
        match (Url::parse(link), Url::parse(&self.base_url)) {
            (Ok(link), Ok(base)) => link.origin() == base.origin(),
            _ => false,
        }
    }

    async fn execute(&self, request: Request<'_>) -> GraphResult<reqwest::Response> {
        let mut attempt = 0;
        loop {
            let mut builder = self.http.request(request.method.clone(), request.url);
            if request.authenticated {
                builder = builder.bearer_auth(self.tokens.access_token().await?);
            }
            for prefer in request.prefer {
                builder = builder.header("Prefer", *prefer);
            }
            builder = match &request.body {
                // Graph answers 411 to a body-less POST without Content-Length.
                Body::Empty if request.method == Method::POST => builder.header(CONTENT_LENGTH, 0),
                Body::Empty => builder,
                Body::Json(value) => builder.json(value),
                Body::Chunk { data, range } => builder
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .header(CONTENT_RANGE, *range)
                    .body(data.to_vec()),
            };
            let response = builder.send().await.map_err(|e| {
                Failure::Other(Error::network(format!(
                    "Could not reach Microsoft Graph: {}",
                    // Upload-session URLs carry a credential.
                    e.without_url()
                )))
            })?;
            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }
            if is_retryable(status) && attempt < MAX_RETRIES {
                let retry_after = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok());
                let delay = retry_delay(retry_after, attempt);
                tracing::debug!("Graph returned {status}; retrying in {delay:?}");
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }
            let body = response.bytes().await.unwrap_or_default();
            let (code, message) = parse_error(serde_json::from_slice(&body).unwrap_or_default());
            return Err(Failure::Graph {
                status,
                code,
                message,
            });
        }
    }

    async fn read_json<T: DeserializeOwned>(response: reqwest::Response) -> GraphResult<T> {
        let bytes = response.bytes().await.map_err(|e| {
            Failure::Other(Error::network(format!(
                "Could not read the response from Microsoft Graph: {}",
                e.without_url()
            )))
        })?;
        serde_json::from_slice(&bytes).map_err(unexpected)
    }

    pub async fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        prefer: &[&str],
    ) -> GraphResult<T> {
        let response = self
            .execute(Request {
                method: Method::GET,
                url,
                prefer,
                body: Body::Empty,
                authenticated: true,
            })
            .await?;
        Self::read_json(response).await
    }

    pub async fn get_bytes(&self, url: &str, prefer: &[&str]) -> GraphResult<Vec<u8>> {
        let response = self
            .execute(Request {
                method: Method::GET,
                url,
                prefer,
                body: Body::Empty,
                authenticated: true,
            })
            .await?;
        let bytes = response.bytes().await.map_err(|e| {
            Failure::Other(Error::network(format!(
                "Could not download from Microsoft Graph: {}",
                e.without_url()
            )))
        })?;
        Ok(bytes.to_vec())
    }

    /// Sends a JSON body and parses the JSON response.
    pub async fn send_json<T: DeserializeOwned>(
        &self,
        method: Method,
        url: &str,
        body: &Value,
        prefer: &[&str],
    ) -> GraphResult<T> {
        let response = self
            .execute(Request {
                method,
                url,
                prefer,
                body: Body::Json(body),
                authenticated: true,
            })
            .await?;
        Self::read_json(response).await
    }

    /// Sends a request whose response body is not needed (202/204 actions,
    /// DELETE, PATCH).
    pub async fn send_no_content(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
        prefer: &[&str],
    ) -> GraphResult<()> {
        self.execute(Request {
            method,
            url,
            prefer,
            body: body.map_or(Body::Empty, Body::Json),
            authenticated: true,
        })
        .await?;
        Ok(())
    }

    /// Uploads one byte range to a pre-authenticated upload-session URL.
    pub async fn put_chunk(&self, url: &str, data: &[u8], range: &str) -> GraphResult<()> {
        self.execute(Request {
            method: Method::PUT,
            url,
            prefer: &[],
            body: Body::Chunk { data, range },
            authenticated: false,
        })
        .await?;
        Ok(())
    }

    /// One page of a collection; refuses links to other servers.
    pub async fn get_page<T: DeserializeOwned>(
        &self,
        url: &str,
        prefer: &[&str],
    ) -> GraphResult<Page<T>> {
        if !self.is_own_link(url) {
            return Err(Failure::Other(Error::provider(
                "Microsoft Graph returned a link to another server.",
            )));
        }
        self.get_json(url, prefer).await
    }

    /// Every item of a collection, following `@odata.nextLink`.
    pub async fn get_all<T: DeserializeOwned>(
        &self,
        url: &str,
        prefer: &[&str],
    ) -> GraphResult<Vec<T>> {
        let mut items = Vec::new();
        let mut next = Some(url.to_owned());
        for _ in 0..MAX_PAGES {
            let Some(url) = next.take() else {
                return Ok(items);
            };
            let page: Page<T> = self.get_page(&url, prefer).await?;
            items.extend(page.value);
            next = page.next_link;
        }
        match next {
            None => Ok(items),
            Some(_) => Err(Failure::Other(Error::provider(
                "Microsoft Graph returned too many pages.",
            ))),
        }
    }

    /// Runs requests through JSON batching (20 per round trip). Sub-requests
    /// that were throttled are retried like single requests. Returns the
    /// responses by request id; callers check each status.
    pub async fn batch(
        &self,
        requests: &[BatchRequest],
        prefer: &[&str],
    ) -> GraphResult<HashMap<String, BatchResponse>> {
        #[derive(Deserialize)]
        struct Envelope {
            #[serde(default)]
            responses: Vec<BatchResponse>,
        }
        let url = self.url("/$batch");
        let mut results = HashMap::with_capacity(requests.len());
        for chunk in requests.chunks(BATCH_SIZE) {
            let mut pending: Vec<&BatchRequest> = chunk.iter().collect();
            let mut attempt = 0;
            while !pending.is_empty() {
                let payload = json!({
                    "requests": pending.iter().map(|r| r.to_json(prefer)).collect::<Vec<_>>(),
                });
                let envelope: Envelope = self.send_json(Method::POST, &url, &payload, &[]).await?;
                let mut retry = Vec::new();
                let mut delay = Duration::ZERO;
                for response in envelope.responses {
                    if is_retryable(response.status())
                        && attempt < MAX_RETRIES
                        && let Some(request) = pending.iter().find(|r| r.id == response.id)
                    {
                        delay =
                            delay.max(retry_delay(response.header(RETRY_AFTER.as_str()), attempt));
                        retry.push(*request);
                        continue;
                    }
                    results.insert(response.id.clone(), response);
                }
                if !retry.is_empty() {
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                }
                pending = retry;
            }
        }
        Ok(results)
    }
}

/// One request of a JSON batch. `url` is relative to the version root
/// (`/me/messages/...`).
#[derive(Debug, Clone)]
pub(super) struct BatchRequest {
    pub id: String,
    pub method: &'static str,
    pub url: String,
    pub body: Option<Value>,
}

impl BatchRequest {
    pub fn new(id: impl ToString, method: &'static str, url: String, body: Option<Value>) -> Self {
        Self {
            id: id.to_string(),
            method,
            url,
            body,
        }
    }

    pub fn get(id: impl ToString, url: String) -> Self {
        Self::new(id, "GET", url, None)
    }

    fn to_json(&self, prefer: &[&str]) -> Value {
        let mut request = json!({ "id": self.id, "method": self.method, "url": self.url });
        let mut headers = Map::new();
        if !prefer.is_empty() {
            headers.insert("Prefer".into(), prefer.join(", ").into());
        }
        if let Some(body) = &self.body {
            request["body"] = body.clone();
            headers.insert("Content-Type".into(), "application/json".into());
        }
        if !headers.is_empty() {
            request["headers"] = Value::Object(headers);
        }
        request
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct BatchResponse {
    pub id: String,
    pub status: u16,
    #[serde(default)]
    headers: HashMap<String, Value>,
    #[serde(default)]
    pub body: Value,
}

impl BatchResponse {
    pub fn status(&self) -> StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    pub fn is_success(&self) -> bool {
        self.status().is_success()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.as_str())
    }

    /// The error this response represents.
    pub fn error(self) -> Error {
        let status = self.status();
        let (code, message) = parse_error(self.body);
        graph_error(status, &code, &message)
    }

    pub fn json<T: DeserializeOwned>(self) -> Result<T> {
        serde_json::from_value(self.body).map_err(|e| unexpected(e).into())
    }
}

/// Removes the response to request `id`; a missing one is an error.
pub(super) fn take_response(
    responses: &mut HashMap<String, BatchResponse>,
    id: &str,
) -> Result<BatchResponse> {
    responses
        .remove(id)
        .ok_or_else(|| Error::provider("Microsoft Graph did not answer every batched request."))
}
