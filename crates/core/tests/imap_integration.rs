//! End-to-end tests of the IMAP/SMTP provider against a real mail server.
//!
//! Skipped unless `CUMULAS_TEST_IMAP=1`. The defaults match GreenMail started
//! with `-Dgreenmail.setup.test.all -Dgreenmail.users=alice:secret@example.com`
//! (see `.github/workflows/ci.yml`):
//!
//! | Variable                         | Default             |
//! | -------------------------------- | ------------------- |
//! | `CUMULAS_TEST_HOST`              | `127.0.0.1`         |
//! | `CUMULAS_TEST_IMAP_PORT`         | `3143` (plain)      |
//! | `CUMULAS_TEST_SMTP_PORT`         | `3025` (plain)      |
//! | `CUMULAS_TEST_USER`              | `alice`             |
//! | `CUMULAS_TEST_PASSWORD`          | `secret`            |
//! | `CUMULAS_TEST_EMAIL`             | `alice@example.com` |
//!
//! Optional TLS checks (the certificate must be trusted by the system store,
//! e.g. via `SSL_CERT_FILE`): `CUMULAS_TEST_IMAPS_PORT` (implicit TLS),
//! `CUMULAS_TEST_STARTTLS=1` (the plain port offers STARTTLS), and
//! `CUMULAS_TEST_UNTRUSTED_TLS_PORT` (a port with an untrusted certificate,
//! such as GreenMail's self-signed 3993, which must be refused).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use cumulas_core::ErrorCode;
use cumulas_core::model::{
    EmailAddress, Folder, FolderRole, ImapSettings, Importance, MessageSummary, OutgoingAttachment,
    OutgoingMessage, TlsMode,
};
use cumulas_core::providers::imap::{self, ImapCredentials, ImapProvider};
use cumulas_core::providers::{MailProvider, MessageBatch, MessageLocator, SyncWindow};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

struct Config {
    host: String,
    imap_port: u16,
    smtp_port: u16,
    user: String,
    password: String,
    email: String,
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn env_port(name: &str) -> Option<u16> {
    std::env::var(name)
        .ok()
        .map(|p| p.parse().expect("port number"))
}

impl Config {
    fn from_env() -> Option<Self> {
        if std::env::var("CUMULAS_TEST_IMAP").as_deref() != Ok("1") {
            eprintln!("skipping IMAP integration test; set CUMULAS_TEST_IMAP=1 to run it");
            return None;
        }
        Some(Self {
            host: env("CUMULAS_TEST_HOST", "127.0.0.1"),
            imap_port: env("CUMULAS_TEST_IMAP_PORT", "3143").parse().expect("port"),
            smtp_port: env("CUMULAS_TEST_SMTP_PORT", "3025").parse().expect("port"),
            user: env("CUMULAS_TEST_USER", "alice"),
            password: env("CUMULAS_TEST_PASSWORD", "secret"),
            email: env("CUMULAS_TEST_EMAIL", "alice@example.com"),
        })
    }

    fn credentials(&self) -> ImapCredentials {
        ImapCredentials {
            settings: ImapSettings {
                imap_host: self.host.clone(),
                imap_port: self.imap_port,
                imap_tls: TlsMode::None,
                smtp_host: self.host.clone(),
                smtp_port: self.smtp_port,
                smtp_tls: TlsMode::None,
                username: self.user.clone(),
            },
            password: self.password.clone(),
            display_name: "Alice Tester".into(),
            email: self.email.clone(),
        }
    }

