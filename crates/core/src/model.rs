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
    /// When to remind about the task; `None` for no reminder.
    #[serde(default)]
    pub reminder_at: Option<DateTime<Utc>>,
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
    #[serde(default)]
    pub reminder_at: Option<DateTime<Utc>>,
}

/// Partial update; `None` leaves a field unchanged. To clear the due date,
/// notes or reminder, set `clear_due_date` / `clear_notes` /
/// `clear_reminder`.
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
    #[serde(default)]
    pub reminder_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub clear_reminder: bool,
}

// ---------------------------------------------------------------------------
// Signatures
// ---------------------------------------------------------------------------

/// Whether the account's organization signs its mail with Cumulas
/// Signatures, as of the last check (see [`crate::signatures`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrganizationSignatureState {
    /// IMAP, a personal Microsoft account, an account without a tenant, or
    /// an app built without the signature service's client ID.
    Unsupported,
    /// Supported, but not checked yet.
    Unchecked,
    /// The organization doesn't use Cumulas Signatures (Microsoft refused
    /// the token: its tenant hasn't added the service's app), or the service
    /// has no signature for this person right now. Asked again once a day.
    NotUsed,
    /// The organization's signature, which the app adds.
    Ready,
    /// The organization's mail server adds the signature on send (to
    /// replies and forwards only with `on_replies`); the app adds none.
    AddedOnSend,
    /// The organization has Cumulas Signatures' app, but Cumulas Mail isn't
    /// allowed: by the organization (Microsoft refused for consent) or by
    /// the service's operator (`app-not-allowed`). Asked again once a day.
    NotAllowed,
}

impl OrganizationSignatureState {
    /// Whether the organization signs the account's mail, so that the
    /// person's own signature isn't used.
    pub fn is_managed(self) -> bool {
        matches!(self, Self::Ready | Self::AddedOnSend)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationSignature {
    pub state: OrganizationSignatureState,
    /// Sanitized HTML; only while `Ready`.
    pub html: Option<String>,
    /// The organization's signature also goes on replies and forwards,
    /// added by the app (`Ready`) or by its mail server (`AddedOnSend`);
    /// false in other states.
    pub on_replies: bool,
    /// When the organization last answered (whatever it said).
    pub checked_at: Option<DateTime<Utc>>,
    /// The last check failed (no network, the service unavailable): `state`
    /// is what was known before.
    pub stale: bool,
}

/// An account's signatures: the person's own and the organization's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSignature {
    pub account_id: String,
    /// Sanitized HTML; empty for none.
    pub own_html: String,
    /// The own signature also goes on replies and forwards.
    pub own_on_replies: bool,
    pub organization: OrganizationSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComposeSignatureKind {
    None,
    Organization,
    Own,
    /// The organization's mail server signs the message on send: nothing
    /// to insert. A reply or forward it leaves unsigned is `None`.
    AddedOnSend,
}

/// The signature a new message, reply or forward starts with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeSignature {
    pub kind: ComposeSignatureKind,
    /// Sanitized HTML for `Organization` and `Own`.
    pub html: Option<String>,
}

impl ComposeSignature {
    pub fn none() -> Self {
        Self {
            kind: ComposeSignatureKind::None,
            html: None,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReminderKind {
    Event,
    Task,
}

impl ReminderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ReminderKind::Event => "event",
            ReminderKind::Task => "task",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "event" => Some(ReminderKind::Event),
            "task" => Some(ReminderKind::Task),
            _ => None,
        }
    }
}

/// A reminder that is due or snoozed. Produced by the reminder scheduler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveReminder {
    /// `{event|task}:{local id}:{own reminder time}`: the same whenever it
    /// goes off (snoozed, or as the server says); a moved event, or a
    /// changed reminder, gets a new key.
    pub key: String,
    pub kind: ReminderKind,
    pub item_id: String,
    pub account_id: String,
    pub title: String,
    /// When the event starts (an all-day event: local midnight of its first
    /// day); `None` for tasks.
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub online_meeting_url: Option<String>,
    /// When it goes off, snoozes aside.
    pub fire_at: DateTime<Utc>,
    /// Snoozed until then: not due again yet.
    pub snoozed_until: Option<DateTime<Utc>>,
    /// The event has started or the task is past its reminder time.
    pub overdue: bool,
    /// The event was over before its reminder could be shown: it's listed,
    /// but it never showed as a notification.
    #[serde(default)]
    pub missed: bool,
}

