// In-memory implementation of the Backend, mirroring the Rust service's
// behaviour closely enough for UI development, tests and demos.

import type { Backend, Unsubscribe } from "../backend";
import type {
  Account,
  BackendError,
  CalendarEvent,
  Contact,
  DataChangedEvent,
  DataKind,
  ErrorCode,
  Message,
  MessageSummary,
  Settings,
  SyncStatus,
  Task,
} from "../types";
import { type MockState, createDemoState, recount } from "./data";

const DEFAULT_SETTINGS: Settings = {
  theme: "system",
  readingPane: "right",
  remoteContent: "block",
  markReadAfterSeconds: 0,
  syncIntervalMinutes: 5,
  microsoftClientId: null,
  microsoftTenant: null,
  notificationsEnabled: true,
};

function fail(code: ErrorCode, message: string, details?: BackendError["details"]): never {
  const error: BackendError = { code, message, ...(details ? { details } : {}) };
  throw error;
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

function summary(m: Message): MessageSummary {
  return {
    id: m.id,
    accountId: m.accountId,
    folderId: m.folderId,
    remoteId: m.remoteId,
    conversationId: m.conversationId,
    subject: m.subject,
    from: m.from,
    to: m.to,
    cc: m.cc,
    preview: m.preview,
    receivedAt: m.receivedAt,
    isRead: m.isRead,
    isFlagged: m.isFlagged,
    isDraft: m.isDraft,
    hasAttachments: m.hasAttachments,
    importance: m.importance,
  };
}

const REMOTE_IMG = /<img\b[^>]*\bsrc=["']https?:[^"']*["'][^>]*>/gi;

function newestFirst(a: { receivedAt: string }, b: { receivedAt: string }) {
  return b.receivedAt.localeCompare(a.receivedAt);
}

export interface MockOptions {
  /** Artificial latency in ms for every call (default 0). */
  latency?: number;
  /** Start without any accounts (shows onboarding). */
  empty?: boolean;
}

export class MockBackend implements Backend {
  private state: MockState;
  private settings: Settings = { ...DEFAULT_SETTINGS };
  private dataHandlers = new Set<(e: DataChangedEvent) => void>();
  private syncHandlers = new Set<(s: SyncStatus) => void>();
  private seq = 1000;
  private latency: number;
  /** Messages sent through the mock, for tests. */
  readonly sent: Parameters<Backend["sendMessage"]>[0][] = [];

  constructor(options: MockOptions = {}) {
    this.latency = options.latency ?? 0;
    this.state = options.empty ? emptyState() : createDemoState();
  }

  private async delay(): Promise<void> {
    if (this.latency > 0) await new Promise((r) => setTimeout(r, this.latency));
  }

  private id(prefix: string): string {
    this.seq += 1;
    return `${prefix}-${this.seq}`;
  }

  private emit(kinds: DataKind[], accountId: string | null = null): void {
    const event = { kinds, accountId };
    queueMicrotask(() => this.dataHandlers.forEach((h) => h(event)));
  }

  private message(id: string): Message {
    return this.state.messages.find((m) => m.id === id) ?? fail("notFound", "message not found");
  }

  private account(id: string): Account {
    return this.state.accounts.find((a) => a.id === id) ?? fail("notFound", "account not found");
  }

  // --- app --------------------------------------------------------------------

  async getAppInfo() {
    return { version: "0.1.0-demo", persistentSecrets: true, microsoftClientIdBuiltIn: true };
  }

  async openLink(url: string) {
    window.open(url, "_blank", "noopener,noreferrer");
  }

  // --- accounts & settings ------------------------------------------------------

  async listAccounts() {
    await this.delay();
    return clone(this.state.accounts);
  }

  async addImapAccount(input: Parameters<Backend["addImapAccount"]>[0]) {
    await this.delay();
    if (!input.email.includes("@")) fail("invalidInput", "Enter a valid email address.");
    if (input.password === "wrong")
      fail("authRequired", "The server rejected the user name or password.");
    if (this.state.accounts.some((a) => a.email.toLowerCase() === input.email.toLowerCase())) {
      fail("invalidInput", `${input.email} has already been added.`);
    }
    const account: Account = {
      id: this.id("acct"),
      kind: "imap",
      displayName: input.displayName || input.email,
      email: input.email,
      color: "#038387",
      settings: { kind: "imap", ...input.settings },
      createdAt: new Date().toISOString(),
      lastSyncedAt: new Date().toISOString(),
      syncError: null,
    };
    this.state.accounts.push(account);
    for (const [name, role] of [
      ["Inbox", "inbox"],
      ["Drafts", "drafts"],
      ["Sent", "sent"],
      ["Archive", "archive"],
      ["Junk", "junk"],
      ["Trash", "trash"],
    ] as const) {
      const id = this.id("folder");
      this.state.folders.push({
        id,
        accountId: account.id,
        remoteId: id,
        parentId: null,
        name,
        role,
        unreadCount: 0,
        totalCount: 0,
      });
    }
    this.state.calendars.push({
      id: this.id("cal"),
      accountId: account.id,
      remoteId: "local:calendar",
      name: "Calendar",
      color: account.color,
      canEdit: true,
      isDefault: true,
    });
    this.state.taskLists.push({
      id: this.id("tl"),
      accountId: account.id,
      remoteId: "local:tasks",
      name: "Tasks",
      isDefault: true,
    });
    this.emit(["accounts", "mail", "calendar", "tasks"], account.id);
    return clone(account);
  }

  async addMicrosoftAccount(loginHint?: string) {
    await this.delay();
    const hint = loginHint?.trim() ?? "";
    // Demo hooks: "admin@..." simulates a tenant that blocks user consent,
    // "cancel@..." a cancelled browser sign-in.
    if (hint.startsWith("admin")) {
      const tenant = hint.split("@")[1] ?? "organizations";
      fail(
        "adminConsentRequired",
        "Your organization requires an administrator to approve Cumulas Mail before you can sign in.",
        {
          tenant,
          adminConsentUrl: `https://login.microsoftonline.com/${tenant}/v2.0/adminconsent?client_id=00000000-0000-0000-0000-000000000000`,
        },
      );
    }
    if (hint.startsWith("cancel")) fail("signInCancelled", "Sign-in was cancelled.");
    const email = hint.includes("@") ? hint : "new.user@contoso.example";
    const account: Account = {
      id: this.id("acct"),
      kind: "microsoft",
      displayName: email.split("@")[0] ?? email,
      email,
      color: "#ca5010",
      settings: { kind: "microsoft", tenantId: "demo", userId: this.id("user") },
      createdAt: new Date().toISOString(),
      lastSyncedAt: null,
      syncError: null,
    };
    this.state.accounts.push(account);
    this.state.folders.push({
      id: this.id("folder"),
      accountId: account.id,
      remoteId: "inbox",
      parentId: null,
      name: "Inbox",
      role: "inbox",
      unreadCount: 0,
      totalCount: 0,
    });
    this.emit(["accounts", "mail"], account.id);
    return clone(account);
  }

  async updateAccount(accountId: string, displayName?: string, color?: string) {
    await this.delay();
    const account = this.account(accountId);
    if (displayName?.trim()) account.displayName = displayName.trim();
    if (color) account.color = color;
    this.emit(["accounts"], accountId);
    return clone(account);
  }

  async removeAccount(accountId: string) {
    await this.delay();
    this.account(accountId);
    const s = this.state;
    s.accounts = s.accounts.filter((a) => a.id !== accountId);
    s.folders = s.folders.filter((f) => f.accountId !== accountId);
    s.messages = s.messages.filter((m) => m.accountId !== accountId);
    s.calendars = s.calendars.filter((c) => c.accountId !== accountId);
    s.events = s.events.filter((e) => e.accountId !== accountId);
    s.contacts = s.contacts.filter((c) => c.accountId !== accountId);
    s.taskLists = s.taskLists.filter((l) => l.accountId !== accountId);
    s.tasks = s.tasks.filter((t) => t.accountId !== accountId);
    this.emit(["accounts", "mail", "calendar", "contacts", "tasks"]);
  }

  async getSettings() {
    await this.delay();
    return clone(this.settings);
  }

  async updateSettings(settings: Settings) {
    await this.delay();
    if (settings.syncIntervalMinutes < 1)
      fail("invalidInput", "Sync interval must be at least one minute.");
    this.settings = clone(settings);
    return clone(settings);
  }

  async syncAccount(accountId: string) {
    const account = this.account(accountId);
    const status = (state: SyncStatus["state"], message: string | null): SyncStatus => ({
      accountId,
      state,
      message,
      lastSyncedAt: account.lastSyncedAt,
    });
    this.syncHandlers.forEach((h) => h(status("syncing", "Syncing Inbox…")));
    setTimeout(() => {
      account.lastSyncedAt = new Date().toISOString();
      this.syncHandlers.forEach((h) => h(status("idle", null)));
      this.emit(["accounts"], accountId);
    }, 600);
  }

  async syncAll() {
    for (const a of this.state.accounts) await this.syncAccount(a.id);
  }

  async getSyncStatuses() {
    return this.state.accounts.map((a) => ({
      accountId: a.id,
      state: a.syncError ? ("error" as const) : ("idle" as const),
      message: a.syncError,
      lastSyncedAt: a.lastSyncedAt,
    }));
  }

  // --- mail ---------------------------------------------------------------------

  async listFolders(accountId?: string) {
    await this.delay();
    const rank = ["inbox", "drafts", "sent", "archive", "junk", "trash", "outbox", "other"];
    return clone(
      this.state.folders
        .filter((f) => !accountId || f.accountId === accountId)
        .sort(
          (a, b) =>
            rank.indexOf(a.role) - rank.indexOf(b.role) ||
            a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
        ),
    );
  }

  async listMessages(folderId: string, query: Parameters<Backend["listMessages"]>[1]) {
    await this.delay();
    const all = this.state.messages
      .filter((m) => m.folderId === folderId)
      .filter((m) => !query.unreadOnly || !m.isRead)
      .filter((m) => !query.flaggedOnly || m.isFlagged)
      .sort(newestFirst);
    const limit = query.limit || 100;
    return {
      items: all.slice(query.offset, query.offset + limit).map(summary),
      total: all.length,
    };
  }

  async searchMessages(text: string, accountId?: string) {
    await this.delay();
    const terms = text.toLowerCase().split(/\s+/).filter(Boolean);
    if (terms.length === 0) return [];
    return this.state.messages
      .filter((m) => !accountId || m.accountId === accountId)
      .filter((m) => {
        const haystack = [
          m.subject,
          m.from?.name,
          m.from?.address,
          ...m.to.map((t) => `${t.name ?? ""} ${t.address}`),
          m.bodyText,
        ]
          .join(" ")
          .toLowerCase();
        return terms.every((t) => haystack.includes(t));
      })
      .sort(newestFirst)
      .map(summary);
  }

  async getMessage(messageId: string, allowRemote = false) {
    await this.delay();
    const m = clone(this.message(messageId));
    const allow = allowRemote || this.settings.remoteContent === "allow";
    if (m.bodyHtml && !allow && REMOTE_IMG.test(m.bodyHtml)) {
      m.bodyHtml = m.bodyHtml.replace(REMOTE_IMG, "");
      m.hasBlockedContent = true;
    }
    REMOTE_IMG.lastIndex = 0;
    return m;
  }

  async setRead(messageIds: string[], read: boolean) {
    await this.delay();
    for (const id of messageIds) this.message(id).isRead = read;
    recount(this.state);
    this.emit(["mail"]);
  }

  async setFlagged(messageIds: string[], flagged: boolean) {
    await this.delay();
    for (const id of messageIds) this.message(id).isFlagged = flagged;
    this.emit(["mail"]);
  }

  async moveMessages(messageIds: string[], folderId: string) {
    await this.delay();
    const destination =
      this.state.folders.find((f) => f.id === folderId) ?? fail("notFound", "folder not found");
    for (const id of messageIds) {
      const m = this.message(id);
      if (m.accountId !== destination.accountId) {
        fail("unsupported", "Messages can only be moved between folders of the same account.");
      }
      m.folderId = folderId;
    }
    recount(this.state);
    this.emit(["mail"], destination.accountId);
  }

  async archiveMessages(messageIds: string[]) {
    for (const id of messageIds) {
      const m = this.message(id);
      const archive =
        this.state.folders.find((f) => f.accountId === m.accountId && f.role === "archive") ??
        fail("unsupported", "This account has no Archive folder.");
      await this.moveMessages([id], archive.id);
    }
  }

  async deleteMessages(messageIds: string[]) {
    await this.delay();
    for (const id of messageIds) {
      const m = this.message(id);
      const current = this.state.folders.find((f) => f.id === m.folderId);
      const trash = this.state.folders.find(
        (f) => f.accountId === m.accountId && f.role === "trash",
      );
      if (trash && current && current.role !== "trash" && current.role !== "junk") {
        m.folderId = trash.id;
      } else {
        this.state.messages = this.state.messages.filter((x) => x.id !== id);
      }
    }
    recount(this.state);
    this.emit(["mail"]);
  }

  async sendMessage(message: Parameters<Backend["sendMessage"]>[0]) {
    await this.delay();
    if (message.to.length + message.cc.length + message.bcc.length === 0) {
      fail("invalidInput", "Add at least one recipient.");
    }
    const account = this.account(message.accountId);
    this.sent.push(clone(message));
    const sentFolder = this.state.folders.find(
      (f) => f.accountId === account.id && f.role === "sent",
    );
    if (sentFolder) {
      const id = this.id("msg");
      const text = message.bodyHtml
        .replace(/<[^>]+>/g, " ")
        .replace(/\s+/g, " ")
        .trim();
      this.state.messages.push({
        id,
        accountId: account.id,
        folderId: sentFolder.id,
        remoteId: id,
        conversationId: null,
        subject: message.subject,
        from: { name: account.displayName, address: account.email },
        to: message.to,
        cc: message.cc,
        preview: text.slice(0, 180),
        receivedAt: new Date().toISOString(),
        isRead: true,
        isFlagged: false,
        isDraft: false,
        hasAttachments: message.attachments.length > 0,
        importance: message.importance ?? "normal",
        bcc: message.bcc,
        replyTo: [],
        bodyHtml: message.bodyHtml,
        bodyText: text,
        hasBlockedContent: false,
        attachments: message.attachments.map((a, i) => ({
          id: `${id}-att-${i}`,
          name: a.name,
          contentType: a.contentType,
          size: Math.floor((a.dataBase64.length * 3) / 4),
          isInline: false,
          contentId: null,
        })),
        internetMessageId: `<${id}@demo.example>`,
        inReplyTo: null,
        references: [],
      });
      recount(this.state);
    }
    this.emit(["mail"], account.id);
  }

  async suggestAddresses(text: string) {
    await this.delay();
    const q = text.trim().toLowerCase();
    if (!q) return [];
    const seen = new Set<string>();
    const out: { name: string | null; address: string }[] = [];
    const add = (name: string | null, address: string) => {
      const key = address.toLowerCase();
      if (seen.has(key)) return;
      if (!(name ?? "").toLowerCase().includes(q) && !key.includes(q)) return;
      seen.add(key);
      out.push({ name, address });
    };
    for (const c of this.state.contacts) for (const e of c.emails) add(c.displayName, e.address);
    for (const m of this.state.messages) if (m.from) add(m.from.name, m.from.address);
    return out.slice(0, 8);
  }

  async saveAttachment(messageId: string, attachmentId: string) {
    await this.delay();
    const att =
      this.message(messageId).attachments.find((a) => a.id === attachmentId) ??
      fail("notFound", "attachment not found");
    return `/home/alex/Downloads/${att.name}`;
  }

  async openAttachment(messageId: string, attachmentId: string) {
    await this.delay();
    if (!this.message(messageId).attachments.some((a) => a.id === attachmentId)) {
      fail("notFound", "attachment not found");
    }
  }

  // --- calendar -------------------------------------------------------------------

  async listCalendars() {
    await this.delay();
    return clone(this.state.calendars);
  }

  async listEvents(start: string, end: string, calendarIds?: string[]) {
    await this.delay();
    return clone(
      this.state.events
        .filter((e) => e.start < end && e.end > start)
        .filter((e) => !calendarIds || calendarIds.includes(e.calendarId))
        .sort((a, b) => a.start.localeCompare(b.start) || b.end.localeCompare(a.end)),
    );
  }

  async refreshEvents() {
    await this.delay();
  }

  private eventFromDraft(id: string, draft: Parameters<Backend["createEvent"]>[0]): CalendarEvent {
    if (!draft.subject.trim()) fail("invalidInput", "Add a title for the event.");
    if (draft.end < draft.start) fail("invalidInput", "The event ends before it starts.");
    const calendar =
      this.state.calendars.find((c) => c.id === draft.calendarId) ??
      fail("notFound", "calendar not found");
    if (!calendar.canEdit) fail("invalidInput", "This calendar is read-only.");
    const account = this.account(calendar.accountId);
    return {
      id,
      accountId: calendar.accountId,
      calendarId: calendar.id,
      remoteId: id,
      subject: draft.subject.trim(),
      location: draft.location?.trim() || null,
      bodyHtml: draft.bodyHtml ?? null,
      bodyPreview: draft.bodyHtml
        ? draft.bodyHtml
            .replace(/<[^>]+>/g, " ")
            .trim()
            .slice(0, 200)
        : null,
      start: draft.start,
      end: draft.end,
      isAllDay: draft.isAllDay,
      organizer: { name: account.displayName, address: account.email },
      attendees: draft.attendees.map((email) => ({ email, kind: "required", response: "none" })),
      showAs: draft.showAs,
      responseStatus: "organizer",
      isOnlineMeeting: draft.isOnlineMeeting && account.kind === "microsoft",
      onlineMeetingUrl:
        draft.isOnlineMeeting && account.kind === "microsoft" ? `https://meet.example/${id}` : null,
      reminderMinutes: draft.reminderMinutes ?? null,
      isRecurring: false,
      isCancelled: false,
    };
  }

  async createEvent(draft: Parameters<Backend["createEvent"]>[0]) {
    await this.delay();
    const event = this.eventFromDraft(this.id("evt"), draft);
    this.state.events.push(event);
    this.emit(["calendar"], event.accountId);
    return clone(event);
  }

  async updateEvent(eventId: string, draft: Parameters<Backend["updateEvent"]>[1]) {
    await this.delay();
    const index = this.state.events.findIndex((e) => e.id === eventId);
    if (index < 0) fail("notFound", "event not found");
    const existing = this.state.events[index]!;
    const event = {
      ...this.eventFromDraft(eventId, draft),
      isRecurring: existing.isRecurring,
      organizer: existing.organizer,
    };
    this.state.events[index] = event;
    this.emit(["calendar"], event.accountId);
    return clone(event);
  }

  async deleteEvent(eventId: string) {
    await this.delay();
    this.state.events = this.state.events.filter((e) => e.id !== eventId);
    this.emit(["calendar"]);
  }

  async respondToEvent(eventId: string, response: Parameters<Backend["respondToEvent"]>[1]) {
    await this.delay();
    const event =
      this.state.events.find((e) => e.id === eventId) ?? fail("notFound", "event not found");
    if (this.account(event.accountId).kind !== "microsoft") {
      fail("unsupported", "Meeting responses need a Microsoft account.");
    }
    if (response === "decline") {
      this.state.events = this.state.events.filter((e) => e.id !== eventId);
    } else {
      event.responseStatus = response === "accept" ? "accepted" : "tentative";
    }
    this.emit(["calendar"], event.accountId);
    return clone(event);
  }

  // --- contacts -------------------------------------------------------------------

  async listContacts(accountId?: string, text?: string) {
    await this.delay();
    const q = text?.trim().toLowerCase();
    return clone(
      this.state.contacts
        .filter((c) => !accountId || c.accountId === accountId)
        .filter(
          (c) =>
            !q ||
            [c.displayName, c.givenName, c.surname, c.company, ...c.emails.map((e) => e.address)]
              .join(" ")
              .toLowerCase()
              .includes(q),
        )
        .sort((a, b) =>
          a.displayName.localeCompare(b.displayName, undefined, { sensitivity: "base" }),
        ),
    );
  }

  private contactFromDraft(id: string, draft: Parameters<Backend["createContact"]>[0]): Contact {
    if (!draft.displayName.trim()) fail("invalidInput", "Add a name for the contact.");
    this.account(draft.accountId);
    return {
      id,
      accountId: draft.accountId,
      remoteId: id,
      displayName: draft.displayName.trim(),
      givenName: draft.givenName ?? null,
      surname: draft.surname ?? null,
      emails: draft.emails,
      phones: draft.phones,
      company: draft.company ?? null,
      jobTitle: draft.jobTitle ?? null,
      department: draft.department ?? null,
      address: draft.address ?? null,
      birthday: draft.birthday ?? null,
      notes: draft.notes ?? null,
    };
  }

  async createContact(draft: Parameters<Backend["createContact"]>[0]) {
    await this.delay();
    const contact = this.contactFromDraft(this.id("contact"), draft);
    this.state.contacts.push(contact);
    this.emit(["contacts"], contact.accountId);
    return clone(contact);
  }

  async updateContact(contactId: string, draft: Parameters<Backend["updateContact"]>[1]) {
    await this.delay();
    const index = this.state.contacts.findIndex((c) => c.id === contactId);
    if (index < 0) fail("notFound", "contact not found");
    if (this.state.contacts[index]!.accountId !== draft.accountId) {
      fail("unsupported", "Contacts cannot be moved between accounts.");
    }
    const contact = this.contactFromDraft(contactId, draft);
    this.state.contacts[index] = contact;
    this.emit(["contacts"], contact.accountId);
    return clone(contact);
  }

  async deleteContact(contactId: string) {
    await this.delay();
    this.state.contacts = this.state.contacts.filter((c) => c.id !== contactId);
    this.emit(["contacts"]);
  }

  // --- tasks ----------------------------------------------------------------------

  async listTaskLists() {
    await this.delay();
    return clone(this.state.taskLists);
  }

  async listTasks(listId: string | null, includeCompleted: boolean) {
    await this.delay();
    return clone(
      this.state.tasks
        .filter((t) => !listId || t.listId === listId)
        .filter((t) => includeCompleted || t.status !== "completed")
        .sort(
          (a, b) =>
            Number(a.status === "completed") - Number(b.status === "completed") ||
            Number(a.dueDate === null) - Number(b.dueDate === null) ||
            (a.dueDate ?? "").localeCompare(b.dueDate ?? "") ||
            b.createdAt.localeCompare(a.createdAt),
        ),
    );
  }

  async createTask(draft: Parameters<Backend["createTask"]>[0]) {
    await this.delay();
    if (!draft.title.trim()) fail("invalidInput", "Add a title for the task.");
    const list =
      this.state.taskLists.find((l) => l.id === draft.listId) ??
      fail("notFound", "task list not found");
    const now = new Date().toISOString();
    const id = this.id("task");
    const task: Task = {
      id,
      accountId: list.accountId,
      listId: list.id,
      remoteId: id,
      title: draft.title.trim(),
      notes: draft.notes?.trim() || null,
      status: "notStarted",
      importance: draft.importance ?? "normal",
      dueDate: draft.dueDate ?? null,
      completedAt: null,
      createdAt: now,
      updatedAt: now,
    };
    this.state.tasks.push(task);
    this.emit(["tasks"], task.accountId);
    return clone(task);
  }

  async updateTask(taskId: string, patch: Parameters<Backend["updateTask"]>[1]) {
    await this.delay();
    const task =
      this.state.tasks.find((t) => t.id === taskId) ?? fail("notFound", "task not found");
    if (patch.title !== undefined) {
      if (!patch.title.trim()) fail("invalidInput", "A task needs a title.");
      task.title = patch.title.trim();
    }
    if (patch.clearNotes) task.notes = null;
    else if (patch.notes !== undefined) task.notes = patch.notes;
    if (patch.importance) task.importance = patch.importance;
    if (patch.clearDueDate) task.dueDate = null;
    else if (patch.dueDate) task.dueDate = patch.dueDate;
    if (patch.status) {
      if (patch.status === "completed" && task.status !== "completed") {
        task.completedAt = new Date().toISOString();
      } else if (patch.status !== "completed") {
        task.completedAt = null;
      }
      task.status = patch.status;
    }
    task.updatedAt = new Date().toISOString();
    this.emit(["tasks"], task.accountId);
    return clone(task);
  }

  async deleteTask(taskId: string) {
    await this.delay();
    this.state.tasks = this.state.tasks.filter((t) => t.id !== taskId);
    this.emit(["tasks"]);
  }

  // --- events ---------------------------------------------------------------------

  onDataChanged(handler: (event: DataChangedEvent) => void): Unsubscribe {
    this.dataHandlers.add(handler);
    return () => this.dataHandlers.delete(handler);
  }

  onSyncStatus(handler: (status: SyncStatus) => void): Unsubscribe {
    this.syncHandlers.add(handler);
    return () => this.syncHandlers.delete(handler);
  }
}

function emptyState(): MockState {
  return {
    accounts: [],
    folders: [],
    messages: [],
    calendars: [],
    events: [],
    contacts: [],
    taskLists: [],
    tasks: [],
  };
}
