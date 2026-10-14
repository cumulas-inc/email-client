//! Protocol providers. Each account kind supplies implementations of these
//! traits; the sync engine and [`crate::service::CoreService`] only talk to the
//! traits, which keeps them testable with fakes.
//!
//! Providers return fully-formed model types with local ids already derived via
//! [`crate::model::local_id`] from the account id and the remote id.
//!
//! Microsoft accounts implement all four traits on top of Microsoft Graph.
//! IMAP accounts implement [`MailProvider`] only; their calendar, contacts and
//! tasks live in the local store.

pub mod graph;
pub mod imap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::model::{
    AttachmentMeta, Calendar, Contact, ContactDraft, EmailAddress, Event, EventDraft,
    EventResponse, Folder, Message, MessageSummary, OutgoingMessage, Task, TaskDraft, TaskList,
    TaskPatch,
};

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
    /// When set, the complete list of remote ids currently in the folder: any
    /// stored message of the folder not listed is removed. Lets providers that
    /// cannot report deletions (IMAP without QRESYNC) reconcile cheaply.
    pub retain_only: Option<Vec<String>>,
    /// When true, `upserts` is the complete content of the folder within the
    /// sync window and anything else stored locally for it must be dropped
    /// (e.g. IMAP UIDVALIDITY changed or a Graph delta token expired).
    pub full_resync: bool,
    /// Opaque provider cursor to pass to the next sync of this folder
    /// (Graph `@odata.deltaLink`, IMAP `uidvalidity:uidnext:modseq`, ...).
    pub sync_state: Option<String>,
}

/// Body and full headers of a message, as fetched from the server.
#[derive(Debug, Clone, Default)]
pub struct MessageContent {
    /// Raw (unsanitised) HTML body. `cid:` references to inline images should
    /// already be rewritten to `data:` URIs by the provider where possible.
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub bcc: Vec<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub attachments: Vec<AttachmentMeta>,
    pub internet_message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
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
