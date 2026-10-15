//! Push for Microsoft accounts: Graph change subscriptions, delivered
//! through the Cumulas push service ([`super::relay`]).
//!
//! [`GraphPush`] keeps a stream to the service open and the account's
//! subscriptions (the Inbox's messages and the calendar's events) in place:
//!
//! 1. It gets a token for the service with at least 10 minutes left, the
//!    account's channel, then the installation's stream.
//! 2. On every `ready` it reconciles with Graph: it lists the subscriptions,
//!    renews ours (those on the channel's URL), deletes ours left on an old
//!    URL, creates what's missing (a 409 means another installation just did:
//!    it lists again and adopts that one), then catches up on the Inbox and
//!    the calendar. Only then is push live.
//! 3. While live, each change syncs what its subscription watches. Each
//!    subscription is renewed at least every 30 minutes, since Graph pauses
//!    one whose token has expired, and never twice within 10 minutes. `renew`
//!    (or waking up) opens a new stream that replaces the old one once it is
//!    ready: make before break.
//! 4. Whenever it isn't live, the Inbox is checked every 30 seconds.
//!
//! | The service or Microsoft says                | Then (polling meanwhile)              |
//! | -------------------------------------------- | ------------------------------------- |
//! | No consent to the service's scope            | ConsentMissing; ask again in 6 hours  |
//! | The scope isn't offered                      | Unsupported; ask again in 6 hours     |
//! | 401 twice (the second time a fresh token)    | Unsupported; 6 hours                  |
//! | 403 `tenant_not_allowed`                     | Unsupported; 6 hours                  |
//! | 400 `bad_installation`                       | Logged as a bug; 6 hours              |
//! | 429 or 503                                   | Back off, at least `Retry-After`      |
//! | `bye`, a dropped or silent stream            | Back off, at least its `retry`        |
//! | Graph refuses the Inbox's subscription (403) | Unsupported; 6 hours                  |
//! | No sign-in, or a locked keyring              | Parked until a sync gets past it      |
//!
//! Subscriptions are only deleted when the account goes or stops using the
//! service (see `CoreService::drop_push_subscriptions`): stopping or
//! restarting push leaves them for the next connection to reconcile.

use std::collections::{HashSet, VecDeque};
use std::convert::Infallible;
use std::future::{Future, pending};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use tokio::sync::watch;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::backoff::Backoff;
use super::relay::{Channel, EventStream, Lifecycle, Relay, RelayError, RelayEvent, StreamEnd};
use super::supervisor::{EventFn, MICROSOFT_POLL, polling, sleep_until, went_offline, woken};
use super::{PushEvent, PushHealth, PushMode};
use crate::auth::{SyncTokenError, SyncTokens};
use crate::error::{Error, ErrorCode};
use crate::providers::{ChangeSubscriptions, NewSubscription, Subscription};
use crate::store::{PushSubscription, Store};

/// What the Inbox's subscription watches.
pub const INBOX_RESOURCE: &str = "me/mailFolders('inbox')/messages";
/// What the calendar's subscription watches.
pub const EVENTS_RESOURCE: &str = "me/events";
const CHANGE_TYPES: &str = "created,updated,deleted";

/// A stream is opened with a token that has at least this long left (the
/// service wants six minutes).
pub const MIN_TOKEN_LEFT: Duration = Duration::from_secs(10 * 60);
/// Subscriptions are made to last this long, within Graph's maximum for
/// messages and events.
pub const SUBSCRIPTION_MINUTES: i64 = 4230;
/// While connected, each subscription is renewed (and so reauthorized) at
/// least this often.
pub const RENEW_EVERY: Duration = Duration::from_secs(30 * 60);
/// Graph wants a subscription's reauthorizations at least this far apart.
pub const REAUTHORIZE_GAP: Duration = Duration::from_secs(10 * 60);
/// How long the service is left alone after it (or Microsoft) refused the
/// account for a reason that won't pass soon.
pub const COOL_DOWN: Duration = Duration::from_secs(6 * 3600);
/// A renewal or creation that failed is tried again after this, doubling
/// with each failure up to an hour.
pub const RETRY_SUBSCRIPTION: Duration = Duration::from_secs(2 * 60);
const MAX_RETRY_SUBSCRIPTION: Duration = Duration::from_secs(3600);
/// A replacement stream that failed while the old one still works is tried
/// again after this, doubling with each failure.
const RETRY_REPLACEMENT: Duration = Duration::from_secs(30);

/// The hint shown while consent to the service is missing.
pub const CONSENT_HINT: &str = "Sign in again to turn on instant updates.";

/// Receives the mode, health and an explanation for the user.
pub type ReportFn = Arc<dyn Fn(PushMode, PushHealth, Option<String>) + Send + Sync>;
/// Parks push (see `CoreService::park_push`) until the account's sign-in
/// or the keyring works again.
pub type ParkFn = Arc<dyn Fn(&Error) + Send + Sync>;
/// The current time, by the wall clock (tests tie it to Tokio's).
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// How a Microsoft account reaches the push service.
#[derive(Clone)]
pub struct RelaySource {
    pub relay: Arc<dyn Relay>,
    pub tokens: Arc<dyn SyncTokens>,
}

/// What a subscription watches, and so what its changes sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Watched {
    Inbox,
    Calendar,
}