impl ActiveReminder {
    /// Due now, rather than snoozed.
    pub fn is_due(&self) -> bool {
        self.snoozed_until.is_none()
    }
}

/// How long to snooze a reminder for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snooze {
    Minutes(u32),
    Until(DateTime<Utc>),
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
    /// An account's signatures (its own, or the organization's).
    Signatures,
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

/// The typeface for the interface: menus, lists, buttons and labels.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum InterfaceFont {
    /// Inter, which ships with the app.
    #[default]
    Default,
    /// The desktop's interface font (CSS `system-ui`).
    System,
    /// An installed font family, by name.
    Family { family: String },
}

impl InterfaceFont {
    pub fn family(&self) -> Option<&str> {
        match self {
            Self::Family { family } => Some(family),
            Self::Default | Self::System => None,
        }
    }
}

/// The typeface for what people read: subjects, pane titles, the open
/// message and the compose editor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ReadingFont {
    /// Charis SIL, which ships with the app.
    #[default]
    Default,
    /// The same typeface as the interface.
    Interface,
    /// An installed font family, by name.
    Family { family: String },
}

impl ReadingFont {
    pub fn family(&self) -> Option<&str> {
        match self {
            Self::Family { family } => Some(family),
            Self::Default | Self::Interface => None,
        }
    }
}

/// Scales the whole window, text and controls alike.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextSize {
    Smaller,
    #[default]
    Default,
    Larger,
    Largest,
}

impl TextSize {
    /// The webview zoom factor. `TEXT_SCALE` in `src/lib/fonts.ts` must match.
    pub fn zoom(self) -> f64 {
        match self {
            Self::Smaller => 0.92,
            Self::Default => 1.0,
            Self::Larger => 1.1,
            Self::Largest => 1.2,
        }
    }
}

/// The longest font family name accepted, in characters. Real names stay
/// well under 100.
pub const FONT_FAMILY_MAX_CHARS: usize = 200;

/// Whether `name` can be stored and used as a font family name: not blank,
/// no surrounding spaces, not absurdly long, and no control characters,
/// line separators or bidirectional controls. The UI also quotes and
/// escapes the name wherever it goes into CSS.
pub fn is_valid_font_family(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= FONT_FAMILY_MAX_CHARS
        && !name.chars().any(is_disallowed_in_font_family)
}

fn is_disallowed_in_font_family(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            // Line and paragraph separators.
            '\u{2028}' | '\u{2029}'
            // Bidirectional marks, embeddings, overrides and isolates: they
            // would reorder the text around the name in the UI.
            | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        )
}

