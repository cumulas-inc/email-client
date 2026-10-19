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
    /// Microsoft accounts: optional features whose permission Microsoft
    /// hasn't granted, in enum order. Empty for IMAP accounts and for
    /// accounts whose grant isn't recorded yet.
    #[serde(default)]
    pub needs_permission: Vec<AccountFeature>,
    /// Microsoft accounts: optional features that have their permission but
    /// that Microsoft 365 refused (never `mailboxSettings`), in enum order.
    /// Empty for IMAP.
    #[serde(default)]
    pub refused_features: Vec<AccountFeature>,
}

/// An optional part of a Microsoft account that needs a permission of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountFeature {
    Calendar,
    Contacts,
    Tasks,
    MailboxSettings,
    SensitivityLabels,
}

impl AccountFeature {
    /// Every feature, in display order.
    pub const ALL: [AccountFeature; 5] = [
        AccountFeature::Calendar,
        AccountFeature::Contacts,
        AccountFeature::Tasks,
        AccountFeature::MailboxSettings,
        AccountFeature::SensitivityLabels,
    ];

    /// What the feature is called in messages: "the {noun} of {email}".
    pub fn noun(self) -> &'static str {
        match self {
            AccountFeature::Calendar => "calendar",
            AccountFeature::Contacts => "contacts",
            AccountFeature::Tasks => "tasks",
            AccountFeature::MailboxSettings => "mailbox time zone",
            AccountFeature::SensitivityLabels => "sensitivity labels",
        }
    }

    /// Asked for only when turned on by itself ("Turn on"), never with the
    /// others at the first sign-in or by "Sign in again" without a record:
    /// personal accounts can't have it, and an organization that approved
    /// the app before it existed would have to approve again.
    pub fn on_request_only(self) -> bool {
        matches!(self, Self::SensitivityLabels)
    }

    /// Whether a personal Microsoft account can have it.
    pub fn for_personal_accounts(self) -> bool {
        !matches!(self, Self::SensitivityLabels)
    }
}

/// What a Microsoft sign-in asks permission for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PermissionRequest {
    /// Mail and every optional feature: adding an account.
    #[default]
    All,
    /// Mail only ("Try with mail only").
    MailOnly,
    /// What the account has now; everything for an account without a
    /// record ("Sign in again").
    Current,
    /// What the account has now, plus one more feature ("Turn on").
    Feature { feature: AccountFeature },
}

/// What a Microsoft account was granted, as stored in
/// `accounts.microsoft_grant`. Never sent to the UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MicrosoftGrant {
    /// Graph permissions Microsoft last said the account has, unqualified
    /// (`Mail.ReadWrite`), never the OpenID Connect scopes.
    pub scopes: Vec<String>,
    /// Features Microsoft Graph refused (403 on the listing) although their
    /// permission was granted. Ones a newer version wrote that this one
    /// doesn't know are dropped: they never cost the grant its `scopes`.
    #[serde(deserialize_with = "known_features")]
    pub refused: Vec<AccountFeature>,
    /// When Microsoft 365 last refused a feature, the first time or when
    /// it was tried again.
    pub refused_at: Option<DateTime<Utc>>,
}

/// Reads a list of features, leaving out the names this version doesn't know.
fn known_features<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<AccountFeature>, D::Error> {
    let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .filter_map(|feature| serde_json::from_value(feature).ok())
        .collect())
}

/// Where an administrator approves Cumulas Mail for an organization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminConsentLink {
    pub admin_consent_url: String,
    /// The organization: a tenant id or domain, or `organizations`.
    pub tenant: String,
    /// The page for IT administrators; only for the published app
    /// registration.
    pub admin_page_url: Option<String>,
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
    /// How the message is protected; None for ordinary mail.
    #[serde(default)]
    pub protection: Option<Protection>,
    /// Its sensitivity label from the account's own organization (Microsoft
    /// work and school accounts); None without one. See
    /// [`crate::protection::own_label`].
    #[serde(default)]
    pub label: Option<MessageLabel>,
    /// A copy in a Sent folder of mail Cumulas Mail sent: what it asked for.
    #[serde(default)]
    pub requested: Option<RequestedProtection>,
}

// --- Protected mail -----------------------------------------------------------

/// How a message is protected, as far as the app can tell (see
/// [`crate::protection`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProtectionKind {
    /// Microsoft Purview encryption (a label that encrypts, Encrypt, Do Not
    /// Forward): the content is in `message.rpmsg`, which the app can't open.
    RightsManaged,
    /// S/MIME encryption (`smime.p7m`), or S/MIME content the app couldn't
    /// classify (its `.p7m` couldn't be read).
    SmimeEncrypted,
    /// Signed with S/MIME; the signature isn't checked.
    SmimeSigned,
    /// Purview Message Encryption's "Read the message" form: a link to
    /// Microsoft's encrypted message portal.
    PortalLink,
}

impl ProtectionKind {
    /// Its content is encrypted: replies and forwards go through Outlook on
    /// the web.
    pub fn is_encrypted(self) -> bool {
        matches!(self, Self::RightsManaged | Self::SmimeEncrypted)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Protection {
    pub kind: ProtectionKind,
    /// Attachments that only hold the protected content or the signature
    /// (message.rpmsg, smime.p7m, smime.p7s): explained, never offered.
    pub container_ids: Vec<String>,
    /// The first link in the message to Microsoft's encrypted message portal,
    /// as written (a Safe Links wrapper kept); None without one, and always
    /// None in Sent and Drafts.
    pub portal_url: Option<String>,
    /// Signed: some of the content (the attachments of a clear-signed message
    /// on Microsoft 365, or all of an opaque-signed one) is inside a
    /// container the app couldn't unpack.
    pub content_hidden: bool,
}

/// A sensitivity label on a message (its organization's), or one asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageLabel {
    /// The label's GUID, lowercase.
    pub id: String,
    /// "Parent › Child" from the account's labels; None without them. Never
    /// the name a message gives, which any sender can write.
    pub name: Option<String>,
    /// `#rrggbb` from the account's labels; None when unknown.
    pub color: Option<String>,
    /// The label is one of the account's labels (Microsoft 365's list),
    /// named and colored from it. Without that list, a message's label is
    /// only one it names for the organization: unverified.
    pub from_catalog: bool,
}

/// Encryption the organization's mail server can be asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProtectAction {
    Encrypt,
    DoNotForward,
}

