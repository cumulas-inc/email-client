import type {
  Account,
  AppInfo,
  Calendar,
  CalendarEvent,
  Contact,
  ContactDraft,
  DataChangedEvent,
  EmailAddress,
  EventDraft,
  EventResponse,
  Folder,
  Message,
  MessagePage,
  MessageQuery,
  MessageSummary,
  NewImapAccount,
  OutgoingMessage,
  Settings,
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

  // Accounts & settings
  listAccounts(): Promise<Account[]>;
  addImapAccount(input: NewImapAccount): Promise<Account>;
  /** Opens the system browser for Microsoft sign-in and resolves once done. */
  addMicrosoftAccount(loginHint?: string): Promise<Account>;
  updateAccount(accountId: string, displayName?: string, color?: string): Promise<Account>;
  removeAccount(accountId: string): Promise<void>;
  getSettings(): Promise<Settings>;
  updateSettings(settings: Settings): Promise<Settings>;
  syncAccount(accountId: string): Promise<void>;
  syncAll(): Promise<void>;
  getSyncStatuses(): Promise<SyncStatus[]>;

  // Mail
  listFolders(accountId?: string): Promise<Folder[]>;
  listMessages(folderId: string, query: MessageQuery): Promise<MessagePage>;
  searchMessages(text: string, accountId?: string): Promise<MessageSummary[]>;
  getMessage(messageId: string, allowRemote?: boolean): Promise<Message>;
  setRead(messageIds: string[], read: boolean): Promise<void>;
  setFlagged(messageIds: string[], flagged: boolean): Promise<void>;
  moveMessages(messageIds: string[], folderId: string): Promise<void>;
  archiveMessages(messageIds: string[]): Promise<void>;
  /** Moves to Deleted Items, or deletes permanently when already there. */
  deleteMessages(messageIds: string[]): Promise<void>;
  sendMessage(message: OutgoingMessage): Promise<void>;
  suggestAddresses(text: string): Promise<EmailAddress[]>;
  /** Resolves to the saved path, or null if the user cancelled. */
  saveAttachment(messageId: string, attachmentId: string): Promise<string | null>;
  openAttachment(messageId: string, attachmentId: string): Promise<void>;

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
}
