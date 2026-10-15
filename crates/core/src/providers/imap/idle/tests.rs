//! The watcher against a scripted IMAP server on the loopback interface.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};

use super::*;
use crate::model::{ImapSettings, TlsMode};
use crate::push::backoff::Backoff;
use crate::push::supervisor::Supervisor;
use crate::push::{PushHealth, WatchSignal};

const PASSWORD: &str = "secret";

/// How the scripted server behaves.
#[derive(Clone)]
struct Script {
    capabilities: &'static str,
    /// Answer to NOTIFY: `None` accepts it.
    notify_refusal: Option<&'static str>,
    /// Answer to LOGIN with the right password: `None` accepts it.
    login_refusal: Option<&'static str>,
    refuse_idle: bool,
    /// Accept IDLE and end it straight away.
    end_idle_at_once: bool,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            capabilities: "IMAP4rev1 IDLE",
            notify_refusal: None,
            login_refusal: None,
            refuse_idle: false,
            end_idle_at_once: false,
        }
    }
}

#[derive(Clone, Debug)]
enum Inject {
    Line(String),
    Close,
}

/// A loopback IMAP server that follows a [`Script`], logs every command
/// (without tags or arguments after the command name, except for a few) and
/// lets the test push untagged responses or cut the connection.
struct ScriptedServer {
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
    connections: Arc<AtomicUsize>,
    inject: broadcast::Sender<Inject>,
}

impl ScriptedServer {
    async fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let (inject, _) = broadcast::channel(16);
        tokio::spawn({
            let (log, connections, inject) = (log.clone(), connections.clone(), inject.clone());
            async move {
                while let Ok((stream, _)) = listener.accept().await {
                    // Small replies must not wait for delayed ACKs, which run
                    // on real time while tests may pause the clock.
                    stream.set_nodelay(true).unwrap();
                    connections.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(serve(
                        stream,
                        script.clone(),
                        log.clone(),
                        inject.subscribe(),
                    ));
                }
            }
        });
        Self {
            port,
            log,
            connections,
            inject,
        }
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn count(&self, command: &str) -> usize {
        self.log().iter().filter(|c| c.as_str() == command).count()
    }

    fn send(&self, line: &str) {
        self.inject.send(Inject::Line(line.into())).unwrap();
    }

    fn cut(&self) {
        self.inject.send(Inject::Close).unwrap();
    }

    fn credentials(&self, password: &str) -> ImapCredentials {
        ImapCredentials {
            settings: ImapSettings {
                imap_host: "127.0.0.1".into(),
                imap_port: self.port,
                imap_tls: TlsMode::None,
                smtp_host: "127.0.0.1".into(),
                smtp_port: 1,
                smtp_tls: TlsMode::None,
                username: "ada".into(),
            },
            password: password.into(),
            display_name: "Ada".into(),
            email: "ada@example.com".into(),
        }
    }

    /// Waits until `command` was logged `times` times.
    async fn wait_for(&self, command: &str, times: usize) {
        for _ in 0..500 {
            if self.count(command) >= times {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{command} x{times} never came: {:?}", self.log());
    }
}

async fn serve(
    stream: TcpStream,
    script: Script,
    log: Arc<Mutex<Vec<String>>>,
    mut inject: broadcast::Receiver<Inject>,
) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let out = |text: String| {
        let mut bytes = text.into_bytes();
        bytes.extend_from_slice(b"\r\n");
        bytes
    };
    if write
        .write_all(&out("* OK scripted IMAP ready".into()))
        .await
        .is_err()
    {
        return;
    }
    let mut idling: Option<String> = None;
    loop {
        let reply: Vec<String> = tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { return };
                if let Some(tag) = &idling && line == "DONE" {
                    log.lock().unwrap().push("DONE".into());
                    let reply = vec![format!("{tag} OK IDLE terminated")];
                    idling = None;
                    reply
                } else {
                    let (tag, rest) = line.split_once(' ').unwrap_or((&line, ""));
                    let command = rest.split(' ').next().unwrap_or("").to_ascii_uppercase();
                    log.lock().unwrap().push(match command.as_str() {
                        "EXAMINE" | "SELECT" | "NOTIFY" => rest.to_owned(),
                        _ => command.clone(),
                    });
                    match command.as_str() {
                        "LOGIN" if !rest.contains(PASSWORD) => {
                            vec![format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials")]
                        }
                        "LOGIN" => match script.login_refusal {
                            Some(refusal) => vec![format!("{tag} NO {refusal}")],
                            None => vec![format!("{tag} OK LOGIN completed")],
                        },
                        "CAPABILITY" => vec![
                            format!("* CAPABILITY {}", script.capabilities),
                            format!("{tag} OK CAPABILITY completed"),
                        ],
                        "NOTIFY" => match script.notify_refusal {
                            Some(refusal) => vec![format!("{tag} NO {refusal}")],
                            None => vec![format!("{tag} OK NOTIFY completed")],
                        },
                        "EXAMINE" | "SELECT" => vec![
                            "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)".into(),
                            "* 3 EXISTS".into(),
                            "* 0 RECENT".into(),
                            "* OK [UIDVALIDITY 1700000000] UIDs valid".into(),
                            "* OK [UIDNEXT 4] Predicted next UID".into(),
                            format!("{tag} OK [READ-ONLY] EXAMINE completed"),
                        ],
                        "IDLE" if script.refuse_idle => {
                            vec![format!("{tag} BAD IDLE not allowed")]
                        }
                        "IDLE" if script.end_idle_at_once => {
                            vec!["+ idling".into(), format!("{tag} OK IDLE over")]
                        }
                        "IDLE" => {
                            idling = Some(tag.to_owned());
                            vec!["+ idling".into()]
                        }
                        "LOGOUT" => {
                            let bye = [
                                "* BYE logging out".to_owned(),
                                format!("{tag} OK LOGOUT completed"),
                            ];
                            for line in bye {
                                let _ = write.write_all(&out(line)).await;
                            }
                            return;
                        }
                        _ => vec![format!("{tag} BAD unknown command")],
                    }
                }
            }
            injected = inject.recv() => match injected {
                Ok(Inject::Line(line)) => vec![line],
                Ok(Inject::Close) => return,
                Err(_) => vec![],
            },
        };
        for line in reply {
            if write.write_all(&out(line)).await.is_err() {
                return;
            }
        }
    }
}