impl ProtectAction {
    /// The x-cumulas-protect value.
    pub fn header_value(self) -> &'static str {
        match self {
            Self::Encrypt => "encrypt",
            Self::DoNotForward => "do-not-forward",
        }
    }

    /// The action an x-cumulas-protect value asks for (trimmed, ASCII case
    /// ignored).
    pub fn from_header_value(value: &str) -> Option<Self> {
        let value = value.trim();
        [Self::Encrypt, Self::DoNotForward]
            .into_iter()
            .find(|a| a.header_value().eq_ignore_ascii_case(value))
    }

    /// Microsoft's name for it, in messages and errors.
    pub fn name(self) -> &'static str {
        match self {
            Self::Encrypt => "Encrypt",
            Self::DoNotForward => "Do Not Forward",
        }
    }
}

/// What a message sent from Cumulas Mail asked its organization's mail
/// server for (its Sent Items copy's x-cumulas-* properties or headers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestedProtection {
    pub action: Option<ProtectAction>,
    pub label: Option<MessageLabel>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerSearchStatus {
    /// The server answered (perhaps with nothing).
    Done,
    /// A newer search replaced this one, or it was canceled; nothing was
    /// stored.
    Cancelled,
    /// The server couldn't be asked, or failed; `error` says why.
    Failed,
}

/// What one call of a search of an account's whole mailbox on its server
/// found (one or more pages). (No `PartialEq`: [`crate::error::Error`] has
/// none.)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSearchResult {
    pub search_id: String,
    pub account_id: String,
    pub status: ServerSearchStatus,
    /// Local ids of the messages found on the pages this call read that
    /// pass the query's filters, newest first; all stored now.
    pub message_ids: Vec<String>,
    /// How many of them weren't stored on this device before.
    pub added: u32,
    /// Another page can be asked for (`more`).
    pub has_more: bool,
    /// Why it failed (status `failed`), else null.
    pub error: Option<crate::error::Error>,
}

/// How search results are ordered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchSort {
    /// Best matches first: words in the subject and sender count most, and recent mail leads.
    #[default]
    Relevance,
    /// Newest first.
    Newest,
}

/// How to search, besides what for. By default best matches first, and
/// weeks from Sunday.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchOptions {
    pub sort: SearchSort,
    /// First day of the week, for `received:"this week"` and `"last week"`:
    /// 0 is Sunday … 6 Saturday (as `Date.getDay`); others count modulo 7.
    pub week_starts_on: u8,
}

/// A piece of text, and whether a search matched it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    pub text: String,
    pub hit: bool,
}

/// A message a search found.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    #[serde(flatten)]
    pub summary: MessageSummary,
    /// The body's words around what matched there (`…` where cut), when
    /// the body matched: shown in place of the preview. Otherwise null.
    pub snippet: Option<Vec<TextRun>>,
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
    /// Encryption to ask the organization's mail server for
    /// (x-cumulas-protect); refused unless the account's check found it
    /// works (and not expired).
    #[serde(default)]
    pub protect: Option<ProtectAction>,
    /// The sensitivity label to ask for (x-cumulas-label), a GUID; refused
    /// unless the account's check found it works (and not expired).
    #[serde(default)]
    pub label_id: Option<String>,
}

// --- Protection setup ---------------------------------------------------------

/// One thing a check sends a test message for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CheckItem {
    Action { action: ProtectAction },
    Label { label_id: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckState {
    /// Never checked.
    #[default]
    NotChecked,
    /// The test message went out; waiting for it (and the control) to arrive.
    /// Shown while a check runs, never stored: the last decided check still
    /// counts until then (`CheckResult::passed_at`).
    Waiting,
    /// It arrived encrypted or labeled, and the control didn't: offered.
    Applied,
    /// Applied, but more than `CHECK_VALID_DAYS` ago: not offered until
    /// checked again. Never stored; computed when the setup is read.
    Expired,
    /// It arrived without it (as far as the app can tell), or Microsoft 365
    /// didn't keep the property on its draft (note `HeaderNotKept`).
    NotApplied,
    /// The app can't tell: see the note.
    Unknown,
}

/// Why a check ended as it did, when the state alone doesn't say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckNote {
    /// The control arrived encrypted (or with the same label) too: another
    /// rule in the organization does it. (With `Unknown`.)
    OtherRule,
    /// It arrived encrypted, but x-cumulas-protect wasn't removed, so the
    /// rule that encrypted it may not be Cumulas Mail's. (With `Unknown`.)
    HeaderKept,
    /// It, or the control, didn't arrive within `CHECK_TIMEOUT_MINUTES`.
    /// (With `Unknown`; or with `Applied`, when an earlier pass still
    /// counts.)
    NotArrived,
    /// Microsoft 365 didn't keep the property on the test message's draft,
    /// so it wasn't sent. (With `NotApplied`.)
    HeaderNotKept,
    /// The request reached delivery, but nothing acted on it. (With
    /// `NotApplied`.)
    RequestIgnored,
    /// The request was gone when it arrived, and nothing was applied.
    /// (With `NotApplied`.)
    RequestRemoved,
    /// It went through the Cumulas Signatures relay and came back as mail
    /// from outside the organization. (With `NotApplied`.)
    RelayNotInternal,
    /// The organization's setup changed on this computer too recently to
    /// tell (`CheckResult::changed_at`). (With `NotApplied`.)
    TooSoon,
    /// Another label of the organization arrived instead
    /// (`CheckResult::other_label_id`). (With `NotApplied`.)
    OtherLabel,
    /// Encrypt or Do Not Forward: it arrived protected another way (S/MIME,
    /// a link to Microsoft's encrypted message portal). (With `Unknown`.)
    OtherProtection,
    /// A label's test message arrived encrypted, but no label of the
    /// organization could be read on it. (With `Unknown`.)
    LabelUnreadable,
    /// It passed before the account's mail went through the Cumulas
    /// Signatures relay. (With `Expired`: worked out when the setup is
    /// read, never stored.)
    RelayChanged,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CheckResult {
    pub state: CheckState,
    /// When it was decided (or sent, while waiting); None if never checked.
    pub at: Option<DateTime<Utc>>,
    pub note: Option<CheckNote>,
    /// The test message arrived and can be opened in Outlook on the web.
    pub test_message: bool,
    /// When the test message that lets the option be offered arrived: set
    /// while that pass is less than `CHECK_VALID_DAYS` old, a new check
    /// waiting or not; None when it isn't offered.
    pub passed_at: Option<DateTime<Utc>>,
    /// With note `OtherLabel`: the organization's label that arrived
    /// instead (lowercase GUID).
    pub other_label_id: Option<String>,
    /// With note `OtherLabel`: the control arrived with that label too.
    pub other_label_on_control: bool,
    /// A setup change from this computer recent enough to matter when this
    /// was judged; None when there was none.
    pub changed_at: Option<DateTime<Utc>>,
}

/// A sensitivity label Microsoft 365 lists for the person, for email.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogLabel {
    /// GUID, lowercase.
    pub id: String,
    /// Graph's displayName, else its name.
    pub name: String,
    pub parent_name: Option<String>,
    pub tooltip: Option<String>,
    /// `#rrggbb` only.
    pub color: Option<String>,
    /// It encrypts (Graph's hasProtection).
    pub has_protection: bool,
    /// Higher is more sensitive; None when Microsoft 365 doesn't say.
    pub sensitivity: Option<i32>,
}

