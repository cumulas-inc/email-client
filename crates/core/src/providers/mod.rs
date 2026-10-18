//! Protocol providers. Each account kind supplies implementations of these
//! traits; the sync engine and [`crate::service::CoreService`] only talk to the
//! traits, which keeps them testable with fakes.
//!
//! Providers return fully-formed model types with local ids already derived via
//! [`crate::model::local_id`] from the account id and the remote id.
//!
//! Microsoft accounts implement all four traits on top of Microsoft Graph,
//! and work and school accounts [`LabelsProvider`] too. IMAP accounts
//! implement [`MailProvider`] only; their calendar, contacts and tasks live
//! in the local store.

pub mod graph;
pub mod imap;

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::error::{Error, Result};
use crate::model::{
    AttachmentMeta, Calendar, Contact, ContactDraft, EmailAddress, Event, EventDraft,
    EventResponse, Folder, LabelCatalog, Message, MessageSummary, OutgoingMessage, ProtectAction,
    Task, TaskDraft, TaskList, TaskPatch,
};
use crate::search::SearchQuery;
use crate::search::dates::Received;

/// Where a message lives on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageLocator {
    pub folder_remote_id: String,
    pub remote_id: String,
}

/// How far back the initial sync of a folder reaches.
#[derive(Debug, Clone, Copy)]
pub struct SyncWindow {
    /// Only messages received after this instant are synced.
    pub since: DateTime<Utc>,
    /// Hard cap on the number of messages synced per folder.
    pub max_messages: u32,
}

/// Read/flag state change of a message whose other fields did not change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagUpdate {
    pub remote_id: String,
    pub is_read: bool,
    pub is_flagged: bool,
}

/// Result of syncing one folder.
#[derive(Debug, Clone, Default)]
pub struct MessageBatch {
    /// New or changed messages (summaries only; bodies are fetched lazily).
    /// An empty `preview` keeps the preview already stored for the message.
    pub upserts: Vec<MessageSummary>,
    /// Read/flagged changes for messages already stored; unknown ids are ignored.
    pub flag_updates: Vec<FlagUpdate>,
    /// Remote ids of messages that disappeared from the folder.
    pub removed_remote_ids: Vec<String>,
    /// When set, the complete list of remote ids currently in the folder
    /// within the sync window: any stored message of the folder not listed is
    /// removed, but for messages server search fetched that `retain_covers`
    /// doesn't name (from outside the window, as a rule). Lets
    /// providers that cannot report deletions (IMAP without QRESYNC)
    /// reconcile cheaply.
    pub retain_only: Option<Vec<String>>,
    /// With `retain_only`: the remote ids it covers, so that messages server
    /// search fetched among them are removed too when not listed (IMAP: the
    /// folder's UIDs from the window's lowest up, which may hold new mail a
    /// search found before the sync did). `None` covers none of them.
    pub retain_covers: Option<NumberedIds>,
    /// When true, `upserts` is the complete content of the folder within the
    /// sync window and anything else stored locally for it must be dropped
    /// (e.g. IMAP UIDVALIDITY changed or a Graph delta token expired), but
    /// for messages server search fetched from outside the window.
    pub full_resync: bool,
    /// Opaque provider cursor to pass to the next sync of this folder
    /// (Graph `@odata.deltaLink`, IMAP `uidvalidity:uidnext:modseq`, ...).
    pub sync_state: Option<String>,
    /// Messages that left this folder for another folder of the account,
    /// with `folder_id` naming the one they are in now (Graph reports a move
    /// out as a removal; the provider looks such removals up). A stored copy
    /// still in this folder moves there, keeping its fetched body; one whose
    /// destination isn't stored is removed; others are ignored (never
    /// inserted).
    pub moved_out: Vec<MessageSummary>,
    /// With `full_resync`: the folder's remote ids were reassigned (IMAP
    /// UIDVALIDITY changed), so rows server search fetched go too.
    pub ids_reassigned: bool,
}

/// The remote ids made of `prefix` and a number from `from` up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberedIds {
    pub prefix: String,
    pub from: u64,
}

impl NumberedIds {
    pub fn contains(&self, remote_id: &str) -> bool {
        remote_id
            .strip_prefix(self.prefix.as_str())
            .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|n| n.parse::<u64>().ok())
            .is_some_and(|n| n >= self.from)
    }
}

/// A folder's message counts as the server reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderCounts {
    pub unread: u32,
    pub total: u32,
}