/// Collects what a watcher reports.
fn recording_sink() -> (WatchSink, mpsc::UnboundedReceiver<WatchSignal>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let sink = WatchSink::new(move |signal| {
        let _ = tx.send(signal);
    });
    (sink, rx)
}

async fn next(rx: &mut mpsc::UnboundedReceiver<WatchSignal>) -> WatchSignal {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("a signal in time")
        .expect("the sink is open")
}

fn watcher(server: &ScriptedServer, refresh: Duration) -> Arc<ImapIdleWatcher> {
    Arc::new(ImapIdleWatcher::new(
        server.credentials(PASSWORD),
        IdleConfig {
            refresh,
            use_notify: true,
        },
    ))
}

#[tokio::test]
async fn examines_the_inbox_idles_and_reports_changes() {
    let server = ScriptedServer::start(Script::default()).await;
    let watcher = watcher(&server, Duration::from_secs(600));
    let (sink, mut signals) = recording_sink();
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let (watcher, cancel) = (watcher.clone(), cancel.clone());
        async move { watcher.watch(&sink, &cancel).await }
    });

    assert_eq!(
        next(&mut signals).await,
        WatchSignal::Connected(PushMode::ImapIdle)
    );
    assert_eq!(
        next(&mut signals).await,
        WatchSignal::Changed(PushEvent::CatchUp)
    );
    server.wait_for("IDLE", 1).await;
    server.send("* 4 EXISTS");
    server.send("* 1 RECENT");
    server.send("* OK Still here");
    server.send("* 2 FETCH (FLAGS (\\Seen))");
    server.send("* 1 EXPUNGE");
    for _ in 0..3 {
        assert_eq!(
            next(&mut signals).await,
            WatchSignal::Changed(PushEvent::InboxChanged)
        );
    }
    // Changes don't end the IDLE.
    assert_eq!(server.count("DONE"), 0);

    cancel.cancel();
    assert!(matches!(task.await.unwrap(), EndReason::Cancelled));
    let log = server.log();
    assert_eq!(
        log,
        [
            "LOGIN",
            "CAPABILITY",
            "EXAMINE \"INBOX\"",
            "IDLE",
            "DONE",
            "LOGOUT"
        ],
        "read-only, without NOTIFY when it isn't offered"
    );
}

