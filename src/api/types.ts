// Mirrors crates/core/src/model.rs. Keep the two in sync.
// Dates are ISO-8601 strings (UTC instants) or "YYYY-MM-DD" for plain dates.

export type AccountKind = "microsoft" | "imap";
export type TlsMode = "tls" | "startTls" | "none";

export interface ImapSettings {
  imapHost: string;
  imapPort: number;
  imapTls: TlsMode;
  smtpHost: string;
  smtpPort: number;
  smtpTls: TlsMode;
  username: string;
}

export type AccountSettings =
  | ({ kind: "microsoft" } & { tenantId: string; userId: string })
  | ({ kind: "imap" } & ImapSettings);

export interface Account {
  id: string;
  kind: AccountKind;
  displayName: string;
  email: string;
  color: string;
  settings: AccountSettings;
  createdAt: string;
  lastSyncedAt: string | null;
  syncError: string | null;
}

export interface NewImapAccount {
  displayName: string;
  email: string;
  password: string;
  settings: ImapSettings;
}

/** Changes to an IMAP account; omitted fields keep their current value. */
export interface ImapAccountUpdate {
  password?: string | null;
  settings?: ImapSettings | null;
}

// --- Mail -------------------------------------------------------------------

export interface EmailAddress {
  name: string | null;
  address: string;
}

export type FolderRole =
  "inbox" | "drafts" | "sent" | "archive" | "junk" | "trash" | "outbox" | "other";

export interface Folder {
  id: string;
  accountId: string;
  remoteId: string;
  parentId: string | null;
  name: string;
  role: FolderRole;
  unreadCount: number;
  totalCount: number;
}

export type Importance = "low" | "normal" | "high";

export interface MessageSummary {
  id: string;
  accountId: string;
  folderId: string;
  remoteId: string;
  conversationId: string | null;
  subject: string;
  from: EmailAddress | null;
  to: EmailAddress[];
  cc: EmailAddress[];
  preview: string;
  receivedAt: string;
  isRead: boolean;
  isFlagged: boolean;
  isDraft: boolean;
  hasAttachments: boolean;
  importance: Importance;
}

export interface AttachmentMeta {
  id: string;
  name: string;
  contentType: string;
  size: number;
  isInline: boolean;
  contentId: string | null;
}

export interface Message extends MessageSummary {
  bcc: EmailAddress[];
  replyTo: EmailAddress[];
  /** Sanitized HTML; render only inside the sandboxed MessageBody frame. */
  bodyHtml: string | null;
  bodyText: string | null;
  hasBlockedContent: boolean;
  attachments: AttachmentMeta[];
  internetMessageId: string | null;
  inReplyTo: string | null;
  references: string[];
}

export interface MessageQuery {
  offset: number;
  limit: number;
  unreadOnly?: boolean;
  flaggedOnly?: boolean;
}

export interface MessagePage {
  items: MessageSummary[];
  total: number;
}

export interface OutgoingAttachment {
  name: string;
  contentType: string;
  dataBase64: string;
}

export type ReplyKind = "reply" | "replyAll" | "forward";

export interface OutgoingMessage {
  accountId: string;
  to: EmailAddress[];
  cc: EmailAddress[];
  bcc: EmailAddress[];
  subject: string;
  bodyHtml: string;
  attachments: OutgoingAttachment[];
  importance?: Importance | null;
  replyToMessageId?: string | null;
  replyKind?: ReplyKind | null;
}

// --- Calendar ---------------------------------------------------------------

export interface Calendar {
  id: string;
  accountId: string;
  remoteId: string;
  name: string;
  color: string;
  canEdit: boolean;
  isDefault: boolean;
}

export type ShowAs = "free" | "tentative" | "busy" | "outOfOffice" | "workingElsewhere";
export type ResponseStatus =
  "none" | "organizer" | "accepted" | "tentative" | "declined" | "notResponded";
export type AttendeeKind = "required" | "optional" | "resource";

export interface Attendee {
  email: EmailAddress;
  kind: AttendeeKind;
  response: ResponseStatus;
}

export interface CalendarEvent {
  id: string;
  accountId: string;
  calendarId: string;
  remoteId: string;
  subject: string;
  location: string | null;
  bodyHtml: string | null;
  bodyPreview: string | null;
  /** For all-day events: midnight UTC of the first day. */
  start: string;
  /** For all-day events: midnight UTC of the day after the last day. */
  end: string;
  isAllDay: boolean;
  organizer: EmailAddress | null;
  attendees: Attendee[];
  showAs: ShowAs;
  responseStatus: ResponseStatus;
  isOnlineMeeting: boolean;
  onlineMeetingUrl: string | null;
  reminderMinutes: number | null;
  isRecurring: boolean;
  isCancelled: boolean;
}

export interface EventDraft {
  calendarId: string;
  subject: string;
  location?: string | null;
  bodyHtml?: string | null;
  start: string;
  end: string;
  isAllDay: boolean;
  attendees: EmailAddress[];
  reminderMinutes?: number | null;
  showAs: ShowAs;
  isOnlineMeeting: boolean;
}

export type EventResponse = "accept" | "tentative" | "decline";

// --- Contacts ---------------------------------------------------------------