/// Which folders raise new-mail notifications.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotifyScope {
    /// Only the Inbox.
    #[default]
    Inbox,
    /// Every folder except Junk, Deleted Items, Sent, Drafts and Outbox.
    AllFolders,
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
    /// How often everything is synced: every folder, the calendar, contacts
    /// and tasks. New mail in the Inbox arrives sooner through push.
    pub sync_interval_minutes: u32,
    /// Overrides the built-in Microsoft Entra application (client) id.
    pub microsoft_client_id: Option<String>,
    /// `common` (default), `organizations`, `consumers` or a tenant id/domain.
    pub microsoft_tenant: Option<String>,
    pub notifications_enabled: bool,
    pub interface_font: InterfaceFont,
    pub reading_font: ReadingFont,
    pub text_size: TextSize,
    pub notify_scope: NotifyScope,
    /// Show the sender and subject in notifications.
    pub notify_show_preview: bool,
    pub notify_sound: bool,
    /// Accounts whose new mail never raises a notification.
    pub muted_account_ids: Vec<String>,
    /// No new-mail notifications until then ("Pause mail notifications");
    /// reminders still show.
    pub notifications_paused_until: Option<DateTime<Utc>>,
    /// Event and task reminders.
    pub reminders_enabled: bool,
    pub default_snooze_minutes: u32,
    /// Keep a connection open (IMAP IDLE) or listen to the push service so
    /// new mail shows up within seconds.
    pub instant_push: bool,
    /// Microsoft accounts get changes through the Cumulas push service;
    /// when off they check the Inbox about every 30 seconds instead.
    pub use_push_relay: bool,
    /// Closing the window keeps the app running for notifications.
    pub run_in_background: bool,
    pub start_at_login: bool,
    /// Started at login, the app opens without a window.
    pub start_hidden: bool,
    pub show_tray_icon: bool,
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
            interface_font: InterfaceFont::default(),
            reading_font: ReadingFont::default(),
            text_size: TextSize::default(),
            notify_scope: NotifyScope::default(),
            notify_show_preview: true,
            notify_sound: true,
            muted_account_ids: Vec::new(),
            notifications_paused_until: None,
            reminders_enabled: true,
            default_snooze_minutes: 5,
            instant_push: true,
            use_push_relay: true,
            run_in_background: true,
            start_at_login: false,
            start_hidden: true,
            show_tray_icon: true,
        }
    }
}

impl Settings {
    /// Reads settings saved by this or any other version. Missing fields take
    /// their defaults (settings saved before a field existed), and a field
    /// whose stored value can't be read (an option from a newer version, a
    /// font name that fails validation) falls back to its own default instead
    /// of resetting every setting. Unknown fields are dropped.
    pub fn from_stored_json(json: &str) -> Settings {
        use serde_json::Value;

        let Ok(Value::Object(stored)) = serde_json::from_str::<Value>(json) else {
            return Settings::default();
        };
        let Ok(Value::Object(mut merged)) = serde_json::to_value(Settings::default()) else {
            return Settings::default();
        };
        for (key, value) in stored {
            let Some(default) = merged.get(&key).cloned() else {
                continue;
            };
            merged.insert(key.clone(), value);
            if serde_json::from_value::<Settings>(Value::Object(merged.clone())).is_err() {
                merged.insert(key, default);
            }
        }
        serde_json::from_value::<Settings>(Value::Object(merged))
            .unwrap_or_default()
            .with_valid_fonts()
    }

    /// Replaces font families that fail [`is_valid_font_family`] with the
    /// defaults.
    fn with_valid_fonts(mut self) -> Self {
        if self
            .interface_font
            .family()
            .is_some_and(|f| !is_valid_font_family(f))
        {
            self.interface_font = InterfaceFont::default();
        }
        if self
            .reading_font
            .family()
            .is_some_and(|f| !is_valid_font_family(f))
        {
            self.reading_font = ReadingFont::default();
        }
        self
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

    #[test]
    fn appearance_defaults_to_the_bundled_fonts_at_normal_size() {
        let s = Settings::default();
        assert_eq!(s.interface_font, InterfaceFont::Default);
        assert_eq!(s.reading_font, ReadingFont::Default);
        assert_eq!(s.text_size, TextSize::Default);
        assert_eq!(s.text_size.zoom(), 1.0);

        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json["interfaceFont"],
            serde_json::json!({ "kind": "default" })
        );
        assert_eq!(
            json["readingFont"],
            serde_json::json!({ "kind": "default" })
        );
        assert_eq!(json["textSize"], "default");
    }