#[tokio::test]
async fn uses_notify_when_offered() {
    let server = ScriptedServer::start(Script {
        capabilities: "IMAP4rev1 IDLE NOTIFY",
        ..Script::default()
    })
    .await;
    let watcher = watcher(&server, Duration::from_secs(600));
    let (sink, mut signals) = recording_sink();
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let (watcher, cancel) = (watcher.clone(), cancel.clone());
        async move { watcher.watch(&sink, &cancel).await }
    });
    assert_eq!(
        next(&mut signals).await,
        WatchSignal::Connected(PushMode::ImapNotify)
    );
    assert_eq!(
        next(&mut signals).await,
        WatchSignal::Changed(PushEvent::CatchUp)
    );
    let log = server.log();
    assert!(
        log[2].starts_with("NOTIFY SET (selected (MessageNew (UID) MessageExpunge FlagChange))"),
        "{log:?}"
    );
    assert_eq!(log[3], "EXAMINE \"INBOX\"");

    server.wait_for("IDLE", 1).await;
    server.send("* STATUS \"Lists/Rust\" (MESSAGES 52 UIDNEXT 99)");
    server.send("* LIST () \"/\" \"Projects\"");
    server.send("* OK [NOTIFICATIONOVERFLOW] Too many changes");
    for expected in [
        PushEvent::FolderChanged {
            remote_id: "Lists/Rust".into(),
        },
        PushEvent::FolderListChanged,
        PushEvent::CatchUp,
    ] {
        assert_eq!(next(&mut signals).await, WatchSignal::Changed(expected));
    }
    cancel.cancel();
    task.await.unwrap();
}

#[tokio::test]
async fn a_refused_notify_falls_back_to_idle() {
    let server = ScriptedServer::start(Script {
        capabilities: "IMAP4rev1 IDLE NOTIFY",
        notify_refusal: Some("[BADEVENT] Event not supported"),
        ..Script::default()
    })
    .await;
    let watcher = watcher(&server, Duration::from_secs(600));
    let (sink, mut signals) = recording_sink();
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let (watcher, cancel) = (watcher.clone(), cancel.clone());
        async move { watcher.watch(&sink, &cancel).await }
    });
    assert_eq!(
        next(&mut signals).await,
        WatchSignal::Connected(PushMode::ImapIdle)
    );
    server.wait_for("IDLE", 1).await;
    server.send("* 5 EXISTS");
    assert_eq!(
        next(&mut signals).await,
        WatchSignal::Changed(PushEvent::CatchUp)
    );
    assert_eq!(
        next(&mut signals).await,
        WatchSignal::Changed(PushEvent::InboxChanged)
    );
    cancel.cancel();
    task.await.unwrap();
}

#[tokio::test]
async fn idle_is_renewed_after_the_refresh_interval() {
    let server = ScriptedServer::start(Script::default()).await;
    let watcher = watcher(&server, Duration::from_millis(200));
    let (sink, _signals) = recording_sink();
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let (watcher, cancel) = (watcher.clone(), cancel.clone());
        async move { watcher.watch(&sink, &cancel).await }
    });
    server.wait_for("IDLE", 3).await;
    cancel.cancel();
    task.await.unwrap();
    let log = server.log();
    let idle: Vec<&str> = log
        .iter()
        .map(String::as_str)
        .skip_while(|c| *c != "IDLE")
        .collect();
    // IDLE, DONE, IDLE, DONE, ... on one connection.
    for pair in idle.chunks(2) {
        if pair.len() == 2 {
            assert_eq!(pair, ["IDLE", "DONE"], "{log:?}");
        }
    }
    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_long_silence_in_idle_is_not_a_timeout() {
    let server = ScriptedServer::start(Script::default()).await;
    // Far beyond the 60-second read timeout of other commands.
    let watcher = watcher(&server, Duration::from_secs(20 * 60));
    let (sink, mut signals) = recording_sink();
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let (watcher, cancel) = (watcher.clone(), cancel.clone());
        async move { watcher.watch(&sink, &cancel).await }
    });
    server.wait_for("IDLE", 1).await;
    tokio::time::sleep(Duration::from_secs(19 * 60)).await;
    assert!(!task.is_finished(), "{:?}", server.log());
    assert_eq!(server.count("DONE"), 0);
    // Then the IDLE is renewed on the same connection.
    tokio::time::sleep(Duration::from_secs(61)).await;
    server.wait_for("IDLE", 2).await;
    assert_eq!(server.count("DONE"), 1);
    server.send("* 9 EXISTS");
    while next(&mut signals).await != WatchSignal::Changed(PushEvent::InboxChanged) {}
    cancel.cancel();
    assert!(matches!(task.await.unwrap(), EndReason::Cancelled));
    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn servers_that_cannot_push_are_unsupported() {
    let without_idle = ScriptedServer::start(Script {
        capabilities: "IMAP4rev1 UIDPLUS",
        ..Script::default()
    })
    .await;
    let (sink, _signals) = recording_sink();
    let end = watcher(&without_idle, Duration::from_secs(600))
        .watch(&sink, &CancellationToken::new())
        .await;
    assert!(matches!(end, EndReason::Unsupported), "{end:?}");
    assert_eq!(without_idle.log(), ["LOGIN", "CAPABILITY", "LOGOUT"]);

    let refusing = ScriptedServer::start(Script {
        refuse_idle: true,
        ..Script::default()
    })
    .await;
    let end = watcher(&refusing, Duration::from_secs(600))
        .watch(&sink, &CancellationToken::new())
        .await;
    assert!(matches!(end, EndReason::Unsupported), "{end:?}");
}

