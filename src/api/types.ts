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
  /** Microsoft accounts: optional features whose permission Microsoft hasn't granted. Empty for IMAP. */
  needsPermission: AccountFeature[];
  /** Microsoft accounts: features that have permission but that Microsoft 365 refused. Empty for IMAP. */
  refusedFeatures: AccountFeature[];
}

/** An optional part of a Microsoft account that needs a permission of its own (in display order). */
export type AccountFeature =
  "calendar" | "contacts" | "tasks" | "mailboxSettings" | "sensitivityLabels";

/** What a Microsoft sign-in asks permission for. */
export type PermissionRequest =
  | { kind: "all" }
  | { kind: "mailOnly" }
  | { kind: "current" }
  | { kind: "feature"; feature: AccountFeature };

/** Where an administrator approves Cumulas Mail for an organization. */
export interface AdminConsentLink {
  adminConsentUrl: string;
  tenant: string;
  /** The page for IT administrators; null unless the sign-in uses Cumulas Mail's published app registration. */
  adminPageUrl: string | null;
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
  /** How the message is protected; null for ordinary mail. */
  protection: Protection | null;
  /** Its sensitivity label from the account's own organization (Microsoft work and school accounts). */
  label: MessageLabel | null;
  /** A copy in a Sent folder of mail Cumulas Mail sent: what it asked for. */
  requested: RequestedProtection | null;
}

/** How a message is protected, as far as the app can tell. */
export type ProtectionKind = "rightsManaged" | "smimeEncrypted" | "smimeSigned" | "portalLink";

export interface Protection {
  kind: ProtectionKind;
  /** Attachments that only hold the protected content or the signature: hidden, explained. */
  containerIds: string[];
  /** The verified link to Microsoft's encrypted message portal, as written; null without one. */
  portalUrl: string | null;
  /** Signed: some content is inside a container Cumulas Mail couldn't unpack. */
  contentHidden: boolean;
}

export interface MessageLabel {
  id: string;
  /** "Parent › Child" from the account's labels, or null. Never a name the message gives. */
  name: string | null;
  /** "#rrggbb" or null. */
  color: string | null;
  /**
   * One of the account's labels, named and colored from them. Otherwise only
   * a label the message names for the organization, which anyone can write: unverified.
   */
  fromCatalog: boolean;
}

/** Encryption the organization's mail server can be asked for. */
export type ProtectAction = "encrypt" | "doNotForward";

