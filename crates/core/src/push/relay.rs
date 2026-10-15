//! The client of Cumulas Sync, the push service for Microsoft accounts.
//!
//! Microsoft Graph only posts change notifications to a public HTTPS
//! address, which an app on someone's computer doesn't have. So the app asks
//! the service for the account's channel (`POST /v1/channels`), subscribes in
//! Graph with the channel's URL and clientState, and keeps a stream of
//! server-sent events open (`GET /v1/events`) on which the service passes
//! each notification on as a subscription ID and a change type. The service
//! only ever sees tokens for its own scope (`Sync.Receive`), never a Graph
//! token, and nothing is replayed: every stream starts with `ready`, and the
//! app catches up then (see [`super::graph`]).
//!
//! [`Relay`] is the service as the rest of the app sees it; [`HttpRelay`]
//! talks to the real one, over one HTTP/2 client shared by every account.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Response, StatusCode};
use serde::Deserialize;
use serde_json::json;
use tokio::time::Instant;
use url::Url;

use super::sse::{SseItem, SseParser};

/// Where the push service runs, unless the build or the environment says
/// otherwise (see [`sync_url`]).
pub const DEFAULT_SYNC_URL: &str = "https://sync.cumulas.co";
/// The variable that points a build (at build time) or a run (at run time)
/// at another push service.
pub const SYNC_URL_VAR: &str = "CUMULAS_SYNC_URL";
const BUILT_IN_SYNC_URL: Option<&str> = option_env!("CUMULAS_SYNC_URL");

/// The header naming this installation on every request.
const INSTALLATION_HEADER: &str = "Cumulas-Installation";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The most a request (or a stream's response headers) may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How often the service sends a heartbeat when it doesn't say.
const DEFAULT_HEARTBEAT: Duration = Duration::from_secs(25);
/// The most a `Retry-After` or `bye` makes the app wait.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

/// The push service's address: the `CUMULAS_SYNC_URL` environment variable,
/// else the address built in with the same variable, else
/// [`DEFAULT_SYNC_URL`]. Tokens go there, so an address that isn't HTTPS
/// (other than one on this computer) is ignored.
pub fn sync_url() -> String {
    let runtime = std::env::var(SYNC_URL_VAR).ok();
    resolve_sync_url(runtime.as_deref(), BUILT_IN_SYNC_URL)
}

/// The push service's host name, for the UI's privacy note.
pub fn sync_host() -> String {
    Url::parse(&sync_url())
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn resolve_sync_url(runtime: Option<&str>, built_in: Option<&str>) -> String {
    for (candidate, origin) in [(runtime, "the environment"), (built_in, "the build")] {
        let Some(candidate) = candidate.map(str::trim).filter(|c| !c.is_empty()) else {
            continue;
        };
        match usable_sync_url(candidate) {
            Some(url) => return url,
            None => tracing::warn!(
                "ignoring the push service address from {origin}: it must start with https:// \
                 (or http:// for this computer)"
            ),
        }
    }
    DEFAULT_SYNC_URL.to_owned()
}

fn usable_sync_url(text: &str) -> Option<String> {
    let url = Url::parse(text).ok()?;
    let host = url.host_str()?;
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    let secure = url.scheme() == "https" || (url.scheme() == "http" && loopback);
    (secure && url.query().is_none() && url.fragment().is_none())
        .then(|| text.trim_end_matches('/').to_owned())
}

/// One Microsoft account's channel. Every installation signed in to the
/// account gets the same one, derived by the service rather than stored, so
/// it survives restarts; the clientState is fetched each time, never kept.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// The protocol version; 1.
    #[serde(default)]
    pub v: Option<u32>,
    #[serde(default)]
    pub notification_url: String,
    #[serde(default)]
    pub lifecycle_notification_url: String,
    #[serde(default)]
    pub client_state: String,
    /// The stream's address (from `POST /v1/channels` only).
    #[serde(default)]
    pub events: Option<String>,
    #[serde(default)]
    pub heartbeat_seconds: Option<u64>,
    /// When the stream ends (from `ready` only).
    #[serde(default)]
    pub ends_at: Option<DateTime<Utc>>,
}

impl Channel {
    /// Whether the service gave everything a subscription needs, in a
    /// version of the protocol this app speaks.
    fn is_complete(&self) -> bool {
        self.v.is_none_or(|v| v == 1)
            && !self.notification_url.is_empty()
            && !self.lifecycle_notification_url.is_empty()
            && !self.client_state.is_empty()
    }

    /// How long the stream may stay silent before it counts as dead: two
    /// and a half heartbeats.
    fn silence_limit(&self) -> Duration {
        let heartbeat = self
            .heartbeat_seconds
            .filter(|s| *s > 0)
            .map_or(DEFAULT_HEARTBEAT, |s| Duration::from_secs(s.min(300)));
        heartbeat * 5 / 2
    }
}

/// The URLs name the account's channel and the clientState is a secret:
/// neither goes into logs.
impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel")
            .field("v", &self.v)
            .field("heartbeat_seconds", &self.heartbeat_seconds)
            .field("ends_at", &self.ends_at)
            .finish_non_exhaustive()
    }
}

