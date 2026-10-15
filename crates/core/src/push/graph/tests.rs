//! Push through the push service, against scripted fakes on paused time:
//! connecting, reconciling, renewing, the events, refusals and polling.
//! At the end, Graph's side on wiremock and the service over HTTP.

use std::sync::Mutex;

use super::*;
use crate::push::fakes::{
    CLIENT_STATE, FakeGraph, FakeTokens, NOTIFICATION_URL, ScriptedRelay, graph_status,
};
use crate::push::relay::StreamEnd;
use crate::store::test_support::imap_account;

const POLL: PushMode = PushMode::Polling { every_secs: 30 };

type Status = (PushMode, PushHealth, Option<String>);

/// A running GraphPush and the fakes around it.
struct Run {
    relay: Arc<ScriptedRelay>,
    tokens: Arc<FakeTokens>,
    graph: Arc<FakeGraph>,
    store: Arc<Store>,
    cancel: CancellationToken,
    wake: watch::Sender<u64>,
    online: watch::Sender<bool>,
    statuses: Arc<Mutex<Vec<Status>>>,
    events: Arc<Mutex<Vec<(Instant, PushEvent)>>>,
    /// Why push asked to be parked, each time.
    parked: Arc<Mutex<Vec<ErrorCode>>>,
    task: tokio::task::JoinHandle<()>,
}

/// The wall clock, moving with Tokio's paused one.
fn paused_clock() -> Clock {
    let (utc, origin) = (Utc::now(), Instant::now());
    Arc::new(move || utc + chrono::Duration::from_std(origin.elapsed()).unwrap())
}

impl Run {
    /// Starts push after `script` has set up the fakes.
    fn start(script: impl FnOnce(&ScriptedRelay, &FakeTokens, &FakeGraph)) -> Self {
        Self::start_with(None, script)
    }

    /// Starts push on `service` instead of the scripted relay, if given.
    fn start_with(
        service: Option<Arc<dyn Relay>>,
        script: impl FnOnce(&ScriptedRelay, &FakeTokens, &FakeGraph),
    ) -> Self {
        let (relay, tokens, graph) = (ScriptedRelay::new(), FakeTokens::new(), FakeGraph::new());
        script(&relay, &tokens, &graph);
        let store = Arc::new(Store::open_in_memory().unwrap());
        store.insert_account(&imap_account("acct")).unwrap();
        let cancel = CancellationToken::new();
        let (wake, wake_rx) = watch::channel(0);
        let (online, online_rx) = watch::channel(true);
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let parked = Arc::new(Mutex::new(Vec::new()));
        let push = GraphPush::new(
            "acct",
            "install-0123456789abcdef",
            RelaySource {
                relay: service.unwrap_or_else(|| relay.clone()),
                tokens: tokens.clone(),
            },
            graph.clone(),
            store.clone(),
            {
                let events = events.clone();
                Arc::new(move |event| events.lock().unwrap().push((Instant::now(), event)))
            },
            {
                let statuses = statuses.clone();
                Arc::new(move |mode, health, detail| {
                    statuses.lock().unwrap().push((mode, health, detail));
                })
            },
            cancel.clone(),
            wake_rx,
            online_rx,
        )
        .with_backoff(Backoff::seeded(3))
        .with_clock(paused_clock())
        .with_park({
            let parked = parked.clone();
            Arc::new(move |e: &Error| parked.lock().unwrap().push(e.code))
        });
        let task = tokio::spawn(push.run());
        Self {
            relay,
            tokens,
            graph,
            store,
            cancel,
            wake,
            online,
            statuses,
            events,
            parked,
            task,
        }
    }

    fn status(&self) -> Status {
        self.statuses.lock().unwrap().last().cloned().unwrap()
    }

    fn is_live(&self) -> bool {
        let statuses = self.statuses.lock().unwrap();
        statuses.last().is_some_and(|(mode, health, _)| {
            *mode == PushMode::GraphRelay && *health == PushHealth::Live
        })
    }

