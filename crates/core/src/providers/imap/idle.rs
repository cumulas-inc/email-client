//! Push for IMAP accounts: a second connection waits in IDLE (RFC 2177) on
//! the Inbox and, where the server offers NOTIFY (RFC 5465), hears about the
//! other folders too. It only listens; syncing happens on the account's
//! operations session, so a sync never interrupts the IDLE.

use std::time::Duration;

use async_imap::imap_proto::Response;
use async_trait::async_trait;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::ImapCredentials;
use super::connection::{IdleExit, ImapSession, Untagged, classify, is_server_busy};
use crate::error::{Error, ErrorCode, Result};
use crate::push::{EndReason, MailWatcher, PushEvent, PushMode, WatchSink};

/// NOTIFY's events: new and removed messages and flag changes in the
/// selected mailbox (the Inbox), and the same plus folder changes
/// elsewhere, which arrive as `STATUS` and `LIST` responses.
const NOTIFY_EVENTS: &str = "(selected (MessageNew (UID) MessageExpunge FlagChange)) \
     (personal (MessageNew MessageExpunge FlagChange MailboxName))";
/// How long logging out may take when stopping.
const LOGOUT_LIMIT: Duration = Duration::from_millis(500);
/// A notification overflow this soon after the last one ends the connection
/// rather than setting NOTIFY again, so a server that keeps overflowing
/// meets the supervisor's backoff instead of a loop.
const OVERFLOW_MIN_GAP: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct IdleConfig {
    /// IDLE is re-issued this often. RFC 2177 asks for less than 29
    /// minutes, since servers may end an IDLE after 30.
    pub refresh: Duration,
    /// Use NOTIFY when the server offers it.
    pub use_notify: bool,
}

impl Default for IdleConfig {
    fn default() -> Self {
        Self {
            refresh: Duration::from_secs(24 * 60),
            use_notify: true,
        }
    }
}

/// [`MailWatcher`] for an IMAP account.
pub struct ImapIdleWatcher {
    credentials: ImapCredentials,
    config: IdleConfig,
}

impl ImapIdleWatcher {
    pub fn new(credentials: ImapCredentials, config: IdleConfig) -> Self {
        Self {
            credentials,
            config,
        }
    }

    /// Logs in, turns on NOTIFY if offered and examines the Inbox. `None`
    /// when the server has no IDLE.
    async fn open(&self) -> Result<Option<(ImapSession, PushMode)>> {
        let mut session = ImapSession::connect(&self.credentials).await?;
        if !session.has_capability("IDLE") {
            session.logout_within(LOGOUT_LIMIT).await;
            return Ok(None);
        }
        session.skip_unparsable();
        let mut mode = PushMode::ImapIdle;
        if self.config.use_notify
            && session.has_capability("NOTIFY")
            && set_notify(&mut session, &format!("NOTIFY SET {NOTIFY_EVENTS}"), |_| {}).await?
        {
            mode = PushMode::ImapNotify;
        }
        // Read-only, so this session never changes \Recent or anything else.
        session.select("INBOX", false).await?;
        Ok(Some((session, mode)))
    }
}

#[async_trait]
impl MailWatcher for ImapIdleWatcher {
    async fn watch(&self, sink: &WatchSink, cancel: &CancellationToken) -> EndReason {
        let opened = tokio::select! {
            biased;
            _ = cancel.cancelled() => return EndReason::Cancelled,
            opened = self.open() => opened,
        };
        let (mut session, mut mode) = match opened {
            Ok(Some(opened)) => opened,
            Ok(None) => return EndReason::Unsupported,
            Err(e) => return end_reason(e),
        };
        sink.connected(mode.clone());
        // Whatever changed while there was no connection.
        sink.changed(PushEvent::CatchUp);
        let mut overflowed_at: Option<Instant> = None;
        loop {
            let idled = session
                .idle(self.config.refresh, cancel, |event| sink.changed(event))
                .await;
            match idled {
                Ok(IdleExit::Refreshed) => {}
                // Without NOTIFY there's nothing to set again.
                Ok(IdleExit::Overflowed) if mode != PushMode::ImapNotify => {}
                Ok(IdleExit::Overflowed) => {
                    if overflowed_at.is_some_and(|at| at.elapsed() < OVERFLOW_MIN_GAP) {
                        session.logout_within(LOGOUT_LIMIT).await;
                        return EndReason::Failed(Error::network(
                            "The IMAP server keeps dropping notifications.",
                        ));
                    }
                    overflowed_at = Some(Instant::now());
                    // NOTIFY is off (RFC 5465 §5.8). STATUS has the server
                    // report every folder, which catches up on the news it
                    // dropped.
                    let command = format!("NOTIFY SET STATUS {NOTIFY_EVENTS}");
                    let report = |response: &Response<'_>| {
                        if let Untagged::Changed(event) = classify(response) {
                            sink.changed(event);
                        }
                    };
                    let set = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return EndReason::Cancelled,
                        set = set_notify(&mut session, &command, report) => set,
                    };
                    match set {
                        Ok(true) => {}
                        Ok(false) => {
                            mode = PushMode::ImapIdle;
                            sink.connected(mode.clone());
                        }
                        Err(e) => return end_reason(e),
                    }
                }
                Ok(IdleExit::Cancelled) => {
                    session.logout_within(LOGOUT_LIMIT).await;
                    return EndReason::Cancelled;
                }
                // Stopped while starting IDLE, so the session is dropped.
                Err(_) if cancel.is_cancelled() => return EndReason::Cancelled,
                Err(e) => return end_reason(e),
            }
        }
    }
}

/// Turns NOTIFY on with `command`, passing its responses to `on`. False
/// when the server refuses: IDLE on the Inbox works all the same.
async fn set_notify(
    session: &mut ImapSession,
    command: &str,
    on: impl FnMut(&Response<'_>) + Send,
) -> Result<bool> {
    match session.execute(command, on).await {
        Ok(()) => Ok(true),
        Err(e) if session.is_healthy() => {
            tracing::debug!("the IMAP server refused NOTIFY ({e}); using IDLE alone");
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

fn end_reason(error: Error) -> EndReason {
    match error.code {
        ErrorCode::AuthRequired => EndReason::AuthRequired,
        ErrorCode::Unsupported => EndReason::Unsupported,
        _ if is_server_busy(&error) => EndReason::TooManyConnections(error),
        _ => EndReason::Failed(error),
    }
}

#[cfg(test)]
mod tests;
