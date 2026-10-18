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
//!
//! The push tests (`idle_*`, `core_push_end_to_end`) each get a mailbox of
//! their own, so they can run alongside the others: GreenMail creates a user
//! for any address it delivers to, with the address as login and password.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use cumulas_core::model::{
    Account, DataKind, EmailAddress, Folder, FolderRole, ImapSettings, Importance, MessageSummary,
    NewImapAccount, OutgoingAttachment, OutgoingMessage, SearchOptions, SearchSort,
    ServerSearchStatus, SyncStatus, TlsMode,
};
use cumulas_core::providers::imap::{
    self, IdleConfig, ImapCredentials, ImapIdleWatcher, ImapProvider,
};
use cumulas_core::providers::{
    MailProvider, MessageBatch, MessageLocator, SearchPage, ServerQuery, SyncWindow,
};
use cumulas_core::push::backoff::Backoff;
use cumulas_core::push::supervisor::Supervisor;
use cumulas_core::push::{
    EndReason, MailWatcher, PushEvent, PushHealth, PushMode, PushStatus, WatchSignal, WatchSink,
};
use cumulas_core::search::{SearchQuery, dates};
use cumulas_core::secrets::MemorySecretStore;
use cumulas_core::service::{EventSink, LiveConnector, MailAlert};
use cumulas_core::store::Store;
use cumulas_core::{CoreService, ErrorCode};
use futures::TryStreamExt;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

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

// --- server search -------------------------------------------------------------

/// A plain message as another client would append it.
fn rfc822(from: &str, subject: &str, date: chrono::DateTime<Utc>, body: &str) -> String {
    format!(
        "From: {from}\r\nTo: Alice <alice@example.com>\r\nSubject: {subject}\r\n\
         Date: {}\r\nMessage-ID: <{}@example.com>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n",
        date.to_rfc2822(),
        uuid::Uuid::new_v4().simple()
    )
}

/// `APPEND`'s date-time argument.
fn internal_date(date: chrono::DateTime<Utc>) -> String {
    format!("\"{}\"", date.format("%d-%b-%Y %H:%M:%S +0000"))
}

/// Mail outside the sync window and in other folders, for searching:
/// returns the folders, in the order the app searches them.
async fn searchable_mailbox(config: &Config, credentials: &ImapCredentials) -> Vec<Folder> {
    let mut raw = config.other_device(credentials).await;
    let capabilities = raw.capabilities().await.expect("capabilities");
    let literal_plus = capabilities.has_str("LITERAL+") || capabilities.has_str("LITERAL-");
    println!(
        "server search literals: {}",
        if literal_plus {
            "non-synchronizing (LITERAL+/-)"
        } else {
            "synchronizing"
        }
    );
    raw.create("Archive").await.expect("create Archive");
    let now = Utc::now();
    let old = now - chrono::Duration::days(400);
    let messages = [
        (
            "INBOX",
            old,
            rfc822(
                "Northwind Billing <billing@northwind.example>",
                "An old zebracorn invoice",
                old,
                "Invoice 2023-118 is attached.",
            ),
        ),
        (
            "INBOX",
            now - chrono::Duration::days(2),
            rfc822(
                "Renée <renee@example.com>",
                "=?UTF-8?Q?Caf=C3=A9_=C3=BCber_alles?=",
                now - chrono::Duration::days(2),
                "Rendez-vous au café.",
            ),
        ),
        (
            "INBOX",
            now - chrono::Duration::days(3),
            "From: Bob <bob@example.com>\r\nTo: Alice <alice@example.com>\r\n\
             Subject: Budget\r\nMIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n\
             --b1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe budget.\r\n\
             --b1\r\nContent-Type: application/pdf; name=\"budget.pdf\"\r\n\
             Content-Disposition: attachment; filename=\"budget.pdf\"\r\n\
             Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--b1--\r\n"
                .to_owned(),
        ),
        (
            "Archive",
            now - chrono::Duration::days(200),
            rfc822(
                "Grace Hopper <grace@example.com>",
                "Archived zebracorn notes",
                now - chrono::Duration::days(200),
                "Notes from the offsite.",
            ),
        ),
    ];
    for (mailbox, date, message) in messages {
        raw.append(
            mailbox,
            Some("()"),
            Some(&internal_date(date)),
            message.as_bytes(),
        )
        .await
        .expect("append");
    }
    let _ = raw.logout().await;
    let provider = ImapProvider::new("acct", credentials.clone());
    let mut folders = provider.list_folders().await.expect("folders");
    folders.sort_by_key(|f| f.role.sort_rank());
    folders
}