    /// A second, independent IMAP client ("another device").
    async fn raw_session(&self) -> async_imap::Session<TcpStream> {
        let tcp = TcpStream::connect((self.host.as_str(), self.imap_port))
            .await
            .expect("connect");
        let mut client = async_imap::Client::new(tcp);
        client.read_response().await.expect("greeting");
        client
            .login(&self.user, &self.password)
            .await
            .map_err(|(e, _)| e)
            .expect("raw login")
    }
}

fn window() -> SyncWindow {
    SyncWindow {
        since: Utc::now() - chrono::Duration::days(30),
        max_messages: 500,
    }
}

fn locator(folder: &Folder, message: &MessageSummary) -> MessageLocator {
    MessageLocator {
        folder_remote_id: folder.remote_id.clone(),
        remote_id: message.remote_id.clone(),
    }
}

/// Syncs `folder` incrementally until `done` accepts a batch.
async fn sync_until(
    provider: &ImapProvider,
    folder: &Folder,
    state: &mut Option<String>,
    what: &str,
    done: impl Fn(&MessageBatch) -> bool,
) -> MessageBatch {
    for _ in 0..40 {
        let batch = provider
            .sync_messages(folder, state.as_deref(), window())
            .await
            .expect("sync");
        *state = batch.sync_state.clone();
        if done(&batch) {
            return batch;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("timed out waiting for {what} in {}", folder.name);
}

fn find<'a>(batch: &'a MessageBatch, subject: &str) -> Option<&'a MessageSummary> {
    batch.upserts.iter().find(|m| m.subject == subject)
}

/// UID of a message from its remote id (`mailbox \x1f uidvalidity \x1f uid`).
fn uid_of(message: &MessageSummary) -> u32 {
    message
        .remote_id
        .rsplit('\u{1f}')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn imap_provider_end_to_end() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let tag = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();

    // --- connection checks -------------------------------------------------
    imap::test_connection(&config.credentials())
        .await
        .expect("valid credentials are accepted");
    let mut wrong = config.credentials();
    wrong.password = "not-the-password".into();
    let err = imap::test_connection(&wrong).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthRequired, "{err:?}");
    assert!(err.message.contains("app password"), "{}", err.message);

    // --- folders -------------------------------------------------------------
    let archive_name = format!("Archive-{tag}");
    let utf7_name = format!("Entw&APw-rfe-{tag}");
    let mut raw = config.raw_session().await;
    raw.create(&archive_name).await.expect("create archive");
    raw.create(&utf7_name).await.expect("create utf-7 folder");

    let provider = ImapProvider::new("acct", config.credentials());
    let folders = provider.list_folders().await.expect("list folders");
    let inbox = folders
        .iter()
        .find(|f| f.role == FolderRole::Inbox)
        .expect("inbox")
        .clone();
    assert_eq!(inbox.name, "Inbox");
    let archive = folders
        .iter()
        .find(|f| f.remote_id == archive_name)
        .expect("archive folder")
        .clone();
    let umlaut = folders
        .iter()
        .find(|f| f.remote_id == utf7_name)
        .expect("utf-7 folder");
    assert_eq!(umlaut.name, format!("Entwürfe-{tag}"));

    // --- initial sync ----------------------------------------------------------
    let first = provider
        .sync_messages(&inbox, None, window())
        .await
        .expect("initial sync");
    assert!(first.full_resync);
    let mut inbox_state = first.sync_state.clone();
    assert!(inbox_state.as_deref().unwrap().starts_with("v1:"));

    // --- send to self ------------------------------------------------------------
    let subject = format!("Quarterly numbers {tag}");
    let attachment = b"region,total\nnorth,42\n".to_vec();
    provider
        .send(
            &OutgoingMessage {
                account_id: "acct".into(),
                to: vec![EmailAddress::new(Some("Alice"), &config.email)],
                cc: vec![],
                bcc: vec![EmailAddress::new(None, "bob@example.com")],
                subject: subject.clone(),
                body_html: "<p>Hello <b>Alice</b>, the numbers are attached.</p>".into(),
                attachments: vec![OutgoingAttachment {
                    name: "numbers.csv".into(),
                    content_type: "text/csv".into(),
                    data_base64: BASE64.encode(&attachment),
                }],
                importance: Some(Importance::High),
                reply_to_message_id: None,
                reply_kind: None,
            },
            None,
        )
        .await
        .expect("send");