    #[test]
    fn font_choices_serialize_with_a_kind_tag() {
        let family = InterfaceFont::Family {
            family: "Fira Sans".into(),
        };
        assert_eq!(
            serde_json::to_value(&family).unwrap(),
            serde_json::json!({ "kind": "family", "family": "Fira Sans" })
        );
        assert_eq!(
            serde_json::to_value(InterfaceFont::System).unwrap(),
            serde_json::json!({ "kind": "system" })
        );
        assert_eq!(
            serde_json::to_value(ReadingFont::Interface).unwrap(),
            serde_json::json!({ "kind": "interface" })
        );
        let parsed: ReadingFont =
            serde_json::from_str(r#"{"kind":"family","family":"Literata"}"#).unwrap();
        assert_eq!(parsed.family(), Some("Literata"));
        assert_eq!(ReadingFont::Default.family(), None);
    }

    #[test]
    fn text_sizes_grow_in_order() {
        let zooms: Vec<f64> = [
            TextSize::Smaller,
            TextSize::Default,
            TextSize::Larger,
            TextSize::Largest,
        ]
        .map(TextSize::zoom)
        .to_vec();
        assert_eq!(zooms, vec![0.92, 1.0, 1.1, 1.2]);
        let parsed: TextSize = serde_json::from_str(r#""largest""#).unwrap();
        assert_eq!(parsed, TextSize::Largest);
    }

    #[test]
    fn font_family_names_are_validated() {
        for ok in [
            "Inter",
            "Noto Sans CJK JP",
            "文泉驿正黑",
            "Source Serif 4",
            "O'Reilly \"Mono\"",
            r"Back\slash",
            "</style><b>x",
            // U+FEFF isn't White_Space: a font named with one is matched
            // with it (src/lib/fonts.test.ts keeps it too).
            "\u{FEFF}Foo",
        ] {
            assert!(is_valid_font_family(ok), "{ok:?} should be valid");
        }
        let long = "a".repeat(FONT_FAMILY_MAX_CHARS + 1);
        for bad in [
            "",
            " ",
            " Inter",
            "Inter ",
            "\u{A0}Inter\u{3000}",
            "Inter\nBold",
            "Tab\there",
            "Nul\0",
            "Del\u{7f}",
            "C1\u{85}",
            "Line\u{2028}sep",
            "Over\u{202E}ride",
            "Iso\u{2066}late",
            long.as_str(),
        ] {
            assert!(!is_valid_font_family(bad), "{bad:?} should be invalid");
        }
        assert!(is_valid_font_family(&"a".repeat(FONT_FAMILY_MAX_CHARS)));
    }

    #[test]
    fn stored_settings_from_before_fonts_keep_their_values() {
        let old = r#"{"theme":"dark","readingPane":"bottom","remoteContent":"allow",
            "markReadAfterSeconds":null,"syncIntervalMinutes":15,"microsoftClientId":null,
            "microsoftTenant":"organizations","notificationsEnabled":false}"#;
        let s = Settings::from_stored_json(old);
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.reading_pane, ReadingPane::Bottom);
        assert_eq!(s.remote_content, RemoteContentPolicy::Allow);
        assert_eq!(s.mark_read_after_seconds, None);
        assert_eq!(s.sync_interval_minutes, 15);
        assert_eq!(s.microsoft_tenant.as_deref(), Some("organizations"));
        assert!(!s.notifications_enabled);
        assert_eq!(s.interface_font, InterfaceFont::Default);
        assert_eq!(s.reading_font, ReadingFont::Default);
        assert_eq!(s.text_size, TextSize::Default);
    }