impl Watched {
    const ALL: [Watched; 2] = [Watched::Inbox, Watched::Calendar];

    fn resource(self) -> &'static str {
        match self {
            Watched::Inbox => INBOX_RESOURCE,
            Watched::Calendar => EVENTS_RESOURCE,
        }
    }

    fn event(self) -> PushEvent {
        match self {
            Watched::Inbox => PushEvent::InboxChanged,
            Watched::Calendar => PushEvent::CalendarChanged,
        }
    }

    /// Recognizes a resource as Graph lists it (in its own case, perhaps
    /// with a leading slash).
    fn of(resource: &str) -> Option<Self> {
        let resource = resource.trim_start_matches('/').to_ascii_lowercase();
        if resource == INBOX_RESOURCE.to_ascii_lowercase() {
            Some(Watched::Inbox)
        } else if resource == EVENTS_RESOURCE {
            Some(Watched::Calendar)
        } else {
            None
        }
    }
}

/// One of our subscriptions.
#[derive(Debug, Clone)]
struct Sub {
    id: String,
    resource: String,
    watched: Option<Watched>,
    expires_at: DateTime<Utc>,
    /// When it was last created or renewed, here or (judging by its expiry)
    /// by another installation.
    renewed: Instant,
    /// When to renew it next.
    due: Instant,
    /// Renewals that failed in a row.
    failures: u32,
}

/// A watched resource without a subscription.
#[derive(Debug, Clone, Copy)]
struct Missing {
    watched: Watched,
    /// When to create it.
    at: Instant,
    /// Creations that failed in a row.
    failures: u32,
}

/// How long to wait after `failures` failures in a row.
fn retry_after(failures: u32) -> Duration {
    (RETRY_SUBSCRIPTION * 2u32.pow(failures.saturating_sub(1).min(5))).min(MAX_RETRY_SUBSCRIPTION)
}

/// Why a connection ended, and what comes next.
#[derive(Debug)]
enum Ended {
    Cancelled,
    Offline,
    /// Woken up while connecting: start again now.
    Woken,
    /// Try again after the backoff, and no sooner than `at_least`.
    Retry {
        reason: String,
        at_least: Duration,
    },
    /// Refused for a reason that won't pass soon: poll, and ask again after
    /// `wait`.
    CoolDown {
        health: PushHealth,
        detail: String,
        wait: Duration,
    },
    /// The sign-in is missing or the keyring is locked: asking again would
    /// only bring the keyring's prompt back, so push waits to be restarted.
    Park(Error),
}

impl Ended {
    fn retry(reason: impl Into<String>) -> Self {
        Ended::Retry {
            reason: reason.into(),
            at_least: Duration::ZERO,
        }
    }

    fn unsupported(detail: impl Into<String>) -> Self {
        Ended::CoolDown {
            health: PushHealth::Unsupported,
            detail: detail.into(),
            wait: COOL_DOWN,
        }
    }
}

fn token_ended(e: SyncTokenError) -> Ended {
    match e {
        SyncTokenError::ConsentMissing => Ended::CoolDown {
            health: PushHealth::ConsentMissing,
            detail: CONSENT_HINT.into(),
            wait: COOL_DOWN,
        },
        SyncTokenError::NotOffered => Ended::unsupported(
            "This app registration doesn't offer instant updates for Microsoft accounts yet.",
        ),
        SyncTokenError::Failed(e)
            if matches!(e.code, ErrorCode::Storage | ErrorCode::AuthRequired) =>
        {
            Ended::Park(e)
        }
        // Microsoft's own text can name the user: not for logs.
        SyncTokenError::Failed(e) => Ended::retry(match e.code {
            ErrorCode::Network => "Could not reach Microsoft sign-in.",
            _ => "Microsoft sign-in didn't give a token for the push service.",
        }),
    }
}

/// A refusal after any retry with a fresh token.
fn refusal_ended(e: RelayError) -> Ended {
    match e {
        RelayError::Token { .. } => {
            Ended::unsupported("The push service didn't accept this account's sign-in.")
        }
        RelayError::BadRequest(error) => {
            tracing::error!("the push service refused a request as malformed ({error})");
            Ended::CoolDown {
                health: PushHealth::Degraded {
                    reason: "The push service refused the app's request.".into(),
                },
                detail: "The push service refused the app's request.".into(),
                wait: COOL_DOWN,
            }
        }
        RelayError::NotAllowed => {
            Ended::unsupported("The push service doesn't serve this organization.")
        }
        RelayError::Busy { retry_after } => Ended::Retry {
            reason: e.to_string(),
            at_least: retry_after.unwrap_or_default(),
        },
        RelayError::Failed(why) => Ended::retry(why),
    }
}

/// Why a Graph call failed, fit for logs and the UI: Graph's own messages
/// can name subscriptions and users.
fn graph_reason(e: &Error) -> String {
    match (e.code, e.http_status()) {
        (ErrorCode::Network, _) => "Could not reach Microsoft Graph.".into(),
        (ErrorCode::AuthRequired, _) => "Microsoft didn't accept the sign-in.".into(),
        (_, Some(status)) => format!("Microsoft Graph refused the subscription (HTTP {status})."),
        _ => "Microsoft Graph failed.".into(),
    }
}

