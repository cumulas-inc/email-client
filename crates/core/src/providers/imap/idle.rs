//! Push for IMAP accounts: a second connection waits in IDLE (RFC 2177) on
//! the Inbox and, where the server offers NOTIFY (RFC 5465), hears about the
//! other folders too. It only listens; syncing happens on the account's
//! operations session, so a sync never interrupts the IDLE.

use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use super::ImapCredentials;
use super::connection::{IdleExit, ImapSession, is_server_busy};
use crate::error::{Error, ErrorCode, Result};
use crate::push::{EndReason, MailWatcher, PushEvent, PushMode, WatchSink};

/// Asks for new and removed messages and flag changes in the selected
/// mailbox (the Inbox), and for the same plus folder changes elsewhere,
/// which arrive as `STATUS` and `LIST` responses.
const NOTIFY_SET: &str = "NOTIFY SET (selected (MessageNew (UID) MessageExpunge FlagChange)) \
     (personal (MessageNew MessageExpunge FlagChange MailboxName))";
/// How long logging out may take when stopping.
const LOGOUT_LIMIT: Duration = Duration::from_millis(500);

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
        let mut mode = PushMode::ImapIdle;
        if self.config.use_notify && session.has_capability("NOTIFY") {
            match session.execute(NOTIFY_SET, |_| {}).await {
                Ok(()) => mode = PushMode::ImapNotify,
                // Refused: IDLE on the Inbox works all the same.
                Err(e) if session.is_healthy() => {
                    tracing::debug!("the IMAP server refused NOTIFY ({e}); using IDLE alone");
                }
                Err(e) => return Err(e),
            }
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
        let (mut session, mode) = match opened {
            Ok(Some(opened)) => opened,
            Ok(None) => return EndReason::Unsupported,
            Err(e) => return end_reason(e),
        };
        sink.connected(mode);
        // Whatever changed while there was no connection.
        sink.changed(PushEvent::CatchUp);
        loop {
            let idled = session
                .idle(self.config.refresh, cancel, |event| sink.changed(event))
                .await;
            match idled {
                Ok(IdleExit::Refreshed) => {}
                Ok(IdleExit::Cancelled) => {
                    session.logout_within(LOGOUT_LIMIT).await;
                    return EndReason::Cancelled;
                }
                Err(e) => return end_reason(e),
            }
        }
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