impl CatalogLabel {
    /// "Parent › Child", or the name.
    pub fn title(&self) -> String {
        match &self.parent_name {
            Some(parent) => format!("{parent} › {}", self.name),
            None => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LabelPolicy {
    /// A label is required (isMandatory).
    pub required: bool,
    pub default_label_id: Option<String>,
    /// Lowering a label needs a reason (isDowngradeJustificationRequired).
    pub justify_downgrade: bool,
    /// https only.
    pub more_info_url: Option<String>,
}

/// What Microsoft 365 lists: the labels in its order, and the policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LabelCatalog {
    pub labels: Vec<CatalogLabel>,
    pub policy: LabelPolicy,
}

impl LabelCatalog {
    /// The listed label with this id (ASCII case ignored).
    pub fn label(&self, id: &str) -> Option<&CatalogLabel> {
        self.labels.iter().find(|l| l.id.eq_ignore_ascii_case(id))
    }
}

/// A label as Settings and the composer show it: with its check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelOption {
    #[serde(flatten)]
    pub label: CatalogLabel,
    pub check: CheckResult,
}

/// What an account can send protected, and how its checks went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectionSetup {
    pub account_id: String,
    /// A Microsoft 365 work or school account: the only kind that can.
    pub supported: bool,
    pub encrypt: CheckResult,
    pub do_not_forward: CheckResult,
    /// Empty without the permission, or until first listed.
    pub labels: Vec<LabelOption>,
    pub policy: LabelPolicy,
    /// When Microsoft 365 last listed the labels; None until then.
    pub labels_listed_at: Option<DateTime<Utc>>,
    /// Why the last listing failed; None after a success. `labels` are the
    /// ones listed before.
    pub labels_error: Option<String>,
    /// A check is waiting for its messages.
    pub checking: bool,
}

// --- Organization setup (administrators) ---------------------------------------
//
// An administrator signs in with the separate setup app registration to
// read, create and repair the organization's mail flow rules and
// auto-labeling policies (`crate::orgsetup`), or copies a PowerShell script
// that does the same.

/// The administrator signed in for organization setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSetupAdmin {
    /// The sign-in name.
    pub username: String,
    pub display_name: String,
}

/// Whether setup can run from here for an account's organization, and
/// what this computer did last. Known without asking Microsoft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSetupInfo {
    /// This build names the setup app, so setup can run from here;
    /// otherwise only the script.
    pub automatic: bool,
    pub tenant_id: String,
    /// Signed in for setup on this computer now, for this organization.
    pub signed_in: Option<OrgSetupAdmin>,
    /// The setup app's approval link for this organization; None without
    /// automatic setup.
    pub approval_url: Option<String>,
    /// When setup last changed mail flow rules from this computer; None if
    /// never.
    pub rules_changed_at: Option<DateTime<Utc>>,
    /// When setup last changed auto-labeling policies or their rules from
    /// this computer; None if never.
    pub policies_changed_at: Option<DateTime<Utc>>,
}

/// Where a status row goes in the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrgSetupSection {
    Approval,
    Encryption,
    Labels,
    Routing,
}

/// How a status row stands; each has a chip with words, never color alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrgSetupRowState {
    Ok,
    Missing,
    Differs,
    Off,
    Waiting,
    Blocked,
    Unknown,
    Info,
}

/// What a status row offers to fix it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrgSetupFix {
    /// The footer's Set up or Repair.
    Setup,
    /// Cumulas Mail's own approval link.
    Approval,
    /// Cumulas Signatures' page for administrators (`OrgSetupRow::link`).
    Signatures,
    /// How to fix it by hand (`OrgSetupRow::detail`), and the script.
    Manual,
    /// Signing in with another account.
    SignIn,
}

/// One thing setup needs, as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSetupRow {
    /// Stable: approval, setupApp, exchangeAccess, irm, templates,
    /// encryptRule, dnfRule, labelsAccess, stripRule, licensing,
    /// `label:<id>`, routing, `otherRouting:<name>`, `leftover:<name>`.
    pub id: String,
    pub section: OrgSetupSection,
    pub title: String,
    pub state: OrgSetupRowState,
    pub summary: String,
    pub detail: Option<String>,
    /// The Exchange or Purview object it's about, if any.
    pub object_name: Option<String>,
    pub fix: Option<OrgSetupFix>,
    /// For fix `Signatures`: Cumulas Signatures' page for administrators.
    pub link: Option<String>,
}

/// A sensitivity label setup can make an auto-labeling policy for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSetupLabel {
    /// The ID people's Cumulas Mail sends (Microsoft Graph's), lowercase.
    pub id: String,
    pub title: String,
    /// In the label list of the account the dialog was opened from.
    pub in_catalog: bool,
    pub encrypts: bool,
    /// Purview lists it as a label for email; None when it doesn't say.
    pub for_email: Option<bool>,
    pub state: OrgSetupRowState,
}