/// One page of a server-side search.
#[derive(Debug, Clone, Default)]
pub struct SearchPage {
    /// Matches on this page, newest first, with `folder_id` set to the local
    /// id of the folder each is in (it may name a folder that isn't stored).
    pub messages: Vec<MessageSummary>,
    /// Opaque cursor for the next page; `None` when there is none.
    pub next: Option<String>,
}

/// How long a page of a server search may take when nothing says
/// otherwise ([`ServerQuery::whole_mailbox`]).
const SEARCH_PAGE_TIME: Duration = Duration::from_secs(45);

/// What a provider searches its server for: a query, its dates resolved, and where.
#[derive(Debug, Clone, Copy)]
pub struct ServerQuery<'a> {
    pub query: &'a SearchQuery,
    pub received: Received,
    /// Every stored folder of the account, well-known ones first.
    pub folders: &'a [Folder],
    /// The folders `in:` names in this account (non-empty), or None: the whole mailbox.
    pub scope: Option<&'a [Folder]>,
    /// When the caller stops waiting for the page: a provider that takes
    /// its time (IMAP looks through folder after folder) keeps within it.
    pub deadline: Instant,
}

impl<'a> ServerQuery<'a> {
    /// `query` anywhere in the mailbox, at any time, within 45 seconds.
    pub fn whole_mailbox(query: &'a SearchQuery, folders: &'a [Folder]) -> Self {
        Self {
            query,
            received: Received::default(),
            folders,
            scope: None,
            deadline: Instant::now() + SEARCH_PAGE_TIME,
        }
    }
}

/// Body and full headers of a message, as fetched from the server.
#[derive(Debug, Clone, Default)]
pub struct MessageContent {
    /// Raw (unsanitized) HTML body. `cid:` references to inline images should
    /// already be rewritten to `data:` URIs by the provider where possible.
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub bcc: Vec<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub attachments: Vec<AttachmentMeta>,
    pub internet_message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// What bears on protection and labels (see [`MessageSignals`]).
    pub signals: MessageSignals,
}

/// S/MIME content, from the CMS ContentInfo OID (or IMAP's smime-type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SmimeContent {
    Signed,
    Enveloped,
}

/// What the server says that bears on protection and labels; kept with the
/// content so detection and diagnostics work from the cache.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessageSignals {
    /// Content-Class: Graph's named property, else the header.
    pub content_class: Option<String>,
    /// The MAPI message class (Graph `String 0x001A`, in the detail GET).
    pub message_class: Option<String>,
    /// The top-level MIME type, lowercase, with its `smime-type` and
    /// `protocol` parameters when present (IMAP, Graph's MIME fallback):
    /// "multipart/signed; protocol=application/pkcs7-signature".
    pub mime_type: Option<String>,
    /// What the `.p7m` holds: IMAP's smime-type, or its first bytes.
    pub smime_content: Option<SmimeContent>,
    /// The msip_labels value, unfolded, unparsed.
    pub msip_labels: Option<String>,
    /// Where it came from: Some("header") | Some("property").
    pub msip_labels_source: Option<String>,
    /// Graph's hasAttachments.
    pub has_attachments_flag: Option<bool>,
    /// The x-cumulas-protect and x-cumulas-label values, trimmed: the named
    /// properties first, else the headers.
    pub requested_action: Option<String>,
    pub requested_label: Option<String>,
    /// Graph: the batched named properties couldn't be read this time.
    pub properties_unavailable: bool,
    /// Graph: fetches in a row that couldn't read them (the service counts;
    /// the cache retries while below `MAX_PROPERTY_ATTEMPTS`).
    pub properties_attempts: u8,
    /// Graph: the body and attachments came from the message's MIME.
    pub body_from_mime: bool,
}

/// What a message that asks for protection or a label must carry when it's
/// sent, and what the provider checks on its draft before sending it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderRequest {
    pub protect: Option<ProtectAction>,
    /// Lowercase GUID.
    pub label_id: Option<String>,
    /// The account's tenant: an enabled label of it already on a reply's or
    /// forward's draft must be `label_id`.
    pub tenant_id: String,
    /// The requested label encrypts: no label of the tenant may already be
    /// on the draft.
    pub label_encrypts: bool,
}

#[async_trait]
pub trait MailProvider: Send + Sync {
    /// All folders of the account, with local ids, roles and counts.
    async fn list_folders(&self) -> Result<Vec<Folder>>;

