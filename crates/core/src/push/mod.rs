//! Push: hearing about new mail within seconds instead of at the next sync.
//!
//! Every account gets a supervisor ([`supervisor::Supervisor`]) while the app
//! runs. For IMAP accounts it keeps a [`MailWatcher`] connected (IDLE on a
//! second connection, plus NOTIFY where offered); for Microsoft accounts it
//! checks the Inbox on a timer until the push service arrives. Whatever the
//! source, changes become [`PushEvent`]s that feed the account's targeted
//! sync queue (`service::targeted`), which syncs just the Inbox or the
//! folder that changed.
//!
//! The supervisor reconnects with [`backoff::Backoff`], falls back to polling
//! when push is unavailable, pauses while offline and starts over at once
//! when the computer wakes up ([`wake`]). Its progress is published as a
//! [`PushStatus`] per account.

pub mod backoff;
#[cfg(test)]
pub(crate) mod fakes;
pub mod graph;
pub(crate) mod hub;
pub mod relay;
pub(crate) mod sse;
pub mod supervisor;
pub(crate) mod wake;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::error::Error;

/// Something changed on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushEvent {
    /// Mail arrived, left or changed in the Inbox.
    InboxChanged,
    /// Another folder changed (IMAP NOTIFY); `remote_id` is the folder's.
    FolderChanged { remote_id: String },
    /// Folders were created, renamed or deleted.
    FolderListChanged,
    /// Changes may have been missed (a new connection, a notification
    /// overflow): look again.
    CatchUp,
    /// Events changed (Microsoft 365).
    CalendarChanged,
}

/// How an account hears about changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PushMode {
    /// IMAP IDLE on the Inbox.
    ImapIdle,
    /// IMAP IDLE with NOTIFY, which also covers the other folders.
    ImapNotify,
    /// The Cumulas push service, fed by Microsoft Graph subscriptions.
    GraphRelay,
    /// The Inbox is checked on a timer.
    Polling {
        #[serde(rename = "everySecs")]
        every_secs: u32,
    },
    /// Instant updates are off; only the periodic sync runs.
    Off,
}

/// Whether the mode works as intended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PushHealth {
    Connecting,
    Live,
    /// Retrying after a failure; `reason` says what went wrong.
    Degraded {
        reason: String,
    },
    /// The server rejected the credentials; push resumes once the account is
    /// signed in again.
    AuthRequired,
    /// The account hasn't consented to the push service.
    ConsentMissing,
    /// The server can't push changes, so the Inbox is polled.
    Unsupported,
}

/// The push state of one account, for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushStatus {
    pub account_id: String,
    pub mode: PushMode,
    pub health: PushHealth,
    /// An explanation for the user, when there is one.
    pub detail: Option<String>,
    /// When the mode or health last changed.
    pub since: DateTime<Utc>,
}

/// Why a watcher stopped.
#[derive(Debug)]
pub enum EndReason {
    Cancelled,
    /// The server rejected the credentials. Not retried until the account
    /// changes.
    AuthRequired,
    /// The server can't push changes (no IDLE).
    Unsupported,
    /// The server refused another connection; try again much later.
    TooManyConnections(Error),
    /// The connection failed or dropped; try again soon.
    Failed(Error),
}

/// What a watcher reports while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchSignal {
    /// Connected and listening.
    Connected(PushMode),
    Changed(PushEvent),
}

/// Where a watcher reports to. Reporting never blocks the watcher.
#[derive(Clone)]
pub struct WatchSink(Arc<dyn Fn(WatchSignal) + Send + Sync>);

impl WatchSink {
    pub fn new(report: impl Fn(WatchSignal) + Send + Sync + 'static) -> Self {
        Self(Arc::new(report))
    }

    pub fn connected(&self, mode: PushMode) {
        (self.0)(WatchSignal::Connected(mode));
    }

    pub fn changed(&self, event: PushEvent) {
        (self.0)(WatchSignal::Changed(event));
    }
}

/// Keeps one connection to a mail server open and reports changes.
#[async_trait]
pub trait MailWatcher: Send + Sync {
    /// Connects and watches until the connection ends or `cancel` fires.
    /// Reports [`WatchSignal::Connected`] once listening, followed by a
    /// [`PushEvent::CatchUp`] for anything missed while disconnected.
    async fn watch(&self, sink: &WatchSink, cancel: &CancellationToken) -> EndReason;
}

/// Why the app should look again at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeReason {
    /// The computer woke up from suspend.
    Resume,
    /// The network came back.
    Network,
    /// Asked for, e.g. "Check for new mail".
    User,
}
