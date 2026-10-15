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
//!    one whose token has expired, and never twice within 10 minutes. Time
//!    is told by both the wall clock, which counts time asleep but can be
//!    set back, and Tokio's, which can't be set but stops while asleep:
//!    whichever first says the 30 (or 10) minutes are up is believed.
//!    `renew` (or waking up) opens a new stream that replaces the old one
//!    once it is ready: make before break.
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
//! | `bye` `replaced`, not by a stream of its own | Another copy took over; 1 hour        |
//! | Graph refuses the Inbox's subscription (403) | Unsupported; 6 hours                  |
//! | The Inbox's is on another push service (409) | Degraded; ask again hourly            |
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
/// How far ahead of this one another installation's clock can be, as
/// synchronized clocks are, for its renewals to be taken at their word.
const CLOCKS_DIFFER: Duration = Duration::from_secs(60);
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
/// How long the stream is left to another running copy of the app that
/// shares this installation's ID (the service keeps one stream per
/// installation and account, and each copy's would replace the other's).
pub const TAKEN_OVER_WAIT: Duration = Duration::from_secs(3600);

/// The hint shown while consent to the service is missing.
pub const CONSENT_HINT: &str = "Sign in again to turn on instant updates.";
/// Why push polls while another copy of the app has the stream.
pub const TAKEN_OVER: &str =
    "Another copy of the app, sharing this one's data, took over instant updates for this account.";

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

/// A moment by two clocks. The wall clock counts time asleep, as Graph
/// does, but can be set back (and another installation's can run ahead);
/// Tokio's can't be set, but stops while the computer sleeps. A deadline
/// has come once either clock reaches it, and so a renewal's age is the
/// longer of the two: neither clock's fault holds a renewal up.
#[derive(Debug, Clone, Copy)]
struct Moment {
    wall: DateTime<Utc>,
    /// `None` when it lies before anything Tokio's clock can express: the
    /// wall clock alone tells.
    mono: Option<Instant>,
}

impl Moment {
    fn after(self, by: Duration) -> Self {
        Moment {
            wall: self.wall + by,
            mono: self.mono.map(|at| at + by),
        }
    }

    fn before(self, by: Duration) -> Self {
        Moment {
            wall: self.wall - by,
            mono: self.mono.and_then(|at| at.checked_sub(by)),
        }
    }

    /// Whether this deadline has come by `now`.
    fn come(self, now: Moment) -> bool {
        self.wall <= now.wall || matches!((self.mono, now.mono), (Some(at), Some(now)) if at <= now)
    }

    /// The deadline that comes when either of these does.
    fn earliest(self, other: Moment) -> Self {
        Moment {
            wall: self.wall.min(other.wall),
            mono: match (self.mono, other.mono) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
        }
    }

    /// The later of two renewals, by each clock.
    fn latest(self, other: Moment) -> Self {
        Moment {
            wall: self.wall.max(other.wall),
            mono: self.mono.max(other.mono),
        }
    }

    /// When this deadline comes on Tokio's clock, as it stands at `now`.
    fn instant(self, now: Instant, wall_now: DateTime<Utc>) -> Instant {
        let by_wall = now + (self.wall - wall_now).to_std().unwrap_or_default();
        self.mono.map_or(by_wall, |at| at.min(by_wall))
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
    renewed: Moment,
    /// When to renew it next.
    due: Moment,
    /// Renewals that failed in a row.
    failures: u32,
}

/// A watched resource without a subscription.
#[derive(Debug, Clone, Copy)]
struct Missing {
    watched: Watched,
    /// When to create it.
    at: Moment,
    /// Creations that failed in a row.
    failures: u32,
}

/// How long to wait after `failures` failures in a row.
fn retry_after(failures: u32) -> Duration {
    (RETRY_SUBSCRIPTION * 2u32.pow(failures.saturating_sub(1).min(5))).min(MAX_RETRY_SUBSCRIPTION)
}

/// Why a subscription wasn't made.
#[derive(Debug)]
enum Unmade {
    Graph(Error),
    /// Graph keeps one subscription per resource for the app, and another
    /// installation has it on another push service (at this host).
    Elsewhere(String),
}

impl From<Error> for Unmade {
    fn from(e: Error) -> Self {
        Unmade::Graph(e)
    }
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
    /// Ours on an old URL that Graph didn't delete: kept on record, so that
    /// the next reconcile (or removing the account) tries again.
    undeleted: Vec<PushSubscription>,
    /// Graph refused the calendar's subscription: the calendar syncs on
    /// schedule until the next reconcile tries again.
    calendar_refused: bool,
    /// Why the Inbox's subscription isn't in order; `None` when it is.
    problem: Option<String>,
}

impl Subs {
    fn now(&self) -> Moment {
        Moment {
            wall: (self.clock)(),
            mono: Some(Instant::now()),
        }
    }