export interface Phone {
  kind: "mobile" | "business" | "home" | "other";
  number: string;
}

export interface Contact {
  id: string;
  accountId: string;
  remoteId: string;
  displayName: string;
  givenName: string | null;
  surname: string | null;
  emails: EmailAddress[];
  phones: Phone[];
  company: string | null;
  jobTitle: string | null;
  department: string | null;
  address: string | null;
  birthday: string | null;
  notes: string | null;
}

export interface ContactDraft {
  accountId: string;
  displayName: string;
  givenName?: string | null;
  surname?: string | null;
  emails: EmailAddress[];
  phones: Phone[];
  company?: string | null;
  jobTitle?: string | null;
  department?: string | null;
  address?: string | null;
  birthday?: string | null;
  notes?: string | null;
}

// --- Tasks ------------------------------------------------------------------

export interface TaskList {
  id: string;
  accountId: string;
  remoteId: string;
  name: string;
  isDefault: boolean;
}

export type TaskStatus = "notStarted" | "inProgress" | "completed";

export interface Task {
  id: string;
  accountId: string;
  listId: string;
  remoteId: string;
  title: string;
  notes: string | null;
  status: TaskStatus;
  importance: Importance;
  dueDate: string | null;
  completedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface TaskDraft {
  listId: string;
  title: string;
  notes?: string | null;
  importance?: Importance | null;
  dueDate?: string | null;
}

export interface TaskPatch {
  title?: string;
  notes?: string;
  clearNotes?: boolean;
  status?: TaskStatus;
  importance?: Importance;
  dueDate?: string;
  clearDueDate?: boolean;
}

// --- Sync, settings, app ----------------------------------------------------

export type SyncState = "idle" | "syncing" | "error";

export interface SyncStatus {
  accountId: string;
  state: SyncState;
  message: string | null;
  lastSyncedAt: string | null;
}

export type DataKind = "accounts" | "mail" | "calendar" | "contacts" | "tasks";

export interface DataChangedEvent {
  kinds: DataKind[];
  accountId: string | null;
}

export type RemoteContentPolicy = "block" | "allowContacts" | "allow";
export type Theme = "system" | "light" | "dark";
export type ReadingPane = "right" | "bottom" | "off";

/** The typeface for the interface: Inter (bundled), the desktop's, or an installed family. */
export type InterfaceFont =
  { kind: "default" } | { kind: "system" } | { kind: "family"; family: string };

/**
 * The typeface for what people read (subjects, pane titles, the open message,
 * the compose editor): Charis SIL (bundled), the interface's, or an installed
 * family.
 */
export type ReadingFont =
  { kind: "default" } | { kind: "interface" } | { kind: "family"; family: string };

/** Scales the whole window; see TEXT_SCALE in src/lib/fonts.ts. */
export type TextSize = "smaller" | "default" | "larger" | "largest";

/** Which folders raise new-mail notifications. "allFolders" leaves out Junk, Deleted Items, Sent, Drafts and Outbox. */
export type NotifyScope = "inbox" | "allFolders";

export interface Settings {
  theme: Theme;
  readingPane: ReadingPane;
  remoteContent: RemoteContentPolicy;
  markReadAfterSeconds: number | null;
  /** How often everything is synced; new Inbox mail arrives sooner through push. */
  syncIntervalMinutes: number;
  microsoftClientId: string | null;
  microsoftTenant: string | null;
  notificationsEnabled: boolean;
  interfaceFont: InterfaceFont;
  readingFont: ReadingFont;
  textSize: TextSize;
  notifyScope: NotifyScope;
  /** Show the sender and subject in notifications. */
  notifyShowPreview: boolean;
  notifySound: boolean;
  /** Accounts whose new mail never raises a notification. */
  mutedAccountIds: string[];
  /** No new-mail notifications until then (ISO 8601). */
  notificationsPausedUntil: string | null;
  remindersEnabled: boolean;
  defaultSnoozeMinutes: number;
  /** Keep a connection open (IMAP IDLE) or use the push service. */
  instantPush: boolean;
  /** Microsoft accounts use the Cumulas push service; otherwise they poll every 30 seconds. */
  usePushRelay: boolean;
  /** Closing the window keeps the app running for notifications. */
  runInBackground: boolean;
  startAtLogin: boolean;
  /** Started at login, the app opens without a window. */
  startHidden: boolean;
  showTrayIcon: boolean;
}

export interface AppInfo {
  version: string;
  persistentSecrets: boolean;
  microsoftClientIdBuiltIn: boolean;
}

// --- Errors -----------------------------------------------------------------

export type ErrorCode =
  | "authRequired"
  | "adminConsentRequired"
  | "consentDeclined"
  | "signInCancelled"
  | "notConfigured"
  | "network"
  | "provider"
  | "notFound"
  | "invalidInput"
  | "unsupported"
  | "storage"
  | "internal";

/** Shape of errors rejected by backend calls. */
export interface BackendError {
  code: ErrorCode;
  message: string;
  details?: {
    adminConsentUrl?: string;
    tenant?: string;
    /** The tenant has Microsoft's admin consent workflow ("Request approval") enabled. */
    canRequestApproval?: boolean;
    [key: string]: unknown;
  };
}