/// The cancel, wake and network signals the push task obeys.
struct Signals {
    cancel: CancellationToken,
    wake: watch::Receiver<u64>,
    online: watch::Receiver<bool>,
}

/// Runs `work` until it ends, or cancel, offline or a wake ends it first.
async fn guard<T>(
    signals: &mut Signals,
    work: impl Future<Output = Result<T, Ended>>,
) -> Result<T, Ended> {
    tokio::select! {
        biased;
        _ = signals.cancel.cancelled() => Err(Ended::Cancelled),
        _ = went_offline(&mut signals.online) => Err(Ended::Offline),
        _ = woken(&mut signals.wake) => Err(Ended::Woken),
        out = work => out,
    }
}

/// Reaches the push service.
#[derive(Clone)]
struct Link {
    source: RelaySource,
    installation: String,
}

impl Link {
    async fn token(&self, fresh: bool) -> Result<String, Ended> {
        self.source
            .tokens
            .sync_token(MIN_TOKEN_LEFT, fresh)
            .await
            .map_err(token_ended)
    }

    /// The account's channel, and the token that got it. A refused token
    /// is tried once more, fresh.
    async fn channel(&self) -> Result<(String, Channel), Ended> {
        let mut fresh = false;
        loop {
            let token = self.token(fresh).await?;
            match self.source.relay.channel(&token, &self.installation).await {
                Ok(channel) => return Ok((token, channel)),
                Err(RelayError::Token { .. }) if !fresh => fresh = true,
                Err(e) => return Err(refusal_ended(e)),
            }
        }
    }

    /// Opens a stream (with `token` if given, else a new one; `fresh`
    /// bypasses the cache) and waits for its `ready`: the stream, and the
    /// channel as `ready` gave it. A refused token is tried once more,
    /// fresh.
    async fn open(
        self,
        channel: Channel,
        mut token: Option<String>,
        mut fresh: bool,
    ) -> Result<(Box<dyn EventStream>, Channel), Ended> {
        let mut stream = loop {
            let t = match token.take() {
                Some(t) => t,
                None => self.token(fresh).await?,
            };
            match self
                .source
                .relay
                .events(&t, &self.installation, &channel)
                .await
            {
                Ok(stream) => break stream,
                Err(RelayError::Token { .. }) if !fresh => fresh = true,
                Err(e) => return Err(refusal_ended(e)),
            }
        };
        match stream.next().await {
            Ok(RelayEvent::Ready(ready)) => {
                let ready = merge(ready, &channel);
                if ready.notification_url.is_empty() || ready.client_state.is_empty() {
                    return Err(Ended::retry("The push service sent an incomplete channel."));
                }
                Ok((stream, ready))
            }
            Ok(_) => Err(Ended::retry(
                "The push service didn't start the stream with ready.",
            )),
            Err(end) => Err(stream_ended(end)),
        }
    }
}

fn stream_ended(end: StreamEnd) -> Ended {
    Ended::Retry {
        reason: end.reason,
        at_least: end.retry.unwrap_or_default(),
    }
}

/// `ready`'s channel, completed from the one `POST /v1/channels` gave.
fn merge(mut ready: Channel, channel: &Channel) -> Channel {
    let fill = |field: &mut String, from: &String| {
        if field.is_empty() {
            field.clone_from(from);
        }
    };
    fill(&mut ready.notification_url, &channel.notification_url);
    fill(
        &mut ready.lifecycle_notification_url,
        &channel.lifecycle_notification_url,
    );
    fill(&mut ready.client_state, &channel.client_state);
    ready.events = channel.events.clone();
    ready.heartbeat_seconds = ready.heartbeat_seconds.or(channel.heartbeat_seconds);
    ready
}

/// Whether two URLs share scheme, host and port.
fn same_origin(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}

/// Graph work a connection does between events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    /// After every `ready`: list, renew, clean up, create, catch up.
    Reconcile,
    /// Renew what's due; create again what went missing.
    Maintain,
}

/// The account's subscriptions, and the Graph calls that keep them.
struct Subs {
    account_id: String,
    graph: Arc<dyn ChangeSubscriptions>,
    store: Arc<Store>,
    events: EventFn,
    clock: Clock,
    list: Vec<Sub>,
    missing: Vec<Missing>,
    /// Graph refused the calendar's subscription: the calendar syncs on
    /// schedule until the next reconcile tries again.
    calendar_refused: bool,
    /// Why the Inbox's subscription isn't in order; `None` when it is.
    problem: Option<String>,
}

impl Subs {
    fn expiry(&self) -> DateTime<Utc> {
        (self.clock)() + chrono::Duration::minutes(SUBSCRIPTION_MINUTES)
    }

    /// When a listed subscription was last renewed, by anyone: every
    /// installation sets its expiry to the same distance from then.
    fn renewed_from_expiry(&self, expires_at: DateTime<Utc>) -> Option<Instant> {
        let renewed = expires_at - chrono::Duration::minutes(SUBSCRIPTION_MINUTES);
        let age = ((self.clock)() - renewed).to_std().unwrap_or_default();
        Instant::now().checked_sub(age)
    }