    /// Incrementally syncs one folder. `sync_state` is the cursor returned by
    /// the previous call (or `None` for an initial sync).
    async fn sync_messages(
        &self,
        folder: &Folder,
        sync_state: Option<&str>,
        window: SyncWindow,
    ) -> Result<MessageBatch>;

    /// The folder's current unread and total counts. Targeted syncs ask for
    /// them when messages changed, since they don't list every folder.
    /// `None` when the provider can't tell.
    async fn folder_counts(&self, _folder: &Folder) -> Result<Option<FolderCounts>> {
        Ok(None)
    }

    async fn fetch_message(&self, message: &MessageLocator) -> Result<MessageContent>;

    async fn fetch_attachment(
        &self,
        message: &MessageLocator,
        attachment_id: &str,
    ) -> Result<Vec<u8>>;

    async fn set_read(&self, messages: &[MessageLocator], read: bool) -> Result<()>;

    async fn set_flagged(&self, messages: &[MessageLocator], flagged: bool) -> Result<()>;

    /// Moves messages; returns the new remote id of each message when the
    /// server reports it (in input order), `None` when unknown.
    async fn move_messages(
        &self,
        messages: &[MessageLocator],
        destination_folder_remote_id: &str,
    ) -> Result<Vec<Option<String>>>;

    /// Permanently deletes messages (no move to trash).
    async fn delete_messages(&self, messages: &[MessageLocator]) -> Result<()>;

    /// Sends a message. `original` is the message being replied to or
    /// forwarded, if any, so threading headers can be set. The provider is
    /// responsible for leaving a copy in the Sent folder.
    async fn send(&self, message: &OutgoingMessage, original: Option<&Message>) -> Result<()>;

    /// Searches the mailbox on the server (or the folders the search is
    /// scoped to), at most `limit` messages a page. `cursor` continues a
    /// previous page. What a server can't filter by exactly (read, flagged,
    /// dates in its own zones) it may leave out: the caller checks every
    /// message it returns against the query's filters.
    async fn search(
        &self,
        _search: &ServerQuery<'_>,
        _cursor: Option<&str>,
        _limit: u32,
    ) -> Result<SearchPage> {
        Err(Error::unsupported(
            "This account can't be searched on the server.",
        ))
    }

    /// The address that opens the message in the provider's web app
    /// (Graph's webLink, https only); `None` when there's none.
    async fn web_link(&self, _message: &MessageLocator) -> Result<Option<String>> {
        Ok(None)
    }

    /// The newest Inbox message received since `since` whose subject
    /// contains `[token]` (ASCII case ignored), for each token found.
    /// Microsoft only.
    async fn find_in_inbox(
        &self,
        _tokens: &[String],
        _since: DateTime<Utc>,
    ) -> Result<HashMap<String, MessageLocator>> {
        Err(Error::unsupported(
            "Only Microsoft 365 accounts can look for test messages.",
        ))
    }

    /// The mailbox's own address (Graph `mail`, else `userPrincipalName`).
    async fn own_address(&self) -> Result<Option<EmailAddress>> {
        Ok(None)
    }

    /// Sends a message that asks for protection or a label: a draft, the
    /// x-cumulas-* named properties, a read-back, then send. Nothing is sent
    /// unless the read-back matches `request`.
    async fn send_requesting(
        &self,
        _message: &OutgoingMessage,
        _original: Option<&Message>,
        _request: &HeaderRequest,
    ) -> Result<()> {
        Err(Error::unsupported(
            crate::protection::PROTECTED_SENDING_UNSUPPORTED,
        ))
    }
}

/// The sensitivity labels of a Microsoft 365 work or school account.
#[async_trait]
pub trait LabelsProvider: Send + Sync {
    /// The labels the person may apply to email, and their label policy
    /// (Graph beta). Fails with HTTP 403 when Microsoft 365 refuses it.
    async fn label_catalog(&self) -> Result<LabelCatalog>;
}

/// A Microsoft Graph change subscription, as Graph lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub id: String,
    /// What it watches, such as `me/mailFolders('inbox')/messages`.
    pub resource: String,
    /// Where Graph posts its notifications; absent for some other apps'.
    pub notification_url: Option<String>,
    pub expires_at: DateTime<Utc>,
}

