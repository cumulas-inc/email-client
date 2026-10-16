import type {
  Account,
  AccountSignature,
  ActiveReminder,
  AppInfo,
  BackgroundState,
  Calendar,
  CalendarEvent,
  ComposeSignature,
  Contact,
  ContactDraft,
  DataChangedEvent,
  EmailAddress,
  EventDraft,
  EventResponse,
  Folder,
  ImapAccountUpdate,
  Message,
  MessagePage,
  MessageQuery,
  MessageSummary,
  Navigate,
  NewImapAccount,
  OutgoingMessage,
  PushStatus,
  RemindersChangedEvent,
  ReplyKind,
  ServerSearchResult,
  Settings,
  SnoozeFor,
  SyncStatus,
  Task,
  TaskDraft,
  TaskList,
  TaskPatch,
} from "./types";

export type Unsubscribe = () => void;

/**
 * Everything the UI can ask of the backend. Implemented by the Tauri bridge
 * (`tauri.ts`) and by an in-memory demo backend (`mock/`) used in the
 * browser, in tests and for screenshots.
 *
 * Calls reject with a `BackendError` ({ code, message, details }).
 */
export interface Backend {
  getAppInfo(): Promise<AppInfo>;
  openLink(url: string): Promise<void>;
  /** Quits, even while running in the background is on. */
  quitApp(): Promise<void>;
  /**
   * The user kept their unsent message when asked whether to quit: the
   * tray's next Quit asks again, rather than quitting at once.
   */
  cancelQuit(): Promise<void>;

  // Accounts & settings
  listAccounts(): Promise<Account[]>;
  addImapAccount(input: NewImapAccount): Promise<Account>;
  /** Opens the system browser for Microsoft sign-in and resolves once done. */
  addMicrosoftAccount(loginHint?: string): Promise<Account>;
  /** Abandons a pending addMicrosoftAccount, which rejects with signInCancelled. */
  cancelMicrosoftSignIn(): Promise<void>;
  /** Checks new credentials/servers against the servers, then saves them. */
  updateImapAccount(accountId: string, update: ImapAccountUpdate): Promise<Account>;
  updateAccount(accountId: string, displayName?: string, color?: string): Promise<Account>;
  removeAccount(accountId: string): Promise<void>;
  getSettings(): Promise<Settings>;
  /**
   * Saves the settings. `runInBackground`, `startAtLogin` and `startHidden`
   * keep their saved values: they change through setRunInBackground and
   * setStartAtLogin, which also tell the system.
   */
  updateSettings(settings: Settings): Promise<Settings>;
  /** Installed font families, sorted, for the font settings. */
  listFonts(): Promise<string[]>;
  /**
   * Scales the whole window, like a browser's zoom (the text size setting).
   * The desktop app zooms its webview, so layout, pointer positions and the
   * message frame all follow; the browser demo approximates it with CSS zoom.
   */
  setZoom(factor: number): Promise<void>;
  syncAccount(accountId: string): Promise<void>;
  syncAll(): Promise<void>;
  getSyncStatuses(): Promise<SyncStatus[]>;
  /** How each account hears about new mail (IMAP IDLE, polling, ...). */
  getPushStatuses(): Promise<PushStatus[]>;

  // Desktop
  getBackgroundState(): Promise<BackgroundState>;
  /** Keeps the app running when its window closes (asking the system in a Flatpak). */
  setRunInBackground(enabled: boolean): Promise<BackgroundState>;
  /** Starts the app at login; `hidden` starts it without a window. */
  setStartAtLogin(enabled: boolean, hidden: boolean): Promise<BackgroundState>;
  /** Shows a notification like new mail's. */
  sendTestNotification(): Promise<void>;
  /**
   * Where a notification clicked before the page started asked to go. Call
   * once the page listens for onNavigate.
   */
  takePendingNavigation(): Promise<Navigate | null>;

  // Reminders
  /** The reminders due now, or snoozed. */
  listActiveReminders(): Promise<ActiveReminder[]>;
  /** Snoozes a reminder; resolves to the reminders then active. */
  snoozeReminder(key: string, snooze: SnoozeFor): Promise<ActiveReminder[]>;
  /** Dismisses reminders; resolves to the reminders then active. */
  dismissReminders(keys: string[]): Promise<ActiveReminder[]>;

