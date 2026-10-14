//! Domain model shared by the store, the providers and the UI.
//!
//! Every type here is serialized to the frontend with `camelCase` field names;
//! `src/api/types.ts` mirrors these definitions and must be kept in sync.
//!
//! Identifiers: every entity has a local `id` that is stable and unique across
//! accounts (see [`local_id`]) plus the provider's own `remote_id`. The UI only
//! ever deals with local ids.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Namespace for deterministic local ids (UUIDv5).
const ID_NAMESPACE: Uuid = Uuid::from_u128(0x6f1d_9c2e_4b7a_4e0f_9a51_2c3d_8e7f_1a20);

/// Kind of entity an id refers to; part of the id derivation so that a folder
/// and a message with the same remote id never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Folder,
    Message,
    Calendar,
    Event,
    Contact,
    TaskList,
    Task,
}

impl EntityKind {
    fn as_str(self) -> &'static str {
        match self {
            EntityKind::Folder => "folder",
            EntityKind::Message => "message",
            EntityKind::Calendar => "calendar",
            EntityKind::Event => "event",
            EntityKind::Contact => "contact",
            EntityKind::TaskList => "task-list",
            EntityKind::Task => "task",
        }
    }
}

/// Derives the stable local id for an entity from its account and remote id.
pub fn local_id(account_id: &str, kind: EntityKind, remote_id: &str) -> String {
    let name = format!("{account_id}/{}/{remote_id}", kind.as_str());
    Uuid::new_v5(&ID_NAMESPACE, name.as_bytes()).to_string()
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountKind {
    /// Microsoft 365 work/school account or personal Outlook.com account,
    /// accessed through Microsoft Graph.
    Microsoft,
    /// Any standards-based IMAP + SMTP account. Calendar, contacts and tasks
    /// for these accounts are stored locally.
    Imap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TlsMode {
    /// Implicit TLS (IMAPS 993, SMTPS 465).
    Tls,
    /// Plain connection upgraded with STARTTLS (IMAP 143, submission 587).
    StartTls,
    /// No encryption. Only allowed for `localhost` servers.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapSettings {
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: TlsMode,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: TlsMode,
    /// Login name; usually the email address.
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrosoftSettings {
    /// Directory (tenant) id from the `tid` claim, or `consumers` for
    /// personal Microsoft accounts.
    pub tenant_id: String,
    /// Object id of the signed-in user (`oid` claim).
    pub user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AccountSettings {
    Microsoft(MicrosoftSettings),
    Imap(ImapSettings),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub kind: AccountKind,
    pub display_name: String,
    pub email: String,
    /// Accent color used in the UI (`#rrggbb`).
    pub color: String,
    pub settings: AccountSettings,
    pub created_at: DateTime<Utc>,
    pub last_synced_at: Option<DateTime<Utc>>,
    /// Last sync error, cleared on the next successful sync.
    pub sync_error: Option<String>,
}

/// Input for adding an IMAP/SMTP account. The password is moved straight into
/// the secret store and never persisted in the database.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewImapAccount {
    pub display_name: String,
    pub email: String,
    pub password: String,
    pub settings: ImapSettings,
}

/// Changes to an existing IMAP account, e.g. after a password change. Fields
/// left `None` keep their current value.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapAccountUpdate {
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub settings: Option<ImapSettings>,
}

// ---------------------------------------------------------------------------
// Mail
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: String,
}

impl EmailAddress {
    pub fn new(name: Option<&str>, address: &str) -> Self {
        Self {
            name: name.map(str::to_owned).filter(|n| !n.trim().is_empty()),
            address: address.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderRole {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    Outbox,
    Other,
}

impl FolderRole {
    pub fn as_str(self) -> &'static str {
        match self {
            FolderRole::Inbox => "inbox",
            FolderRole::Drafts => "drafts",
            FolderRole::Sent => "sent",
            FolderRole::Archive => "archive",
            FolderRole::Junk => "junk",
            FolderRole::Trash => "trash",
            FolderRole::Outbox => "outbox",
            FolderRole::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "inbox" => FolderRole::Inbox,
            "drafts" => FolderRole::Drafts,
            "sent" => FolderRole::Sent,
            "archive" => FolderRole::Archive,
            "junk" => FolderRole::Junk,
            "trash" => FolderRole::Trash,
            "outbox" => FolderRole::Outbox,
            _ => FolderRole::Other,
        }
    }

    /// Ordering used when listing folders: well-known folders first.
    pub fn sort_rank(self) -> u8 {
        match self {
            FolderRole::Inbox => 0,
            FolderRole::Drafts => 1,
            FolderRole::Sent => 2,
            FolderRole::Archive => 3,
            FolderRole::Junk => 4,
            FolderRole::Trash => 5,
            FolderRole::Outbox => 6,
            FolderRole::Other => 7,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub role: FolderRole,
    pub unread_count: u32,
    pub total_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Importance {
    Low,
    Normal,
    High,
}

impl Importance {
    pub fn as_str(self) -> &'static str {
        match self {
            Importance::Low => "low",
            Importance::Normal => "normal",
            Importance::High => "high",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "low" => Importance::Low,
            "high" => Importance::High,
            _ => Importance::Normal,
        }
    }
}

/// What the message list needs; cheap to load in bulk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSummary {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub remote_id: String,
    pub conversation_id: Option<String>,
    pub subject: String,
    pub from: Option<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    /// Plain-text snippet of the body (at most ~200 chars).
    pub preview: String,
    pub received_at: DateTime<Utc>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub is_draft: bool,
    pub has_attachments: bool,
    pub importance: Importance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    pub is_inline: bool,
    pub content_id: Option<String>,
}

/// A message with its body, ready for the reading pane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    #[serde(flatten)]
    pub summary: MessageSummary,
    pub bcc: Vec<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    /// Sanitized HTML body (scripts, forms and event handlers removed; remote
    /// resources blocked unless the user allowed them). `None` for plain-text
    /// messages.
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    /// True when the original HTML referenced remote images/styles that were
    /// blocked, so the UI can offer "Show remote content".
    pub has_blocked_content: bool,
    pub attachments: Vec<AttachmentMeta>,
    pub internet_message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageQuery {
    pub offset: u32,
    pub limit: u32,
    #[serde(default)]
    pub unread_only: bool,
    #[serde(default)]
    pub flagged_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    pub items: Vec<MessageSummary>,
    pub total: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingAttachment {
    pub name: String,
    pub content_type: String,
    /// Base64-encoded content.
    pub data_base64: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplyKind {
    Reply,
    ReplyAll,
    Forward,
}

/// A message to send. `reply_to_message_id` (local id) links replies and
/// forwards to their original so threading headers are set correctly.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingMessage {
    pub account_id: String,
    pub to: Vec<EmailAddress>,
    #[serde(default)]
    pub cc: Vec<EmailAddress>,
    #[serde(default)]
    pub bcc: Vec<EmailAddress>,
    pub subject: String,
    pub body_html: String,
    #[serde(default)]
    pub attachments: Vec<OutgoingAttachment>,
    #[serde(default)]
    pub importance: Option<Importance>,
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
    #[serde(default)]
    pub reply_kind: Option<ReplyKind>,
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Calendar {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub name: String,
    /// `#rrggbb`
    pub color: String,
    pub can_edit: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShowAs {
    Free,
    Tentative,
    Busy,
    OutOfOffice,
    WorkingElsewhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResponseStatus {
    None,
    Organizer,
    Accepted,
    Tentative,
    Declined,
    NotResponded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AttendeeKind {
    Required,
    Optional,
    Resource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    pub email: EmailAddress,
    pub kind: AttendeeKind,
    pub response: ResponseStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub account_id: String,
    pub calendar_id: String,
    pub remote_id: String,
    pub subject: String,
    pub location: Option<String>,
    pub body_html: Option<String>,
    pub body_preview: Option<String>,
    /// Start/end are absolute instants. For all-day events they are midnight
    /// UTC of the first day and midnight UTC of the day after the last day.
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub is_all_day: bool,
    pub organizer: Option<EmailAddress>,
    pub attendees: Vec<Attendee>,
    pub show_as: ShowAs,
    /// The signed-in user's response to this meeting.
    pub response_status: ResponseStatus,
    pub is_online_meeting: bool,
    pub online_meeting_url: Option<String>,
    pub reminder_minutes: Option<u32>,
    pub is_recurring: bool,
    pub is_cancelled: bool,
}

/// Fields the user can set when creating or editing an event.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDraft {
    pub calendar_id: String,
    pub subject: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub body_html: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub is_all_day: bool,
    #[serde(default)]
    pub attendees: Vec<EmailAddress>,
    #[serde(default)]
    pub reminder_minutes: Option<u32>,
    pub show_as: ShowAs,
    /// Ask the server to attach a Teams meeting (Microsoft accounts only).
    #[serde(default)]
    pub is_online_meeting: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EventResponse {
    Accept,
    Tentative,
    Decline,
}

// ---------------------------------------------------------------------------
// Contacts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Phone {
    /// `mobile`, `business`, `home` or `other`.
    pub kind: String,
    pub number: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub display_name: String,
    pub given_name: Option<String>,
    pub surname: Option<String>,
    pub emails: Vec<EmailAddress>,
    pub phones: Vec<Phone>,
    pub company: Option<String>,
    pub job_title: Option<String>,
    pub department: Option<String>,
    pub address: Option<String>,
    pub birthday: Option<NaiveDate>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactDraft {
    pub account_id: String,
    pub display_name: String,
    #[serde(default)]
    pub given_name: Option<String>,
    #[serde(default)]
    pub surname: Option<String>,
    #[serde(default)]
    pub emails: Vec<EmailAddress>,
    #[serde(default)]
    pub phones: Vec<Phone>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub job_title: Option<String>,
    #[serde(default)]
    pub department: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub birthday: Option<NaiveDate>,
    #[serde(default)]
    pub notes: Option<String>,
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskList {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    NotStarted,
    InProgress,
    Completed,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::NotStarted => "notStarted",
            TaskStatus::InProgress => "inProgress",
            TaskStatus::Completed => "completed",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "inProgress" => TaskStatus::InProgress,
            "completed" => TaskStatus::Completed,
            _ => TaskStatus::NotStarted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub account_id: String,
    pub list_id: String,
    pub remote_id: String,
    pub title: String,
    pub notes: Option<String>,
    pub status: TaskStatus,
    pub importance: Importance,
    pub due_date: Option<NaiveDate>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDraft {
    pub list_id: String,
    pub title: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub importance: Option<Importance>,
    #[serde(default)]
    pub due_date: Option<NaiveDate>,
}

/// Partial update; `None` leaves a field unchanged. To clear the due date or
/// notes, set `clear_due_date` / `clear_notes`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub clear_notes: bool,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub importance: Option<Importance>,
    #[serde(default)]
    pub due_date: Option<NaiveDate>,
    #[serde(default)]
    pub clear_due_date: bool,
}

// ---------------------------------------------------------------------------
// Sync status & settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncState {
    Idle,
    Syncing,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub account_id: String,
    pub state: SyncState,
    /// Human-readable description of the current phase ("Syncing Inbox…").
    pub message: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
}

/// Which parts of the local store changed; sent to the UI so it can refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataKind {
    Accounts,
    Mail,
    Calendar,
    Contacts,
    Tasks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteContentPolicy {
    /// Block remote images and styles; show a banner to load them.
    Block,
    /// Load remote content only for senders in the user's contacts.
    AllowContacts,
    /// Always load remote content.
    Allow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadingPane {
    Right,
    Bottom,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: Theme,
    pub reading_pane: ReadingPane,
    pub remote_content: RemoteContentPolicy,
    /// Mark a message read after it has been open this many seconds;
    /// `None` = only when explicitly marked.
    pub mark_read_after_seconds: Option<u32>,
    pub sync_interval_minutes: u32,
    /// Overrides the built-in Microsoft Entra application (client) id.
    pub microsoft_client_id: Option<String>,
    /// `common` (default), `organizations`, `consumers` or a tenant id/domain.
    pub microsoft_tenant: Option<String>,
    pub notifications_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            reading_pane: ReadingPane::Right,
            remote_content: RemoteContentPolicy::Block,
            mark_read_after_seconds: Some(0),
            sync_interval_minutes: 5,
            microsoft_client_id: None,
            microsoft_tenant: None,
            notifications_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_ids_are_stable_and_distinct() {
        let a = local_id("acct", EntityKind::Message, "r1");
        assert_eq!(a, local_id("acct", EntityKind::Message, "r1"));
        assert_ne!(a, local_id("acct", EntityKind::Folder, "r1"));
        assert_ne!(a, local_id("other", EntityKind::Message, "r1"));
    }

    #[test]
    fn account_settings_serialise_with_kind_tag() {
        let s = AccountSettings::Microsoft(MicrosoftSettings {
            tenant_id: "t".into(),
            user_id: "u".into(),
        });
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["kind"], "microsoft");
        assert_eq!(json["tenantId"], "t");
    }

    #[test]
    fn settings_fill_missing_fields_with_defaults() {
        let s: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.sync_interval_minutes, 5);
    }
}