fn subjects(page: &cumulas_core::providers::SearchPage) -> Vec<&str> {
    page.messages.iter().map(|m| m.subject.as_str()).collect()
}

#[tokio::test]
async fn imap_server_search() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let credentials = config.fresh_mailbox().await;
    let folders = searchable_mailbox(&config, &credentials).await;
    let inbox = folders
        .iter()
        .find(|f| f.role == FolderRole::Inbox)
        .unwrap();
    let archive = folders.iter().find(|f| f.remote_id == "Archive").unwrap();
    let provider = ImapProvider::new("acct", credentials.clone());

    // The sync window doesn't reach the old one.
    let synced = provider
        .sync_messages(inbox, None, window())
        .await
        .expect("sync");
    assert!(
        find(&synced, "An old zebracorn invoice").is_none(),
        "{:?}",
        synced
            .upserts
            .iter()
            .map(|m| &m.subject)
            .collect::<Vec<_>>()
    );

    let search = |text: &str, cursor: Option<String>, limit: u32| {
        let provider = &provider;
        let folders = &folders;
        let query = SearchQuery::parse(text);
        let text = text.to_owned();
        async move {
            provider
                .search(
                    &ServerQuery::whole_mailbox(&query, folders),
                    cursor.as_deref(),
                    limit,
                )
                .await
                .unwrap_or_else(|e| panic!("searching {text:?}: {e}"))
        }
    };

    // Every folder, newest first, each in its own.
    let page = search("zebracorn", None, 50).await;
    assert_eq!(
        subjects(&page),
        ["Archived zebracorn notes", "An old zebracorn invoice"]
    );
    assert_eq!(page.messages[0].folder_id, archive.id);
    assert_eq!(page.messages[1].folder_id, inbox.id);
    assert_eq!(page.next, None);
    assert!(page.messages[1].received_at < Utc::now() - chrono::Duration::days(399));

    // Non-ASCII: CHARSET UTF-8 and a literal.
    let page = search("subject:café", None, 50).await;
    assert_eq!(subjects(&page), ["Café über alles"]);
    let page = search("über", None, 50).await;
    assert_eq!(subjects(&page), ["Café über alles"]);

    assert_eq!(
        subjects(&search("has:attachment", None, 50).await),
        ["Budget"]
    );
    assert_eq!(
        subjects(&search("from:billing@northwind.example", None, 50).await),
        ["An old zebracorn invoice"]
    );
    assert_eq!(
        subjects(&search("body:2023-118", None, 50).await),
        ["An old zebracorn invoice"]
    );
    assert!(
        search("zebracorn has:attachment", None, 50)
            .await
            .messages
            .is_empty()
    );
    assert!(
        search("nothing-matches-this", None, 50)
            .await
            .messages
            .is_empty()
    );

    // A page at a time; the session serves others between pages.
    let first = search("zebracorn", None, 1).await;
    assert_eq!(subjects(&first), ["Archived zebracorn notes"]);
    assert!(first.next.is_some());
    let welcome = find(&synced, "Welcome").expect("the welcome message");
    provider
        .fetch_message(&locator(inbox, welcome))
        .await
        .expect("open a message between pages");
    provider
        .sync_messages(archive, None, window())
        .await
        .expect("sync between pages");
    let second = search("zebracorn", first.next, 1).await;
    assert_eq!(subjects(&second), ["An old zebracorn invoice"]);
    assert_eq!(second.next, None);
}