    fn ours(&self, s: Subscription) -> Sub {
        let now = Instant::now();
        let known = self.list.iter().find(|k| k.id.eq_ignore_ascii_case(&s.id));
        // Whichever renewal was later: ours, or another installation's.
        let renewed = match (
            known.map(|k| k.renewed),
            self.renewed_from_expiry(s.expires_at),
        ) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        let renewed = renewed.unwrap_or_else(|| now.checked_sub(RENEW_EVERY).unwrap_or(now));
        Sub {
            watched: Watched::of(&s.resource),
            id: s.id,
            resource: s.resource,
            expires_at: s.expires_at,
            renewed,
            due: renewed + RENEW_EVERY,
            failures: 0,
        }
    }

    fn created(s: Subscription, watched: Watched) -> Sub {
        let now = Instant::now();
        Sub {
            id: s.id,
            resource: watched.resource().to_owned(),
            watched: Some(watched),
            expires_at: s.expires_at,
            renewed: now,
            due: now + RENEW_EVERY,
            failures: 0,
        }
    }

    fn inbox_ok(&self) -> bool {
        self.problem.is_none() && self.list.iter().any(|s| s.watched == Some(Watched::Inbox))
    }

    /// When the next renewal or creation is due.
    fn next_due(&self) -> Option<Instant> {
        let renewals = self.list.iter().map(|s| s.due);
        let creations = self.missing.iter().map(|m| m.at);
        renewals.chain(creations).min()
    }

    fn catch_up(&self) {
        for watched in Watched::ALL {
            (self.events)(watched.event());
        }
    }

    /// Syncs what a subscription watches; everything when it's unknown.
    fn changed(&self, id: &str) {
        match self.find(id).and_then(|s| s.watched) {
            Some(watched) => (self.events)(watched.event()),
            None => self.catch_up(),
        }
    }

    fn find(&self, id: &str) -> Option<&Sub> {
        self.list.iter().find(|s| s.id.eq_ignore_ascii_case(id))
    }

    /// Graph asked for a reauthorization: renew as soon as the gap since
    /// the last one allows.
    fn reauthorize(&mut self, id: &str) {
        let now = Instant::now();
        if let Some(sub) = self.list.iter_mut().find(|s| s.id.eq_ignore_ascii_case(id)) {
            sub.due = sub.due.min((sub.renewed + REAUTHORIZE_GAP).max(now));
        }
    }

    /// Graph removed a subscription: make it again now.
    fn removed(&mut self, id: &str) {
        let Some(at) = self.list.iter().position(|s| s.id.eq_ignore_ascii_case(id)) else {
            return;
        };
        let sub = self.list.remove(at);
        if let Some(watched) = sub.watched {
            self.need(watched, Instant::now(), 0);
        }
        self.save();
    }

    fn need(&mut self, watched: Watched, at: Instant, failures: u32) {
        if !self.missing.iter().any(|m| m.watched == watched) {
            self.missing.push(Missing {
                watched,
                at,
                failures,
            });
        }
    }

    fn save(&self) {
        let rows: Vec<PushSubscription> = self
            .list
            .iter()
            .map(|s| PushSubscription {
                id: s.id.clone(),
                resource: s.resource.clone(),
                expires_at: s.expires_at,
            })
            .collect();
        if let Err(e) = self
            .store
            .replace_push_subscriptions(&self.account_id, &rows)
        {
            tracing::warn!(
                "push for account {}: could not save its subscriptions: {e}",
                self.account_id
            );
        }
    }

    async fn run(&mut self, job: Job, channel: &Channel) -> Result<(), Ended> {
        match job {
            Job::Reconcile => self.reconcile(channel).await,
            Job::Maintain => self.maintain(channel).await,
        }
    }

    /// Brings Graph's subscriptions in line with the channel, then catches
    /// up on what they watch.
    async fn reconcile(&mut self, channel: &Channel) -> Result<(), Ended> {
        let listed = self
            .graph
            .list_subscriptions()
            .await
            .map_err(|e| Ended::retry(graph_reason(&e)))?;
        // Ours: what this installation kept, or made, before.
        let mut known: HashSet<String> = self.list.iter().map(|s| s.id.to_lowercase()).collect();
        match self.store.push_subscriptions(&self.account_id) {
            Ok(rows) => known.extend(rows.into_iter().map(|r| r.id.to_lowercase())),
            Err(e) => tracing::warn!(
                "push for account {}: could not read its subscriptions: {e}",
                self.account_id
            ),
        }
        let mut current = Vec::new();
        let mut stale = Vec::new();
        for s in listed {
            let url = s.notification_url.as_deref().unwrap_or_default();
            if url == channel.notification_url {
                current.push(self.ours(s));
            } else if known.contains(&s.id.to_lowercase())
                || (same_origin(url, &channel.notification_url)
                    && Watched::of(&s.resource).is_some())
            {
                // Ours on a URL the service no longer uses (its key changed),
                // or one this installation made for another service.
                stale.push(s);
            }
            // Anything else is another app's, or another service's: left
            // alone.
        }
        // Stale ones go first, so their replacements don't conflict.
        for s in &stale {
            self.graph
                .delete_subscription(&s.id)
                .await
                .map_err(|e| Ended::retry(graph_reason(&e)))?;
        }
        let now = Instant::now();
        let mut renewed = Vec::new();
        for mut sub in current {
            if now.saturating_duration_since(sub.renewed) >= REAUTHORIZE_GAP {
                match self.graph.renew_subscription(&sub.id, self.expiry()).await {
                    Ok(s) => {
                        sub.expires_at = s.expires_at;
                        sub.renewed = Instant::now();
                        sub.due = sub.renewed + RENEW_EVERY;
                    }
                    // Gone since it was listed.
                    Err(e) if e.code == ErrorCode::NotFound => continue,
                    Err(e) => return Err(Ended::retry(graph_reason(&e))),
                }
            }
            renewed.push(sub);
        }
        self.list = renewed;
        self.missing.clear();
        self.calendar_refused = false;
        self.problem = None;
        for watched in Watched::ALL {
            if self.list.iter().any(|s| s.watched == Some(watched)) {
                continue;
            }
            match self.create(watched, channel).await {
                Ok(sub) => self.list.push(sub),
                Err(e) => {
                    self.save();
                    self.refused(watched, e, 1)?;
                }
            }
        }
        self.save();
        self.catch_up();
        Ok(())
    }

