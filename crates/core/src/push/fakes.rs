//! Fakes of the push service, its tokens and Graph's subscriptions, for the
//! push and service tests. They work on paused time: nothing touches a
//! socket.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::relay::{Channel, EventStream, Relay, RelayError, RelayEvent, StreamEnd};
use crate::auth::{SyncTokenError, SyncTokens};
use crate::error::{Error, ErrorCode, Result};
use crate::providers::{ChangeSubscriptions, NewSubscription, Subscription};

pub(crate) const NOTIFICATION_URL: &str =
    "https://sync.example.org/graph/v1/k1AbCdEfGhIjKlMnOpQrStUvWxYz012345";
pub(crate) const CLIENT_STATE: &str = "client-state-from-the-relay";

pub(crate) fn channel() -> Channel {
    Channel {
        v: Some(1),
        notification_url: NOTIFICATION_URL.into(),
        lifecycle_notification_url: NOTIFICATION_URL.into(),
        client_state: CLIENT_STATE.into(),
        events: Some("https://sync.example.org/v1/events".into()),
        heartbeat_seconds: Some(25),
        ends_at: None,
    }
}

/// An error as Graph gives it, with its HTTP status.
pub(crate) fn graph_status(status: u16) -> Error {
    if status == 404 {
        return Error::new(ErrorCode::NotFound, "The object was not found.");
    }
    Error::provider(format!("Graph said {status}"))
        .with_details(json!({ "status": status, "code": "ExtensionError" }))
}

type Item = std::result::Result<RelayEvent, StreamEnd>;

struct OpenStream {
    events: mpsc::UnboundedSender<Item>,
    open: Arc<AtomicBool>,
}

#[derive(Default)]
struct RelayState {
    channel_replies: VecDeque<std::result::Result<Channel, RelayError>>,
    stream_replies: VecDeque<std::result::Result<(), RelayError>>,
    /// The next stream doesn't send `ready` until told.
    hold_ready: bool,
    /// Each request: "channel" or "events", with its token and when.
    requests: Vec<(&'static str, String, Instant)>,
    streams: Vec<OpenStream>,
}

/// A push service whose answers and events the test scripts. Unscripted,
/// it gives the channel and opens streams that start with `ready`.
#[derive(Default)]
pub(crate) struct ScriptedRelay {
    state: Mutex<RelayState>,
}

impl ScriptedRelay {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn refuse_channel(&self, error: RelayError) {
        self.state
            .lock()
            .unwrap()
            .channel_replies
            .push_back(Err(error));
    }

    pub(crate) fn refuse_stream(&self, error: RelayError) {
        self.state
            .lock()
            .unwrap()
            .stream_replies
            .push_back(Err(error));
    }

    /// The next stream waits for [`Self::ready`] before its `ready`.
    pub(crate) fn hold_ready(&self) {
        self.state.lock().unwrap().hold_ready = true;
    }

    /// Sends `ready` on the latest stream.
    pub(crate) fn ready(&self) {
        self.send(RelayEvent::Ready(channel()));
    }

    /// Sends an event on the latest stream.
    pub(crate) fn send(&self, event: RelayEvent) {
        self.push(Ok(event));
    }

    /// Ends the latest stream as if the connection dropped.
    pub(crate) fn drop_stream(&self) {
        self.send_end(StreamEnd {
            reason: "The push service closed the stream.".into(),
            retry: None,
        });
    }

    /// Ends the latest stream with `end`.
    pub(crate) fn send_end(&self, end: StreamEnd) {
        self.push(Err(end));
    }

    fn push(&self, item: Item) {
        let state = self.state.lock().unwrap();
        let _ = state.streams.last().unwrap().events.send(item);
    }

    /// Requests of `kind` ("channel" or "events"): their tokens and times.
    pub(crate) fn requests(&self, kind: &str) -> Vec<(String, Instant)> {
        self.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(k, _, _)| *k == kind)
            .map(|(_, token, at)| (token.clone(), *at))
            .collect()
    }

    /// How many streams were opened.
    pub(crate) fn opened(&self) -> usize {
        self.state.lock().unwrap().streams.len()
    }

    /// How many streams the app still holds.
    pub(crate) fn open(&self) -> usize {
        let state = self.state.lock().unwrap();
        state
            .streams
            .iter()
            .filter(|s| s.open.load(Ordering::SeqCst))
            .count()
    }
}

#[async_trait]
impl Relay for ScriptedRelay {
    async fn channel(
        &self,
        token: &str,
        _installation: &str,
    ) -> std::result::Result<Channel, RelayError> {
        let mut state = self.state.lock().unwrap();
        state
            .requests
            .push(("channel", token.to_owned(), Instant::now()));
        state.channel_replies.pop_front().unwrap_or(Ok(channel()))
    }

    async fn events(
        &self,
        token: &str,
        _installation: &str,
        _channel: &Channel,
    ) -> std::result::Result<Box<dyn EventStream>, RelayError> {
        let mut state = self.state.lock().unwrap();
        state
            .requests
            .push(("events", token.to_owned(), Instant::now()));
        state.stream_replies.pop_front().unwrap_or(Ok(()))?;
        let (tx, rx) = mpsc::unbounded_channel();
        if !std::mem::take(&mut state.hold_ready) {
            tx.send(Ok(RelayEvent::Ready(channel()))).unwrap();
        }
        let open = Arc::new(AtomicBool::new(true));
        state.streams.push(OpenStream {
            events: tx,
            open: open.clone(),
        });
        Ok(Box::new(ScriptedStream { events: rx, open }))
    }
}

