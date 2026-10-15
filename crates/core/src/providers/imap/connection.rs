//! Connecting to an IMAP server and running commands on an authenticated
//! session.
//!
//! Commands go through [`ImapSession::run`] rather than the convenience
//! methods of `async-imap`: those ignore the tagged `NO`/`BAD` result of some
//! commands (e.g. `UID FETCH`) and hide response codes such as `COPYUID`.
//! IDLE is raw as well ([`ImapSession::idle`]): `Session::idle` consumes the
//! session and its wait loop swallows `* OK [NOTIFICATIONOVERFLOW]`.

use std::collections::HashSet;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use async_imap::imap_proto::{
    AttributeValue, Capability, MailboxDatum, MessageSection, Response, ResponseCode, SectionPath,
    Status,
};
use async_imap::{Client, Session};
use chrono::{DateTime, Utc};
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use socket2::{SockRef, TcpKeepalive};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::{Sleep, timeout};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_util::sync::CancellationToken;

use super::ImapCredentials;
use super::mime::{self, StructureInfo};
use crate::error::{Error, Result};
use crate::model::TlsMode;
use crate::push::PushEvent;

/// Upper bound for establishing a connection (TCP, TLS and greeting).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// A read or write that makes no progress for this long fails the connection.
/// Large downloads are fine as long as data keeps flowing.
const IO_TIMEOUT: Duration = Duration::from_secs(60);
/// Sessions unused for longer are replaced instead of reused: servers and NAT
/// gateways silently drop idle connections, and detecting that costs a timeout.
const MAX_IDLE: Duration = Duration::from_secs(240);
/// TCP keepalive: the first probe after this much silence, then one every
/// interval. Keeps NAT mappings open during IDLE and notices a vanished
/// server in about three and a half minutes.
const KEEPALIVE_TIME: Duration = Duration::from_secs(90);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const KEEPALIVE_RETRIES: u32 = 4;
/// During IDLE the server may stay silent until the refresh; this is the
/// allowance beyond it before the connection counts as dead.
const IDLE_READ_SLACK: Duration = Duration::from_secs(90);
/// Time for the server to confirm the end of IDLE.
const IDLE_DONE_TIMEOUT: Duration = Duration::from_secs(30);
/// The same when shutting down, which must be quick.
const IDLE_CANCEL_TIMEOUT: Duration = Duration::from_secs(1);
/// A server ending IDLE by itself sooner than this is treated as a failure,
/// so reconnecting backs off instead of re-issuing IDLE in a tight loop.
const IDLE_MIN_SERVER_END: Duration = Duration::from_secs(60);

/// Advice shown when the server rejects the password.
pub(super) const APP_PASSWORD_HINT: &str = "If your provider uses two-step verification \
     (e.g. Gmail, iCloud, Yahoo or Fastmail), create an app password in your account's \
     security settings and use it instead of your normal password.";

#[derive(Debug)]
enum Transport {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

/// The connection to the server, plain or TLS. Writes that stall for
/// [`IO_TIMEOUT`], and reads that stall for the read timeout (the same,
/// except during IDLE), fail with [`io::ErrorKind::TimedOut`].
#[derive(Debug)]
pub(super) struct MailStream {
    transport: Transport,
    read_timeout: Duration,
    read_timer: Option<Pin<Box<Sleep>>>,
    write_timer: Option<Pin<Box<Sleep>>>,
}

impl MailStream {
    fn new(transport: Transport) -> Self {
        Self {
            transport,
            read_timeout: IO_TIMEOUT,
            read_timer: None,
            write_timer: None,
        }
    }

    /// Changes how long a read may make no progress, starting afresh.
    fn set_read_timeout(&mut self, limit: Duration) {
        self.read_timeout = limit;
        self.read_timer = None;
    }
}

/// Turns a pending I/O poll into a timeout error once it has been pending for
/// `limit` without any progress in between.
fn guard_progress<T>(
    result: Poll<io::Result<T>>,
    timer: &mut Option<Pin<Box<Sleep>>>,
    limit: Duration,
    cx: &mut Context<'_>,
) -> Poll<io::Result<T>> {
    if result.is_ready() {
        *timer = None;
        return result;
    }
    let sleep = timer.get_or_insert_with(|| Box::pin(tokio::time::sleep(limit)));
    if sleep.as_mut().poll(cx).is_pending() {
        return Poll::Pending;
    }
    *timer = None;
    Poll::Ready(Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "the server stopped responding",
    )))
}

impl AsyncRead for MailStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let result = match &mut this.transport {
            Transport::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Transport::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        };
        guard_progress(result, &mut this.read_timer, this.read_timeout, cx)
    }
}

