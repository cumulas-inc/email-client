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
  /** When to remind about the task (ISO 8601); null for no reminder. */
  reminderAt: string | null;
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
  reminderAt?: string | null;
}

export interface TaskPatch {
  title?: string;
  notes?: string;
  clearNotes?: boolean;
  status?: TaskStatus;
  importance?: Importance;
  dueDate?: string;
  clearDueDate?: boolean;
  /** A new reminder time (ISO 8601). */
  reminderAt?: string;
  clearReminder?: boolean;
}

// --- Reminders ----------------------------------------------------------------

export type ReminderKind = "event" | "task";

/** A reminder that is due, or snoozed. */
export interface ActiveReminder {
  /** `{event|task}:{item id}:{fire time}`; a moved event's reminder gets a new key. */
  key: string;
  kind: ReminderKind;
  /** The event's or task's id. */
  itemId: string;
  accountId: string;
  title: string;
  /** When the event starts (an all-day event: local midnight); null for tasks. */
  startsAt: string | null;
  endsAt: string | null;
  location: string | null;
  onlineMeetingUrl: string | null;
  /** When the reminder went off. */
  fireAt: string;
  /** Snoozed until then (not due again yet); null when due. */
  snoozedUntil: string | null;
  /** The event has started, or the task is past its reminder time. */
  overdue: boolean;
  /** The event was over before the reminder could show: listed, never shown as a notification. */
  missed: boolean;
}

/** The `reminders-changed` event: every active reminder, the ones that just came due, and the keys of those no longer due. */
export interface RemindersChangedEvent {
  active: ActiveReminder[];
  newlyDue: ActiveReminder[];
  cleared: string[];
}

/** How long to snooze: some minutes, or until a time (ISO 8601). */
export type SnoozeFor = { minutes: number } | { until: string };

// --- Signatures ---------------------------------------------------------------

/**
 * What the account's organization last said about its signature:
 * - unsupported: an IMAP or personal Microsoft account, or company signatures are off.
 * - unchecked: not asked yet.
 * - notUsed: the organization doesn't use Cumulas Signatures, or has no signature for this person.
 * - ready: the organization's signature, which the app adds (`html`).
 * - addedOnSend: the organization's mail server adds it on send; the app adds none.
 * - notAllowed: Microsoft or the service didn't let Cumulas Mail get the signature, which may
 *   be how the service is set up rather than the organization's choice.
 */
export type OrganizationSignatureState =
  "unsupported" | "unchecked" | "notUsed" | "ready" | "addedOnSend" | "notAllowed";

export interface OrganizationSignature {
  state: OrganizationSignatureState;
  /** Sanitized HTML; only while `ready`. */
  html: string | null;
  /** It also goes on replies and forwards; false unless `ready`. */
  onReplies: boolean;
  /** When the organization last answered (ISO 8601). */
  checkedAt: string | null;
  /** The last check failed (offline, the service down): `state` is the last known. */
  stale: boolean;
}

/** An account's signatures: the person's own and the organization's. */
export interface AccountSignature {
  accountId: string;
  /** Sanitized HTML; "" for none. */
  ownHtml: string;
  /** The own signature also goes on replies and forwards. */
  ownOnReplies: boolean;
  organization: OrganizationSignature;
}

export type ComposeSignatureKind = "none" | "organization" | "own" | "addedOnSend";

/** The signature a new message, reply or forward starts with. */
export interface ComposeSignature {
  kind: ComposeSignatureKind;
  /** Set for "organization" and "own". */
  html: string | null;
}

// --- Sync, settings, app ----------------------------------------------------

export type SyncState = "idle" | "syncing" | "error";

export interface SyncStatus {
  accountId: string;
  state: SyncState;
  message: string | null;
  lastSyncedAt: string | null;
}

export type DataKind = "accounts" | "mail" | "calendar" | "contacts" | "tasks" | "signatures";

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

/** How an account hears about new mail. */
export type PushMode =
  | { kind: "imapIdle" }
  | { kind: "imapNotify" }
  | { kind: "graphRelay" }
  | { kind: "polling"; everySecs: number }
  | { kind: "off" };

/** Whether the push mode works as intended. */
export type PushHealth =
  | { kind: "connecting" }
  | { kind: "live" }
  | { kind: "degraded"; reason: string }
  | { kind: "authRequired" }
  | { kind: "consentMissing" }
  | { kind: "unsupported" };

export interface PushStatus {
  accountId: string;
  mode: PushMode;
  health: PushHealth;
  detail: string | null;
  /**
   * Consent to the push service is missing because it was refused at the
   * account's last sign-in, so signing in again won't ask for it again.
   */
  consentRefused: boolean;
  /**
   * Set while consent to the push service is missing because the account's
   * organization requires an administrator to approve it.
   */
  adminConsent: AdminConsent | null;
  /** When the mode or health last changed (ISO 8601). */
  since: string;
}

/** The approval an organization's administrator can give for the push service. */
export interface AdminConsent {
  /** The page where an administrator approves Cumulas Mail for the organization. */
  url: string;
  /** The organization: a tenant id or domain. */
  tenant: string;
}

/** How starting at login works: the Background portal (Flatpak) or an autostart file. */
export type AutostartBackend = "portal" | "xdgAutostartFile" | "unsupported";

/** How notifications reach the desktop. */
export type NotifierKind = "portal" | "freedesktop";

/** Running in the background, starting at login and the tray, for Settings. */
export interface BackgroundState {
  runInBackground: boolean;
  /**
   * Whether the system lets the app run without a window: always outside a
   * Flatpak; inside, the Background portal's answer (null until asked).
   */
  backgroundAllowed: boolean | null;
  startAtLogin: boolean;
  startHidden: boolean;
  autostart: AutostartBackend;
  /** A tray icon can be shown: the desktop shows tray icons and the library is installed. */
  trayAvailable: boolean;
  /** Null when no notification service answered. */
  notifications: NotifierKind | null;
}

/** Where the desktop asks the page to go (a notification or the tray was clicked). */
export type Navigate =
  | { kind: "message"; messageId: string; folderId: string; accountId: string }
  /** An account's Inbox, or every Inbox; `notice` says why, when there's something to say. */
  | { kind: "inbox"; accountId: string | null; notice: string | null }
  | { kind: "compose" }
  /** An event: the day it starts on (the UTC date of `start` for an all-day event). */
  | { kind: "event"; eventId: string; start: string; isAllDay: boolean }
  | { kind: "task"; taskId: string; listId: string }
  /** The Reminders panel; `notice` says why, when there's something to say. */
  | { kind: "reminders"; notice: string | null };

export interface AppInfo {
  version: string;
  persistentSecrets: boolean;
  microsoftClientIdBuiltIn: boolean;
  /** Host of the push service Microsoft accounts use, for the privacy note. */
  syncServer: string;
  /**
   * Host of the service company signatures come from (normally
   * signature.cumulas.co); null when this build has none, and company
   * signatures are off.
   */
  signatureServer: string | null;
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