struct ScriptedStream {
    events: mpsc::UnboundedReceiver<Item>,
    open: Arc<AtomicBool>,
}

impl Drop for ScriptedStream {
    fn drop(&mut self) {
        self.open.store(false, Ordering::SeqCst);
    }
}

#[async_trait]
impl EventStream for ScriptedStream {
    async fn next(&mut self) -> Item {
        match self.events.recv().await {
            Some(item) => item,
            None => std::future::pending().await,
        }
    }
}

/// Push service tokens: `token-1`, `token-2`... unless scripted otherwise.
#[derive(Default)]
pub(crate) struct FakeTokens {
    script: Mutex<VecDeque<std::result::Result<(), SyncTokenError>>>,
    /// Each request: the time it had to last, and whether fresh.
    pub calls: Mutex<Vec<(Duration, bool)>>,
    issued: AtomicUsize,
}

impl FakeTokens {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn refuse(&self, error: SyncTokenError) {
        self.script.lock().unwrap().push_back(Err(error));
    }

    pub(crate) fn calls(&self) -> Vec<(Duration, bool)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl SyncTokens for FakeTokens {
    async fn sync_token(
        &self,
        min_left: Duration,
        fresh: bool,
    ) -> std::result::Result<String, SyncTokenError> {
        self.calls.lock().unwrap().push((min_left, fresh));
        self.script.lock().unwrap().pop_front().unwrap_or(Ok(()))?;
        let n = self.issued.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(format!("token-{n}"))
    }
}

/// Graph's subscriptions, in memory.
#[derive(Default)]
pub(crate) struct GraphState {
    pub subs: Vec<Subscription>,
    pub created: Vec<NewSubscription>,
    /// Renewed ids, and when.
    pub renewals: Vec<(String, Instant)>,
    /// Renewals asked for, whether they worked or not: ids and times.
    pub renew_attempts: Vec<(String, Instant)>,
    pub deleted: Vec<String>,
    pub lists: usize,
    pub fail_list: VecDeque<Error>,
    pub fail_create: VecDeque<Error>,
    pub fail_renew: VecDeque<Error>,
    /// `fail_renew` applies only to this subscription, when set.
    pub fail_renew_of: Option<String>,
    next_id: usize,
}

#[derive(Default)]
pub(crate) struct FakeGraph {
    pub state: Mutex<GraphState>,
}

impl FakeGraph {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The id of the subscription for `resource`.
    pub(crate) fn id_of(&self, resource: &str) -> String {
        let state = self.state.lock().unwrap();
        state
            .subs
            .iter()
            .find(|s| s.resource == resource)
            .unwrap_or_else(|| panic!("no subscription for {resource}"))
            .id
            .clone()
    }

    /// Renewals of the subscription for `resource`.
    pub(crate) fn renewals_of(&self, resource: &str) -> Vec<Instant> {
        let id = self.id_of(resource);
        let state = self.state.lock().unwrap();
        state
            .renewals
            .iter()
            .filter(|(r, _)| *r == id)
            .map(|(_, at)| *at)
            .collect()
    }
}

#[async_trait]
impl ChangeSubscriptions for FakeGraph {
    async fn list_subscriptions(&self) -> Result<Vec<Subscription>> {
        let mut state = self.state.lock().unwrap();
        state.lists += 1;
        if let Some(e) = state.fail_list.pop_front() {
            return Err(e);
        }
        Ok(state.subs.clone())
    }

    async fn create_subscription(&self, new: &NewSubscription) -> Result<Subscription> {
        let mut state = self.state.lock().unwrap();
        state.created.push(new.clone());
        if let Some(e) = state.fail_create.pop_front() {
            return Err(e);
        }
        state.next_id += 1;
        let sub = Subscription {
            id: format!("5ab1c0de-0000-0000-0000-{:012}", state.next_id),
            resource: new.resource.clone(),
            notification_url: Some(new.notification_url.clone()),
            expires_at: new.expires_at,
        };
        state.subs.push(sub.clone());
        Ok(sub)
    }

    async fn renew_subscription(
        &self,
        id: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<Subscription> {
        let mut state = self.state.lock().unwrap();
        state.renew_attempts.push((id.to_owned(), Instant::now()));
        if state.fail_renew_of.as_deref().is_none_or(|of| of == id)
            && let Some(e) = state.fail_renew.pop_front()
        {
            return Err(e);
        }
        let Some(sub) = state.subs.iter_mut().find(|s| s.id == id) else {
            return Err(graph_status(404));
        };
        sub.expires_at = expires_at;
        let sub = sub.clone();
        state.renewals.push((id.to_owned(), Instant::now()));
        Ok(sub)
    }

    async fn delete_subscription(&self, id: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.deleted.push(id.to_owned());
        state.subs.retain(|s| s.id != id);
        Ok(())
    }
}