impl AsyncWrite for MailStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = match &mut this.transport {
            Transport::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Transport::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        };
        guard_progress(result, &mut this.write_timer, IO_TIMEOUT, cx)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let result = match &mut this.transport {
            Transport::Plain(s) => Pin::new(s).poll_flush(cx),
            Transport::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        };
        guard_progress(result, &mut this.write_timer, IO_TIMEOUT, cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().transport {
            Transport::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Transport::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// State of the selected mailbox, from the `SELECT`/`EXAMINE` response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct MailboxInfo {
    pub uid_validity: u32,
    pub uid_next: u32,
    pub exists: u32,
    /// Present when the server supports CONDSTORE for this mailbox.
    pub highest_modseq: Option<u64>,
}

#[derive(Debug)]
struct Selected {
    mailbox: String,
    writable: bool,
    info: MailboxInfo,
}

/// System flags of a message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub draft: bool,
    pub deleted: bool,
}

/// The parts of a `FETCH` response the provider uses, copied out of the
/// response buffer.
#[derive(Debug, Default)]
pub(super) struct FetchedMessage {
    pub uid: u32,
    pub flags: Flags,
    pub internal_date: Option<DateTime<Utc>>,
    /// `BODY[HEADER…]` data.
    pub header: Option<Vec<u8>>,
    /// `BODY[<part>.MIME]` data.
    pub mime: Option<Vec<u8>>,
    /// Any other body section (`BODY[]`, `BODY[TEXT]`, `BODY[1.2]`, partial).
    pub body: Option<Vec<u8>>,
    pub structure: Option<StructureInfo>,
}