    let batch = sync_until(
        &provider,
        &inbox,
        &mut inbox_state,
        "the sent message",
        |b| find(b, &subject).is_some(),
    )
    .await;
    assert!(!batch.full_resync, "the second sync is incremental");
    assert!(
        batch
            .upserts
            .iter()
            .all(|m| !first.upserts.iter().any(|f| f.remote_id == m.remote_id)),
        "incremental sync only returns new messages"
    );
    let received = find(&batch, &subject).unwrap().clone();
    assert_eq!(received.from.as_ref().unwrap().address, config.email);
    assert_eq!(
        received.from.as_ref().unwrap().name.as_deref(),
        Some("Alice Tester")
    );
    assert_eq!(received.to[0].address, config.email);
    assert!(
        received.preview.starts_with("Hello Alice, the numbers"),
        "{}",
        received.preview
    );
    assert!(received.has_attachments);
    assert_eq!(received.importance, Importance::High);
    assert!(!received.is_read);
    assert!(!received.is_flagged);
    assert!(received.conversation_id.is_some());
    assert!((Utc::now() - received.received_at).num_minutes().abs() < 10);

    // --- message body and attachment -------------------------------------------------
    let received_at = locator(&inbox, &received);
    let content = provider.fetch_message(&received_at).await.expect("fetch");
    assert!(
        content
            .body_html
            .as_deref()
            .unwrap()
            .contains("<b>Alice</b>")
    );
    assert!(
        content
            .body_text
            .as_deref()
            .unwrap()
            .contains("Hello Alice")
    );
    assert!(content.internet_message_id.is_some());
    assert!(
        content.bcc.is_empty(),
        "Bcc must not be in the delivered headers"
    );
    assert_eq!(content.attachments.len(), 1);
    let meta = &content.attachments[0];
    assert_eq!(meta.name, "numbers.csv");
    assert!(meta.content_type.starts_with("text/csv"));
    let data = provider
        .fetch_attachment(&received_at, &meta.id)
        .await
        .expect("fetch attachment");
    assert_eq!(data, attachment);
    // Fetching does not mark the message read.
    let unchanged = sync_until(&provider, &inbox, &mut inbox_state, "a sync", |_| true).await;
    assert!(
        unchanged
            .flag_updates
            .iter()
            .filter(|u| u.remote_id == received.remote_id)
            .all(|u| !u.is_read)
    );

    // --- copy in Sent -----------------------------------------------------------------
    let folders = provider.list_folders().await.expect("list folders");
    let sent = folders
        .iter()
        .find(|f| f.role == FolderRole::Sent)
        .expect("a Sent folder exists (created if needed)")
        .clone();
    let counted = folders.iter().find(|f| f.id == inbox.id).unwrap();
    assert!(
        counted.total_count >= 1 && counted.unread_count >= 1,
        "{counted:?}"
    );
    let mut sent_state = None;
    let sent_batch = sync_until(&provider, &sent, &mut sent_state, "the Sent copy", |b| {
        find(b, &subject).is_some()
    })
    .await;
    let copy = find(&sent_batch, &subject).unwrap();
    assert!(copy.is_read, "the Sent copy is marked read");
    let copy_content = provider
        .fetch_message(&locator(&sent, copy))
        .await
        .expect("fetch sent copy");
    assert!(
        copy_content.bcc.is_empty(),
        "Bcc is not stored in the Sent copy"
    );
    assert_eq!(
        copy_content.internet_message_id,
        content.internet_message_id
    );