    /// Renews the subscriptions that are due and creates the missing ones
    /// whose turn has come. Failures are tried again later; meanwhile a
    /// broken Inbox subscription means polling.
    async fn maintain(&mut self, channel: &Channel) -> Result<(), Ended> {
        let now = Instant::now();
        let mut changed = false;
        let mut gone = Vec::new();
        for at in 0..self.list.len() {
            if self.list[at].due > now {
                continue;
            }
            let id = self.list[at].id.clone();
            let result = self.graph.renew_subscription(&id, self.expiry()).await;
            let sub = &mut self.list[at];
            match result {
                Ok(s) => {
                    sub.expires_at = s.expires_at;
                    sub.renewed = Instant::now();
                    sub.due = sub.renewed + RENEW_EVERY;
                    sub.failures = 0;
                    if sub.watched == Some(Watched::Inbox) {
                        self.problem = None;
                    }
                    changed = true;
                }
                // Expired, or deleted elsewhere: made again below.
                Err(e) if e.code == ErrorCode::NotFound => gone.push(at),
                Err(e) => {
                    let reason = graph_reason(&e);
                    tracing::info!(
                        "push for account {}: renewing a subscription failed: {reason}",
                        self.account_id
                    );
                    sub.failures += 1;
                    sub.due = Instant::now() + retry_after(sub.failures);
                    if sub.watched == Some(Watched::Inbox) {
                        self.problem = Some(reason);
                    }
                }
            }
        }
        for at in gone.into_iter().rev() {
            if let Some(watched) = self.list.remove(at).watched {
                self.need(watched, now, 0);
            }
            changed = true;
        }
        let due: Vec<Missing> = self
            .missing
            .iter()
            .filter(|m| m.at <= now)
            .copied()
            .collect();
        for Missing {
            watched, failures, ..
        } in due
        {
            self.missing.retain(|m| m.watched != watched);
            match self.create(watched, channel).await {
                Ok(sub) => {
                    self.list.push(sub);
                    if watched == Watched::Inbox {
                        self.problem = None;
                    }
                    changed = true;
                    // Changes between its removal and now weren't sent.
                    (self.events)(watched.event());
                }
                Err(e) => {
                    if changed {
                        self.save();
                    }
                    self.refused(watched, e, failures + 1)?;
                }
            }
        }
        if changed {
            self.save();
        }
        Ok(())
    }

    /// A subscription couldn't be created (for the `failures`th time in a
    /// row): Graph not offering the Inbox's ends the connection; the
    /// calendar's is left until the next reconcile; anything else is tried
    /// again later.
    fn refused(&mut self, watched: Watched, e: Error, failures: u32) -> Result<(), Ended> {
        if e.http_status() == Some(403) {
            return match watched {
                Watched::Inbox => Err(Ended::unsupported(
                    "Microsoft doesn't offer instant updates for this mailbox.",
                )),
                Watched::Calendar => {
                    if !self.calendar_refused {
                        tracing::info!(
                            "push for account {}: Microsoft refused to watch the calendar; \
                             it syncs on schedule",
                            self.account_id
                        );
                    }
                    self.calendar_refused = true;
                    Ok(())
                }
            };
        }
        let reason = graph_reason(&e);
        tracing::info!(
            "push for account {}: creating a subscription failed: {reason}",
            self.account_id
        );
        if watched == Watched::Inbox {
            self.problem = Some(reason);
        }
        self.need(watched, Instant::now() + retry_after(failures), failures);
        Ok(())
    }

    /// Creates the subscription for `watched` on the channel. A conflict
    /// means another installation of the account just made it: that one is
    /// adopted.
    async fn create(&self, watched: Watched, channel: &Channel) -> Result<Sub, Error> {
        let new = NewSubscription {
            resource: watched.resource().to_owned(),
            change_type: CHANGE_TYPES.to_owned(),
            notification_url: channel.notification_url.clone(),
            lifecycle_notification_url: channel.lifecycle_notification_url.clone(),
            client_state: channel.client_state.clone(),
            expires_at: self.expiry(),
        };
        match self.graph.create_subscription(&new).await {
            Ok(s) => Ok(Self::created(s, watched)),
            Err(e) if e.http_status() == Some(409) => {
                let listed = self.graph.list_subscriptions().await?;
                listed
                    .into_iter()
                    .find(|s| {
                        s.notification_url.as_deref() == Some(channel.notification_url.as_str())
                            && Watched::of(&s.resource) == Some(watched)
                    })
                    .map(|s| self.ours(s))
                    .ok_or(e)
            }
            Err(e) => Err(e),
        }
    }
}