/// A subscription to create. Never with resource data: notifications only
/// say what changed, and the app fetches it.
#[derive(Clone, PartialEq, Eq)]
pub struct NewSubscription {
    pub resource: String,
    /// Such as `created,updated,deleted`.
    pub change_type: String,
    pub notification_url: String,
    pub lifecycle_notification_url: String,
    /// Graph sends it back with every notification; the push service checks
    /// it.
    pub client_state: String,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for NewSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewSubscription")
            .field("resource", &self.resource)
            .field("change_type", &self.change_type)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Change subscriptions (Microsoft Graph webhooks), for push through the
/// Cumulas push service. Errors keep Graph's HTTP status
/// ([`crate::Error::http_status`]); a missing subscription is `NotFound`.
#[async_trait]
pub trait ChangeSubscriptions: Send + Sync {
    /// This app's subscriptions for the signed-in user (every page).
    async fn list_subscriptions(&self) -> Result<Vec<Subscription>>;

    async fn create_subscription(&self, new: &NewSubscription) -> Result<Subscription>;

    /// Renews a subscription until `expires_at`, which also reauthorizes it.
    async fn renew_subscription(&self, id: &str, expires_at: DateTime<Utc>)
    -> Result<Subscription>;

    /// Deletes a subscription; one already gone counts as deleted.
    async fn delete_subscription(&self, id: &str) -> Result<()>;
}

/// A reminder as the server reports it (Microsoft 365's `reminderView`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerReminder {
    /// The event's remote id.
    pub event_id: String,
    /// When the reminder goes off: its event's start less the reminder time,
    /// or later when it was snoozed.
    pub fire_at: DateTime<Utc>,
    pub change_key: Option<String>,
    pub event_start: DateTime<Utc>,
    pub event_end: DateTime<Utc>,
    pub subject: String,
}

#[async_trait]
pub trait CalendarProvider: Send + Sync {
    async fn list_calendars(&self) -> Result<Vec<Calendar>>;

    /// Events overlapping `[start, end)`, with recurring series expanded into
    /// individual occurrences.
    async fn list_events(
        &self,
        calendar: &Calendar,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Event>>;

    async fn create_event(&self, calendar: &Calendar, draft: &EventDraft) -> Result<Event>;

    async fn update_event(
        &self,
        calendar: &Calendar,
        remote_id: &str,
        draft: &EventDraft,
    ) -> Result<Event>;

    async fn delete_event(&self, calendar: &Calendar, remote_id: &str) -> Result<()>;

    async fn respond_to_event(
        &self,
        calendar: &Calendar,
        remote_id: &str,
        response: EventResponse,
        comment: Option<&str>,
        send_response: bool,
    ) -> Result<()>;

    /// The reminders of events between `start` and `end` as the server keeps
    /// them, dismissed and snoozed ones as they are now; `None` when the
    /// server doesn't keep reminders.
    async fn reminders(
        &self,
        _start: DateTime<Utc>,
        _end: DateTime<Utc>,
    ) -> Result<Option<Vec<ServerReminder>>> {
        Ok(None)
    }

    /// Dismisses an event's reminder on the server.
    async fn dismiss_reminder(&self, _remote_id: &str) -> Result<()> {
        Err(crate::error::Error::unsupported(
            "This calendar doesn't keep reminders.",
        ))
    }

    /// Snoozes an event's reminder on the server until `until`.
    async fn snooze_reminder(&self, _remote_id: &str, _until: DateTime<Utc>) -> Result<()> {
        Err(crate::error::Error::unsupported(
            "This calendar doesn't keep reminders.",
        ))
    }
}

#[async_trait]
pub trait ContactsProvider: Send + Sync {
    async fn list_contacts(&self) -> Result<Vec<Contact>>;
    async fn create_contact(&self, draft: &ContactDraft) -> Result<Contact>;
    async fn update_contact(&self, remote_id: &str, draft: &ContactDraft) -> Result<Contact>;
    async fn delete_contact(&self, remote_id: &str) -> Result<()>;
}

#[async_trait]
pub trait TasksProvider: Send + Sync {
    async fn list_task_lists(&self) -> Result<Vec<TaskList>>;
    async fn list_tasks(&self, list: &TaskList) -> Result<Vec<Task>>;
    async fn create_task(&self, list: &TaskList, draft: &TaskDraft) -> Result<Task>;
    async fn update_task(&self, list: &TaskList, current: &Task, patch: &TaskPatch)
    -> Result<Task>;
    async fn delete_task(&self, list: &TaskList, remote_id: &str) -> Result<()>;
}