    // --- another client appends a message with an inline image ---------------------------
    let fixture = include_str!("fixtures/related-inline-image.eml")
        .replace("{TAG}", &tag)
        .replace("\r\n", "\n")
        .replace('\n', "\r\n");
    raw.append("INBOX", Some("()"), None, fixture.as_bytes())
        .await
        .expect("append fixture");
    let inline_subject = format!("Diagramme für {tag}");
    let batch = sync_until(
        &provider,
        &inbox,
        &mut inbox_state,
        "the appended message",
        |b| find(b, &inline_subject).is_some(),
    )
    .await;
    let inline = find(&batch, &inline_subject).unwrap().clone();
    assert_eq!(inline.from.as_ref().unwrap().name.as_deref(), Some("Renée"));
    assert_eq!(inline.preview, "Voilà le diagramme établi hier.");
    assert!(!inline.has_attachments, "inline images are not attachments");
    assert_eq!(inline.importance, Importance::Low);
    let inline_content = provider
        .fetch_message(&locator(&inbox, &inline))
        .await
        .expect("fetch inline");
    let html = inline_content.body_html.unwrap();
    assert!(
        html.contains("src=\"data:image/png;base64,iVBORw0KGgo"),
        "{html}"
    );
    assert!(!html.contains("cid:"));
    assert_eq!(inline_content.attachments.len(), 1);
    assert!(inline_content.attachments[0].is_inline);
    assert!(
        inline_content
            .body_text
            .unwrap()
            .contains("Voilà le diagramme")
    );

    // --- flags ---------------------------------------------------------------------------
    provider
        .set_read(std::slice::from_ref(&received_at), true)
        .await
        .expect("set read");
    provider
        .set_flagged(std::slice::from_ref(&received_at), true)
        .await
        .expect("set flagged");
    sync_until(
        &provider,
        &inbox,
        &mut inbox_state,
        "the flag change",
        |b| {
            b.flag_updates
                .iter()
                .any(|u| u.remote_id == received.remote_id && u.is_read && u.is_flagged)
        },
    )
    .await;
    // Changed by another client.
    raw.select("INBOX").await.expect("select");
    raw.run_command_and_check_ok(format!("UID STORE {} -FLAGS (\\Seen)", uid_of(&received)))
        .await
        .expect("raw store");
    sync_until(
        &provider,
        &inbox,
        &mut inbox_state,
        "the external flag change",
        |b| {
            b.flag_updates
                .iter()
                .any(|u| u.remote_id == received.remote_id && !u.is_read && u.is_flagged)
        },
    )
    .await;

    // --- move ---------------------------------------------------------------------------
    let moved = provider
        .move_messages(std::slice::from_ref(&received_at), &archive.remote_id)
        .await
        .expect("move");
    assert_eq!(moved.len(), 1);
    sync_until(
        &provider,
        &inbox,
        &mut inbox_state,
        "the move out of Inbox",
        |b| {
            b.retain_only.as_ref().is_some_and(|ids| {
                !ids.contains(&received.remote_id) && ids.contains(&inline.remote_id)
            })
        },
    )
    .await;
    let mut archive_state = None;
    let archived = sync_until(
        &provider,
        &archive,
        &mut archive_state,
        "the moved message",
        |b| find(b, &subject).is_some(),
    )
    .await;
    let archived = find(&archived, &subject).unwrap().clone();
    assert_eq!(
        moved[0].as_ref(),
        Some(&archived.remote_id),
        "COPYUID gives the new remote id"
    );
    assert!(archived.is_flagged, "flags move with the message");

    // --- delete -----------------------------------------------------------------------------
    provider
        .delete_messages(&[locator(&archive, &archived)])
        .await
        .expect("delete");
    sync_until(
        &provider,
        &archive,
        &mut archive_state,
        "the deletion",
        |b| {
            b.retain_only
                .as_ref()
                .is_some_and(|ids| !ids.contains(&archived.remote_id))
        },
    )
    .await;

    // A message another client flags \Deleted (without expunging) disappears.
    raw.run_command_and_check_ok(format!("UID STORE {} +FLAGS (\\Deleted)", uid_of(&inline)))
        .await
        .expect("raw delete flag");
    sync_until(
        &provider,
        &inbox,
        &mut inbox_state,
        "the \\Deleted flag",
        |b| b.removed_remote_ids.contains(&inline.remote_id),
    )
    .await;
    raw.run_command_and_check_ok("EXPUNGE")
        .await
        .expect("expunge");