/// The first page of a search for `text`, its dates in UTC, in `scope` or
/// the whole mailbox.
async fn search_filtered(
    provider: &ImapProvider,
    folders: &[Folder],
    text: &str,
    scope: Option<&[Folder]>,
) -> SearchPage {
    let query = SearchQuery::parse(text);
    let received = dates::resolve(&query.dates, Utc::now(), &Utc, 0);
    let search = ServerQuery {
        received,
        scope,
        ..ServerQuery::whole_mailbox(&query, folders)
    };
    provider
        .search(&search, None, 50)
        .await
        .unwrap_or_else(|e| panic!("searching {text:?}: {e}"))
}

fn sorted(mut subjects: Vec<&str>) -> Vec<&str> {
    subjects.sort_unstable();
    subjects
}

/// Filters reach the server: flags as `FLAGGED` and `UNSEEN`, dates as
/// `SINCE` and `BEFORE`, and folders by looking in those alone.
#[tokio::test]
async fn imap_server_search_filters() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let credentials = config.fresh_mailbox().await;
    let folders = searchable_mailbox(&config, &credentials).await;
    let archive = folders
        .iter()
        .find(|f| f.remote_id == "Archive")
        .unwrap()
        .clone();
    let provider = ImapProvider::new("acct", credentials.clone());
    let search = |text: &'static str| search_filtered(&provider, &folders, text, None);

    assert!(search("is:flagged").await.messages.is_empty());
    // Flagged by another client.
    let old = search("subject:\"old zebracorn\"").await.messages[0].clone();
    let mut other = config.other_device(&credentials).await;
    other.select("INBOX").await.expect("select");
    other
        .run_command_and_check_ok(format!(
            "UID STORE {} +FLAGS.SILENT (\\Flagged)",
            uid_of(&old)
        ))
        .await
        .expect("flag");
    let _ = other.logout().await;
    assert_eq!(
        subjects(&search("is:flagged").await),
        ["An old zebracorn invoice"]
    );
    assert_eq!(
        subjects(&search("zebracorn is:flagged").await),
        ["An old zebracorn invoice"]
    );
    // The day it arrived on.
    let day = old.received_at.date_naive().format("%Y-%m-%d").to_string();
    let page = search_filtered(&provider, &folders, &format!("received:{day}"), None).await;
    assert_eq!(subjects(&page), ["An old zebracorn invoice"]);
    let page = search_filtered(
        &provider,
        &folders,
        &format!("zebracorn received:{day}.."),
        None,
    )
    .await;
    assert_eq!(
        subjects(&page),
        ["Archived zebracorn notes", "An old zebracorn invoice"]
    );
    // The server's days reach further than the device's, which checks
    // what it finds.
    let before = format!("zebracorn before:{day}");
    let page = search_filtered(&provider, &folders, &before, None).await;
    let query = SearchQuery::parse(&before);
    let received = dates::resolve(&query.dates, Utc::now(), &Utc, 0);
    assert!(
        page.messages
            .iter()
            .all(|m| !query.admits(m, &received, None))
    );
    // In one folder only.
    let scoped = search_filtered(
        &provider,
        &folders,
        "zebracorn in:archive",
        Some(std::slice::from_ref(&archive)),
    )
    .await;
    assert_eq!(subjects(&scoped), ["Archived zebracorn notes"]);
    // Unread mail, nothing else: all of it, here.
    assert_eq!(
        sorted(subjects(&search("is:unread").await)),
        [
            "An old zebracorn invoice",
            "Archived zebracorn notes",
            "Budget",
            "Café über alles",
            "Welcome"
        ]
    );
    assert!(search("is:read").await.messages.is_empty());
}