    fn expiry(&self) -> DateTime<Utc> {
        (self.clock)() + chrono::Duration::minutes(SUBSCRIPTION_MINUTES)
    }

    fn ours(&self, s: Subscription) -> Sub {
        let now = self.now();
        let known = self
            .list
            .iter()
            .find(|k| k.id.eq_ignore_ascii_case(&s.id))
            .map(|k| k.renewed);
        // Every installation sets the expiry the same distance from its
        // renewal, so the expiry dates the latest one, by the clock of
        // whoever made it. Dated after now, it was made by a clock running
        // ahead (another installation's, or this one's before it was set
        // back): a little, and it was just now; more, and it says nothing
        // about when, so it is left out.
        let from_expiry = s.expires_at - chrono::Duration::minutes(SUBSCRIPTION_MINUTES);
        let by_expiry = (from_expiry <= now.wall + CLOCKS_DIFFER).then(|| {
            let wall = from_expiry.min(now.wall);
            Moment {
                wall,
                mono: now
                    .mono
                    .and_then(|at| at.checked_sub((now.wall - wall).to_std().ok()?)),
            }
        });
        let renewed = match (known, by_expiry) {
            // Another installation renewed it since this one did.
            (Some(known), Some(theirs)) if theirs.wall > known.wall => known.latest(theirs),
            (Some(known), _) => known,
            (None, Some(theirs)) => theirs,
            // Nobody knows when: renewed at once.
            (None, None) => now.before(RENEW_EVERY),
        };
        Sub {
            watched: Watched::of(&s.resource),
            id: s.id,
            resource: s.resource,
            expires_at: s.expires_at,
            renewed,
            due: renewed.after(RENEW_EVERY),
            failures: 0,
        }
    }

    fn created(&self, s: Subscription, watched: Watched) -> Sub {
        let now = self.now();
        Sub {
            id: s.id,
            resource: watched.resource().to_owned(),
            watched: Some(watched),
            expires_at: s.expires_at,
            renewed: now,
            due: now.after(RENEW_EVERY),
            failures: 0,
        }
    }

    fn inbox_ok(&self) -> bool {
        self.problem.is_none() && self.list.iter().any(|s| s.watched == Some(Watched::Inbox))
    }

    /// When the next renewal or creation is due, on Tokio's clock. That one
    /// stops while the computer sleeps, and the wall clock can be set, so
    /// this is asked again after every event and every wake.
    fn next_due(&self) -> Option<Instant> {
        let (now, wall_now) = (Instant::now(), (self.clock)());
        let renewals = self.list.iter().map(|s| s.due);
        let creations = self.missing.iter().map(|m| m.at);
        renewals
            .chain(creations)
            .map(|due| due.instant(now, wall_now))
            .min()
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
        if let Some(sub) = self.list.iter_mut().find(|s| s.id.eq_ignore_ascii_case(id)) {
            sub.due = sub.due.earliest(sub.renewed.after(REAUTHORIZE_GAP));
        }
    }

    /// Graph removed a subscription: make it again now.
    fn removed(&mut self, id: &str) {
        let Some(at) = self.list.iter().position(|s| s.id.eq_ignore_ascii_case(id)) else {
            return;
        };
        let sub = self.list.remove(at);
        if let Some(watched) = sub.watched {
            self.need(watched, self.now(), 0);
        }
        self.save();
    }