/// What happened to a subscription itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifecycle {
    /// Graph wants the subscription reauthorized (renewing does that).
    ReauthorizationRequired,
    /// Graph removed it: create it again, then catch up.
    SubscriptionRemoved,
    /// Graph couldn't deliver some notifications: catch up.
    Missed,
    Other(String),
}

/// An event on the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayEvent {
    /// The stream is open: the channel again, and when the stream ends.
    Ready(Channel),
    /// What a subscription watches changed.
    Change { subscription: String },
    Lifecycle {
        subscription: String,
        event: Lifecycle,
    },
    /// Changes came too fast to pass on one by one: catch up on everything.
    Resync,
    /// Open a new stream with a new token; it replaces this one.
    Renew,
    /// The service is ending the stream; come back after `retry`.
    Bye {
        reason: String,
        retry: Option<Duration>,
    },
}

/// Why the service didn't open a channel or a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayError {
    /// 401: the token was missing or refused, or ends too soon
    /// (`expiring`).
    Token { expiring: bool },
    /// 400 (`bad_installation`): a bug in the app.
    BadRequest(String),
    /// 403 (`tenant_not_allowed`): the service doesn't serve this account.
    NotAllowed,
    /// 429 or 503: too many streams or requests, or busy; come back after
    /// `retry_after` when it says.
    Busy { retry_after: Option<Duration> },
    /// Unreachable, or an answer that made no sense.
    Failed(String),
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token { .. } => write!(f, "The push service didn't accept the sign-in."),
            Self::BadRequest(error) => write!(f, "The push service refused the request ({error})."),
            Self::NotAllowed => write!(f, "The push service doesn't serve this organization."),
            Self::Busy { .. } => write!(f, "The push service is busy."),
            Self::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// Why a stream ended (other than with `bye`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamEnd {
    pub reason: String,
    /// The stream's own `retry:` hint, for when to come back.
    pub retry: Option<Duration>,
}

/// The push service.
#[async_trait]
pub trait Relay: Send + Sync {
    /// The account's channel (`POST /v1/channels`).
    async fn channel(&self, token: &str, installation: &str) -> Result<Channel, RelayError>;

    /// Opens this installation's stream (`GET <events>`); its first event
    /// is `ready`. A new stream replaces the installation's earlier one.
    async fn events(
        &self,
        token: &str,
        installation: &str,
        channel: &Channel,
    ) -> Result<Box<dyn EventStream>, RelayError>;
}

/// An open stream of events.
#[async_trait]
pub trait EventStream: Send {
    /// The next event. Cancel-safe: nothing is lost when the future is
    /// dropped before it resolves.
    async fn next(&mut self) -> Result<RelayEvent, StreamEnd>;
}

/// The push service over HTTPS.
pub struct HttpRelay {
    http: reqwest::Client,
    base: String,
}