#[tokio::test]
async fn a_server_ending_idle_at_once_is_a_failure_not_a_loop() {
    let server = ScriptedServer::start(Script {
        end_idle_at_once: true,
        ..Script::default()
    })
    .await;
    let (sink, _signals) = recording_sink();
    let end = watcher(&server, Duration::from_secs(600))
        .watch(&sink, &CancellationToken::new())
        .await;
    assert!(matches!(end, EndReason::Failed(_)), "{end:?}");
    assert_eq!(server.count("IDLE"), 1);
}

#[tokio::test]
async fn a_busy_server_is_told_apart_from_a_wrong_password() {
    let busy = ScriptedServer::start(Script {
        login_refusal: Some("[UNAVAILABLE] Maximum number of connections from user+IP exceeded"),
        ..Script::default()
    })
    .await;
    let (sink, _signals) = recording_sink();
    let end = watcher(&busy, Duration::from_secs(600))
        .watch(&sink, &CancellationToken::new())
        .await;
    assert!(matches!(end, EndReason::TooManyConnections(_)), "{end:?}");

    let server = ScriptedServer::start(Script::default()).await;
    let wrong = ImapIdleWatcher::new(server.credentials("wrong"), IdleConfig::default());
    let end = wrong.watch(&sink, &CancellationToken::new()).await;
    assert!(matches!(end, EndReason::AuthRequired), "{end:?}");
}

/// A running supervisor and what it reported.
struct Supervised {
    cancel: CancellationToken,
    healths: Arc<Mutex<Vec<PushHealth>>>,
    events: mpsc::UnboundedReceiver<PushEvent>,
    task: tokio::task::JoinHandle<()>,
}

/// A supervisor around `watcher`, reporting statuses and events.
fn supervise(watcher: Arc<dyn MailWatcher>) -> Supervised {
    let cancel = CancellationToken::new();
    let healths = Arc::new(Mutex::new(Vec::new()));
    let (tx, events) = mpsc::unbounded_channel();
    let (_wake, wake_rx) = watch::channel(0);
    let (_online, online_rx) = watch::channel(true);
    let supervisor = Supervisor::new(
        "acct",
        Arc::new(move |event| {
            let _ = tx.send(event);
        }),
        {
            let healths = healths.clone();
            Arc::new(move |_, health| healths.lock().unwrap().push(health))
        },
        cancel.clone(),
        wake_rx,
        online_rx,
    )
    .with_backoff(Backoff::seeded(1));
    let task = tokio::spawn(async move {
        // Keeps the senders alive as long as the supervisor.
        let _keep = (_wake, _online);
        supervisor.watch(watcher).await
    });
    Supervised {
        cancel,
        healths,
        events,
        task,
    }
}

#[tokio::test]
async fn reconnects_and_catches_up_after_the_server_closes() {
    let server = ScriptedServer::start(Script::default()).await;
    let Supervised {
        cancel,
        healths,
        mut events,
        task,
    } = supervise(watcher(&server, Duration::from_secs(600)));
    let first = tokio::time::timeout(Duration::from_secs(5), events.recv()).await;
    assert_eq!(first.unwrap(), Some(PushEvent::CatchUp));
    server.wait_for("IDLE", 1).await;

    server.cut();
    let again = tokio::time::timeout(Duration::from_secs(5), events.recv()).await;
    assert_eq!(again.unwrap(), Some(PushEvent::CatchUp));
    server.wait_for("IDLE", 2).await;
    assert_eq!(server.connections.load(Ordering::SeqCst), 2);
    let seen = healths.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|h| matches!(h, PushHealth::Degraded { .. })),
        "{seen:?}"
    );
    assert_eq!(seen.last(), Some(&PushHealth::Live));

    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("stops within two seconds")
        .unwrap();
    assert_eq!(server.log().last().map(String::as_str), Some("LOGOUT"));
}

#[tokio::test]
async fn an_auth_failure_stops_after_one_attempt() {
    let server = ScriptedServer::start(Script::default()).await;
    let wrong = Arc::new(ImapIdleWatcher::new(
        server.credentials("wrong"),
        IdleConfig::default(),
    ));
    let Supervised {
        cancel,
        healths,
        task,
        ..
    } = supervise(wrong);
    for _ in 0..200 {
        if healths.lock().unwrap().contains(&PushHealth::AuthRequired) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        healths.lock().unwrap().last(),
        Some(&PushHealth::AuthRequired)
    );
    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    assert!(!task.is_finished(), "parked until restarted");
    cancel.cancel();
    task.await.unwrap();
}
