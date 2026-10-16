//! Tells the core when the network comes and goes (the NetworkMonitor
//! portal), so that push pauses while offline and reconnects at once when
//! the network is back.
//!
//! The monitor can be wrong: it says offline when NetworkManager runs but
//! doesn't manage the connection, or when the network reaches the mail
//! servers without a default route. So while it says offline, a sync is
//! still tried now and then, and one that gets through means the network
//! is there after all.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use ashpd::desktop::network_monitor::NetworkMonitor;
use chrono::Utc;
use cumulas_core::CoreService;
use cumulas_core::model::SyncState;
use cumulas_core::push::WakeReason;
use futures::{Stream, StreamExt};
use tokio::time::Instant;

/// How long after the monitor says offline a sync is tried anyway.
const FIRST_TRY: Duration = Duration::from_secs(30);
/// How often one is tried after that, while the monitor still says so.
const RETRY: Duration = Duration::from_secs(5 * 60);

pub async fn watch(service: Arc<CoreService>) {
    let monitor = match NetworkMonitor::new().await {
        Ok(monitor) => monitor,
        Err(e) => {
            // Without the portal, the core assumes it's online and finds
            // out the hard way, as before.
            tracing::info!("no network monitor: {e}");
            return;
        }
    };
    let changes = match monitor.receive_changed().await {
        Ok(changes) => changes,
        Err(e) => {
            tracing::info!("can't follow the network monitor: {e}");
            return;
        }
    };
    // What the monitor says now and after each change, until it fails.
    let answers = futures::stream::once(async {})
        .chain(changes)
        .then(|()| monitor.is_available())
        .scan((), |(), available| {
            std::future::ready(
                available
                    .inspect_err(|e| tracing::info!("the network monitor stopped answering: {e}"))
                    .ok(),
            )
        });
    follow(
        answers,
        |available| service.network_changed(available),
        || service.wake(WakeReason::Network),
        || sync_gets_through(&service),
    )
    .await;
}

/// Passes the monitor's answers on to `tell`. While the last one is
/// offline, `try_sync` runs after [`FIRST_TRY`], then every [`RETRY`],
/// until one gets through; that's told as online, and it stands until the
/// monitor changes its answer. When the monitor stops answering, that's
/// online too.
///
/// A sync can get through during a real outage (a server on this computer,
/// such as a mail bridge), and then the other accounts' push fails into its
/// backoff. So when the monitor then says online, that's still the network
/// coming back: `wake` reconnects at once, since telling the core online
/// again changes nothing there.
async fn follow<T, F>(
    answers: impl Stream<Item = bool>,
    mut tell: impl FnMut(bool),
    mut wake: impl FnMut(),
    mut try_sync: T,
) where
    T: FnMut() -> F,
    F: Future<Output = bool>,
{
    let mut answers = std::pin::pin!(answers);
    let mut last = None;
    let mut next_try = None;
    // Online because a sync got through, while the monitor says offline.
    let mut overridden = false;
    loop {
        let answer = match next_try {
            Some(at) => tokio::select! {
                answer = answers.next() => answer,
                () = tokio::time::sleep_until(at) => {
                    if try_sync().await {
                        tracing::info!(
                            "a sync got through while the network monitor says offline; \
                             going on as online"
                        );
                        tell(true);
                        overridden = true;
                        next_try = None;
                    } else {
                        next_try = Some(Instant::now() + RETRY);
                    }
                    continue;
                }
            },
            None => answers.next().await,
        };
        let Some(available) = answer else {
            // Without the monitor, online, as without the portal.
            tell(true);
            return;
        };
        if last == Some(available) {
            continue;
        }
        tracing::debug!("network monitor: available {available}");
        tell(available);
        if available && overridden {
            wake();
        }
        overridden = false;
        last = Some(available);
        next_try = (!available).then(|| Instant::now() + FIRST_TRY);
    }
}