/** A Sent copy of mail Cumulas Mail sent: what it asked the organization's mail server for. */
export interface RequestedProtection {
  action: ProtectAction | null;
  label: MessageLabel | null;
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

/**
 * How a server search ended: the server answered (perhaps with nothing), a
 * newer search replaced it or it was canceled (nothing was stored), or the
 * server couldn't be asked or failed (`error` says why).
 */
export type ServerSearchStatus = "done" | "cancelled" | "failed";

/** One page of one account's server search. */
export interface ServerSearchResult {
  searchId: string;
  accountId: string;
  status: ServerSearchStatus;
  /** Local ids found on this page, newest first; all stored now (getMessage etc. work). */
  messageIds: string[];
  /** How many weren't on this device before. */
  added: number;
  hasMore: boolean;
  error: BackendError | null;
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
  /** Encryption to ask the organization's mail server for; refused unless its check passed. */
  protect?: ProtectAction | null;
  /** The sensitivity label to ask for (a GUID); refused unless its check passed. */
  labelId?: string | null;
}

// --- Protection setup ---------------------------------------------------------

/** One thing a check sends a test message for. */
export type CheckItem =
  { kind: "action"; action: ProtectAction } | { kind: "label"; labelId: string };

/**
 * A check's outcome: never checked, waiting for its test message, arrived
 * protected (offered), applied but too long ago (not offered until checked
 * again), arrived without it, or the app can't tell (see the note).
 */
export type CheckState =
  "notChecked" | "waiting" | "applied" | "expired" | "notApplied" | "unknown";

/** Why a check ended as it did, when the state alone doesn't say. */
export type CheckNote = "otherRule" | "headerKept" | "notArrived" | "headerNotKept";

export interface CheckResult {
  state: CheckState;
  /** When it was decided (or sent, while waiting); null if never checked. */
  at: string | null;
  note: CheckNote | null;
  /** The test message arrived and can be opened in Outlook on the web. */
  testMessage: boolean;
  /**
   * When the test message that lets the composer offer it arrived: set while
   * that pass is less than CHECK_VALID_DAYS old, also while a new check waits;
   * null when it isn't offered.
   */
  passedAt: string | null;
}

/** A sensitivity label Microsoft 365 lists for the person, with its check. */
export interface LabelOption {
  /** GUID, lowercase. */
  id: string;
  name: string;
  parentName: string | null;
  tooltip: string | null;
  /** "#rrggbb" or null. */
  color: string | null;
  /** It encrypts. */
  hasProtection: boolean;
  /** Higher is more sensitive; null when Microsoft 365 doesn't say. */
  sensitivity: number | null;
  check: CheckResult;
}

export interface LabelPolicy {
  /** A label is required. */
  required: boolean;
  defaultLabelId: string | null;
  /** Lowering a label needs a reason, which Cumulas Mail can't give. */
  justifyDowngrade: boolean;
  /** https only. */
  moreInfoUrl: string | null;
}

/** What an account can send protected, and how its checks went. */
export interface ProtectionSetup {
  accountId: string;
  /** A Microsoft 365 work or school account: the only kind that can. */
  supported: boolean;
  encrypt: CheckResult;
  doNotForward: CheckResult;
  /** Empty without the permission, or until first listed. */
  labels: LabelOption[];
  policy: LabelPolicy;
  /** When Microsoft 365 last listed the labels; null until then. */
  labelsListedAt: string | null;
  /** Why the last listing failed; null after a success. `labels` are the ones listed before. */
  labelsError: string | null;
  /** A check is waiting for its messages. */
  checking: boolean;
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
 * - unchecked: not answered yet; with `stale`, the first check failed.
 * - notUsed: Microsoft refused the token (the organization hasn't added Cumulas Signatures'
 *   app), or the service has no signature for this person.
 * - ready: the organization's signature, which the app adds (`html`).
 * - addedOnSend: the organization's mail server adds it on send (to replies and forwards only
 *   with `onReplies`); the app adds none.
 * - notAllowed: Microsoft or the service didn't let Cumulas Mail get the signature, which may
 *   be how the service is set up rather than the organization's choice.
 */
export type OrganizationSignatureState =
  "unsupported" | "unchecked" | "notUsed" | "ready" | "addedOnSend" | "notAllowed";

export interface OrganizationSignature {
  state: OrganizationSignatureState;
  /** Sanitized HTML; only while `ready`. */
  html: string | null;
  /**
   * It also goes on replies and forwards, added by the app (`ready`) or by
   * the organization's mail server (`addedOnSend`); false otherwise.
   */
  onReplies: boolean;
  /** When the organization last answered (ISO 8601). */
  checkedAt: string | null;
  /** The last check failed (offline, the service down): `state` is the last known. */
  stale: boolean;
  /**
   * The earliest that checking now would ask the organization (ISO 8601): a
   * minute after the last check ended, and not before the service said to
   * wait until. Null when it would at once.
   */
  checkableAt: string | null;
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

export type DataKind =
  "accounts" | "mail" | "calendar" | "contacts" | "tasks" | "signatures" | "protection";

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
  /** The page for IT administrators; null unless the sign-in uses Cumulas Mail's published app registration. */
  adminPageUrl: string | null;
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

/**
 * Restarting into an installed update: not asked for yet, under way (the new
 * version is starting, to take over), or the last try failed (it can be tried
 * again).
 */
export type RestartPhase = "idle" | "restarting" | "failed";

/**
 * An update of the Flatpak, as the Flatpak portal reports it (it checks every
 * 30 minutes). Software centers and `flatpak update` install updates; the app
 * says when one is installed and restarts into it. Always { kind: "none" }
 * outside a Flatpak.
 */
export type UpdateState =
  | { kind: "none" }
  /** A version other than the running one is installed. */
  | { kind: "installed"; commit: string; restart: RestartPhase };

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
   * signature.cumulas.co); null when company signatures are off for this
   * build or run.
   */
  signatureServer: string | null;
  /** Started by a restart into an update. */
  restartedToUpdate: boolean;
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
  | "internal"
  | "signInBlocked"
  | "permissionRequired";

/** Why the organization's settings block a Microsoft sign-in. */
export type SignInBlockedReason =
  | "notAssigned"
  | "appDisabled"
  | "appNotSetUp"
  | "serviceNotEnabled"
  | "securityInfo"
  | "conditionalAccess";

/** permissionRequired: the permission isn't granted, or Microsoft 365 refused the feature. */
export type PermissionRequiredReason = "notGranted" | "refused";

/**
 * Why sending refused a message: it replies to or forwards an encrypted one
 * (`messageId`), a label is required, Microsoft 365 didn't keep the request
 * on its draft, or the reply already carries another label (`labelId`).
 */
export type SendRefusedReason =
  "protectedOriginal" | "labelRequired" | "headersNotKept" | "labelCopied";

/** Shape of errors rejected by backend calls. */
export interface BackendError {
  code: ErrorCode;
  message: string;
  details?: {
    adminConsentUrl?: string;
    tenant?: string;
    /** The tenant has Microsoft's admin consent workflow ("Request approval") enabled. */
    canRequestApproval?: boolean;
    adminPageUrl?: string | null;
    /** signInBlocked and authRequired-after-refresh: why. signInCancelled: "timeout". permissionRequired: which state. sendMessage: why it refused. */
    reason?: SignInBlockedReason | "timeout" | PermissionRequiredReason | SendRefusedReason;
    /** signInBlocked (and authRequired after a blocked refresh): Microsoft's AADSTS code. */
    aadsts?: number;
    /** permissionRequired */
    feature?: AccountFeature;
    accountId?: string;
    /** protectedOriginal: the message replied to or forwarded. */
    messageId?: string;
    /** protectedOriginal: how it's protected. */
    protection?: ProtectionKind;
    /** labelCopied: the label already on the reply or forward. */
    labelId?: string;
    [key: string]: unknown;
  };
}