impl FetchedMessage {
    /// `None` for responses without a UID (unsolicited flag updates).
    fn parse(attributes: &[AttributeValue<'_>]) -> Option<Self> {
        let mut message = FetchedMessage::default();
        let mut has_uid = false;
        for attribute in attributes {
            match attribute {
                AttributeValue::Uid(uid) => {
                    message.uid = *uid;
                    has_uid = true;
                }
                AttributeValue::Flags(flags) => {
                    for flag in flags {
                        let flag = flag.as_ref();
                        let is = |name: &str| flag.eq_ignore_ascii_case(name);
                        message.flags.seen |= is("\\Seen");
                        message.flags.flagged |= is("\\Flagged");
                        message.flags.draft |= is("\\Draft");
                        message.flags.deleted |= is("\\Deleted");
                    }
                }
                AttributeValue::InternalDate(date) => {
                    message.internal_date = parse_internal_date(date);
                }
                AttributeValue::BodyStructure(structure) => {
                    message.structure = Some(mime::analyze_structure(structure));
                }
                AttributeValue::BodySection {
                    section,
                    data: Some(data),
                    ..
                } => {
                    let slot = match section {
                        Some(SectionPath::Full(MessageSection::Header)) => &mut message.header,
                        Some(SectionPath::Part(_, Some(MessageSection::Mime))) => &mut message.mime,
                        _ => &mut message.body,
                    };
                    *slot = Some(data.to_vec());
                }
                _ => {}
            }
        }
        has_uid.then_some(message)
    }
}

/// Parses an `INTERNALDATE` (`17-Jul-1996 02:44:25 -0700`).
pub(super) fn parse_internal_date(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_str(value.trim(), "%d-%b-%Y %H:%M:%S %z")
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Quotes a mailbox name or other string argument for a command.
pub(super) fn quote(value: &str) -> Result<String> {
    if value.contains(['\r', '\n', '\0']) {
        return Err(Error::invalid_input(
            "Mailbox names cannot contain line breaks.",
        ));
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

/// Why [`ImapSession::idle`] returned with the session still usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IdleExit {
    /// The refresh interval passed; IDLE should be issued again.
    Refreshed,
    /// Cancelled; IDLE has ended.
    Cancelled,
}

/// What an untagged response means to a session waiting in IDLE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Untagged {
    Changed(PushEvent),
    /// `* OK` without news; servers send these to keep the connection open.
    Keepalive,
    /// The server is closing the connection.
    Bye,
    Other,
}

/// Classifies an untagged response received in IDLE on the Inbox, with
/// NOTIFY's `STATUS` (other folders) and `LIST` (folder list) responses.
pub(super) fn classify(response: &Response<'_>) -> Untagged {
    match response {
        Response::MailboxData(MailboxDatum::Exists(_))
        | Response::Expunge(_)
        | Response::Vanished { .. }
        // Flags changed, e.g. read on another device.
        | Response::Fetch(..) => Untagged::Changed(PushEvent::InboxChanged),
        Response::MailboxData(MailboxDatum::Status { mailbox, .. }) => {
            Untagged::Changed(if mailbox.eq_ignore_ascii_case("INBOX") {
                PushEvent::InboxChanged
            } else {
                PushEvent::FolderChanged {
                    remote_id: mailbox.to_string(),
                }
            })
        }
        Response::MailboxData(MailboxDatum::List(_)) => {
            Untagged::Changed(PushEvent::FolderListChanged)
        }
        Response::Data {
            status: Status::Ok,
            outcome,
        } => {
            // imap-proto doesn't know this RFC 5465 code and leaves it in
            // the text: the server dropped notifications, so catch up.
            let overflow = outcome
                .information
                .as_deref()
                .and_then(|text| text.trim_start().get(..22))
                .is_some_and(|code| code.eq_ignore_ascii_case("[NOTIFICATIONOVERFLOW]"));
            if overflow {
                Untagged::Changed(PushEvent::CatchUp)
            } else {
                Untagged::Keepalive
            }
        }
        Response::Data {
            status: Status::Bye,
            ..
        } => Untagged::Bye,
        _ => Untagged::Other,
    }
}

/// Login refusals that are about the server rather than the password: too
/// many connections, or the server being unavailable for now (RFC 5530
/// `UNAVAILABLE` and `LIMIT`).
fn refuses_more_connections(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "[unavailable]",
        "[limit]",
        "too many",
        "maximum number of connections",
        "connection limit",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

/// Marks errors from [`refuses_more_connections`].
const SERVER_BUSY: &str = "serverBusy";

fn server_busy(host: &str) -> Error {
    Error::network(format!(
        "The IMAP server {host} isn't accepting more connections right now."
    ))
    .with_details(serde_json::json!({ "reason": SERVER_BUSY }))
}

/// Whether the server refused a connection because it has too many, or is
/// unavailable for now; trying again soon won't help.
pub(super) fn is_server_busy(error: &Error) -> bool {
    error
        .details
        .as_ref()
        .and_then(|d| d.get("reason"))
        .is_some_and(|r| r == SERVER_BUSY)
}

/// An authenticated IMAP session.
pub(super) struct ImapSession {
    session: Session<MailStream>,
    credentials: ImapCredentials,
    /// Upper-cased capability names (`MOVE`, `UIDPLUS`, `AUTH=PLAIN`, ...).
    capabilities: HashSet<String>,
    selected: Option<Selected>,
    /// False while a command is in flight and after one failed mid-way: the
    /// protocol state of such a session is unknown, so it must not be reused.
    healthy: bool,
    last_used: Instant,
}

impl ImapSession {
    /// Connects to the server and logs in.
    pub(super) async fn connect(credentials: &ImapCredentials) -> Result<Self> {
        let settings = &credentials.settings;
        let host = settings.imap_host.trim();
        let port = settings.imap_port;
        let client = timeout(CONNECT_TIMEOUT, open_client(host, port, settings.imap_tls))
            .await
            .map_err(|_| {
                Error::network(format!(
                    "Timed out connecting to the IMAP server {host}:{port}."
                ))
            })??;
        let session = match client
            .login(&settings.username, &credentials.password)
            .await
        {
            Ok(session) => session,
            Err((async_imap::error::Error::No(text), _)) if refuses_more_connections(&text) => {
                return Err(server_busy(host));
            }
            Err((async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_), _)) => {
                return Err(Error::auth_required(format!(
                    "The IMAP server {host} rejected the user name or password for {}. \
                     {APP_PASSWORD_HINT}",
                    settings.username
                )));
            }
            Err((e, _)) => return Err(imap_failure(e)),
        };
        let mut session = Self {
            session,
            credentials: credentials.clone(),
            capabilities: HashSet::new(),
            selected: None,
            healthy: true,
            last_used: Instant::now(),
        };
        // Servers often advertise more capabilities once logged in.
        let mut capabilities = HashSet::new();
        session
            .execute("CAPABILITY", |response| {
                if let Response::Capabilities(list) = response {
                    capabilities.extend(list.iter().map(capability_name));
                }
            })
            .await?;
        session.capabilities = capabilities;
        Ok(session)
    }

    /// Replaces a broken connection with a fresh one.
    pub(super) async fn reconnect(&mut self) -> Result<()> {
        *self = Self::connect(&self.credentials).await?;
        Ok(())
    }

    pub(super) fn has_capability(&self, name: &str) -> bool {
        self.capabilities.contains(name)
    }

    pub(super) fn is_healthy(&self) -> bool {
        self.healthy
    }

    /// Whether the session can be used for another operation.
    pub(super) fn is_reusable(&self) -> bool {
        self.healthy && self.last_used.elapsed() < MAX_IDLE
    }

    /// Logs out politely; failures are irrelevant as the session is dropped.
    pub(super) async fn logout(self) {
        self.logout_within(Duration::from_secs(5)).await;
    }

    /// Logs out, giving up after `limit`.
    pub(super) async fn logout_within(mut self, limit: Duration) {
        let _ = timeout(limit, self.session.logout()).await;
    }

    fn set_read_timeout(&mut self, limit: Duration) {
        self.session.get_mut().set_read_timeout(limit);
    }

    /// Waits in IDLE (RFC 2177) on the selected mailbox until `refresh`
    /// passes or `cancel` fires, passing changes to `on` as they arrive.
    /// News doesn't end the IDLE: the operations session syncs while this one
    /// keeps listening. A server that refuses IDLE gives an `Unsupported`
    /// error.
    pub(super) async fn idle(
        &mut self,
        refresh: Duration,
        cancel: &CancellationToken,
        mut on: impl FnMut(PushEvent) + Send,
    ) -> Result<IdleExit> {
        self.healthy = false;
        self.last_used = Instant::now();
        // The server may legitimately say nothing until the refresh.
        self.set_read_timeout(refresh + IDLE_READ_SLACK);
        let result = self.run_idle(refresh, cancel, &mut on).await;
        self.set_read_timeout(IO_TIMEOUT);
        self.last_used = Instant::now();
        result
    }

    async fn run_idle(
        &mut self,
        refresh: Duration,
        cancel: &CancellationToken,
        on: &mut (impl FnMut(PushEvent) + Send),
    ) -> Result<IdleExit> {
        let tag = self
            .session
            .run_command("IDLE")
            .await
            .map_err(imap_failure)?;
        let mut farewell: Option<String> = None;
        // The server accepts with a continuation ("+ idling").
        loop {
            let data = match self.session.read_response().await {
                Ok(Some(data)) => data,
                Ok(None) => return Err(closed(&farewell)),
                Err(e) => return Err(io_failure(e)),
            };
            match data.parsed() {
                Response::Continue(_) => break,
                Response::Done {
                    tag: done,
                    status,
                    outcome,
                } if *done == tag => {
                    // The command is over, so the session is fine; IDLE isn't.
                    self.healthy = true;
                    return Err(Error::unsupported(format!(
                        "The IMAP server refused IDLE ({status:?}: {}).",
                        outcome.information.as_deref().unwrap_or("no reason given")
                    )));
                }
                other => report(other, on, &mut farewell),
            }
        }

        let started = tokio::time::Instant::now();
        let deadline = tokio::time::sleep(refresh);
        tokio::pin!(deadline);
        let exit = loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break IdleExit::Cancelled,
                _ = &mut deadline => break IdleExit::Refreshed,
                // Cancel-safe: the stream keeps partial responses buffered.
                response = self.session.read_response() => {
                    let data = match response {
                        Ok(Some(data)) => data,
                        Ok(None) => return Err(closed(&farewell)),
                        Err(e) => return Err(io_failure(e)),
                    };
                    match data.parsed() {
                        Response::Done { tag: done, status, outcome } if *done == tag => {
                            // The server ended IDLE by itself.
                            check_status("IDLE", status, outcome.information.as_deref())?;
                            self.healthy = true;
                            if started.elapsed() < IDLE_MIN_SERVER_END.min(refresh) {
                                return Err(Error::network("The IMAP server keeps ending IDLE."));
                            }
                            return Ok(IdleExit::Refreshed);
                        }
                        other => report(other, on, &mut farewell),
                    }
                }
            }
        };

        // DONE, then the tagged completion; changes may still arrive between.
        self.set_read_timeout(IO_TIMEOUT);
        let limit = match exit {
            IdleExit::Cancelled => IDLE_CANCEL_TIMEOUT,
            IdleExit::Refreshed => IDLE_DONE_TIMEOUT,
        };
        let finish = async {
            self.session
                .run_command_untagged("DONE")
                .await
                .map_err(imap_failure)?;
            loop {
                let data = match self.session.read_response().await {
                    Ok(Some(data)) => data,
                    Ok(None) => return Err(closed(&farewell)),
                    Err(e) => return Err(io_failure(e)),
                };
                match data.parsed() {
                    Response::Done {
                        tag: done,
                        status,
                        outcome,
                    } if *done == tag => {
                        return check_status("IDLE", status, outcome.information.as_deref());
                    }
                    other => report(other, on, &mut farewell),
                }
            }
        };
        timeout(limit, finish)
            .await
            .map_err(|_| Error::network("The IMAP server did not end IDLE in time."))??;
        self.healthy = true;
        Ok(exit)
    }