    fn take_events(&self) -> Vec<PushEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
            .into_iter()
            .map(|(_, e)| e)
            .collect()
    }

    /// Inbox checks between `from` and `to` that weren't part of a catch-up
    /// (which also brings the calendar).
    fn polls(&self, from: Instant, to: Instant) -> usize {
        let events = self.events.lock().unwrap();
        events
            .iter()
            .enumerate()
            .filter(|(i, (at, e))| {
                *at >= from
                    && *at < to
                    && *e == PushEvent::InboxChanged
                    && events.get(i + 1).is_none_or(|(next_at, next)| {
                        !(next_at == at && *next == PushEvent::CalendarChanged)
                    })
            })
            .count()
    }

    fn inbox_id(&self) -> String {
        self.graph.id_of(INBOX_RESOURCE)
    }

    async fn stop(self) {
        self.cancel.cancel();
        tokio::time::timeout(Duration::from_millis(10), self.task)
            .await
            .expect("stops at once")
            .unwrap();
    }
}

async fn advance(by: Duration) {
    tokio::time::sleep(by).await;
}

const SECOND: Duration = Duration::from_secs(1);
const MINUTE: Duration = Duration::from_secs(60);

#[tokio::test(start_paused = true)]
async fn it_subscribes_on_ready_then_goes_live_and_follows_changes() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    assert!(run.is_live(), "{:?}", run.status());
    // A token with ten minutes left opened it, from the cache.
    assert_eq!(run.tokens.calls(), [(MIN_TOKEN_LEFT, false)]);
    assert_eq!(run.relay.requests("channel")[0].0, "token-1");
    assert_eq!(run.relay.requests("events")[0].0, "token-1");

    // Both subscriptions, on the channel, without resource data.
    {
        let graph = run.graph.state.lock().unwrap();
        let created: Vec<&str> = graph.created.iter().map(|n| n.resource.as_str()).collect();
        assert_eq!(created, [INBOX_RESOURCE, EVENTS_RESOURCE]);
        for new in &graph.created {
            assert_eq!(new.change_type, "created,updated,deleted");
            assert_eq!(new.notification_url, NOTIFICATION_URL);
            assert_eq!(new.lifecycle_notification_url, NOTIFICATION_URL);
            assert_eq!(new.client_state, CLIENT_STATE);
            let minutes = (new.expires_at - Utc::now()).num_minutes();
            assert!((SUBSCRIPTION_MINUTES - 2..=SUBSCRIPTION_MINUTES).contains(&minutes));
        }
    }
    // Kept, without the clientState.
    let stored = run.store.push_subscriptions("acct").unwrap();
    assert_eq!(stored.len(), 2);
    // Then it caught up on both.
    assert_eq!(
        run.take_events(),
        [PushEvent::InboxChanged, PushEvent::CalendarChanged]
    );

    let inbox = run.inbox_id();
    let calendar = run.graph.id_of(EVENTS_RESOURCE);
    run.relay.send(RelayEvent::Change {
        subscription: inbox.to_uppercase(),
    });
    run.relay.send(RelayEvent::Change {
        subscription: calendar,
    });
    advance(SECOND).await;
    assert_eq!(
        run.take_events(),
        [PushEvent::InboxChanged, PushEvent::CalendarChanged]
    );
    // One nobody knows: both, to be safe.
    run.relay.send(RelayEvent::Change {
        subscription: "someone-elses".into(),
    });
    advance(SECOND).await;
    assert_eq!(
        run.take_events(),
        [PushEvent::InboxChanged, PushEvent::CalendarChanged]
    );
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn missed_resync_and_removed_subscriptions_catch_up() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    run.take_events();
    let inbox = run.inbox_id();

    run.relay.send(RelayEvent::Lifecycle {
        subscription: inbox.clone(),
        event: Lifecycle::Missed,
    });
    run.relay.send(RelayEvent::Resync);
    advance(SECOND).await;
    assert_eq!(
        run.take_events(),
        [
            PushEvent::InboxChanged,
            PushEvent::CalendarChanged,
            PushEvent::InboxChanged,
            PushEvent::CalendarChanged
        ]
    );

    // Graph removed the Inbox's: made again, then the Inbox caught up.
    run.graph
        .state
        .lock()
        .unwrap()
        .subs
        .retain(|s| s.id != inbox);
    run.relay.send(RelayEvent::Lifecycle {
        subscription: inbox.clone(),
        event: Lifecycle::SubscriptionRemoved,
    });
    advance(SECOND).await;
    let again = run.inbox_id();
    assert_ne!(again, inbox);
    assert_eq!(run.take_events(), [PushEvent::InboxChanged]);
    assert!(run.is_live());
    let stored: Vec<String> = run
        .store
        .push_subscriptions("acct")
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert!(stored.contains(&again) && !stored.contains(&inbox));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn subscriptions_are_renewed_every_thirty_minutes_while_connected() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    let start = Instant::now();
    advance(29 * MINUTE).await;
    assert!(run.graph.renewals_of(INBOX_RESOURCE).is_empty());
    advance(2 * MINUTE).await;
    advance(30 * MINUTE).await;
    for resource in [INBOX_RESOURCE, EVENTS_RESOURCE] {
        let renewals = run.graph.renewals_of(resource);
        assert_eq!(renewals.len(), 2, "{resource}");
        let gaps: Vec<Duration> = renewals.iter().map(|t| *t - start).collect();
        assert!(
            gaps[0] <= 30 * MINUTE && gaps[1] - gaps[0] == RENEW_EVERY,
            "{gaps:?}"
        );
    }
    // Each renewal asked for the full lifetime again.
    let sub = run.graph.state.lock().unwrap().subs[0].clone();
    let minutes = (sub.expires_at - (paused_clock())()).num_minutes();
    assert!(minutes > SUBSCRIPTION_MINUTES - 35, "{minutes}");
    assert!(run.is_live());
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn reauthorizations_are_ten_minutes_apart() {
    let created = Instant::now();
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    let inbox = run.inbox_id();
    let reauthorize = || {
        run.relay.send(RelayEvent::Lifecycle {
            subscription: inbox.clone(),
            event: Lifecycle::ReauthorizationRequired,
        });
    };

    // Two minutes after it was made: it waits for the ten-minute mark.
    advance(2 * MINUTE).await;
    reauthorize();
    advance(7 * MINUTE).await;
    assert!(run.graph.renewals_of(INBOX_RESOURCE).is_empty());
    advance(2 * MINUTE).await;
    let renewals = run.graph.renewals_of(INBOX_RESOURCE);
    assert_eq!(renewals.len(), 1);
    assert!(renewals[0] - created >= REAUTHORIZE_GAP);
    assert!(renewals[0] - created < REAUTHORIZE_GAP + SECOND);

    // Long enough after the last one: at once.
    advance(15 * MINUTE).await;
    reauthorize();
    advance(SECOND).await;
    let renewals = run.graph.renewals_of(INBOX_RESOURCE);
    assert_eq!(renewals.len(), 2);
    assert!(renewals[1] - renewals[0] >= REAUTHORIZE_GAP);
    // The calendar's wasn't touched before its thirty minutes.
    assert_eq!(run.graph.renewals_of(EVENTS_RESOURCE).len(), 0);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn reconnecting_renews_only_what_wasnt_renewed_in_ten_minutes() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    // The stream drops three minutes on: the reconnection's reconcile
    // leaves the subscriptions be.
    advance(3 * MINUTE).await;
    run.relay.drop_stream();
    advance(5 * SECOND).await;
    assert_eq!(run.relay.opened(), 2);
    assert!(run.is_live());
    assert_eq!(run.graph.state.lock().unwrap().lists, 2);
    assert!(run.graph.state.lock().unwrap().renewals.is_empty());
    // Fifteen minutes on, they're renewed.
    advance(12 * MINUTE).await;
    run.relay.drop_stream();
    advance(5 * SECOND).await;
    assert_eq!(run.relay.opened(), 3);
    assert_eq!(run.graph.state.lock().unwrap().renewals.len(), 2);
    // Nothing was created twice.
    assert_eq!(run.graph.state.lock().unwrap().created.len(), 2);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn the_inbox_is_polled_only_while_the_stream_is_down() {
    let run = Run::start(|relay, _, _| {
        // The first attempt is told to come back in two minutes and a bit.
        relay.refuse_stream(RelayError::Busy {
            retry_after: Some(125 * SECOND),
        });
    });
    let start = Instant::now();
    advance(SECOND).await;
    assert_eq!(
        run.status().1,
        PushHealth::Degraded {
            reason: "The push service is busy.".into()
        }
    );
    advance(2 * MINUTE + 10 * SECOND).await;
    assert!(run.is_live());
    let live_at = Instant::now();
    // Every 30 seconds while down: 4 in the two minutes.
    assert_eq!(run.polls(start, live_at), 4);
    advance(10 * MINUTE).await;
    assert_eq!(run.polls(live_at, Instant::now()), 0);

    // The stream drops, and coming back takes a while.
    run.relay.refuse_stream(RelayError::Busy {
        retry_after: Some(5 * MINUTE),
    });
    let dropped = Instant::now();
    run.relay.drop_stream();
    advance(SECOND).await;
    assert_eq!(run.status().0, POLL);
    advance(5 * MINUTE + 5 * SECOND).await;
    assert!(run.is_live());
    assert_eq!(run.polls(dropped, Instant::now()), 10);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn missing_consent_means_polling_and_a_hint_not_an_account_error() {
    let run = Run::start(|_, tokens, _| tokens.refuse(SyncTokenError::ConsentMissing));
    advance(SECOND).await;
    assert_eq!(
        run.status(),
        (
            POLL,
            PushHealth::ConsentMissing,
            Some(CONSENT_HINT.to_owned())
        )
    );
    let start = Instant::now();
    advance(5 * MINUTE).await;
    assert_eq!(run.polls(start, Instant::now()), 10);
    assert!(run.relay.requests("channel").is_empty());
    // Waking up doesn't make it ask again...
    run.wake.send_modify(|n| *n += 1);
    advance(5 * 3600 * SECOND).await;
    assert_eq!(run.tokens.calls().len(), 1);
    // ...six hours do.
    advance(3600 * SECOND).await;
    assert_eq!(run.tokens.calls().len(), 2);
    assert!(run.is_live());
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_locked_keyring_parks_push_instead_of_asking_again() {
    let run = Run::start(|_, tokens, _| {
        tokens.refuse(SyncTokenError::Failed(Error::new(
            ErrorCode::Storage,
            "could not access the system keyring",
        )))
    });
    advance(SECOND).await;
    assert_eq!(*run.parked.lock().unwrap(), [ErrorCode::Storage]);
    assert!(matches!(run.status().1, PushHealth::Degraded { .. }));
    advance(3600 * SECOND).await;
    assert_eq!(run.tokens.calls().len(), 1);
    assert!(run.relay.requests("channel").is_empty());
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn an_app_registration_without_the_scope_polls() {
    let run = Run::start(|_, tokens, _| tokens.refuse(SyncTokenError::NotOffered));
    advance(SECOND).await;
    let (mode, health, detail) = run.status();
    assert_eq!((mode, health), (POLL, PushHealth::Unsupported));
    assert!(detail.unwrap().contains("app registration"));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_refused_token_is_retried_once_fresh() {
    let run = Run::start(|relay, _, _| {
        relay.refuse_channel(RelayError::Token { expiring: false });
        relay.refuse_stream(RelayError::Token { expiring: true });
    });
    advance(SECOND).await;
    assert!(run.is_live());
    assert_eq!(
        run.tokens.calls(),
        [
            (MIN_TOKEN_LEFT, false),
            (MIN_TOKEN_LEFT, true),
            (MIN_TOKEN_LEFT, true)
        ]
    );
    let channels = run.relay.requests("channel");
    assert_eq!(
        (channels[0].0.as_str(), channels[1].0.as_str()),
        ("token-1", "token-2")
    );
    let streams = run.relay.requests("events");
    assert_eq!(
        (streams[0].0.as_str(), streams[1].0.as_str()),
        ("token-2", "token-3")
    );
    run.stop().await;

    // Refused again with a fresh token: not asked again for six hours.
    let run = Run::start(|relay, _, _| {
        relay.refuse_channel(RelayError::Token { expiring: false });
        relay.refuse_channel(RelayError::Token { expiring: false });
    });
    advance(SECOND).await;
    assert_eq!(run.status().1, PushHealth::Unsupported);
    advance(5 * 3600 * SECOND).await;
    assert_eq!(run.relay.requests("channel").len(), 2);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn refusals_wait_as_long_as_they_say() {
    // 403: six hours.
    let run = Run::start(|relay, _, _| relay.refuse_channel(RelayError::NotAllowed));
    advance(SECOND).await;
    let (mode, health, detail) = run.status();
    assert_eq!((mode, health), (POLL, PushHealth::Unsupported));
    assert!(detail.unwrap().contains("organization"));
    advance(6 * 3600 * SECOND - 2 * SECOND).await;
    assert_eq!(run.relay.requests("channel").len(), 1);
    advance(3 * SECOND).await;
    assert_eq!(run.relay.requests("channel").len(), 2);
    assert!(run.is_live());
    run.stop().await;

    // 429: at least Retry-After.
    let run = Run::start(|relay, _, _| {
        relay.refuse_stream(RelayError::Busy {
            retry_after: Some(90 * SECOND),
        })
    });
    advance(5 * MINUTE).await;
    let streams = run.relay.requests("events");
    assert_eq!(streams.len(), 2);
    assert!(streams[1].1 - streams[0].1 >= 90 * SECOND);
    assert!(run.is_live());
    run.stop().await;

    // 400: a bug; polls for six hours.
    let run = Run::start(|relay, _, _| {
        relay.refuse_channel(RelayError::BadRequest("bad_installation".into()))
    });
    advance(3600 * SECOND).await;
    assert_eq!(run.relay.requests("channel").len(), 1);
    assert_eq!(run.status().0, POLL);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn renew_opens_a_new_stream_before_dropping_the_old() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    run.take_events();
    run.relay.hold_ready();
    run.relay.send(RelayEvent::Renew);
    advance(SECOND).await;
    // A fresh token opened the new one; the old one still delivers.
    assert_eq!(run.tokens.calls().last(), Some(&(MIN_TOKEN_LEFT, true)));
    assert_eq!((run.relay.opened(), run.relay.open()), (2, 2));
    assert!(run.is_live());

    // Once the new one is ready, the old one goes; everything is
    // reconciled again and caught up.
    run.relay.ready();
    advance(SECOND).await;
    assert_eq!(run.relay.open(), 1);
    assert_eq!(run.graph.state.lock().unwrap().lists, 2);
    assert_eq!(
        run.take_events(),
        [PushEvent::InboxChanged, PushEvent::CalendarChanged]
    );
    let live_throughout = run
        .statuses
        .lock()
        .unwrap()
        .iter()
        .skip_while(|(_, h, _)| *h != PushHealth::Live)
        .all(|(m, h, _)| *m == PushMode::GraphRelay && *h == PushHealth::Live);
    assert!(live_throughout);
    // Events now come on the new stream.
    run.relay.send(RelayEvent::Change {
        subscription: run.inbox_id(),
    });
    advance(SECOND).await;
    assert_eq!(run.take_events(), [PushEvent::InboxChanged]);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn bye_reconnects_after_the_time_it_gives() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    run.relay.send(RelayEvent::Bye {
        reason: "restart".into(),
        retry: Some(20 * SECOND),
    });
    advance(SECOND).await;
    assert_eq!(run.status().0, POLL);
    advance(18 * SECOND).await;
    assert_eq!(run.relay.opened(), 1);
    advance(5 * SECOND).await;
    assert_eq!(run.relay.opened(), 2);
    assert!(run.is_live());
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn renewal_failures_poll_until_a_renewal_works() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    // At the thirty-minute renewal Graph fails once.
    run.graph
        .state
        .lock()
        .unwrap()
        .fail_renew
        .push_back(graph_status(503));
    advance(30 * MINUTE).await;
    let (mode, health, _) = run.status();
    assert_eq!(mode, POLL);
    assert!(matches!(health, PushHealth::Degraded { reason } if reason.contains("503")));
    // Tried again two minutes later, and live again.
    advance(RETRY_SUBSCRIPTION + SECOND).await;
    assert!(run.is_live());
    assert_eq!(run.graph.renewals_of(INBOX_RESOURCE).len(), 1);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn renewals_that_keep_failing_back_off() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    let inbox = run.inbox_id();
    {
        let mut graph = run.graph.state.lock().unwrap();
        graph.fail_renew_of = Some(inbox.clone());
        for _ in 0..4 {
            graph.fail_renew.push_back(graph_status(500));
        }
    }
    // The calendar's renewals work; the Inbox's fails four times.
    advance(70 * MINUTE).await;
    let attempts: Vec<Instant> = run
        .graph
        .state
        .lock()
        .unwrap()
        .renew_attempts
        .iter()
        .filter(|(id, _)| *id == inbox)
        .map(|(_, at)| *at)
        .collect();
    let retries: Vec<Duration> = attempts.windows(2).map(|w| w[1] - w[0]).take(4).collect();
    // 2, 4, 8 and 16 minutes apart, then it worked.
    assert_eq!(
        retries,
        [2 * MINUTE, 4 * MINUTE, 8 * MINUTE, 16 * MINUTE],
        "{attempts:?}"
    );
    assert!(run.is_live());
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_replacement_refused_for_good_waits_for_the_old_stream_to_end() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    run.tokens.refuse(SyncTokenError::ConsentMissing);
    run.relay.send(RelayEvent::Renew);
    advance(4 * MINUTE).await;
    // Asked once; the old stream still serves.
    assert_eq!(run.tokens.calls().len(), 2);
    assert!(run.is_live());
    run.relay.send(RelayEvent::Bye {
        reason: "expired".into(),
        retry: None,
    });
    advance(SECOND).await;
    assert_eq!(run.status().1, PushHealth::ConsentMissing);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn an_expired_subscription_is_made_again_at_its_renewal() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    run.take_events();
    let inbox = run.inbox_id();
    run.graph
        .state
        .lock()
        .unwrap()
        .subs
        .retain(|s| s.id != inbox);
    advance(31 * MINUTE).await;
    assert_ne!(run.inbox_id(), inbox);
    assert_eq!(run.take_events(), [PushEvent::InboxChanged]);
    assert!(run.is_live());
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn graph_refusing_the_inbox_means_polling() {
    let run = Run::start(|_, _, graph| {
        graph
            .state
            .lock()
            .unwrap()
            .fail_create
            .push_back(graph_status(403));
    });
    advance(SECOND).await;
    let (mode, health, detail) = run.status();
    assert_eq!((mode, health), (POLL, PushHealth::Unsupported));
    assert!(detail.unwrap().contains("mailbox"));
    run.stop().await;

    // Refusing the calendar's only leaves the calendar to the schedule.
    let run = Run::start(|_, _, graph| {
        let mut state = graph.state.lock().unwrap();
        state.fail_create.push_back(graph_status(503));
        state.fail_create.push_back(graph_status(403));
    });
    advance(SECOND).await;
    // The Inbox's failed: polling, and tried again in two minutes.
    assert_eq!(run.status().0, POLL);
    advance(RETRY_SUBSCRIPTION + SECOND).await;
    assert!(run.is_live());
    let resources: Vec<String> = run
        .graph
        .state
        .lock()
        .unwrap()
        .subs
        .iter()
        .map(|s| s.resource.clone())
        .collect();
    assert_eq!(resources, [INBOX_RESOURCE]);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn offline_pauses_and_waking_reconnects_without_dropping_push() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    run.online.send_replace(false);
    advance(SECOND).await;
    assert_eq!(run.relay.open(), 0);
    assert_eq!(
        run.status().1,
        PushHealth::Degraded {
            reason: "No network connection.".into()
        }
    );
    let start = Instant::now();
    advance(10 * MINUTE).await;
    assert_eq!(run.polls(start, Instant::now()), 0);
    run.online.send_replace(true);
    advance(SECOND).await;
    assert!(run.is_live());
    assert_eq!(run.relay.opened(), 2);

    // Waking up opens a new stream, which replaces the old one once
    // ready: push stays live.
    let before = run.statuses.lock().unwrap().len();
    run.wake.send_modify(|n| *n += 1);
    advance(SECOND).await;
    assert_eq!((run.relay.opened(), run.relay.open()), (3, 1));
    assert_eq!(run.statuses.lock().unwrap().len(), before);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_silent_or_dropped_stream_backs_off() {
    let run = Run::start(|_, _, _| {});
    advance(SECOND).await;
    // Every reconnection fails for a while.
    for _ in 0..6 {
        run.relay
            .refuse_stream(RelayError::Failed("unreachable".into()));
    }
    run.relay.send_end(StreamEnd {
        reason: "The push service went silent.".into(),
        retry: Some(10 * SECOND),
    });
    advance(30 * MINUTE).await;
    let starts: Vec<Instant> = run
        .relay
        .requests("channel")
        .into_iter()
        .map(|(_, at)| at)
        .collect();
    assert_eq!(starts.len(), 8);
    // The first wait honors the stream's `retry`; then they grow.
    let gaps: Vec<Duration> = starts.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps[0] >= 10 * SECOND, "{gaps:?}");
    assert!(gaps.iter().all(|g| *g <= 300 * SECOND + SECOND), "{gaps:?}");
    assert!(run.is_live());
    run.stop().await;
}

// --- Graph's side, on wiremock ----------------------------------------------

mod on_graph {
    use serde_json::{Value, json};
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::auth::StaticToken;
    use crate::providers::graph::GraphProvider;

    const OLD_URL: &str = "https://sync.example.org/graph/v1/k0OldRouteOldRouteOldRouteOldRoute0";
    const OTHER_APP: &str = "https://hooks.other.example/graph";

    fn subs(server: &MockServer) -> (Subs, Arc<Mutex<Vec<PushEvent>>>) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store.insert_account(&imap_account("acct")).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let graph = GraphProvider::with_base_url(
            "acct",
            Arc::new(StaticToken("graph-token".into())),
            server.uri(),
        );
        let subs = Subs {
            account_id: "acct".into(),
            graph: Arc::new(graph),
            store,
            events: {
                let events = events.clone();
                Arc::new(move |e| events.lock().unwrap().push(e))
            },
            clock: Arc::new(Utc::now),
            list: Vec::new(),
            missing: Vec::new(),
            calendar_refused: false,
            problem: None,
        };
        (subs, events)
    }

    /// A subscription as Graph lists it, renewed `age` ago.
    fn listed(id: &str, resource: &str, url: &str, age: chrono::Duration) -> Value {
        let expires = Utc::now() + chrono::Duration::minutes(SUBSCRIPTION_MINUTES) - age;
        json!({
            "id": id,
            "resource": resource,
            "changeType": "created,updated,deleted",
            "notificationUrl": url,
            "lifecycleNotificationUrl": url,
            "expirationDateTime": expires.to_rfc3339(),
        })
    }

    fn created(id: &str, resource: &str) -> ResponseTemplate {
        ResponseTemplate::new(201).set_body_json(listed(
            id,
            resource,
            NOTIFICATION_URL,
            chrono::Duration::zero(),
        ))
    }

    async fn requests(server: &MockServer, verb: &str) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method.as_str() == verb)
            .map(|r| r.url.path().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn stale_urls_are_recreated_and_other_apps_left_alone() {
        let server = MockServer::start().await;
        let hour = chrono::Duration::hours(1);
        Mock::given(method("GET"))
            .and(path("/subscriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "value": [
                    // Ours, on the URL of the service's previous key.
                    listed("stale", INBOX_RESOURCE, OLD_URL, hour),
                    // Another app's, and another service's.
                    listed("other-inbox", INBOX_RESOURCE, OTHER_APP, hour),
                ],
                "@odata.nextLink": format!("{}/subscriptions/next", server.uri()),
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/subscriptions/next"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "value": [
                    listed("ours-events", EVENTS_RESOURCE, NOTIFICATION_URL, hour),
                    listed("other-events", "Me/Events", OTHER_APP, hour),
                ],
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/subscriptions/stale"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/subscriptions/ours-events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listed(
                "ours-events",
                EVENTS_RESOURCE,
                NOTIFICATION_URL,
                chrono::Duration::zero(),
            )))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/subscriptions"))
            .and(body_partial_json(json!({
                "resource": INBOX_RESOURCE,
                "changeType": "created,updated,deleted",
                "notificationUrl": NOTIFICATION_URL,
                "lifecycleNotificationUrl": NOTIFICATION_URL,
                "clientState": CLIENT_STATE,
                "includeResourceData": false,
            })))
            .respond_with(created("new-inbox", INBOX_RESOURCE))
            .expect(1)
            .mount(&server)
            .await;

        let (mut subs, events) = subs(&server);
        subs.reconcile(&crate::push::fakes::channel())
            .await
            .unwrap();
        // The stale one went before its replacement was made.
        let order: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method.as_str() != "GET")
            .map(|r| format!("{} {}", r.method, r.url.path()))
            .collect();
        assert_eq!(
            order,
            [
                "DELETE /subscriptions/stale",
                "PATCH /subscriptions/ours-events",
                "POST /subscriptions"
            ]
        );
        assert!(subs.inbox_ok());
        let mut kept: Vec<String> = subs
            .store
            .push_subscriptions("acct")
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        kept.sort();
        assert_eq!(kept, ["new-inbox", "ours-events"]);
        assert_eq!(
            *events.lock().unwrap(),
            [PushEvent::InboxChanged, PushEvent::CalendarChanged]
        );

        // Reconciling again soon after: nothing is renewed (the last
        // renewals were under ten minutes ago), nothing else changes.
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/subscriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "value": [
                    listed("new-inbox", INBOX_RESOURCE, NOTIFICATION_URL, chrono::Duration::minutes(1)),
                    listed("ours-events", EVENTS_RESOURCE, NOTIFICATION_URL, chrono::Duration::minutes(1)),
                    listed("other-inbox", INBOX_RESOURCE, OTHER_APP, hour),
                ],
            })))
            .mount(&server)
            .await;
        subs.reconcile(&crate::push::fakes::channel())
            .await
            .unwrap();
        assert!(requests(&server, "PATCH").await.is_empty());
        assert!(requests(&server, "POST").await.is_empty());
        assert!(requests(&server, "DELETE").await.is_empty());
    }

    #[tokio::test]
    async fn a_conflict_adopts_the_one_another_installation_made() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/subscriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "value": [] })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/subscriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "value": [listed("theirs", INBOX_RESOURCE, NOTIFICATION_URL, chrono::Duration::seconds(5))],
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/subscriptions"))
            .and(body_partial_json(json!({ "resource": INBOX_RESOURCE })))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "error": { "code": "ExtensionError", "message": "Conflict" }
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/subscriptions"))
            .and(body_partial_json(json!({ "resource": EVENTS_RESOURCE })))
            .respond_with(created("events", EVENTS_RESOURCE))
            .expect(1)
            .mount(&server)
            .await;
        let (mut subs, _) = subs(&server);
        subs.reconcile(&crate::push::fakes::channel())
            .await
            .unwrap();
        assert!(subs.inbox_ok());
        assert_eq!(subs.find("theirs").unwrap().watched, Some(Watched::Inbox));
        // Adopted as it is: made seconds ago, so not renewed.
        assert!(requests(&server, "PATCH").await.is_empty());
    }

    #[tokio::test]
    async fn one_gone_since_the_listing_is_made_again() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/subscriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "value": [
                    listed("expired", INBOX_RESOURCE, NOTIFICATION_URL, chrono::Duration::hours(2)),
                    listed("events", EVENTS_RESOURCE, NOTIFICATION_URL, chrono::Duration::minutes(2)),
                ],
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/subscriptions/expired"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "error": { "code": "ResourceNotFound", "message": "Not found" }
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/subscriptions"))
            .respond_with(created("again", INBOX_RESOURCE))
            .expect(1)
            .mount(&server)
            .await;
        let (mut subs, _) = subs(&server);
        subs.reconcile(&crate::push::fakes::channel())
            .await
            .unwrap();
        assert!(subs.find("again").is_some() && subs.find("expired").is_none());
    }
}