#[tokio::test]
async fn core_server_search_end_to_end() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let credentials = config.fresh_mailbox().await;
    searchable_mailbox(&config, &credentials).await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let secrets = Arc::new(MemorySecretStore::default());
    let service = CoreService::new(
        store.clone(),
        secrets.clone(),
        Arc::new(LiveConnector::new(store, secrets)),
        Arc::new(RecordingSink::default()),
    );
    let account = service
        .add_imap_account(NewImapAccount {
            display_name: "Search Tester".into(),
            email: credentials.email.clone(),
            password: credentials.password.clone(),
            settings: credentials.settings.clone(),
        })
        .await
        .expect("add the account");
    service.sync_account(&account.id).await.expect("sync");
    let local = |include: &[String]| -> Vec<MessageSummary> {
        let newest = SearchOptions {
            sort: SearchSort::Newest,
            ..SearchOptions::default()
        };
        service
            .search_messages("zebracorn", None, include, newest)
            .expect("local search")
            .into_iter()
            .map(|hit| hit.summary)
            .collect()
    };
    // Both are older than the sync window.
    assert!(local(&[]).is_empty());

    let found = service
        .search_server(
            "e2e",
            "zebracorn",
            &account.id,
            false,
            SearchOptions::default(),
        )
        .await
        .expect("server search");
    assert_eq!(found.status, ServerSearchStatus::Done, "{:?}", found.error);
    assert_eq!((found.message_ids.len(), found.added), (2, 2));
    let results = local(&found.message_ids);
    let subjects: Vec<&str> = results.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(
        subjects,
        ["Archived zebracorn notes", "An old zebracorn invoice"]
    );
    let (notes, invoice) = (results[0].clone(), results[1].clone());
    let message = service
        .get_message(&notes.id, false)
        .await
        .expect("open a search result");
    assert!(
        message
            .body_text
            .as_deref()
            .unwrap_or_default()
            .contains("Notes from the offsite"),
        "{message:?}"
    );

    // Syncing again (incrementally) keeps them; they are indexed now.
    service.sync_account(&account.id).await.expect("sync again");
    assert_eq!(local(&[]).len(), 2);

    // Expunged by another client, outside the window: noticed when opened.
    let mut other = config.other_device(&credentials).await;
    other.select("INBOX").await.expect("select");
    other
        .run_command_and_check_ok(format!(
            "UID STORE {} +FLAGS.SILENT (\\Deleted)",
            uid_of(&invoice)
        ))
        .await
        .expect("flag deleted");
    other
        .run_command_and_check_ok("EXPUNGE")
        .await
        .expect("expunge");
    let _ = other.logout().await;
    let error = service.get_message(&invoice.id, false).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound, "{error:?}");
    assert!(service.store().get_message_summary(&invoice.id).is_err());

    // Acting on a search result works like on any message, also once it
    // moved (to a new UID).
    let inbox = service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Inbox)
        .unwrap()
        .unwrap();
    service
        .move_messages(std::slice::from_ref(&notes.id), &inbox.id)
        .await
        .expect("move");
    let moved = local(&[]);
    let [moved] = &moved[..] else {
        panic!("{moved:?}");
    };
    assert_eq!(
        (moved.subject.as_str(), &moved.folder_id),
        ("Archived zebracorn notes", &inbox.id)
    );
    assert_ne!(moved.id, notes.id);
    service
        .set_flagged(std::slice::from_ref(&moved.id), true)
        .await
        .expect("flag");
    service
        .set_read(std::slice::from_ref(&moved.id), false)
        .await
        .expect("mark unread");
    let mut other = config.other_device(&credentials).await;
    other.select("INBOX").await.expect("select");
    let fetched: Vec<_> = other
        .uid_fetch(uid_of(moved).to_string(), "FLAGS")
        .await
        .expect("fetch")
        .try_collect()
        .await
        .expect("flags");
    let flags: Vec<String> = fetched[0].flags().map(|f| format!("{f:?}")).collect();
    assert!(flags.contains(&"Flagged".to_owned()), "{flags:?}");
    assert!(!flags.contains(&"Seen".to_owned()), "{flags:?}");
    let _ = other.logout().await;
    service
        .delete_messages(std::slice::from_ref(&moved.id))
        .await
        .expect("delete");
    assert!(local(&[]).iter().all(|m| m.folder_id != inbox.id));
    let again = service
        .search_server(
            "e2e-again",
            "zebracorn",
            &account.id,
            false,
            SearchOptions::default(),
        )
        .await
        .expect("server search");
    assert_eq!(again.status, ServerSearchStatus::Done);
    assert!(again.message_ids.is_empty(), "{:?}", again.message_ids);
}