impl HttpRelay {
    /// A client for the service at `base` (see [`sync_url`]). Make one per
    /// app: its connection serves every account.
    pub fn new(base: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            // No overall timeout: streams last for hours. Silence is caught
            // by the stream's own watchdog.
            .tcp_keepalive(Duration::from_secs(60))
            .user_agent(concat!("CumulasMail/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self {
            http,
            base: base.into().trim_end_matches('/').to_owned(),
        }
    }

    /// The stream's address. It gets the token too, so an address the
    /// service gives on another origin is not used.
    fn events_url(&self, channel: &Channel) -> String {
        let own = format!("{}/v1/events", self.base);
        let given = channel.events.as_deref().and_then(|e| Url::parse(e).ok());
        match (given, Url::parse(&own)) {
            (Some(given), Ok(own_url)) if given.origin() == own_url.origin() => given.into(),
            _ => own,
        }
    }
}

#[async_trait]
impl Relay for HttpRelay {
    async fn channel(&self, token: &str, installation: &str) -> Result<Channel, RelayError> {
        let response = self
            .http
            .post(format!("{}/v1/channels", self.base))
            .bearer_auth(token)
            .header(INSTALLATION_HEADER, installation)
            .json(&json!({ "installation": installation }))
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(unreachable)?;
        if !response.status().is_success() {
            return Err(refusal(response).await);
        }
        let body = response.bytes().await.map_err(unreachable)?;
        let channel: Channel = serde_json::from_slice(&body).map_err(|_| {
            RelayError::Failed("The push service sent a channel this app can't read.".into())
        })?;
        if !channel.is_complete() {
            return Err(RelayError::Failed(
                "The push service sent an incomplete channel.".into(),
            ));
        }
        Ok(channel)
    }

    async fn events(
        &self,
        token: &str,
        installation: &str,
        channel: &Channel,
    ) -> Result<Box<dyn EventStream>, RelayError> {
        let request = self
            .http
            .get(self.events_url(channel))
            .bearer_auth(token)
            .header(INSTALLATION_HEADER, installation)
            .header(ACCEPT, "text/event-stream")
            // Compression would hold events back in a proxy's buffer.
            .header(ACCEPT_ENCODING, "identity")
            .header(CACHE_CONTROL, "no-cache")
            .send();
        let response = tokio::time::timeout(REQUEST_TIMEOUT, request)
            .await
            .map_err(|_| RelayError::Failed("The push service didn't answer.".into()))?
            .map_err(unreachable)?;
        if !response.status().is_success() {
            return Err(refusal(response).await);
        }
        let is_stream = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        if !is_stream {
            return Err(RelayError::Failed(
                "The push service didn't open a stream.".into(),
            ));
        }
        Ok(Box::new(HttpEventStream::new(
            response,
            channel.silence_limit(),
        )))
    }
}

fn unreachable(e: reqwest::Error) -> RelayError {
    RelayError::Failed(format!(
        "Could not reach the push service: {}",
        e.without_url()
    ))
}

/// Maps a refusal: the status, the JSON `{"error"}` and `Retry-After`.
async fn refusal(response: Response) -> RelayError {
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        error: String,
    }
    let status = response.status();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|s| Duration::from_secs(s).min(MAX_RETRY_AFTER));
    let error = match tokio::time::timeout(Duration::from_secs(5), response.bytes()).await {
        Ok(Ok(body)) => serde_json::from_slice::<Body>(&body)
            .map(|b| b.error)
            .unwrap_or_default(),
        _ => String::new(),
    };
    match status {
        StatusCode::UNAUTHORIZED => RelayError::Token {
            expiring: error == "token_expiring",
        },
        StatusCode::BAD_REQUEST => RelayError::BadRequest(if error.is_empty() {
            "bad_request".into()
        } else {
            error
        }),
        StatusCode::FORBIDDEN => RelayError::NotAllowed,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE => {
            RelayError::Busy { retry_after }
        }
        _ => RelayError::Failed(format!(
            "The push service answered HTTP {}.",
            status.as_u16()
        )),
    }
}