    fn need(&mut self, watched: Watched, at: Moment, failures: u32) {
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
            .chain(self.undeleted.iter().cloned())
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
            // Graph lists only this app's, so anything else is another
            // installation's, through another push service: left alone.
        }
        // Stale ones go first, so their replacements don't conflict. One
        // that stays only holds up the calendar until the next reconcile;
        // the Inbox's is tried again sooner, by connecting again.
        let mut undeleted = Vec::new();
        for s in stale {
            if let Err(e) = self.graph.delete_subscription(&s.id).await {
                if Watched::of(&s.resource) == Some(Watched::Inbox) {
                    return Err(Ended::retry(graph_reason(&e)));
                }
                tracing::info!(
                    "push for account {}: deleting an old subscription failed: {}",
                    self.account_id,
                    graph_reason(&e)
                );
                undeleted.push(PushSubscription {
                    id: s.id,
                    resource: s.resource,
                    expires_at: s.expires_at,
                });
            }
        }
        self.undeleted = undeleted;
        let now = self.now();
        let mut renewed = Vec::new();
        for mut sub in current {
            if sub.renewed.after(REAUTHORIZE_GAP).come(now) {
                match self.graph.renew_subscription(&sub.id, self.expiry()).await {
                    Ok(s) => {
                        sub.expires_at = s.expires_at;
                        sub.renewed = self.now();
                        sub.due = sub.renewed.after(RENEW_EVERY);
                    }
                    // Gone since it was listed.
                    Err(e) if e.code == ErrorCode::NotFound => continue,
                    Err(e) if sub.watched == Some(Watched::Inbox) => {
                        return Err(Ended::retry(graph_reason(&e)));
                    }
                    // Tried again later; the Inbox doesn't wait for it.
                    Err(e) => {
                        tracing::info!(
                            "push for account {}: renewing a subscription failed: {}",
                            self.account_id,
                            graph_reason(&e)
                        );
                        sub.failures = 1;
                        sub.due = self.now().after(retry_after(sub.failures));
                    }
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
                // Kept at once: if push stops before the next one is made,
                // removing the account still finds this one.
                Ok(sub) => {
                    self.list.push(sub);
                    self.save();
                }
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
        let now = self.now();
        let mut changed = false;
        let mut gone = Vec::new();
        for at in 0..self.list.len() {
            if !self.list[at].due.come(now) {
                continue;
            }
            let id = self.list[at].id.clone();
            let result = self.graph.renew_subscription(&id, self.expiry()).await;
            let answered = self.now();
            let sub = &mut self.list[at];
            match result {
                Ok(s) => {
                    sub.expires_at = s.expires_at;
                    sub.renewed = answered;
                    sub.due = answered.after(RENEW_EVERY);
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
                    sub.due = answered.after(retry_after(sub.failures));
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
            .filter(|m| m.at.come(now))
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
                    self.save();
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
    /// calendar's is left until the next reconcile; one another push
    /// service has is asked for again hourly, in case it goes; anything
    /// else is tried again later.
    fn refused(&mut self, watched: Watched, unmade: Unmade, failures: u32) -> Result<(), Ended> {
        let e = match unmade {
            Unmade::Graph(e) => e,
            Unmade::Elsewhere(host) => {
                tracing::info!(
                    "push for account {}: another installation watches {watched:?} through \
                     the push service at {host}; asking again in an hour",
                    self.account_id
                );
                if watched == Watched::Inbox {
                    self.problem = Some(format!(
                        "Another installation of the app gets this account's instant updates \
                         through a different push service ({host})."
                    ));
                }
                self.need(watched, self.now().after(MAX_RETRY_SUBSCRIPTION), failures);
                return Ok(());
            }
        };
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
        self.need(watched, self.now().after(retry_after(failures)), failures);
        Ok(())
    }

    /// Creates the subscription for `watched` on the channel. A conflict
    /// means another installation of the account just made it: that one is
    /// adopted. Unless it is on another push service, which keeps it.
    async fn create(&self, watched: Watched, channel: &Channel) -> Result<Sub, Unmade> {
        let new = NewSubscription {
            resource: watched.resource().to_owned(),
            change_type: CHANGE_TYPES.to_owned(),
            notification_url: channel.notification_url.clone(),
            lifecycle_notification_url: channel.lifecycle_notification_url.clone(),
            client_state: channel.client_state.clone(),
            expires_at: self.expiry(),
        };
        match self.graph.create_subscription(&new).await {
            Ok(s) => Ok(self.created(s, watched)),
            Err(e) if e.http_status() == Some(409) => {
                let mut elsewhere = None;
                for s in self.graph.list_subscriptions().await? {
                    if Watched::of(&s.resource) != Some(watched) {
                        continue;
                    }
                    let url = s.notification_url.as_deref().unwrap_or_default();
                    if url == channel.notification_url {
                        return Ok(self.ours(s));
                    }
                    if !same_origin(url, &channel.notification_url) {
                        elsewhere = Url::parse(url)
                            .ok()
                            .and_then(|url| url.host_str().map(str::to_owned));
                    }
                }
                Err(elsewhere.map_or(Unmade::Graph(e), Unmade::Elsewhere))
            }
            Err(e) => Err(e.into()),
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
    /// Whether a replacement was opened since `current` was: the service
    /// then says `bye` "replaced" to `current` because of it, not because
    /// another copy of the app opened a stream.
    replaced_here: bool,
    /// Whether the replacement asked for a fresh token.
    replacing_fresh: bool,
    /// Replacements that failed in a row, and when to try again.
    replacement_failures: u32,
    retry_replacement: Option<Instant>,
    /// No replacement before this, by the wall clock: the service's
    /// `Retry-After`, which waking up doesn't cut short.
    replace_not_before: Option<DateTime<Utc>>,
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
                undeleted: Vec::new(),
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
                    (delay, Some(at_least))
                }
                Ended::CoolDown {
                    health,
                    detail,
                    wait,
                } => {
                    tracing::info!(
                        "push for account {}: {detail} Checking the Inbox every {}s; \
                         asking again in {} minutes",
                        self.account_id,
                        MICROSOFT_POLL.as_secs(),
                        wait.as_secs() / 60
                    );
                    self.report(polling(MICROSOFT_POLL), health, Some(detail));
                    (wait, None)
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

    /// Waits `wait`. When `woken_ends` is `Some(at_least)`, waking up ends
    /// it early, though not before `at_least` (the service's `Retry-After`)
    /// has passed by the wall clock, which counts time asleep. False when
    /// cancelled.
    async fn pause(&mut self, wait: Duration, woken_ends: Option<Duration>) -> bool {
        let not_before = woken_ends.map(|at_least| (self.subs.clock)() + at_least);
        let mut deadline = Instant::now() + wait;
        loop {
            tokio::select! {
                biased;
                _ = self.signals.cancel.cancelled() => return false,
                _ = went_offline(&mut self.signals.online) => return true,
                _ = tokio::time::sleep_until(deadline) => return true,
                _ = woken(&mut self.signals.wake) => {
                    if let Some(not_before) = not_before {
                        self.backoff.reset();
                        match (not_before - (self.subs.clock)()).to_std() {
                            Ok(left) if !left.is_zero() => {
                                deadline = deadline.min(Instant::now() + left);
                            }
                            _ => return true,
                        }
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
            replaced_here: false,
            replacing_fresh: false,
            replacement_failures: 0,
            retry_replacement: None,
            replace_not_before: None,
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
                // Waking up ends the replacement's backoff, but not a wait
                // the service asked for (nor, if the wall clock was set
                // back, does it make that wait longer).
                let wait = streams
                    .replace_not_before
                    .and_then(|at| (at - (self.subs.clock)()).to_std().ok())
                    .filter(|left| !left.is_zero());
                match wait {
                    Some(left) => {
                        let at = Instant::now() + left;
                        streams.retry_replacement =
                            Some(streams.retry_replacement.map_or(at, |retry| retry.min(at)));
                    }
                    None => self.replace(&mut streams, false),
                }
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
                            streams.replaced_here = false;
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
                            streams.replace_not_before = Some((self.subs.clock)() + at_least);
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
        streams.replace_not_before = None;
        streams.replaced_here = true;
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
            // Another copy of the app with this installation's ID opened a
            // stream: connecting again would only replace that one in turn.
            RelayEvent::Bye { reason, .. } if reason == "replaced" && !streams.replaced_here => {
                tracing::warn!(
                    "push for account {}: another running copy of the app uses this \
                     installation's ID; leaving the push service to it",
                    self.account_id
                );
                streams.open = false;
                streams.ended = Some(Ended::CoolDown {
                    health: PushHealth::Degraded {
                        reason: TAKEN_OVER.into(),
                    },
                    detail: TAKEN_OVER.into(),
                    wait: TAKEN_OVER_WAIT,
                });
            }
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