/// New mail a server search finds before a sync does: the sync still
/// alerts for it, and drops it when another client expunged it meanwhile.
#[tokio::test]
async fn new_mail_found_by_a_search_before_the_sync() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let credentials = config.fresh_mailbox().await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let secrets = Arc::new(MemorySecretStore::default());
    let sink = Arc::new(RecordingSink::default());
    let service = CoreService::new(
        store.clone(),
        secrets.clone(),
        Arc::new(LiveConnector::new(store, secrets)),
        sink.clone(),
    );
    let account = service
        .add_imap_account(NewImapAccount {
            display_name: "Search Tester".into(),
            email: credentials.email.clone(),
            password: credentials.password.clone(),
            settings: credentials.settings.clone(),
        })
        .await
        .expect("add the account");
    service.sync_account(&account.id).await.expect("sync");

    config
        .deliver(&credentials.email, "Fresh zebracorn news")
        .await;
    config
        .deliver(&credentials.email, "Fresh zebracorn offer")
        .await;
    let mut found = None;
    for _ in 0..40 {
        let result = service
            .search_server(
                "fresh",
                "zebracorn",
                &account.id,
                false,
                SearchOptions::default(),
            )
            .await
            .expect("server search");
        if result.message_ids.len() == 2 {
            found = Some(result);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let found = found.expect("both new messages found on the server");
    let offer = service
        .store()
        .get_message_summaries(&found.message_ids)
        .unwrap()
        .into_iter()
        .find(|m| m.subject == "Fresh zebracorn offer")
        .unwrap();

    let mut other = config.other_device(&credentials).await;
    other.select("INBOX").await.expect("select");
    other
        .run_command_and_check_ok(format!(
            "UID STORE {} +FLAGS.SILENT (\\Deleted)",
            uid_of(&offer)
        ))
        .await
        .expect("flag deleted");
    other
        .run_command_and_check_ok("EXPUNGE")
        .await
        .expect("expunge");
    let _ = other.logout().await;

    service.sync_account(&account.id).await.expect("sync again");
    assert_eq!(*sink.new_mail.lock().unwrap(), ["Fresh zebracorn news"]);
    assert!(service.store().get_message_summary(&offer.id).is_err());
    let inbox = service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Inbox)
        .unwrap()
        .unwrap();
    let listed: Vec<String> = service
        .list_messages(&inbox.id, &Default::default())
        .unwrap()
        .items
        .into_iter()
        .map(|m| m.subject)
        .collect();
    assert!(
        listed.contains(&"Fresh zebracorn news".to_owned()),
        "{listed:?}"
    );
    assert!(
        !listed.contains(&"Fresh zebracorn offer".to_owned()),
        "{listed:?}"
    );
}

// --- push ----------------------------------------------------------------------

impl Config {
    /// A mailbox of its own, holding one message.
    async fn fresh_mailbox(&self) -> ImapCredentials {
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let address = format!("push-{}@example.com", &tag[..12]);
        self.deliver(&address, "Welcome").await;
        let mut credentials = self.credentials();
        credentials.settings.username = address.clone();
        credentials.password = address.clone();
        credentials.email = address;
        credentials
    }