/// Syncs every account: true when one of them got through.
async fn sync_gets_through(service: &CoreService) -> bool {
    tracing::debug!("the network monitor says offline; trying a sync anyway");
    let started = Utc::now();
    service.sync_all().await;
    service.sync_statuses().is_ok_and(|statuses| {
        statuses.iter().any(|status| {
            status.state == SyncState::Idle && status.last_synced_at.is_some_and(|at| at >= started)
        })
    })
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use futures::channel::mpsc;

    use super::*;

    /// Lets the follower run until it waits on the monitor or a timer.
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn offline_is_believed_until_a_sync_gets_through() {
        let (monitor, answers) = mpsc::unbounded();
        let told = RefCell::new(Vec::new());
        let wakes = Cell::new(0);
        let tries = Cell::new(0);
        let works = Cell::new(false);
        let follower = follow(
            answers,
            |available| told.borrow_mut().push(available),
            || wakes.set(wakes.get() + 1),
            || {
                tries.set(tries.get() + 1);
                std::future::ready(works.get())
            },
        );
        let test = async {
            monitor.unbounded_send(true).unwrap();
            settle().await;
            tokio::time::sleep(RETRY * 3).await;
            // Online: nothing is tried.
            assert_eq!((told.borrow().clone(), tries.get()), (vec![true], 0));

            monitor.unbounded_send(false).unwrap();
            settle().await;
            assert_eq!(*told.borrow(), [true, false]);
            tokio::time::sleep(FIRST_TRY - Duration::from_secs(1)).await;
            assert_eq!(tries.get(), 0);
            // Really offline: tried again and again, and offline it stays.
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert_eq!(tries.get(), 1);
            // The monitor's changes that keep its answer don't put it off.
            for _ in 0..5 {
                tokio::time::sleep(RETRY / 5).await;
                monitor.unbounded_send(false).unwrap();
            }
            settle().await;
            assert_eq!(tries.get(), 2);
            assert_eq!(*told.borrow(), [true, false]);

            // The monitor is wrong: a sync gets through, so online.
            works.set(true);
            tokio::time::sleep(RETRY).await;
            assert_eq!(
                (told.borrow().clone(), tries.get()),
                (vec![true, false, true], 3)
            );
            tokio::time::sleep(RETRY * 3).await;
            monitor.unbounded_send(false).unwrap();
            settle().await;
            assert_eq!(tries.get(), 3, "tried while going on as online");

            // Until the monitor changes its answer.
            works.set(false);
            monitor.unbounded_send(true).unwrap();
            monitor.unbounded_send(false).unwrap();
            settle().await;
            assert_eq!(*told.borrow(), [true, false, true, true, false]);
            assert_eq!(wakes.get(), 1, "the network coming back wakes push");
            tokio::time::sleep(FIRST_TRY + Duration::from_secs(1)).await;
            assert_eq!(tries.get(), 4);

            // The monitor stops answering: online, as without one.
            drop(monitor);
            settle().await;
            assert_eq!(*told.borrow(), [true, false, true, true, false, true]);
            assert_eq!(wakes.get(), 1);
        };
        tokio::join!(follower, test);
    }

    #[tokio::test(start_paused = true)]
    async fn the_network_coming_back_wakes_push_after_a_sync_got_through() {
        let (monitor, answers) = mpsc::unbounded();
        let told = RefCell::new(Vec::new());
        let wakes = Cell::new(0);
        let follower = follow(
            answers,
            |available| told.borrow_mut().push(available),
            || wakes.set(wakes.get() + 1),
            // A server on this computer (a mail bridge) answers even while
            // the network is really gone.
            || std::future::ready(true),
        );
        let test = async {
            monitor.unbounded_send(false).unwrap();
            settle().await;
            tokio::time::sleep(FIRST_TRY + Duration::from_secs(1)).await;
            // Going on as online: the other accounts' push fails meanwhile
            // and backs off.
            assert_eq!((told.borrow().clone(), wakes.get()), (vec![false, true], 0));

            // The network really comes back: the core is online already,
            // so push is woken to reconnect at once.
            monitor.unbounded_send(true).unwrap();
            settle().await;
            assert_eq!(
                (told.borrow().clone(), wakes.get()),
                (vec![false, true, true], 1)
            );

            // Then the monitor's changes are told as before: the core wakes
            // push itself when it goes from offline to online.
            monitor.unbounded_send(false).unwrap();
            settle().await;
            monitor.unbounded_send(true).unwrap();
            settle().await;
            assert_eq!(
                (told.borrow().clone(), wakes.get()),
                (vec![false, true, true, false, true], 1)
            );
            drop(monitor);
        };
        tokio::join!(follower, test);
    }
}