    #[test]
    fn an_unreadable_stored_field_falls_back_on_its_own() {
        let s = Settings::from_stored_json(
            r#"{"theme":"dark","textSize":"enormous","readingFont":{"kind":"handwriting"},
                "interfaceFont":{"kind":"family","family":"Fira Sans"},"futureField":1}"#,
        );
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.text_size, TextSize::Default);
        assert_eq!(s.reading_font, ReadingFont::Default);
        assert_eq!(s.interface_font.family(), Some("Fira Sans"));
    }

    #[test]
    fn invalid_stored_font_names_reset_to_the_defaults() {
        let s = Settings::from_stored_json(
            r#"{"theme":"light","interfaceFont":{"kind":"family","family":"Bad\u0007Name"},
                "readingFont":{"kind":"family","family":""},"textSize":"larger"}"#,
        );
        assert_eq!(s.theme, Theme::Light);
        assert_eq!(s.interface_font, InterfaceFont::Default);
        assert_eq!(s.reading_font, ReadingFont::Default);
        assert_eq!(s.text_size, TextSize::Larger);
    }

    #[test]
    fn notification_push_and_background_defaults() {
        let s = Settings::default();
        assert_eq!(s.notify_scope, NotifyScope::Inbox);
        assert!(s.notify_show_preview && s.notify_sound && s.reminders_enabled);
        assert!(s.muted_account_ids.is_empty());
        assert_eq!(s.notifications_paused_until, None);
        assert_eq!(s.default_snooze_minutes, 5);
        assert!(s.instant_push && s.use_push_relay);
        assert!(s.run_in_background && s.start_hidden && s.show_tray_icon);
        assert!(!s.start_at_login);

        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["notifyScope"], "inbox");
        assert_eq!(json["mutedAccountIds"], serde_json::json!([]));
        assert_eq!(json["notificationsPausedUntil"], serde_json::Value::Null);
        assert_eq!(json["usePushRelay"], true);
        assert_eq!(
            serde_json::to_value(NotifyScope::AllFolders).unwrap(),
            "allFolders"
        );
    }

    #[test]
    fn stored_settings_from_before_push_get_the_new_defaults() {
        let s = Settings::from_stored_json(
            r#"{"theme":"dark","notificationsEnabled":false,"notifyScope":"everywhere",
                "mutedAccountIds":["a"],"instantPush":false}"#,
        );
        assert_eq!(s.theme, Theme::Dark);
        assert!(!s.notifications_enabled);
        assert_eq!(s.notify_scope, NotifyScope::Inbox);
        assert_eq!(s.muted_account_ids, ["a"]);
        assert!(!s.instant_push);
        assert!(s.run_in_background);
    }

    #[test]
    fn unreadable_stored_settings_are_the_defaults() {
        assert_eq!(Settings::from_stored_json("not json"), Settings::default());
        assert_eq!(Settings::from_stored_json("[1, 2]"), Settings::default());
        assert_eq!(Settings::from_stored_json("{}"), Settings::default());
    }

    /// The shape `src/api/types.ts` mirrors.
    #[test]
    fn signatures_serialize_as_the_frontend_reads_them() {
        let signature = AccountSignature {
            account_id: "a".into(),
            own_html: "<p>Ada</p>".into(),
            own_on_replies: true,
            organization: OrganizationSignature {
                state: OrganizationSignatureState::AddedOnSend,
                html: None,
                on_replies: false,
                checked_at: None,
                stale: true,
            },
        };
        assert_eq!(
            serde_json::to_value(&signature).unwrap(),
            serde_json::json!({
                "accountId": "a",
                "ownHtml": "<p>Ada</p>",
                "ownOnReplies": true,
                "organization": {
                    "state": "addedOnSend",
                    "html": null,
                    "onReplies": false,
                    "checkedAt": null,
                    "stale": true,
                },
            })
        );
        let states = [
            (OrganizationSignatureState::Unsupported, "unsupported"),
            (OrganizationSignatureState::Unchecked, "unchecked"),
            (OrganizationSignatureState::NotUsed, "notUsed"),
            (OrganizationSignatureState::Ready, "ready"),
            (OrganizationSignatureState::AddedOnSend, "addedOnSend"),
            (OrganizationSignatureState::NotAllowed, "notAllowed"),
        ];
        for (state, name) in states {
            assert_eq!(serde_json::to_value(state).unwrap(), name);
        }
        assert_eq!(
            serde_json::to_value(ComposeSignature {
                kind: ComposeSignatureKind::Organization,
                html: Some("<p>x</p>".into()),
            })
            .unwrap(),
            serde_json::json!({ "kind": "organization", "html": "<p>x</p>" })
        );
        let kinds = [
            (ComposeSignatureKind::None, "none"),
            (ComposeSignatureKind::Own, "own"),
            (ComposeSignatureKind::AddedOnSend, "addedOnSend"),
        ];
        for (kind, name) in kinds {
            assert_eq!(serde_json::to_value(kind).unwrap(), name);
        }
        assert_eq!(
            serde_json::to_value(DataKind::Signatures).unwrap(),
            "signatures"
        );
        assert_eq!(
            serde_json::from_str::<ReplyKind>(r#""replyAll""#).unwrap(),
            ReplyKind::ReplyAll
        );
    }
}