    /// Sends a message to `to` through the server's SMTP port.
    async fn deliver(&self, to: &str, subject: &str) {
        let message = lettre::Message::builder()
            .from("Grace Hopper <grace@example.com>".parse().unwrap())
            .to(to.parse().unwrap())
            .subject(subject)
            .body("Hello there.".to_owned())
            .unwrap();
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&self.host)
            .port(self.smtp_port)
            .build()
            .send(message)
            .await
            .expect("SMTP delivery");
    }

    /// Another client logged in to the mailbox of `credentials`.
    async fn other_device(&self, credentials: &ImapCredentials) -> async_imap::Session<TcpStream> {
        let tcp = TcpStream::connect((self.host.as_str(), self.imap_port))
            .await
            .expect("connect");
        let mut client = async_imap::Client::new(tcp);
        client.read_response().await.expect("greeting");
        client
            .login(&credentials.settings.username, &credentials.password)
            .await
            .map_err(|(e, _)| e)
            .expect("login")
    }
}

/// Reports what a watcher sees through a channel.
fn channel_sink() -> (WatchSink, mpsc::UnboundedReceiver<WatchSignal>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let sink = WatchSink::new(move |signal| {
        let _ = tx.send(signal);
    });
    (sink, rx)
}

/// Waits up to five seconds for `wanted`, skipping other signals.
async fn expect_signal(signals: &mut mpsc::UnboundedReceiver<WatchSignal>, wanted: WatchSignal) {
    let found = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(signal) = signals.recv().await {
            if signal == wanted {
                return;
            }
        }
        panic!("the watcher stopped");
    })
    .await;
    assert!(found.is_ok(), "no {wanted:?} within five seconds");
}

/// A watcher running in a task.
struct Watching {
    cancel: CancellationToken,
    signals: mpsc::UnboundedReceiver<WatchSignal>,
    task: tokio::task::JoinHandle<EndReason>,
}

impl Watching {
    async fn stop(self) {
        self.cancel.cancel();
        let end = tokio::time::timeout(Duration::from_secs(2), self.task)
            .await
            .expect("stops within two seconds")
            .unwrap();
        assert!(matches!(end, EndReason::Cancelled), "{end:?}");
    }
}

/// Starts `watcher` and waits until it is listening. GreenMail has no NOTIFY.
async fn start_watching(watcher: ImapIdleWatcher) -> Watching {
    let (sink, mut signals) = channel_sink();
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move { watcher.watch(&sink, &cancel).await }
    });
    expect_signal(&mut signals, WatchSignal::Connected(PushMode::ImapIdle)).await;
    expect_signal(&mut signals, WatchSignal::Changed(PushEvent::CatchUp)).await;
    // The IDLE command follows the catch-up.
    tokio::time::sleep(Duration::from_millis(300)).await;
    Watching {
        cancel,
        signals,
        task,
    }
}

#[tokio::test]
async fn idle_reports_new_mail() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let credentials = config.fresh_mailbox().await;
    let watcher = ImapIdleWatcher::new(credentials.clone(), IdleConfig::default());
    let mut watching = start_watching(watcher).await;
    config.deliver(&credentials.email, "Pushed").await;
    expect_signal(
        &mut watching.signals,
        WatchSignal::Changed(PushEvent::InboxChanged),
    )
    .await;
    watching.stop().await;
}

#[tokio::test]
async fn idle_reports_expunge() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let credentials = config.fresh_mailbox().await;
    let watcher = ImapIdleWatcher::new(credentials.clone(), IdleConfig::default());
    let mut watching = start_watching(watcher).await;
    let mut other = config.other_device(&credentials).await;
    other.select("INBOX").await.expect("select");
    other
        .run_command_and_check_ok("STORE 1 +FLAGS (\\Deleted)")
        .await
        .expect("store");
    other
        .run_command_and_check_ok("EXPUNGE")
        .await
        .expect("expunge");
    expect_signal(
        &mut watching.signals,
        WatchSignal::Changed(PushEvent::InboxChanged),
    )
    .await;
    other.logout().await.ok();
    watching.stop().await;
}