  // Mail
  listFolders(accountId?: string): Promise<Folder[]>;
  listMessages(folderId: string, query: MessageQuery): Promise<MessagePage>;
  /**
   * Local search, instant: messages on this device matching `text` (word
   * prefixes; from:, to:, cc:, subject:, body:, has:attachment), plus
   * `includeIds` (ids server searches found). Newest first.
   */
  searchMessages(
    text: string,
    accountId?: string,
    includeIds?: string[],
  ): Promise<MessageSummary[]>;
  /**
   * Searches one account's whole mailbox on its server (first page, or the next
   * with `more`) and stores what it finds. A new `searchId` cancels the previous
   * search; its pending calls resolve with status "cancelled". Never rejects for
   * the server: status "failed" carries the error.
   */
  searchServer(
    searchId: string,
    text: string,
    accountId: string,
    more?: boolean,
  ): Promise<ServerSearchResult>;
  /** Cancels the search if it is the current one. */
  cancelServerSearch(searchId: string): Promise<void>;
  getMessage(messageId: string, allowRemote?: boolean): Promise<Message>;
  setRead(messageIds: string[], read: boolean): Promise<void>;
  setFlagged(messageIds: string[], flagged: boolean): Promise<void>;
  moveMessages(messageIds: string[], folderId: string): Promise<void>;
  archiveMessages(messageIds: string[]): Promise<void>;
  /** Moves to Deleted Items, or deletes permanently when already there. */
  deleteMessages(messageIds: string[]): Promise<void>;
  sendMessage(message: OutgoingMessage): Promise<void>;
  suggestAddresses(text: string): Promise<EmailAddress[]>;
  /** Resolves to the saved path, or null if the user canceled. */
  saveAttachment(messageId: string, attachmentId: string): Promise<string | null>;
  openAttachment(messageId: string, attachmentId: string): Promise<void>;

  // Signatures
  /** The account's own signature and what its organization last said about its own. */
  getAccountSignature(accountId: string): Promise<AccountSignature>;
  /** Saves the account's own signature, sanitized; more than 512 KB is refused (invalidInput). */
  updateOwnSignature(
    accountId: string,
    html: string,
    onReplies: boolean,
  ): Promise<AccountSignature>;
  /**
   * Asks the account's organization for its signature now, unless it was
   * asked in the last minute or its service said to wait (until
   * `organization.checkableAt`). Doesn't fail for the network or the service:
   * the answer is then the last known state, marked stale.
   */
  checkOrganizationSignature(accountId: string): Promise<AccountSignature>;
  /**
   * The signature a new message (`replyKind` null), reply or forward from the
   * account starts with. Waits (at most a few seconds) for an account's first
   * check.
   */
  getComposeSignature(accountId: string, replyKind: ReplyKind | null): Promise<ComposeSignature>;

  // Calendar
  listCalendars(): Promise<Calendar[]>;
  listEvents(start: string, end: string, calendarIds?: string[]): Promise<CalendarEvent[]>;
  /** Fetches a date range from the servers; results arrive via onDataChanged. */
  refreshEvents(start: string, end: string): Promise<void>;
  createEvent(draft: EventDraft): Promise<CalendarEvent>;
  updateEvent(eventId: string, draft: EventDraft): Promise<CalendarEvent>;
  deleteEvent(eventId: string): Promise<void>;
  respondToEvent(
    eventId: string,
    response: EventResponse,
    comment: string | null,
    sendResponse: boolean,
  ): Promise<CalendarEvent>;

  // Contacts
  listContacts(accountId?: string, text?: string): Promise<Contact[]>;
  createContact(draft: ContactDraft): Promise<Contact>;
  updateContact(contactId: string, draft: ContactDraft): Promise<Contact>;
  deleteContact(contactId: string): Promise<void>;

  // Tasks
  listTaskLists(): Promise<TaskList[]>;
  listTasks(listId: string | null, includeCompleted: boolean): Promise<Task[]>;
  createTask(draft: TaskDraft): Promise<Task>;
  updateTask(taskId: string, patch: TaskPatch): Promise<Task>;
  deleteTask(taskId: string): Promise<void>;

  // Events
  onDataChanged(handler: (event: DataChangedEvent) => void): Unsubscribe;
  onSyncStatus(handler: (status: SyncStatus) => void): Unsubscribe;
  onPushStatus(handler: (status: PushStatus) => void): Unsubscribe;
  onNavigate(handler: (to: Navigate) => void): Unsubscribe;
  onBackgroundState(handler: (state: BackgroundState) => void): Unsubscribe;
  /** Reminders came due, or went (dismissed, snoozed, done). */
  onRemindersChanged(handler: (event: RemindersChangedEvent) => void): Unsubscribe;
  /**
   * Quit from outside the page (the tray): the page quits as for its own
   * Quit, asking first when an unsent message would be lost.
   */
  onQuitRequested(handler: () => void): Unsubscribe;
}