// --- The service over HTTP --------------------------------------------------

mod over_http {
    use super::*;
    use crate::push::relay::HttpRelay;
    use crate::push::relay::tests::{FakeRelay, Reply};

    fn start_on(relay: &FakeRelay) -> Run {
        Run::start_with(Some(Arc::new(HttpRelay::new(&relay.base))), |_, _, _| {})
    }

    async fn until(what: impl Fn() -> bool) {
        for _ in 0..100 {
            if what() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("timed out");
    }

    #[tokio::test]
    async fn refused_tokens_are_retried_fresh_and_busy_streams_wait() {
        let relay = FakeRelay::start(25).await;
        relay.channel_reply(Reply::refuse(401, "token_invalid", None));
        relay.stream_reply(Reply::refuse(429, "too_many_streams", Some(1)));
        let run = start_on(&relay);
        until(|| run.is_live()).await;

        // Refused, retried fresh; then fetched again for the reconnection.
        let channels = relay.requests_to("/v1/channels");
        assert_eq!(channels.len(), 3);
        assert_eq!(channels[0].headers["authorization"], "Bearer token-1");
        assert_eq!(channels[1].headers["authorization"], "Bearer token-2");
        assert_eq!(run.tokens.calls()[1], (MIN_TOKEN_LEFT, true));
        let streams = relay.requests_to("/v1/events");
        assert_eq!(streams.len(), 2);
        assert!(streams[1].at - streams[0].at >= SECOND);
        // Subscribed on the channel the service gave.
        let created = run.graph.state.lock().unwrap().created.clone();
        assert!(
            created
                .iter()
                .all(|n| n.notification_url == relay.notification_url())
        );
        run.stop().await;
    }

    #[tokio::test]
    async fn a_service_that_refuses_the_organization_is_left_alone() {
        let relay = FakeRelay::start(25).await;
        relay.channel_reply(Reply::refuse(403, "tenant_not_allowed", None));
        let run = start_on(&relay);
        until(|| {
            let statuses = run.statuses.lock().unwrap();
            statuses
                .last()
                .is_some_and(|s| s.1 == PushHealth::Unsupported)
        })
        .await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(relay.requests().len(), 1);
        run.stop().await;
    }
}