    /// Runs one command, passing every response (including the tagged
    /// completion) to `on_response`. `NO`/`BAD` completions become errors.
    pub(super) async fn execute(
        &mut self,
        command: &str,
        on_response: impl FnMut(&Response<'_>) + Send,
    ) -> Result<()> {
        let mut results = self.run(&[command.to_owned()], on_response).await?;
        results.pop().unwrap_or(Ok(()))
    }

    /// Sends `commands` pipelined (all before reading any response) and
    /// passes every response to `on_response`. Returns the outcome of each
    /// command; the outer error means the connection failed.
    pub(super) async fn run(
        &mut self,
        commands: &[String],
        mut on_response: impl FnMut(&Response<'_>) + Send,
    ) -> Result<Vec<Result<()>>> {
        self.healthy = false;
        self.last_used = Instant::now();
        let mut tags = Vec::with_capacity(commands.len());
        for command in commands {
            tags.push(
                self.session
                    .run_command(command)
                    .await
                    .map_err(imap_failure)?,
            );
        }
        let mut results: Vec<Option<Result<()>>> = (0..commands.len()).map(|_| None).collect();
        let mut pending = commands.len();
        let mut farewell: Option<String> = None;
        while pending > 0 {
            let data = match self.session.read_response().await {
                Ok(Some(data)) => data,
                Ok(None) => {
                    return Err(Error::network(match farewell {
                        Some(reason) => format!("The IMAP server closed the connection: {reason}"),
                        None => "The IMAP server closed the connection.".to_owned(),
                    }));
                }
                Err(e) => return Err(io_failure(e)),
            };
            let response = data.parsed();
            match response {
                Response::Done {
                    tag,
                    status,
                    outcome,
                } => {
                    if let Some(i) = tags.iter().position(|t| t == tag)
                        && results[i].is_none()
                    {
                        results[i] = Some(check_status(
                            &commands[i],
                            status,
                            outcome.information.as_deref(),
                        ));
                        pending -= 1;
                    }
                }
                Response::Data {
                    status: Status::Bye,
                    outcome,
                } => farewell = outcome.information.as_deref().map(str::to_owned),
                _ => {}
            }
            on_response(response);
        }
        self.healthy = true;
        self.last_used = Instant::now();
        Ok(results.into_iter().map(|r| r.unwrap_or(Ok(()))).collect())
    }

    /// Selects (`writable`) or examines a mailbox, always asking the server
    /// for its current state.
    pub(super) async fn select(&mut self, mailbox: &str, writable: bool) -> Result<MailboxInfo> {
        let condstore = self.has_capability("CONDSTORE");
        let command = format!(
            "{} {}{}",
            if writable { "SELECT" } else { "EXAMINE" },
            quote(mailbox)?,
            if condstore { " (CONDSTORE)" } else { "" }
        );
        // A failed SELECT leaves no mailbox selected.
        self.selected = None;
        let mut info = MailboxInfo::default();
        let mut has_uid_next = false;
        self.execute(&command, |response| match response {
            Response::Data {
                status: Status::Ok,
                outcome,
            } => match outcome.code {
                Some(ResponseCode::UidValidity(v)) => info.uid_validity = v,
                Some(ResponseCode::UidNext(n)) => {
                    info.uid_next = n;
                    has_uid_next = true;
                }
                Some(ResponseCode::HighestModSeq(m)) => info.highest_modseq = Some(m),
                _ => {}
            },
            Response::MailboxData(MailboxDatum::Exists(n)) => info.exists = *n,
            _ => {}
        })
        .await?;
        if !condstore {
            // Some servers report HIGHESTMODSEQ without offering CONDSTORE;
            // CHANGEDSINCE must not be used with them.
            info.highest_modseq = None;
        }
        if !has_uid_next {
            // RFC 3501 requires UIDNEXT but some old servers omit it.
            info.uid_next = if info.exists == 0 {
                1
            } else {
                let mut highest = 0;
                self.execute("UID FETCH * (UID)", |response| {
                    if let Response::Fetch(_, attributes) = response {
                        for attribute in attributes {
                            if let AttributeValue::Uid(uid) = attribute {
                                highest = highest.max(*uid);
                            }
                        }
                    }
                })
                .await?;
                highest.saturating_add(1)
            };
        }
        self.selected = Some(Selected {
            mailbox: mailbox.to_owned(),
            writable,
            info: info.clone(),
        });
        Ok(info)
    }

    /// Makes `mailbox` the selected mailbox, reusing the current selection
    /// when possible.
    pub(super) async fn open(&mut self, mailbox: &str, writable: bool) -> Result<MailboxInfo> {
        if let Some(selected) = &self.selected
            && selected.mailbox == mailbox
            && (selected.writable || !writable)
        {
            return Ok(selected.info.clone());
        }
        self.select(mailbox, writable).await
    }

    /// `UID SEARCH` in the selected mailbox.
    pub(super) async fn uid_search(&mut self, criteria: &str) -> Result<Vec<u32>> {
        let mut uids = Vec::new();
        self.execute(&format!("UID SEARCH {criteria}"), |response| {
            if let Response::MailboxData(MailboxDatum::Search(found)) = response {
                uids.extend_from_slice(found);
            }
        })
        .await?;
        Ok(uids)
    }

    /// `UID FETCH` in the selected mailbox. Unsolicited `FETCH` responses for
    /// other messages may be included; callers match on `uid`.
    pub(super) async fn uid_fetch(
        &mut self,
        uid_set: &str,
        items: &str,
    ) -> Result<Vec<FetchedMessage>> {
        let mut messages = Vec::new();
        self.execute(&format!("UID FETCH {uid_set} {items}"), |response| {
            if let Response::Fetch(_, attributes) = response
                && let Some(message) = FetchedMessage::parse(attributes)
            {
                messages.push(message);
            }
        })
        .await?;
        Ok(messages)
    }

    /// Appends a message to `mailbox` with the given flags (e.g. `(\Seen)`).
    pub(super) async fn append(
        &mut self,
        mailbox: &str,
        flags: &str,
        content: &[u8],
    ) -> Result<()> {
        let command = format!("APPEND {} {flags} {{{}}}", quote(mailbox)?, content.len());
        self.healthy = false;
        self.last_used = Instant::now();
        let tag = self
            .session
            .run_command(&command)
            .await
            .map_err(imap_failure)?;
        let mut sent = false;
        loop {
            let data = match self.session.read_response().await {
                Ok(Some(data)) => data,
                Ok(None) => return Err(Error::network("The IMAP server closed the connection.")),
                Err(e) => return Err(io_failure(e)),
            };
            match data.parsed() {
                Response::Continue(_) if !sent => {
                    let stream = self.session.get_mut();
                    stream.write_all(content).await.map_err(io_failure)?;
                    stream.write_all(b"\r\n").await.map_err(io_failure)?;
                    stream.flush().await.map_err(io_failure)?;
                    sent = true;
                }
                Response::Done {
                    tag: done,
                    status,
                    outcome,
                } if *done == tag => {
                    let result = check_status(&command, status, outcome.information.as_deref());
                    // Rejected before the literal was sent: the command is
                    // complete either way, so the session stays usable.
                    self.healthy = true;
                    return result;
                }
                _ => {}
            }
        }
    }
}

/// Passes a change seen while idling to `on` and remembers a farewell.
fn report(
    response: &Response<'_>,
    on: &mut (impl FnMut(PushEvent) + Send),
    farewell: &mut Option<String>,
) {
    match classify(response) {
        Untagged::Changed(event) => on(event),
        Untagged::Bye => {
            if let Response::Data { outcome, .. } = response {
                *farewell = outcome.information.as_deref().map(str::to_owned);
            }
        }
        Untagged::Keepalive | Untagged::Other => {}
    }
}

fn closed(farewell: &Option<String>) -> Error {
    Error::network(match farewell {
        Some(reason) => format!("The IMAP server closed the connection: {reason}"),
        None => "The IMAP server closed the connection.".to_owned(),
    })
}

/// Opens the TCP connection, negotiates TLS and reads the greeting.
async fn open_client(host: &str, port: u16, tls: TlsMode) -> Result<Client<MailStream>> {
    let tcp = TcpStream::connect((host, port)).await.map_err(|e| {
        Error::network(format!(
            "Could not connect to the IMAP server {host}:{port}: {e}"
        ))
    })?;
    // Commands are small and latency-bound.
    let _ = tcp.set_nodelay(true);
    let keepalive = TcpKeepalive::new()
        .with_time(KEEPALIVE_TIME)
        .with_interval(KEEPALIVE_INTERVAL)
        .with_retries(KEEPALIVE_RETRIES);
    if let Err(e) = SockRef::from(&tcp).set_tcp_keepalive(&keepalive) {
        tracing::debug!("could not enable TCP keepalive: {e}");
    }
    match tls {
        TlsMode::Tls => {
            let stream = tls_handshake(host, tcp).await?;
            greet(Client::new(MailStream::new(Transport::Tls(Box::new(
                stream,
            )))))
            .await
        }
        TlsMode::None => greet(Client::new(MailStream::new(Transport::Plain(tcp)))).await,
        TlsMode::StartTls => {
            let mut client = greet(Client::new(MailStream::new(Transport::Plain(tcp)))).await?;
            client
                .run_command_and_check_ok("STARTTLS", None)
                .await
                .map_err(|e| {
                    Error::network(format!(
                        "The IMAP server {host} does not support STARTTLS ({e}). \
                         Try the SSL/TLS security setting, usually on port 993."
                    ))
                })?;
            let Transport::Plain(tcp) = client.into_inner().transport else {
                return Err(Error::internal("STARTTLS on an encrypted connection"));
            };
            let stream = tls_handshake(host, tcp).await?;
            // The server sends no new greeting after STARTTLS.
            Ok(Client::new(MailStream::new(Transport::Tls(Box::new(
                stream,
            )))))
        }
    }
}

async fn greet(mut client: Client<MailStream>) -> Result<Client<MailStream>> {
    let greeting = client.read_response().await.map_err(io_failure)?;
    match greeting.as_ref().map(|g| g.parsed()) {
        Some(Response::Data {
            status: Status::Ok | Status::PreAuth,
            ..
        }) => Ok(client),
        Some(Response::Data {
            status: Status::Bye,
            outcome,
        }) => {
            let reason = outcome.information.as_deref().unwrap_or("no reason given");
            let error = Error::network(format!("The IMAP server refused the connection: {reason}"));
            Err(if refuses_more_connections(reason) {
                error.with_details(serde_json::json!({ "reason": SERVER_BUSY }))
            } else {
                error
            })
        }
        _ => Err(Error::provider(
            "The server did not greet like an IMAP server. Check the host and port.",
        )),
    }
}

/// TLS client configuration that trusts the system's certificate store,
/// including CAs installed by an organization.
pub(super) fn tls_config() -> Result<Arc<ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .and_then(|builder| builder.with_platform_verifier())
        .map_err(|e| Error::internal(format!("Could not set up TLS: {e}")))?
        .with_no_client_auth();
    Ok(Arc::new(config))
}

async fn tls_handshake(host: &str, tcp: TcpStream) -> Result<TlsStream<TcpStream>> {
    let name = ServerName::try_from(host.to_owned())
        .map_err(|_| Error::invalid_input(format!("\"{host}\" is not a valid host name.")))?;
    TlsConnector::from(tls_config()?)
        .connect(name, tcp)
        .await
        .map_err(|e| {
            Error::network(format!(
                "Could not establish a secure connection to {host}: {e}"
            ))
        })
}

fn capability_name(capability: &Capability<'_>) -> String {
    match capability {
        Capability::Imap4rev1 => "IMAP4REV1".to_owned(),
        Capability::Auth(mechanism) => format!("AUTH={}", mechanism.to_ascii_uppercase()),
        Capability::Atom(name) => name.to_ascii_uppercase(),
    }
}

/// `UID FETCH` → "UID FETCH", `SELECT "x"` → "SELECT": names a command in
/// error messages without echoing its arguments.
fn command_name(command: &str) -> String {
    let mut words = command.split_whitespace();
    match words.next() {
        Some(first) if first.eq_ignore_ascii_case("UID") => {
            format!("UID {}", words.next().unwrap_or_default())
        }
        Some(first) => first.to_owned(),
        None => String::new(),
    }
}

fn check_status(command: &str, status: &Status, information: Option<&str>) -> Result<()> {
    match status {
        Status::Ok => Ok(()),
        _ => Err(Error::provider(format!(
            "The IMAP server rejected {}: {}",
            command_name(command),
            information.unwrap_or("no reason given")
        ))),
    }
}

fn io_failure(e: io::Error) -> Error {
    if e.kind() == io::ErrorKind::TimedOut {
        return Error::network("The IMAP server did not respond in time.");
    }
    let text = e.to_string();
    if text.contains("during parsing of") {
        // The text embeds the whole unparsable response; keep logs readable.
        tracing::warn!(
            "unparsable IMAP response: {}",
            text.chars().take(300).collect::<String>()
        );
        return Error::provider("The IMAP server sent a response that could not be understood.");
    }
    Error::network(format!("The connection to the IMAP server failed: {text}"))
}

fn imap_failure(e: async_imap::error::Error) -> Error {
    match e {
        async_imap::error::Error::Io(e) => io_failure(e),
        async_imap::error::Error::ConnectionLost => {
            Error::network("The connection to the IMAP server was lost.")
        }
        async_imap::error::Error::No(info) | async_imap::error::Error::Bad(info) => {
            Error::provider(format!("The IMAP server returned an error: {info}"))
        }
        other => Error::provider(format!("IMAP error: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_internal_dates() {
        let date = parse_internal_date("17-Jul-1996 02:44:25 -0700").unwrap();
        assert_eq!(date.to_rfc3339(), "1996-07-17T09:44:25+00:00");
        let padded = parse_internal_date(" 7-Jul-1996 02:44:25 +0000").unwrap();
        assert_eq!(padded.to_rfc3339(), "1996-07-07T02:44:25+00:00");
        assert!(parse_internal_date("yesterday").is_none());
    }

    #[test]
    fn quotes_arguments() {
        assert_eq!(quote("INBOX").unwrap(), "\"INBOX\"");
        assert_eq!(quote(r#"a "b" \c"#).unwrap(), r#""a \"b\" \\c""#);
        assert!(quote("a\r\nb").is_err());
    }

    #[test]
    fn names_commands_without_arguments() {
        assert_eq!(command_name("UID FETCH 1:* (FLAGS)"), "UID FETCH");
        assert_eq!(command_name("SELECT \"secret\""), "SELECT");
    }

    #[test]
    fn builds_tls_config_with_platform_verifier() {
        assert!(tls_config().is_ok());
    }

    fn classified(line: &str) -> Untagged {
        let (rest, response) = Response::parse(line.as_bytes())
            .unwrap_or_else(|e| panic!("{line:?} does not parse: {e:?}"));
        assert!(rest.is_empty(), "{line:?}");
        classify(&response)
    }

    #[test]
    fn classifies_what_arrives_during_idle() {
        let inbox = Untagged::Changed(PushEvent::InboxChanged);
        for line in [
            "* 23 EXISTS\r\n",
            "* 3 EXPUNGE\r\n",
            "* 4 FETCH (FLAGS (\\Seen))\r\n",
            "* 4 FETCH (UID 99 FLAGS (\\Seen \\Flagged) MODSEQ (12345))\r\n",
            "* VANISHED 41,43:116\r\n",
            "* VANISHED (EARLIER) 300:310\r\n",
            // NOTIFY may report the selected Inbox like any other folder.
            "* STATUS INBOX (MESSAGES 3 UIDNEXT 9)\r\n",
        ] {
            assert_eq!(classified(line), inbox, "{line:?}");
        }
        assert_eq!(
            classified("* STATUS \"Lists/Rust\" (MESSAGES 52 UIDNEXT 99 UNSEEN 3)\r\n"),
            Untagged::Changed(PushEvent::FolderChanged {
                remote_id: "Lists/Rust".into()
            })
        );
        assert_eq!(
            classified("* LIST () \"/\" \"Projects/New\"\r\n"),
            Untagged::Changed(PushEvent::FolderListChanged)
        );
        assert_eq!(
            classified("* OK [NOTIFICATIONOVERFLOW] Too many changes\r\n"),
            Untagged::Changed(PushEvent::CatchUp)
        );
        assert_eq!(
            classified("* ok [notificationoverflow]\r\n"),
            Untagged::Changed(PushEvent::CatchUp)
        );
        assert_eq!(classified("* OK Still here\r\n"), Untagged::Keepalive);
        assert_eq!(
            classified("* OK [ALERT] Maintenance at 3 am\r\n"),
            Untagged::Keepalive
        );
        assert_eq!(classified("* BYE Server shutting down\r\n"), Untagged::Bye);
        assert_eq!(classified("* 2 RECENT\r\n"), Untagged::Other);
        assert_eq!(
            classified("* FLAGS (\\Seen \\Answered $Junk)\r\n"),
            Untagged::Other
        );
    }

    #[test]
    fn tells_busy_servers_from_wrong_passwords() {
        for text in [
            "code: None, info: Some(\"[UNAVAILABLE] Maximum number of connections from user+IP exceeded\")",
            "code: Some(Alert), info: Some(\"Too many simultaneous connections. (Failure)\")",
            "code: None, info: Some(\"[LIMIT] Connection limit reached\")",
        ] {
            assert!(refuses_more_connections(text), "{text}");
        }
        for text in [
            "code: None, info: Some(\"[AUTHENTICATIONFAILED] Invalid credentials\")",
            "code: None, info: Some(\"LOGIN failed. Invalid login/password\")",
        ] {
            assert!(!refuses_more_connections(text), "{text}");
        }
        assert!(is_server_busy(&server_busy("imap.example.com")));
        assert!(!is_server_busy(&Error::network("The connection was lost.")));
    }
}