/// A stream being opened, until its `ready`.
type Opening = BoxFuture<'static, Result<(Box<dyn EventStream>, Channel), Ended>>;

/// The streams of one connection: the current one, and the one replacing
/// it.
struct Streams {
    current: Box<dyn EventStream>,
    /// Whether `current` still delivers.
    open: bool,
    /// The channel as the latest `ready` gave it.
    channel: Channel,
    /// Why `current` ended, once it did.
    ended: Option<Ended>,
    replacement: Option<Opening>,
    /// Whether the replacement asked for a fresh token.
    replacing_fresh: bool,
    /// Replacements that failed in a row, and when to try again.
    replacement_failures: u32,
    retry_replacement: Option<Instant>,
    /// A replacement refused for a reason that won't pass soon: it applies
    /// once the current stream ends.
    replacement_refused: Option<Ended>,
    /// What `current` delivered while Graph work ran.
    held: VecDeque<Result<RelayEvent, StreamEnd>>,
}

impl Streams {
    /// Records what the stream said; its end closes it.
    fn hold(&mut self, item: Result<RelayEvent, StreamEnd>) {
        if item.is_err() {
            self.open = false;
        }
        self.held.push_back(item);
    }

    /// Whether events still arrive, or will once the replacement is ready.
    fn alive(&self) -> bool {
        self.open || self.replacement.is_some()
    }
}

async fn next_replacement(
    replacement: &mut Option<Opening>,
) -> Result<(Box<dyn EventStream>, Channel), Ended> {
    match replacement {
        Some(future) => future.await,
        None => pending().await,
    }
}

/// Runs Graph `work` while still reading the stream (into `held`), so
/// nothing it sends is lost or held up; cancel and going offline end it.
async fn alongside<T>(
    signals: &mut Signals,
    streams: &mut Streams,
    wake_pending: &mut bool,
    work: impl Future<Output = T>,
) -> Result<T, Ended> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            biased;
            _ = signals.cancel.cancelled() => return Err(Ended::Cancelled),
            _ = went_offline(&mut signals.online) => return Err(Ended::Offline),
            out = &mut work => return Ok(out),
            item = streams.current.next(), if streams.open => streams.hold(item),
            _ = woken(&mut signals.wake) => *wake_pending = true,
        }
    }
}

/// What one connection has shown.
#[derive(Default)]
struct Connected {
    /// When it first went live.
    live_since: Option<Instant>,
    /// Whether it last reported live.
    shown: Option<bool>,
}

/// Push for one Microsoft account through the Cumulas push service.
pub struct GraphPush {
    /// For logs.
    account_id: String,
    link: Link,
    subs: Subs,
    signals: Signals,
    status: ReportFn,
    park: Option<ParkFn>,
    backoff: Backoff,
    /// Whether push is live; the Inbox is polled while it isn't.
    live: watch::Sender<bool>,
}