/// A stream over HTTP. Any bytes (heartbeat comments included) show it's
/// alive; none for [`Channel::silence_limit`] means it's dead.
struct HttpEventStream {
    response: Response,
    parser: SseParser,
    /// Events read but not yet returned. Kept here, not in `next`'s
    /// future, so that dropping the future loses nothing.
    queue: VecDeque<RelayEvent>,
    silence: Duration,
    deadline: Instant,
    retry: Option<Duration>,
}

impl HttpEventStream {
    fn new(response: Response, silence: Duration) -> Self {
        Self {
            response,
            parser: SseParser::default(),
            queue: VecDeque::new(),
            silence,
            deadline: Instant::now() + silence,
            retry: None,
        }
    }

    fn end(&self, reason: impl Into<String>) -> StreamEnd {
        StreamEnd {
            reason: reason.into(),
            retry: self.retry,
        }
    }

    fn read(&mut self, chunk: &[u8]) -> Result<(), StreamEnd> {
        let mut items = Vec::new();
        if self.parser.feed(chunk, &mut items).is_err() {
            return Err(self.end("The push service sent an overlong event."));
        }
        for item in items {
            match item {
                SseItem::Event { name, data } => {
                    if let Some(event) = parse_event(&name, &data) {
                        if let RelayEvent::Ready(channel) = &event
                            && channel.heartbeat_seconds.is_some()
                        {
                            self.silence = channel.silence_limit();
                        }
                        self.queue.push_back(event);
                    } else {
                        tracing::debug!("push service: skipping a {name:?} event");
                    }
                }
                SseItem::Comment => {}
                SseItem::Retry(retry) => self.retry = Some(retry.min(MAX_RETRY_AFTER)),
            }
        }
        Ok(())
    }
}

#[async_trait]
impl EventStream for HttpEventStream {
    async fn next(&mut self) -> Result<RelayEvent, StreamEnd> {
        loop {
            if let Some(event) = self.queue.pop_front() {
                return Ok(event);
            }
            let chunk = tokio::time::timeout_at(self.deadline, self.response.chunk()).await;
            match chunk {
                Err(_) => {
                    tracing::debug!("push service stream went silent");
                    return Err(self.end("The push service went silent."));
                }
                Ok(Ok(Some(bytes))) => {
                    self.deadline = Instant::now() + self.silence;
                    self.read(&bytes)?;
                }
                Ok(Ok(None)) => return Err(self.end("The push service closed the stream.")),
                Ok(Err(e)) => {
                    return Err(self.end(format!(
                        "The push service stream broke: {}",
                        e.without_url()
                    )));
                }
            }
        }
    }
}

/// An event from its name and JSON data; `None` for one this app doesn't
/// know (or can't read), which is skipped.
pub(crate) fn parse_event(name: &str, data: &str) -> Option<RelayEvent> {
    #[derive(Deserialize)]
    struct Change {
        s: String,
    }
    #[derive(Deserialize)]
    struct Life {
        s: String,
        e: String,
    }
    #[derive(Default, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Bye {
        #[serde(default)]
        reason: String,
        retry_ms: Option<u64>,
    }
    Some(match name {
        "ready" => RelayEvent::Ready(serde_json::from_str(data).ok()?),
        "change" => RelayEvent::Change {
            subscription: serde_json::from_str::<Change>(data).ok()?.s,
        },
        "lifecycle" => {
            let life: Life = serde_json::from_str(data).ok()?;
            RelayEvent::Lifecycle {
                subscription: life.s,
                event: match life.e.as_str() {
                    "reauthorizationRequired" => Lifecycle::ReauthorizationRequired,
                    "subscriptionRemoved" => Lifecycle::SubscriptionRemoved,
                    "missed" => Lifecycle::Missed,
                    _ => Lifecycle::Other(life.e),
                },
            }
        }
        "resync" => RelayEvent::Resync,
        "renew" => RelayEvent::Renew,
        "bye" => {
            let bye: Bye = serde_json::from_str(data).unwrap_or_default();
            RelayEvent::Bye {
                reason: bye.reason,
                retry: bye
                    .retry_ms
                    .map(|ms| Duration::from_millis(ms).min(MAX_RETRY_AFTER)),
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
pub(crate) mod tests;
