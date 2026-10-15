//! The push service's HTTP client against [`FakeRelay`], a raw HTTP/1.1
//! server on a loopback listener that answers with scripted JSON and
//! streams scripted server-sent events. Real time: paused time would fire
//! the watchdog while the client waits on the socket.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use super::*;

pub(crate) const TOKEN: &str = "sync-token-1";
pub(crate) const INSTALLATION: &str = "install-0123456789abcdef";
pub(crate) const CLIENT_STATE: &str = "client-state-from-the-relay";
pub(crate) const ROUTE: &str = "k1AbCdEfGhIjKlMnOpQrStUvWxYz012345";

/// What the fake answers one request with.
pub(crate) enum Reply {
    Json {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: String,
    },
    /// A stream, step by step; it closes after the last unless that holds.
    Stream(Vec<Step>),
}

pub(crate) enum Step {
    Send(String),
    Wait(Duration),
    /// Stays open, silent, until the client or the test goes.
    Hold,
}

impl Reply {
    pub(crate) fn refuse(status: u16, error: &str, retry_after: Option<u64>) -> Self {
        Reply::Json {
            status,
            headers: retry_after
                .map(|s| ("Retry-After", s.to_string()))
                .into_iter()
                .collect(),
            body: json!({ "error": error }).to_string(),
        }
    }
}

/// A request the fake received.
#[derive(Debug, Clone)]
pub(crate) struct Recorded {
    pub method: String,
    pub path: String,
    /// Names in lower case.
    pub headers: HashMap<String, String>,
    pub body: String,
    pub at: Instant,
}

#[derive(Default)]
struct Script {
    channels: VecDeque<Reply>,
    streams: VecDeque<Reply>,
    requests: Vec<Recorded>,
}

pub(crate) struct FakeRelay {
    pub base: String,
    pub heartbeat: u64,
    script: Arc<Mutex<Script>>,
    stop: CancellationToken,
}

impl Drop for FakeRelay {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl FakeRelay {
    /// Listens on a free loopback port. Unscripted requests get a channel,
    /// or a stream that sends `ready` and then holds.
    pub(crate) async fn start(heartbeat: u64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let script = Arc::new(Mutex::new(Script::default()));
        let stop = CancellationToken::new();
        let relay = Self {
            base,
            heartbeat,
            script: script.clone(),
            stop: stop.clone(),
        };
        let (base, heartbeat) = (relay.base.clone(), heartbeat);
        tokio::spawn(async move {
            loop {
                let socket = tokio::select! {
                    _ = stop.cancelled() => return,
                    accepted = listener.accept() => match accepted {
                        Ok((socket, _)) => socket,
                        Err(_) => return,
                    },
                };
                let (script, stop, base) = (script.clone(), stop.clone(), base.clone());
                tokio::spawn(async move {
                    tokio::select! {
                        _ = stop.cancelled() => {}
                        _ = serve(socket, script, &base, heartbeat) => {}
                    }
                });
            }
        });
        relay
    }

    pub(crate) fn channel_reply(&self, reply: Reply) {
        self.script.lock().unwrap().channels.push_back(reply);
    }

    pub(crate) fn stream_reply(&self, reply: Reply) {
        self.script.lock().unwrap().streams.push_back(reply);
    }

    pub(crate) fn requests(&self) -> Vec<Recorded> {
        self.script.lock().unwrap().requests.clone()
    }

    pub(crate) fn requests_to(&self, path: &str) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|r| r.path == path)
            .collect()
    }

    pub(crate) fn notification_url(&self) -> String {
        notification_url(&self.base)
    }

    /// The `ready` the service sends first (after its `retry:` hint).
    pub(crate) fn ready(&self) -> String {
        ready(&self.base, self.heartbeat)
    }
}

pub(crate) fn notification_url(base: &str) -> String {
    format!("{base}/graph/v1/{ROUTE}")
}

fn channel(base: &str, heartbeat: u64) -> Value {
    json!({
        "v": 1,
        "channel": ROUTE,
        "notificationUrl": notification_url(base),
        "lifecycleNotificationUrl": notification_url(base),
        "clientState": CLIENT_STATE,
        "events": format!("{base}/v1/events"),
        "heartbeatSeconds": heartbeat,
    })
}