    // --- stale cursors and ids -------------------------------------------------------------
    let stale_cursor = "v1:1:1:0:1:0";
    let resync = provider
        .sync_messages(&inbox, Some(stale_cursor), window())
        .await
        .expect("resync");
    assert!(
        resync.full_resync,
        "a UIDVALIDITY change forces a full resync"
    );
    let mut parts: Vec<&str> = archived.remote_id.split('\u{1f}').collect();
    parts[1] = "1";
    let stale_id = parts.join("\u{1f}");
    let err = provider
        .fetch_message(&MessageLocator {
            folder_remote_id: archive.remote_id.clone(),
            remote_id: stale_id,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    raw.logout().await.ok();
}

#[tokio::test]
async fn tls_connections() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let list_with = |port: u16, tls: TlsMode| {
        let mut credentials = config.credentials();
        credentials.settings.imap_port = port;
        credentials.settings.imap_tls = tls;
        async move { ImapProvider::new("tls", credentials).list_folders().await }
    };
    if let Some(port) = env_port("CUMULAS_TEST_IMAPS_PORT") {
        let folders = list_with(port, TlsMode::Tls).await.expect("implicit TLS");
        assert!(folders.iter().any(|f| f.role == FolderRole::Inbox));
    }
    if std::env::var("CUMULAS_TEST_STARTTLS").as_deref() == Ok("1") {
        let folders = list_with(config.imap_port, TlsMode::StartTls)
            .await
            .expect("STARTTLS");
        assert!(folders.iter().any(|f| f.role == FolderRole::Inbox));
    }
    if let Some(port) = env_port("CUMULAS_TEST_UNTRUSTED_TLS_PORT") {
        let err = list_with(port, TlsMode::Tls).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Network);
        assert!(err.message.contains("secure connection"), "{}", err.message);
    }
}

#[tokio::test]
async fn unreachable_server_is_a_network_error() {
    let Some(config) = Config::from_env() else {
        return;
    };
    // Nothing listens on port 9 (discard) on test machines.
    let mut credentials = config.credentials();
    credentials.settings.imap_host = "127.0.0.1".into();
    credentials.settings.imap_port = 9;
    let err = imap::test_connection(&credentials).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Network, "{err:?}");
}

/// Forwards TCP connections to the server and can cut all open ones at once,
/// like a server restart or a NAT gateway dropping an idle connection.
struct CuttableProxy {
    port: u16,
    connections: Arc<AtomicUsize>,
    cut: broadcast::Sender<()>,
}

impl CuttableProxy {
    async fn start(host: &str, port: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let local_port = listener.local_addr().expect("address").port();
        let connections = Arc::new(AtomicUsize::new(0));
        let (cut, _) = broadcast::channel(4);
        let upstream = format!("{host}:{port}");
        let counter = connections.clone();
        let cutter = cut.clone();
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                let mut server = TcpStream::connect(&upstream).await.expect("upstream");
                let mut cut = cutter.subscribe();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut client, &mut server) => {}
                        _ = cut.recv() => {}
                    }
                });
            }
        });
        Self {
            port: local_port,
            connections,
            cut,
        }
    }
}

#[tokio::test]
async fn reuses_the_session_and_reconnects_after_a_drop() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let proxy = CuttableProxy::start(&config.host, config.imap_port).await;
    let mut credentials = config.credentials();
    credentials.settings.imap_host = "127.0.0.1".into();
    credentials.settings.imap_port = proxy.port;
    let provider = ImapProvider::new("acct", credentials);

    let folders = provider.list_folders().await.expect("list");
    let inbox = folders
        .iter()
        .find(|f| f.role == FolderRole::Inbox)
        .unwrap();
    provider
        .sync_messages(inbox, None, window())
        .await
        .expect("sync");
    assert_eq!(
        proxy.connections.load(Ordering::SeqCst),
        1,
        "one shared session"
    );

    proxy.cut.send(()).expect("cut");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let again = provider
        .list_folders()
        .await
        .expect("transparently reconnected");
    // Other tests may add folders meanwhile; the inbox is always there.
    assert!(again.iter().any(|f| f.id == inbox.id));
    assert_eq!(proxy.connections.load(Ordering::SeqCst), 2);
}