#[tokio::test]
async fn idle_survives_refresh() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let credentials = config.fresh_mailbox().await;
    let proxy = CuttableProxy::start(&config.host, config.imap_port).await;
    let mut proxied = credentials.clone();
    proxied.settings.imap_host = "127.0.0.1".into();
    proxied.settings.imap_port = proxy.port;
    let watcher = ImapIdleWatcher::new(
        proxied,
        IdleConfig {
            refresh: Duration::from_secs(1),
            use_notify: true,
        },
    );
    let mut watching = start_watching(watcher).await;
    // Several refreshes (DONE, IDLE) go by...
    tokio::time::sleep(Duration::from_millis(3500)).await;
    // ...and the connection still reports new mail.
    config.deliver(&credentials.email, "After refreshes").await;
    expect_signal(
        &mut watching.signals,
        WatchSignal::Changed(PushEvent::InboxChanged),
    )
    .await;
    assert_eq!(proxy.connections.load(Ordering::SeqCst), 1);
    watching.stop().await;
}

/// A supervisor running in a task, with its health reports and events.
struct Supervised {
    cancel: CancellationToken,
    healths: Arc<Mutex<Vec<PushHealth>>>,
    events: mpsc::UnboundedReceiver<PushEvent>,
    task: tokio::task::JoinHandle<()>,
}

/// A supervisor around `watcher`.
fn supervise(watcher: ImapIdleWatcher) -> Supervised {
    let cancel = CancellationToken::new();
    let healths = Arc::new(Mutex::new(Vec::new()));
    let (tx, events) = mpsc::unbounded_channel();
    let (wake, wake_rx) = watch::channel(0);
    let (online, online_rx) = watch::channel(true);
    let supervisor = Supervisor::new(
        "integration",
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
    .with_backoff(Backoff::seeded(7));
    let watcher: Arc<dyn MailWatcher> = Arc::new(watcher);
    let task = tokio::spawn(async move {
        let _senders = (wake, online);
        supervisor.watch(watcher).await
    });
    Supervised {
        cancel,
        healths,
        events,
        task,
    }
}

async fn next_event(events: &mut mpsc::UnboundedReceiver<PushEvent>) -> PushEvent {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("an event within five seconds")
        .expect("the supervisor runs")
}

#[tokio::test]
async fn idle_reconnects_after_cut() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let credentials = config.fresh_mailbox().await;
    let proxy = CuttableProxy::start(&config.host, config.imap_port).await;
    let mut proxied = credentials.clone();
    proxied.settings.imap_host = "127.0.0.1".into();
    proxied.settings.imap_port = proxy.port;
    let Supervised {
        cancel,
        healths,
        mut events,
        task,
    } = supervise(ImapIdleWatcher::new(proxied, IdleConfig::default()));
    assert_eq!(next_event(&mut events).await, PushEvent::CatchUp);

    // Like a NAT gateway dropping the connection.
    tokio::time::sleep(Duration::from_millis(300)).await;
    proxy.cut.send(()).expect("cut");
    assert_eq!(next_event(&mut events).await, PushEvent::CatchUp);
    assert_eq!(proxy.connections.load(Ordering::SeqCst), 2);
    assert!(
        healths
            .lock()
            .unwrap()
            .iter()
            .any(|h| matches!(h, PushHealth::Degraded { .. }))
    );

    // The new connection reports new mail.
    tokio::time::sleep(Duration::from_millis(300)).await;
    config.deliver(&credentials.email, "After the cut").await;
    assert_eq!(next_event(&mut events).await, PushEvent::InboxChanged);
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("stops within two seconds")
        .unwrap();
}