impl GraphPush {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account_id: impl Into<String>,
        installation: impl Into<String>,
        source: RelaySource,
        graph: Arc<dyn ChangeSubscriptions>,
        store: Arc<Store>,
        events: EventFn,
        status: ReportFn,
        cancel: CancellationToken,
        wake: watch::Receiver<u64>,
        online: watch::Receiver<bool>,
    ) -> Self {
        let account_id = account_id.into();
        Self {
            link: Link {
                source,
                installation: installation.into(),
            },
            subs: Subs {
                account_id: account_id.clone(),
                graph,
                store,
                events,
                clock: Arc::new(Utc::now),
                list: Vec::new(),
                missing: Vec::new(),
                calendar_refused: false,
                problem: None,
            },
            account_id,
            signals: Signals {
                cancel,
                wake,
                online,
            },
            status,
            park: None,
            backoff: Backoff::new(),
            live: watch::Sender::new(false),
        }
    }

    /// Calls `park` when push should wait to be restarted.
    pub fn with_park(mut self, park: ParkFn) -> Self {
        self.park = Some(park);
        self
    }

    /// Uses `backoff` instead of a randomly seeded one.
    pub fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    /// Reads the wall clock through `clock`.
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.subs.clock = clock;
        self
    }

    fn report(&self, mode: PushMode, health: PushHealth, detail: Option<String>) {
        (self.status)(mode, health, detail);
    }

    /// Runs until cancelled.
    pub async fn run(mut self) {
        let poller = poll_while_down(
            self.live.subscribe(),
            self.signals.online.clone(),
            Arc::clone(&self.subs.events),
        );
        tokio::select! {
            _ = self.supervise() => {}
            _ = poller => {}
        }
    }

    async fn supervise(&mut self) {
        loop {
            if !self.wait_online().await {
                return;
            }
            self.signals.wake.borrow_and_update();
            let mut state = Connected::default();
            let ended = match self.connection(&mut state).await {
                Err(ended) => ended,
                Ok(never) => match never {},
            };
            self.set_live(false);
            self.backoff
                .connection_ended(state.live_since.map(|t| t.elapsed()).unwrap_or_default());
            let (wait, woken_ends) = match ended {
                Ended::Cancelled => return,
                Ended::Offline => continue,
                Ended::Woken => {
                    self.backoff.reset();
                    continue;
                }
                Ended::Retry { reason, at_least } => {
                    let delay = self.backoff.next_delay().max(at_least);
                    tracing::info!(
                        "push for account {}: {reason} Checking the Inbox every {}s; \
                         reconnecting in {:.1}s",
                        self.account_id,
                        MICROSOFT_POLL.as_secs(),
                        delay.as_secs_f32()
                    );
                    self.report(
                        polling(MICROSOFT_POLL),
                        PushHealth::Degraded { reason },
                        None,
                    );
                    (delay, true)
                }
                Ended::CoolDown {
                    health,
                    detail,
                    wait,
                } => {
                    tracing::info!(
                        "push for account {}: {detail} Checking the Inbox every {}s; \
                         asking again in {} hours",
                        self.account_id,
                        MICROSOFT_POLL.as_secs(),
                        wait.as_secs() / 3600
                    );
                    self.report(polling(MICROSOFT_POLL), health, Some(detail));
                    (wait, false)
                }
                Ended::Park(e) => {
                    let health = if e.code == ErrorCode::AuthRequired {
                        PushHealth::AuthRequired
                    } else {
                        PushHealth::Degraded {
                            reason: e.message.clone(),
                        }
                    };
                    self.report(polling(MICROSOFT_POLL), health, None);
                    if let Some(park) = &self.park {
                        park(&e);
                    }
                    self.signals.cancel.cancelled().await;
                    return;
                }
            };
            if !self.pause(wait, woken_ends).await {
                return;
            }
        }
    }

    /// Returns at once when online; otherwise reports it and waits. False
    /// when cancelled meanwhile.
    async fn wait_online(&mut self) -> bool {
        if *self.signals.online.borrow_and_update() {
            return !self.signals.cancel.is_cancelled();
        }
        self.report(
            polling(MICROSOFT_POLL),
            PushHealth::Degraded {
                reason: "No network connection.".to_owned(),
            },
            None,
        );
        tokio::select! {
            _ = self.signals.cancel.cancelled() => false,
            online = self.signals.online.wait_for(|online| *online) => online.is_ok(),
        }
    }

    /// Waits `wait`; waking up ends it early when `woken_ends`. False when
    /// cancelled.
    async fn pause(&mut self, wait: Duration, woken_ends: bool) -> bool {
        let deadline = Instant::now() + wait;
        loop {
            tokio::select! {
                biased;
                _ = self.signals.cancel.cancelled() => return false,
                _ = went_offline(&mut self.signals.online) => return true,
                _ = tokio::time::sleep_until(deadline) => return true,
                _ = woken(&mut self.signals.wake) => {
                    if woken_ends {
                        self.backoff.reset();
                        return true;
                    }
                }
            }
        }
    }

    fn set_live(&self, live: bool) -> bool {
        self.live.send_if_modified(|current| {
            let changed = *current != live;
            *current = live;
            changed
        })
    }

    /// Reports whether push is live, the first time in a connection and
    /// whenever that changes.
    fn update_live(&self, streams: &Streams, state: &mut Connected) {
        let live = streams.alive() && self.subs.inbox_ok();
        self.set_live(live);
        if state.shown == Some(live) {
            return;
        }
        state.shown = Some(live);
        if live {
            tracing::info!("push for account {}: live", self.account_id);
            state.live_since.get_or_insert_with(Instant::now);
            self.report(PushMode::GraphRelay, PushHealth::Live, None);
        } else {
            let reason = self
                .subs
                .problem
                .clone()
                .unwrap_or_else(|| "Reconnecting to the push service.".into());
            self.report(
                polling(MICROSOFT_POLL),
                PushHealth::Degraded { reason },
                None,
            );
        }
    }

    /// One connection: connects, reconciles, then follows the stream (and
    /// its replacements) until something ends it.
    async fn connection(&mut self, state: &mut Connected) -> Result<Infallible, Ended> {
        self.report(polling(MICROSOFT_POLL), PushHealth::Connecting, None);
        let link = self.link.clone();
        let (token, channel) = guard(&mut self.signals, link.channel()).await?;
        let (stream, channel) = guard(
            &mut self.signals,
            link.clone().open(channel, Some(token), false),
        )
        .await?;
        let mut streams = Streams {
            current: stream,
            open: true,
            channel,
            ended: None,
            replacement: None,
            replacing_fresh: false,
            replacement_failures: 0,
            retry_replacement: None,
            replacement_refused: None,
            held: VecDeque::new(),
        };
        let mut jobs = VecDeque::from([Job::Reconcile]);
        let mut wake_pending = false;
        loop {
            // Graph work first, reading the stream meanwhile.
            while let Some(job) = jobs.pop_front() {
                let channel = streams.channel.clone();
                let work = self.subs.run(job, &channel);
                alongside(&mut self.signals, &mut streams, &mut wake_pending, work).await??;
                self.update_live(&streams, state);
            }
            // Then what arrived meanwhile, in order.
            if let Some(item) = streams.held.pop_front() {
                self.on_event(item, &mut streams, &mut jobs);
                continue;
            }
            if wake_pending {
                wake_pending = false;
                self.replace(&mut streams, false);
            }
            if !streams.alive() {
                return Err(streams
                    .replacement_refused
                    .take()
                    .or_else(|| streams.ended.take())
                    .unwrap_or_else(|| Ended::retry("The push service closed the stream.")));
            }
            self.update_live(&streams, state);
            let due = self.subs.next_due();
            let retry_at = streams.retry_replacement;
            tokio::select! {
                biased;
                _ = self.signals.cancel.cancelled() => return Err(Ended::Cancelled),
                _ = went_offline(&mut self.signals.online) => return Err(Ended::Offline),
                item = streams.current.next(), if streams.open => streams.hold(item),
                replaced = next_replacement(&mut streams.replacement) => {
                    streams.replacement = None;
                    match replaced {
                        Ok((stream, channel)) => {
                            tracing::debug!("push for account {}: switched streams", self.account_id);
                            // The old stream's last events are still in `held`.
                            streams.current = stream;
                            streams.open = true;
                            streams.ended = None;
                            streams.channel = channel;
                            streams.replacement_failures = 0;
                            jobs.push_back(Job::Reconcile);
                        }
                        Err(Ended::Retry { reason, at_least }) if streams.open => {
                            tracing::debug!(
                                "push for account {}: a new stream failed ({reason}); keeping the old one",
                                self.account_id
                            );
                            let wait = RETRY_REPLACEMENT * 2u32.pow(streams.replacement_failures.min(4));
                            streams.replacement_failures += 1;
                            streams.retry_replacement = Some(Instant::now() + wait.max(at_least));
                        }
                        // The old stream serves until it ends.
                        Err(ended) if streams.open => streams.replacement_refused = Some(ended),
                        Err(ended) => return Err(ended),
                    }
                }
                _ = woken(&mut self.signals.wake) => wake_pending = true,
                _ = sleep_until(due) => jobs.push_back(Job::Maintain),
                _ = sleep_until(retry_at) => {
                    let fresh = streams.replacing_fresh;
                    self.replace(&mut streams, fresh);
                }
            }
        }
    }

    /// Opens a stream to replace the current one once it's ready.
    fn replace(&self, streams: &mut Streams, fresh: bool) {
        if streams.replacement.is_some() || streams.replacement_refused.is_some() {
            return;
        }
        streams.retry_replacement = None;
        streams.replacing_fresh = fresh;
        let open = self.link.clone().open(streams.channel.clone(), None, fresh);
        streams.replacement = Some(Box::pin(open));
    }

    fn on_event(
        &mut self,
        item: Result<RelayEvent, StreamEnd>,
        streams: &mut Streams,
        jobs: &mut VecDeque<Job>,
    ) {
        let event = match item {
            Ok(event) => event,
            Err(end) => {
                tracing::debug!(
                    "push for account {}: stream ended: {}",
                    self.account_id,
                    end.reason
                );
                streams.ended.get_or_insert_with(|| stream_ended(end));
                return;
            }
        };
        match event {
            RelayEvent::Ready(ready) => {
                streams.channel = merge(ready, &streams.channel);
                jobs.push_back(Job::Reconcile);
            }
            RelayEvent::Change { subscription } => self.subs.changed(&subscription),
            RelayEvent::Lifecycle {
                subscription,
                event,
            } => match event {
                Lifecycle::ReauthorizationRequired => {
                    self.subs.reauthorize(&subscription);
                }
                Lifecycle::SubscriptionRemoved => {
                    if self.subs.find(&subscription).is_some() {
                        self.subs.removed(&subscription);
                        jobs.push_back(Job::Maintain);
                    }
                }
                Lifecycle::Missed | Lifecycle::Other(_) => self.subs.catch_up(),
            },
            RelayEvent::Resync => self.subs.catch_up(),
            RelayEvent::Renew => self.replace(streams, true),
            RelayEvent::Bye { reason, retry } => {
                tracing::debug!(
                    "push for account {}: the service said bye ({reason})",
                    self.account_id
                );
                streams.open = false;
                streams.ended = Some(Ended::Retry {
                    reason: "The push service closed the connection.".into(),
                    at_least: retry.unwrap_or_default(),
                });
            }
        }
    }
}

/// Checks the Inbox every 30 seconds whenever push isn't live (and the
/// network is there), the first time 30 seconds after it stopped being.
async fn poll_while_down(
    mut live: watch::Receiver<bool>,
    mut online: watch::Receiver<bool>,
    events: EventFn,
) {
    loop {
        let down = !*live.borrow_and_update() && *online.borrow_and_update();
        let mut ticker = down.then(|| {
            let mut ticker =
                tokio::time::interval_at(Instant::now() + MICROSOFT_POLL, MICROSOFT_POLL);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            ticker
        });
        loop {
            tokio::select! {
                changed = live.changed() => {
                    if changed.is_err() {
                        return pending().await;
                    }
                    break;
                }
                changed = online.changed() => {
                    if changed.is_err() {
                        return pending().await;
                    }
                    break;
                }
                _ = tick(&mut ticker) => events(PushEvent::InboxChanged),
            }
        }
    }
}

async fn tick(ticker: &mut Option<tokio::time::Interval>) {
    match ticker {
        Some(ticker) => {
            ticker.tick().await;
        }
        None => pending().await,
    }
}

#[cfg(test)]
pub(crate) mod tests;