/// A rights protection template Exchange lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RmsTemplate {
    pub name: String,
    pub id: Option<String>,
}

/// Whether the organization's mail goes through the Cumulas Signatures
/// relay, and whether mail coming back from it counts as internal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelayPath {
    None,
    CumulasInternal,
    CumulasNotInternal,
    /// The relay's rule is there, but its connector couldn't be read.
    CumulasUnknown,
}

/// What the administrator chose to set up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrgSetupChoices {
    /// Set up Encrypt and Do Not Forward.
    pub actions: bool,
    pub encrypt_template: Option<String>,
    pub do_not_forward_template: Option<String>,
    /// Labels to set up or repair, by the ID people's Cumulas Mail sends.
    pub label_ids: Vec<String>,
    /// Set up labels although the license hint says automatic labeling
    /// isn't included.
    pub labels_without_license: bool,
}

/// What a change does to its object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrgSetupAction {
    Create,
    Change,
    TurnOn,
    Move,
    StartSimulation,
}

/// The kind of object a change is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrgSetupObject {
    MailFlowRule,
    AutoLabelingPolicy,
    AutoLabelingRule,
}

/// One change setup would make, as the confirmation sheet shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSetupChange {
    /// `<action>:<object>:<lowercase name>#<digest of its PowerShell>`, the
    /// same for the same status and choices; apply refuses a change whose
    /// calls differ from the confirmed ones as stale.
    pub id: String,
    pub action: OrgSetupAction,
    pub object: OrgSetupObject,
    pub name: String,
    /// What changes, a sentence each.
    pub details: Vec<String>,
    /// The same in PowerShell.
    pub powershell: String,
}

/// The changes some choices make.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSetupPlan {
    pub changes: Vec<OrgSetupChange>,
    /// Why these choices can't be set up; empty when they can.
    pub problems: Vec<String>,
    /// How many of Cumulas Mail's objects are left out because they can't
    /// be changed from here.
    pub blocked: u32,
}

/// What the work account's own licenses say about automatic labeling (a
/// hint about one person, not the organization).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LicenseHintState {
    Included,
    /// Included, but Microsoft 365 hasn't finished turning it on.
    Pending,
    NotIncluded,
    Unknown,
}

/// Everything setup read, and what it would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSetupStatus {
    pub read_at: DateTime<Utc>,
    pub admin: OrgSetupAdmin,
    pub rows: Vec<OrgSetupRow>,
    pub labels: Vec<OrgSetupLabel>,
    pub templates: Vec<RmsTemplate>,
    pub relay: RelayPath,
    /// From the work account's own licenses (a hint about one person).
    pub license: LicenseHintState,
    /// Mail flow rules could be read, so they can be set up from here.
    pub exchange_readable: bool,
    /// Security & Compliance could be read, so auto-labeling policies can
    /// be set up from here.
    pub labels_readable: bool,
    /// Neither half could be read for lack of a role: one message instead
    /// of rows.
    pub no_role: bool,
    /// Some of Cumulas Mail's rules or policies exist: the button says
    /// Repair, else Set up.
    pub any_ours: bool,
    pub defaults: OrgSetupChoices,
    pub plan: OrgSetupPlan,
}

/// How one change went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrgSetupOutcome {
    Done,
    /// Done, but Security & Compliance doesn't list it yet.
    NotYetListed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSetupChangeResult {
    pub change: OrgSetupChange,
    pub outcome: OrgSetupOutcome,
    /// Done: a note (such as turning on after a simulation); failed or
    /// skipped: why.
    pub message: Option<String>,
}

/// What making the changes did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSetupResult {
    /// Something changed since the plan was shown, so nothing was done:
    /// `status.plan` is the plan to review.
    pub stale: bool,
    /// The sign-in ended during the changes; the rest were skipped.
    pub signed_out: bool,
    pub changes: Vec<OrgSetupChangeResult>,
    pub status: OrgSetupStatus,
}

/// What the PowerShell script covers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrgSetupScriptRequest {
    /// From the dialog's choices once signed in; None: from the account.
    pub choices: Option<OrgSetupChoices>,
    /// The script view's Encryption checkbox.
    pub actions: bool,
    /// The script view's Sensitivity labels checkbox.
    pub labels: bool,
}

/// One label's Microsoft Purview portal steps, filled in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalSteps {
    pub label_title: String,
    pub policy_name: String,
    pub rule_name: String,
    pub pattern: String,
}