#[tokio::test]
async fn idle_auth_failure_stops() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let proxy = CuttableProxy::start(&config.host, config.imap_port).await;
    let mut wrong = config.credentials();
    wrong.password = "not-the-password".into();
    wrong.settings.imap_host = "127.0.0.1".into();
    wrong.settings.imap_port = proxy.port;
    let Supervised {
        cancel,
        healths,
        task,
        ..
    } = supervise(ImapIdleWatcher::new(wrong, IdleConfig::default()));
    for _ in 0..100 {
        if healths.lock().unwrap().contains(&PushHealth::AuthRequired) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        healths.lock().unwrap().last(),
        Some(&PushHealth::AuthRequired)
    );
    // No retries.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(proxy.connections.load(Ordering::SeqCst), 1);
    cancel.cancel();
    task.await.unwrap();
}

/// Records what the service reports.
#[derive(Default)]
struct RecordingSink {
    new_mail: Mutex<Vec<String>>,
    synced: AtomicUsize,
    push: Mutex<Vec<PushStatus>>,
}

impl EventSink for RecordingSink {
    fn data_changed(&self, _: &[DataKind], _: Option<&str>) {}

    fn sync_status(&self, status: &SyncStatus) {
        if status.last_synced_at.is_some() && status.message.is_none() {
            self.synced.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn new_mail(&self, _: &Account, alert: &MailAlert) {
        let mut seen = self.new_mail.lock().unwrap();
        seen.extend(alert.messages().iter().map(|m| m.subject.clone()));
    }

    fn push_status(&self, status: &PushStatus) {
        self.push.lock().unwrap().push(status.clone());
    }
}

/// Polls `done` every 50 ms for up to `limit`.
async fn eventually(what: &str, limit: Duration, done: impl Fn() -> bool) {
    let start = tokio::time::Instant::now();
    while !done() {
        assert!(start.elapsed() < limit, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn core_push_end_to_end() {
    let Some(config) = Config::from_env() else {
        return;
    };
    let credentials = config.fresh_mailbox().await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let secrets = Arc::new(MemorySecretStore::default());
    let sink = Arc::new(RecordingSink::default());
    let service = Arc::new(CoreService::new(
        store.clone(),
        secrets.clone(),
        Arc::new(LiveConnector::new(store, secrets)),
        sink.clone(),
    ));
    let account = service
        .add_imap_account(NewImapAccount {
            display_name: "Push Tester".into(),
            email: credentials.email.clone(),
            password: credentials.password.clone(),
            settings: credentials.settings.clone(),
        })
        .await
        .expect("add the account");
    service.start_background();

    // The first full sync (after two seconds) brings the Inbox in; that
    // isn't news.
    eventually("the first sync", Duration::from_secs(15), || {
        sink.synced.load(Ordering::SeqCst) > 0
    })
    .await;
    eventually("push to be live", Duration::from_secs(5), || {
        service.push_statuses().unwrap().iter().any(|s| {
            s.account_id == account.id
                && s.mode == PushMode::ImapIdle
                && s.health == PushHealth::Live
        })
    })
    .await;
    assert!(sink.new_mail.lock().unwrap().is_empty());

    // The next full sync is minutes away, so this can only come by push.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let subject = format!("Pushed {}", &account.id[..8]);
    config.deliver(&credentials.email, &subject).await;
    eventually("the new mail notification", Duration::from_secs(5), || {
        sink.new_mail.lock().unwrap().contains(&subject)
    })
    .await;
    assert_eq!(sink.synced.load(Ordering::SeqCst), 1, "no full sync needed");
    let inbox = service
        .store()
        .find_folder_by_role(&account.id, FolderRole::Inbox)
        .unwrap()
        .unwrap();
    assert_eq!((inbox.unread_count, inbox.total_count), (2, 2));

    let started = tokio::time::Instant::now();
    service.shutdown().await;
    assert!(started.elapsed() <= Duration::from_secs(2));
}