fn ready(base: &str, heartbeat: u64) -> String {
    let mut ready = channel(base, heartbeat);
    let object = ready.as_object_mut().unwrap();
    object.remove("events");
    object.insert("endsAt".into(), json!("2026-10-15T12:00:00.000Z"));
    format!("retry: 10000\n\n{}", event("ready", &ready))
}

/// One event as the service writes it.
pub(crate) fn event(name: &str, data: &Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

async fn serve(mut socket: TcpStream, script: Arc<Mutex<Script>>, base: &str, heartbeat: u64) {
    let Some(request) = read_request(&mut socket).await else {
        return;
    };
    let reply = {
        let mut script = script.lock().unwrap();
        script.requests.push(request.clone());
        match request.path.as_str() {
            "/v1/channels" => script.channels.pop_front().unwrap_or_else(|| Reply::Json {
                status: 200,
                headers: vec![],
                body: channel(base, heartbeat).to_string(),
            }),
            "/v1/events" => script.streams.pop_front().unwrap_or_else(|| {
                Reply::Stream(vec![Step::Send(ready(base, heartbeat)), Step::Hold])
            }),
            _ => Reply::Json {
                status: 404,
                headers: vec![],
                body: r#"{"error":"not_found"}"#.into(),
            },
        }
    };
    match reply {
        Reply::Json {
            status,
            headers,
            body,
        } => {
            let mut head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            let _ = socket
                .write_all(format!("{head}\r\n{body}").as_bytes())
                .await;
        }
        Reply::Stream(steps) => {
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\n\
                        Cache-Control: no-store\r\nConnection: close\r\n\r\n";
            if socket.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            for step in steps {
                match step {
                    Step::Send(text) => {
                        if socket.write_all(text.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    Step::Wait(duration) => tokio::time::sleep(duration).await,
                    Step::Hold => {
                        // Until the client closes its end.
                        let mut byte = [0u8; 1];
                        let _ = socket.read(&mut byte).await;
                        return;
                    }
                }
            }
        }
    }
    let _ = socket.shutdown().await;
}

async fn read_request(socket: &mut TcpStream) -> Option<Recorded> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 || buffer.len() > 64 * 1024 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .and_then(|l| l.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some(Recorded {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
        at: Instant::now(),
    })
}

async fn open(relay: &FakeRelay) -> (HttpRelay, Channel, Box<dyn EventStream>) {
    let client = HttpRelay::new(&relay.base);
    let channel = client.channel(TOKEN, INSTALLATION).await.unwrap();
    let stream = client.events(TOKEN, INSTALLATION, &channel).await.unwrap();
    (client, channel, stream)
}

#[tokio::test]
async fn the_channel_and_stream_carry_the_token_and_installation() {
    let relay = FakeRelay::start(25).await;
    let (_client, channel, mut stream) = open(&relay).await;
    assert_eq!(channel.notification_url, relay.notification_url());
    assert_eq!(channel.lifecycle_notification_url, relay.notification_url());
    assert_eq!(channel.client_state, CLIENT_STATE);
    assert_eq!(channel.heartbeat_seconds, Some(25));
    let Ok(RelayEvent::Ready(ready)) = stream.next().await else {
        panic!("no ready");
    };
    assert_eq!(ready.notification_url, relay.notification_url());
    assert_eq!(ready.client_state, CLIENT_STATE);
    assert_eq!(ready.ends_at, Some("2026-10-15T12:00:00Z".parse().unwrap()));

    let requests = relay.requests();
    assert_eq!(requests.len(), 2);
    let (post, get) = (&requests[0], &requests[1]);
    assert_eq!(
        (post.method.as_str(), post.path.as_str()),
        ("POST", "/v1/channels")
    );
    assert_eq!(post.headers["authorization"], format!("Bearer {TOKEN}"));
    assert_eq!(post.headers["cumulas-installation"], INSTALLATION);
    assert!(post.headers["content-type"].starts_with("application/json"));
    let body: Value = serde_json::from_str(&post.body).unwrap();
    assert_eq!(body, json!({ "installation": INSTALLATION }));

    assert_eq!(
        (get.method.as_str(), get.path.as_str()),
        ("GET", "/v1/events")
    );
    assert!(get.at >= post.at);
    assert_eq!(get.headers["authorization"], format!("Bearer {TOKEN}"));
    assert_eq!(get.headers["cumulas-installation"], INSTALLATION);
    assert_eq!(get.headers["accept"], "text/event-stream");
    assert_eq!(get.headers["accept-encoding"], "identity");
    // Debug output names neither the channel nor its secret.
    let debug = format!("{channel:?}");
    assert!(
        !debug.contains(ROUTE) && !debug.contains(CLIENT_STATE),
        "{debug}"
    );
}

#[tokio::test]
async fn every_event_reaches_the_app() {
    let relay = FakeRelay::start(25).await;
    let sub = "5ab1c0de-1111-2222-3333-444455556666";
    relay.stream_reply(Reply::Stream(vec![
        Step::Send(relay.ready()),
        Step::Send(": ka\n\n".into()),
        Step::Send(event("change", &json!({ "s": sub, "c": "created" }))),
        Step::Send(event(
            "lifecycle",
            &json!({ "s": sub, "e": "reauthorizationRequired" }),
        )),
        Step::Send(event(
            "lifecycle",
            &json!({ "s": sub, "e": "subscriptionRemoved" }),
        )),
        Step::Send(event("lifecycle", &json!({ "s": sub, "e": "missed" }))),
        Step::Send(event("resync", &json!({}))),
        Step::Send(event("someday", &json!({ "x": 1 }))),
        Step::Send(event(
            "renew",
            &json!({ "endsAt": "2026-10-15T12:00:00.000Z" }),
        )),
        Step::Send(event(
            "bye",
            &json!({ "reason": "expired", "retryMs": 1234 }),
        )),
    ]));
    let (_client, _channel, mut stream) = open(&relay).await;
    let mut events = Vec::new();
    let end = loop {
        match stream.next().await {
            Ok(event) => events.push(event),
            Err(end) => break end,
        }
    };
    let life = |event| RelayEvent::Lifecycle {
        subscription: sub.into(),
        event,
    };
    assert!(matches!(events[0], RelayEvent::Ready(_)));
    assert_eq!(
        events[1..],
        [
            RelayEvent::Change {
                subscription: sub.into()
            },
            life(Lifecycle::ReauthorizationRequired),
            life(Lifecycle::SubscriptionRemoved),
            life(Lifecycle::Missed),
            RelayEvent::Resync,
            RelayEvent::Renew,
            RelayEvent::Bye {
                reason: "expired".into(),
                retry: Some(Duration::from_millis(1234)),
            },
        ]
    );
    // Then it closes, with the stream's `retry:` hint.
    assert_eq!(end.retry, Some(Duration::from_secs(10)));
}

#[tokio::test]
async fn a_silent_stream_is_dead_but_heartbeats_keep_it_alive() {
    // One-second heartbeats: silent for 2.5 seconds is dead.
    let relay = FakeRelay::start(1).await;
    let mut steps = vec![Step::Send(relay.ready())];
    for _ in 0..4 {
        steps.push(Step::Wait(Duration::from_millis(800)));
        steps.push(Step::Send(": ka\n\n".into()));
    }
    steps.push(Step::Hold);
    relay.stream_reply(Reply::Stream(steps));
    let (_client, _channel, mut stream) = open(&relay).await;
    let start = std::time::Instant::now();
    assert!(matches!(stream.next().await, Ok(RelayEvent::Ready(_))));
    let end = stream.next().await.unwrap_err();
    let waited = start.elapsed();
    assert!(end.reason.contains("silent"), "{end:?}");
    // The heartbeats (3.2 s of them) kept it open; silence ended it 2.5 s on.
    assert!(
        waited >= Duration::from_millis(5500) && waited < Duration::from_secs(8),
        "{waited:?}"
    );
}

#[tokio::test]
async fn refusals_are_told_apart() {
    let relay = FakeRelay::start(25).await;
    let client = HttpRelay::new(&relay.base);
    for (reply, expected) in [
        (
            Reply::refuse(401, "token_invalid", None),
            RelayError::Token { expiring: false },
        ),
        (
            Reply::refuse(401, "token_expiring", None),
            RelayError::Token { expiring: true },
        ),
        (
            Reply::refuse(400, "bad_installation", None),
            RelayError::BadRequest("bad_installation".into()),
        ),
        (
            Reply::refuse(403, "tenant_not_allowed", None),
            RelayError::NotAllowed,
        ),
        (
            Reply::refuse(429, "too_many_streams", Some(60)),
            RelayError::Busy {
                retry_after: Some(Duration::from_secs(60)),
            },
        ),
        (
            Reply::refuse(503, "busy", Some(7)),
            RelayError::Busy {
                retry_after: Some(Duration::from_secs(7)),
            },
        ),
        (
            Reply::refuse(503, "unavailable", None),
            RelayError::Busy { retry_after: None },
        ),
        (
            Reply::Json {
                status: 502,
                headers: vec![],
                body: "<html>Bad gateway</html>".into(),
            },
            RelayError::Failed("The push service answered HTTP 502.".into()),
        ),
    ] {
        relay.stream_reply(reply);
        let channel = client.channel(TOKEN, INSTALLATION).await.unwrap();
        let refused = client.events(TOKEN, INSTALLATION, &channel).await;
        assert_eq!(refused.err(), Some(expected.clone()));
        relay.channel_reply(Reply::refuse(401, "token_missing", None));
        assert_eq!(
            client.channel(TOKEN, INSTALLATION).await.err(),
            Some(RelayError::Token { expiring: false })
        );
    }
    // A channel missing its clientState is no channel.
    relay.channel_reply(Reply::Json {
        status: 200,
        headers: vec![],
        body: json!({ "v": 1, "notificationUrl": "https://x" }).to_string(),
    });
    assert!(matches!(
        client.channel(TOKEN, INSTALLATION).await,
        Err(RelayError::Failed(_))
    ));
}

#[tokio::test]
async fn the_token_goes_nowhere_but_the_service() {
    let relay = FakeRelay::start(25).await;
    relay.channel_reply(Reply::Json {
        status: 200,
        headers: vec![],
        body: json!({
            "v": 1,
            "notificationUrl": relay.notification_url(),
            "lifecycleNotificationUrl": relay.notification_url(),
            "clientState": CLIENT_STATE,
            "events": "http://127.0.0.2:9/v1/events",
            "heartbeatSeconds": 25,
        })
        .to_string(),
    });
    let (_client, _channel, mut stream) = open(&relay).await;
    assert!(matches!(stream.next().await, Ok(RelayEvent::Ready(_))));
    assert_eq!(relay.requests_to("/v1/events").len(), 1);
}

#[test]
fn the_service_address_comes_from_the_environment_or_the_build() {
    assert_eq!(resolve_sync_url(None, None), DEFAULT_SYNC_URL);
    assert_eq!(
        resolve_sync_url(None, Some("https://sync.example.org/")),
        "https://sync.example.org"
    );
    assert_eq!(
        resolve_sync_url(
            Some(" https://staging.example.org "),
            Some("https://sync.example.org")
        ),
        "https://staging.example.org"
    );
    assert_eq!(
        resolve_sync_url(Some("http://127.0.0.1:8090"), None),
        "http://127.0.0.1:8090"
    );
    // Tokens would cross the network in the clear: ignored.
    assert_eq!(
        resolve_sync_url(Some("http://sync.example.org"), None),
        DEFAULT_SYNC_URL
    );
    assert_eq!(resolve_sync_url(Some(""), None), DEFAULT_SYNC_URL);
    assert_eq!(resolve_sync_url(Some("not a url"), None), DEFAULT_SYNC_URL);
}

#[test]
fn unknown_or_unreadable_events_are_skipped() {
    assert_eq!(parse_event("someday", "{}"), None);
    assert_eq!(parse_event("change", "{not json"), None);
    assert_eq!(parse_event("change", r#"{"c":"created"}"#), None);
    assert_eq!(
        parse_event("lifecycle", r#"{"s":"x","e":"newKind"}"#),
        Some(RelayEvent::Lifecycle {
            subscription: "x".into(),
            event: Lifecycle::Other("newKind".into())
        })
    );
    // A bye always ends the stream, even one the app can't read.
    assert_eq!(
        parse_event("bye", "?"),
        Some(RelayEvent::Bye {
            reason: String::new(),
            retry: None
        })
    );
}