/// The PowerShell script that sets up the same, and the portal steps for
/// admins without Windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSetupScript {
    pub script: String,
    pub portal_steps: Vec<PortalSteps>,
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
    /// company signatures are off for this build or run (see
    /// [`crate::signatures::config::signature_client_id`]).
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
    /// The earliest that checking now would ask the organization: a minute
    /// after the last check ended, and not before the service said to wait
    /// until. `None` when it would at once.
    pub checkable_at: Option<DateTime<Utc>>,
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
    /// An account's protection setup: its checks and its sensitivity labels.
    Protection,
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
    fn search_options_default_when_missing() {
        let options: SearchOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(options, SearchOptions::default());
        assert_eq!(
            (options.sort, options.week_starts_on),
            (SearchSort::Relevance, 0)
        );
        let none: Option<SearchOptions> = serde_json::from_str("null").unwrap();
        assert_eq!(none.unwrap_or_default(), SearchOptions::default());
        let given: SearchOptions =
            serde_json::from_str(r#"{"sort":"newest","weekStartsOn":1}"#).unwrap();
        assert_eq!(
            given,
            SearchOptions {
                sort: SearchSort::Newest,
                week_starts_on: 1
            }
        );
        let sort: SearchOptions = serde_json::from_str(r#"{"sort":"relevance"}"#).unwrap();
        assert_eq!(sort.sort, SearchSort::Relevance);
    }

    #[test]
    fn search_hits_are_flat_with_a_snippet() {
        let summary = MessageSummary {
            id: "m1".into(),
            account_id: "a".into(),
            folder_id: "f".into(),
            remote_id: "r".into(),
            conversation_id: None,
            subject: "Lunch".into(),
            from: Some(EmailAddress::new(Some("Ada"), "ada@x")),
            to: vec![],
            cc: vec![],
            preview: "…".into(),
            received_at: "2026-10-17T09:00:00Z".parse().unwrap(),
            is_read: false,
            is_flagged: false,
            is_draft: false,
            has_attachments: false,
            importance: Importance::Normal,
        };
        let run = |text: &str, hit| TextRun {
            text: text.into(),
            hit,
        };
        let hit = SearchHit {
            summary: summary.clone(),
            snippet: Some(vec![
                run("…we could get ", false),
                run("pineapple", true),
                run(" pizza on Friday…", false),
            ]),
        };
        assert_eq!(
            serde_json::to_value(&hit).unwrap(),
            serde_json::json!({
                "id": "m1", "accountId": "a", "folderId": "f", "remoteId": "r",
                "conversationId": null, "subject": "Lunch",
                "from": { "name": "Ada", "address": "ada@x" }, "to": [], "cc": [],
                "preview": "…", "receivedAt": "2026-10-17T09:00:00Z", "isRead": false,
                "isFlagged": false, "isDraft": false, "hasAttachments": false,
                "importance": "normal",
                "snippet": [
                    { "text": "…we could get ", "hit": false },
                    { "text": "pineapple", "hit": true },
                    { "text": " pizza on Friday…", "hit": false },
                ],
            })
        );
        let plain = SearchHit {
            summary,
            snippet: None,
        };
        assert_eq!(
            serde_json::to_value(&plain).unwrap()["snippet"],
            serde_json::Value::Null
        );
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
                checkable_at: Some("2026-10-15T09:01:00Z".parse::<DateTime<Utc>>().unwrap()),
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
                    "checkableAt": "2026-10-15T09:01:00Z",
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

    #[test]
    fn permission_shapes_match_the_ui() {
        use serde_json::json;
        for (request, wire) in [
            (PermissionRequest::All, json!({ "kind": "all" })),
            (PermissionRequest::MailOnly, json!({ "kind": "mailOnly" })),
            (PermissionRequest::Current, json!({ "kind": "current" })),
            (
                PermissionRequest::Feature {
                    feature: AccountFeature::MailboxSettings,
                },
                json!({ "kind": "feature", "feature": "mailboxSettings" }),
            ),
        ] {
            assert_eq!(serde_json::to_value(request).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<PermissionRequest>(wire).unwrap(),
                request
            );
        }
        assert_eq!(PermissionRequest::default(), PermissionRequest::All);
        assert_eq!(
            serde_json::to_value(AccountFeature::ALL).unwrap(),
            json!([
                "calendar",
                "contacts",
                "tasks",
                "mailboxSettings",
                "sensitivityLabels"
            ])
        );
        // Enum order is the display order.
        let mut sorted = AccountFeature::ALL;
        sorted.sort();
        assert_eq!(sorted, AccountFeature::ALL);
        // Sensitivity labels alone are asked for on request, and only by
        // work and school accounts.
        for feature in AccountFeature::ALL {
            let labels = feature == AccountFeature::SensitivityLabels;
            assert_eq!(feature.on_request_only(), labels, "{feature:?}");
            assert_eq!(feature.for_personal_accounts(), !labels, "{feature:?}");
        }
        assert_eq!(
            AccountFeature::SensitivityLabels.noun(),
            "sensitivity labels"
        );
        assert_eq!(
            serde_json::to_value(PermissionRequest::Feature {
                feature: AccountFeature::SensitivityLabels
            })
            .unwrap(),
            json!({ "kind": "feature", "feature": "sensitivityLabels" })
        );

        let grant = MicrosoftGrant {
            scopes: vec!["User.Read".into()],
            refused: vec![AccountFeature::Tasks],
            refused_at: None,
        };
        assert_eq!(
            serde_json::to_value(&grant).unwrap(),
            json!({ "scopes": ["User.Read"], "refused": ["tasks"], "refusedAt": null })
        );
        // Missing fields take their defaults.
        assert_eq!(
            serde_json::from_str::<MicrosoftGrant>(r#"{"scopes":["User.Read"]}"#).unwrap(),
            MicrosoftGrant {
                refused: vec![],
                ..grant
            }
        );
        let link = AdminConsentLink {
            admin_consent_url: "https://login.microsoftonline.com/x".into(),
            tenant: "organizations".into(),
            admin_page_url: None,
        };
        assert_eq!(
            serde_json::to_value(&link).unwrap(),
            json!({
                "adminConsentUrl": "https://login.microsoftonline.com/x",
                "tenant": "organizations",
                "adminPageUrl": null,
            })
        );
    }

    /// The shapes `src/api/types.ts` mirrors (section 3.2 of the design).
    #[test]
    fn protection_shapes_match_the_ui() {
        use serde_json::json;
        let kinds = [
            (ProtectionKind::RightsManaged, "rightsManaged"),
            (ProtectionKind::SmimeEncrypted, "smimeEncrypted"),
            (ProtectionKind::SmimeSigned, "smimeSigned"),
            (ProtectionKind::PortalLink, "portalLink"),
        ];
        for (kind, name) in kinds {
            assert_eq!(serde_json::to_value(kind).unwrap(), name);
        }
        assert!(ProtectionKind::RightsManaged.is_encrypted());
        assert!(ProtectionKind::SmimeEncrypted.is_encrypted());
        assert!(!ProtectionKind::SmimeSigned.is_encrypted());
        assert!(!ProtectionKind::PortalLink.is_encrypted());

        assert_eq!(
            serde_json::to_value(ProtectAction::DoNotForward).unwrap(),
            "doNotForward"
        );
        assert_eq!(
            serde_json::to_value(ProtectAction::Encrypt).unwrap(),
            "encrypt"
        );
        assert_eq!(ProtectAction::DoNotForward.header_value(), "do-not-forward");
        assert_eq!(
            ProtectAction::from_header_value(" Do-Not-Forward "),
            Some(ProtectAction::DoNotForward)
        );
        assert_eq!(
            ProtectAction::from_header_value("ENCRYPT"),
            Some(ProtectAction::Encrypt)
        );
        assert_eq!(ProtectAction::from_header_value("encrypt-all"), None);
        assert_eq!(ProtectAction::DoNotForward.name(), "Do Not Forward");

        let label = CheckItem::Label {
            label_id: "6c1b3b5e-0000-4000-8000-000000000001".into(),
        };
        let wire = json!({ "kind": "label", "labelId": "6c1b3b5e-0000-4000-8000-000000000001" });
        assert_eq!(serde_json::to_value(&label).unwrap(), wire);
        assert_eq!(serde_json::from_value::<CheckItem>(wire).unwrap(), label);
        let action = json!({ "kind": "action", "action": "encrypt" });
        assert_eq!(
            serde_json::from_value::<CheckItem>(action).unwrap(),
            CheckItem::Action {
                action: ProtectAction::Encrypt
            }
        );
        let states = [
            (CheckState::NotChecked, "notChecked"),
            (CheckState::Waiting, "waiting"),
            (CheckState::Applied, "applied"),
            (CheckState::Expired, "expired"),
            (CheckState::NotApplied, "notApplied"),
            (CheckState::Unknown, "unknown"),
        ];
        for (state, name) in states {
            assert_eq!(serde_json::to_value(state).unwrap(), name);
        }
        let notes = [
            (CheckNote::OtherRule, "otherRule"),
            (CheckNote::HeaderKept, "headerKept"),
            (CheckNote::NotArrived, "notArrived"),
            (CheckNote::HeaderNotKept, "headerNotKept"),
            (CheckNote::RequestIgnored, "requestIgnored"),
            (CheckNote::RequestRemoved, "requestRemoved"),
            (CheckNote::RelayNotInternal, "relayNotInternal"),
            (CheckNote::TooSoon, "tooSoon"),
            (CheckNote::OtherLabel, "otherLabel"),
            (CheckNote::OtherProtection, "otherProtection"),
            (CheckNote::LabelUnreadable, "labelUnreadable"),
            (CheckNote::RelayChanged, "relayChanged"),
        ];
        for (note, name) in notes {
            assert_eq!(serde_json::to_value(note).unwrap(), name);
        }
        // Results without the newer fields read with their defaults.
        let old: CheckResult = serde_json::from_value(json!({
            "state": "applied", "at": null, "note": "otherRule", "testMessage": true,
            "passedAt": null,
        }))
        .unwrap();
        assert_eq!(
            (
                old.other_label_id,
                old.other_label_on_control,
                old.changed_at
            ),
            (None, false, None)
        );
        assert_eq!(
            serde_json::to_value(DataKind::Protection).unwrap(),
            "protection"
        );
        assert_eq!(
            serde_json::to_value(crate::providers::SmimeContent::Enveloped).unwrap(),
            "enveloped"
        );

        let at = "2026-10-16T09:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let general = CatalogLabel {
            id: "6c1b3b5e-0000-4000-8000-000000000001".into(),
            name: "All Employees".into(),
            parent_name: Some("Confidential".into()),
            tooltip: Some("For everyone here".into()),
            color: Some("#b35f26".into()),
            has_protection: true,
            sensitivity: Some(3),
        };
        assert_eq!(general.title(), "Confidential › All Employees");
        let setup = ProtectionSetup {
            account_id: "a".into(),
            supported: true,
            encrypt: CheckResult {
                state: CheckState::Applied,
                at: Some(at),
                note: None,
                test_message: true,
                passed_at: Some(at),
                ..CheckResult::default()
            },
            do_not_forward: CheckResult {
                state: CheckState::NotApplied,
                at: Some(at),
                note: Some(CheckNote::OtherLabel),
                test_message: false,
                passed_at: None,
                other_label_id: Some("6c1b3b5e-0000-4000-8000-000000000002".into()),
                other_label_on_control: true,
                changed_at: Some(at),
            },
            labels: vec![LabelOption {
                label: general,
                check: CheckResult::default(),
            }],
            policy: LabelPolicy {
                required: true,
                default_label_id: Some("6c1b3b5e-0000-4000-8000-000000000001".into()),
                justify_downgrade: false,
                more_info_url: Some("https://contoso.com/labels".into()),
            },
            labels_listed_at: Some(at),
            labels_error: None,
            checking: false,
        };
        assert_eq!(
            serde_json::to_value(&setup).unwrap(),
            json!({
                "accountId": "a",
                "supported": true,
                "encrypt": {
                    "state": "applied",
                    "at": "2026-10-16T09:00:00Z",
                    "note": null,
                    "testMessage": true,
                    "passedAt": "2026-10-16T09:00:00Z",
                    "otherLabelId": null,
                    "otherLabelOnControl": false,
                    "changedAt": null,
                },
                "doNotForward": {
                    "state": "notApplied",
                    "at": "2026-10-16T09:00:00Z",
                    "note": "otherLabel",
                    "testMessage": false,
                    "passedAt": null,
                    "otherLabelId": "6c1b3b5e-0000-4000-8000-000000000002",
                    "otherLabelOnControl": true,
                    "changedAt": "2026-10-16T09:00:00Z",
                },
                "labels": [{
                    "id": "6c1b3b5e-0000-4000-8000-000000000001",
                    "name": "All Employees",
                    "parentName": "Confidential",
                    "tooltip": "For everyone here",
                    "color": "#b35f26",
                    "hasProtection": true,
                    "sensitivity": 3,
                    "check": {
                        "state": "notChecked",
                        "at": null,
                        "note": null,
                        "testMessage": false,
                        "passedAt": null,
                        "otherLabelId": null,
                        "otherLabelOnControl": false,
                        "changedAt": null,
                    },
                }],
                "policy": {
                    "required": true,
                    "defaultLabelId": "6c1b3b5e-0000-4000-8000-000000000001",
                    "justifyDowngrade": false,
                    "moreInfoUrl": "https://contoso.com/labels",
                },
                "labelsListedAt": "2026-10-16T09:00:00Z",
                "labelsError": null,
                "checking": false,
            })
        );

        let message = Message {
            summary: MessageSummary {
                id: "m".into(),
                account_id: "a".into(),
                folder_id: "f".into(),
                remote_id: "r".into(),
                conversation_id: None,
                subject: "Q4".into(),
                from: None,
                to: vec![],
                cc: vec![],
                preview: String::new(),
                received_at: at,
                is_read: false,
                is_flagged: false,
                is_draft: false,
                has_attachments: true,
                importance: Importance::Normal,
            },
            bcc: vec![],
            reply_to: vec![],
            body_html: None,
            body_text: None,
            has_blocked_content: false,
            attachments: vec![],
            internet_message_id: None,
            in_reply_to: None,
            references: vec![],
            protection: Some(Protection {
                kind: ProtectionKind::RightsManaged,
                container_ids: vec!["att-1".into()],
                portal_url: Some("https://outlook.office365.com/Encryption/retrieve.ashx".into()),
                content_hidden: false,
            }),
            label: Some(MessageLabel {
                id: "6c1b3b5e-0000-4000-8000-000000000001".into(),
                name: Some("Confidential › All Employees".into()),
                color: Some("#b35f26".into()),
                from_catalog: true,
            }),
            requested: Some(RequestedProtection {
                action: Some(ProtectAction::Encrypt),
                label: None,
            }),
        };
        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(
            value["protection"],
            json!({
                "kind": "rightsManaged",
                "containerIds": ["att-1"],
                "portalUrl": "https://outlook.office365.com/Encryption/retrieve.ashx",
                "contentHidden": false,
            })
        );
        assert_eq!(
            value["label"],
            json!({
                "id": "6c1b3b5e-0000-4000-8000-000000000001",
                "name": "Confidential › All Employees",
                "color": "#b35f26",
                "fromCatalog": true,
            })
        );
        assert_eq!(
            value["requested"],
            json!({ "action": "encrypt", "label": null })
        );
        // Messages kept before these fields existed read without them.
        let mut old = value.clone();
        for field in ["protection", "label", "requested"] {
            old.as_object_mut().unwrap().remove(field);
        }
        let read: Message = serde_json::from_value(old).unwrap();
        assert_eq!(
            (read.protection, read.label, read.requested),
            (None, None, None)
        );

        // The composer's payload without the new fields still reads.
        let outgoing: OutgoingMessage = serde_json::from_value(json!({
            "accountId": "a",
            "to": [{ "name": null, "address": "pat@contoso.com" }],
            "subject": "Hi",
            "bodyHtml": "<p>Hi</p>",
        }))
        .unwrap();
        assert_eq!((outgoing.protect, outgoing.label_id), (None, None));
        let outgoing: OutgoingMessage = serde_json::from_value(json!({
            "accountId": "a",
            "to": [],
            "subject": "Hi",
            "bodyHtml": "<p>Hi</p>",
            "protect": "doNotForward",
            "labelId": "6c1b3b5e-0000-4000-8000-000000000001",
        }))
        .unwrap();
        assert_eq!(outgoing.protect, Some(ProtectAction::DoNotForward));
        assert_eq!(
            outgoing.label_id.as_deref(),
            Some("6c1b3b5e-0000-4000-8000-000000000001")
        );
    }

    /// What each value of an enum is called on the wire.
    fn wire_names<T: Serialize>(values: &[T]) -> Vec<String> {
        values
            .iter()
            .map(|v| {
                serde_json::to_value(v)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }

    /// The organization setup shapes `src/api/types.ts` mirrors (section 7.2
    /// of the admin setup design).
    #[test]
    fn org_setup_shapes_match_the_ui() {
        use serde_json::json;
        assert_eq!(
            wire_names(&[
                OrgSetupSection::Approval,
                OrgSetupSection::Encryption,
                OrgSetupSection::Labels,
                OrgSetupSection::Routing,
            ]),
            ["approval", "encryption", "labels", "routing"]
        );
        assert_eq!(
            wire_names(&[
                OrgSetupRowState::Ok,
                OrgSetupRowState::Missing,
                OrgSetupRowState::Differs,
                OrgSetupRowState::Off,
                OrgSetupRowState::Waiting,
                OrgSetupRowState::Blocked,
                OrgSetupRowState::Unknown,
                OrgSetupRowState::Info,
            ]),
            [
                "ok", "missing", "differs", "off", "waiting", "blocked", "unknown", "info"
            ]
        );
        assert_eq!(
            wire_names(&[
                OrgSetupFix::Setup,
                OrgSetupFix::Approval,
                OrgSetupFix::Signatures,
                OrgSetupFix::Manual,
                OrgSetupFix::SignIn,
            ]),
            ["setup", "approval", "signatures", "manual", "signIn"]
        );
        assert_eq!(
            wire_names(&[
                RelayPath::None,
                RelayPath::CumulasInternal,
                RelayPath::CumulasNotInternal,
                RelayPath::CumulasUnknown,
            ]),
            [
                "none",
                "cumulasInternal",
                "cumulasNotInternal",
                "cumulasUnknown"
            ]
        );
        assert_eq!(
            wire_names(&[
                OrgSetupAction::Create,
                OrgSetupAction::Change,
                OrgSetupAction::TurnOn,
                OrgSetupAction::Move,
                OrgSetupAction::StartSimulation,
            ]),
            ["create", "change", "turnOn", "move", "startSimulation"]
        );
        assert_eq!(
            wire_names(&[
                OrgSetupObject::MailFlowRule,
                OrgSetupObject::AutoLabelingPolicy,
                OrgSetupObject::AutoLabelingRule,
            ]),
            ["mailFlowRule", "autoLabelingPolicy", "autoLabelingRule"]
        );
        assert_eq!(
            wire_names(&[
                LicenseHintState::Included,
                LicenseHintState::Pending,
                LicenseHintState::NotIncluded,
                LicenseHintState::Unknown,
            ]),
            ["included", "pending", "notIncluded", "unknown"]
        );
        assert_eq!(
            wire_names(&[
                OrgSetupOutcome::Done,
                OrgSetupOutcome::NotYetListed,
                OrgSetupOutcome::Failed,
                OrgSetupOutcome::Skipped,
            ]),
            ["done", "notYetListed", "failed", "skipped"]
        );

        let at = "2026-10-18T14:02:11Z".parse::<DateTime<Utc>>().unwrap();
        let admin = OrgSetupAdmin {
            username: "admin@contoso.com".into(),
            display_name: "Ada Admin".into(),
        };
        let info = OrgSetupInfo {
            automatic: true,
            tenant_id: "85f67998-e79a-4173-95c2-90b0e6dc231d".into(),
            signed_in: Some(admin.clone()),
            approval_url: Some("https://login.microsoftonline.com/x".into()),
            rules_changed_at: Some(at),
            policies_changed_at: None,
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({
                "automatic": true,
                "tenantId": "85f67998-e79a-4173-95c2-90b0e6dc231d",
                "signedIn": { "username": "admin@contoso.com", "displayName": "Ada Admin" },
                "approvalUrl": "https://login.microsoftonline.com/x",
                "rulesChangedAt": "2026-10-18T14:02:11Z",
                "policiesChangedAt": null,
            })
        );

        let choices = OrgSetupChoices {
            actions: true,
            encrypt_template: Some("Encrypt".into()),
            do_not_forward_template: None,
            label_ids: vec!["6c1b3b5e-0000-4000-8000-000000000001".into()],
            labels_without_license: false,
        };
        let change = OrgSetupChange {
            id: "create:mailFlowRule:cumulas mail: encrypt".into(),
            action: OrgSetupAction::Create,
            object: OrgSetupObject::MailFlowRule,
            name: "Cumulas Mail: Encrypt".into(),
            details: vec!["Turns it on".into()],
            powershell: "New-TransportRule -Name 'Cumulas Mail: Encrypt'".into(),
        };
        let status = OrgSetupStatus {
            read_at: at,
            admin,
            rows: vec![OrgSetupRow {
                id: "routing".into(),
                section: OrgSetupSection::Routing,
                title: "Mail routing".into(),
                state: OrgSetupRowState::Differs,
                summary: "Mail goes through the Cumulas Signatures relay.".into(),
                detail: None,
                object_name: Some("Email signatures: add signature".into()),
                fix: Some(OrgSetupFix::Signatures),
                link: Some("https://signature.cumulas.co/manage".into()),
            }],
            labels: vec![OrgSetupLabel {
                id: "6c1b3b5e-0000-4000-8000-000000000001".into(),
                title: "Confidential".into(),
                in_catalog: true,
                encrypts: true,
                for_email: None,
                state: OrgSetupRowState::Missing,
            }],
            templates: vec![RmsTemplate {
                name: "Encrypt".into(),
                id: None,
            }],
            relay: RelayPath::CumulasNotInternal,
            license: LicenseHintState::NotIncluded,
            exchange_readable: true,
            labels_readable: false,
            no_role: false,
            any_ours: true,
            defaults: choices.clone(),
            plan: OrgSetupPlan {
                changes: vec![change.clone()],
                problems: vec![],
                blocked: 1,
            },
        };
        let result = OrgSetupResult {
            stale: false,
            signed_out: true,
            changes: vec![OrgSetupChangeResult {
                change,
                outcome: OrgSetupOutcome::NotYetListed,
                message: None,
            }],
            status,
        };
        let change_json = json!({
            "id": "create:mailFlowRule:cumulas mail: encrypt",
            "action": "create",
            "object": "mailFlowRule",
            "name": "Cumulas Mail: Encrypt",
            "details": ["Turns it on"],
            "powershell": "New-TransportRule -Name 'Cumulas Mail: Encrypt'",
        });
        let choices_json = json!({
            "actions": true,
            "encryptTemplate": "Encrypt",
            "doNotForwardTemplate": null,
            "labelIds": ["6c1b3b5e-0000-4000-8000-000000000001"],
            "labelsWithoutLicense": false,
        });
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({
                "stale": false,
                "signedOut": true,
                "changes": [{ "change": change_json, "outcome": "notYetListed", "message": null }],
                "status": {
                    "readAt": "2026-10-18T14:02:11Z",
                    "admin": { "username": "admin@contoso.com", "displayName": "Ada Admin" },
                    "rows": [{
                        "id": "routing",
                        "section": "routing",
                        "title": "Mail routing",
                        "state": "differs",
                        "summary": "Mail goes through the Cumulas Signatures relay.",
                        "detail": null,
                        "objectName": "Email signatures: add signature",
                        "fix": "signatures",
                        "link": "https://signature.cumulas.co/manage",
                    }],
                    "labels": [{
                        "id": "6c1b3b5e-0000-4000-8000-000000000001",
                        "title": "Confidential",
                        "inCatalog": true,
                        "encrypts": true,
                        "forEmail": null,
                        "state": "missing",
                    }],
                    "templates": [{ "name": "Encrypt", "id": null }],
                    "relay": "cumulasNotInternal",
                    "license": "notIncluded",
                    "exchangeReadable": true,
                    "labelsReadable": false,
                    "noRole": false,
                    "anyOurs": true,
                    "defaults": choices_json,
                    "plan": { "changes": [change_json], "problems": [], "blocked": 1 },
                },
            })
        );

        // What the dialog sends.
        assert_eq!(
            serde_json::from_value::<OrgSetupChoices>(choices_json.clone()).unwrap(),
            choices
        );
        let request: OrgSetupScriptRequest = serde_json::from_value(json!({
            "choices": choices_json, "actions": true, "labels": false,
        }))
        .unwrap();
        assert_eq!(
            request,
            OrgSetupScriptRequest {
                choices: Some(choices),
                actions: true,
                labels: false,
            }
        );
        assert_eq!(
            serde_json::to_value(OrgSetupScript {
                script: "# Cumulas Mail".into(),
                portal_steps: vec![PortalSteps {
                    label_title: "Confidential".into(),
                    policy_name: "Cumulas Mail: Confidential".into(),
                    rule_name: "Cumulas Mail: Confidential (header)".into(),
                    pattern: "(?i)^(6c1b3b5e-0000-4000-8000-000000000001)$".into(),
                }],
            })
            .unwrap(),
            json!({
                "script": "# Cumulas Mail",
                "portalSteps": [{
                    "labelTitle": "Confidential",
                    "policyName": "Cumulas Mail: Confidential",
                    "ruleName": "Cumulas Mail: Confidential (header)",
                    "pattern": "(?i)^(6c1b3b5e-0000-4000-8000-000000000001)$",
                }],
            })
        );
    }
}
