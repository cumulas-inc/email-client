import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Backend, Unsubscribe } from "./backend";
import type { DataChangedEvent, SyncStatus } from "./types";

function subscribe<T>(event: string, handler: (payload: T) => void): Unsubscribe {
  let unlisten: (() => void) | null = null;
  let cancelled = false;
  void listen<T>(event, (e) => handler(e.payload)).then((fn) => {
    if (cancelled) fn();
    else unlisten = fn;
  });
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

/** Backend implemented by the Rust core through Tauri commands. */
export const tauriBackend: Backend = {
  getAppInfo: () => invoke("get_app_info"),
  openLink: (url) => invoke("open_link", { url }),

  listAccounts: () => invoke("list_accounts"),
  addImapAccount: (input) => invoke("add_imap_account", { input }),
  addMicrosoftAccount: (loginHint) =>
    invoke("add_microsoft_account", { loginHint: loginHint ?? null }),
  cancelMicrosoftSignIn: () => invoke("cancel_microsoft_sign_in"),
  updateImapAccount: (accountId, update) => invoke("update_imap_account", { accountId, update }),
  updateAccount: (accountId, displayName, color) =>
    invoke("update_account", {
      accountId,
      displayName: displayName ?? null,
      color: color ?? null,
    }),
  removeAccount: (accountId) => invoke("remove_account", { accountId }),
  getSettings: () => invoke("get_settings"),
  updateSettings: (settings) => invoke("update_settings", { settings }),
  syncAccount: (accountId) => invoke("sync_account", { accountId }),
  syncAll: () => invoke("sync_all"),
  getSyncStatuses: () => invoke("get_sync_statuses"),

  listFolders: (accountId) => invoke("list_folders", { accountId: accountId ?? null }),
  listMessages: (folderId, query) => invoke("list_messages", { folderId, query }),
  searchMessages: (text, accountId) =>
    invoke("search_messages", { text, accountId: accountId ?? null }),
  getMessage: (messageId, allowRemote) =>
    invoke("get_message", { messageId, allowRemote: allowRemote ?? false }),
  setRead: (messageIds, read) => invoke("set_read", { messageIds, read }),
  setFlagged: (messageIds, flagged) => invoke("set_flagged", { messageIds, flagged }),
  moveMessages: (messageIds, folderId) => invoke("move_messages", { messageIds, folderId }),
  archiveMessages: (messageIds) => invoke("archive_messages", { messageIds }),
  deleteMessages: (messageIds) => invoke("delete_messages", { messageIds }),
  sendMessage: (message) => invoke("send_message", { message }),
  suggestAddresses: (text) => invoke("suggest_addresses", { text }),
  saveAttachment: (messageId, attachmentId) =>
    invoke("save_attachment", { messageId, attachmentId }),
  openAttachment: (messageId, attachmentId) =>
    invoke("open_attachment", { messageId, attachmentId }),

  listCalendars: () => invoke("list_calendars"),
  listEvents: (start, end, calendarIds) =>
    invoke("list_events", { start, end, calendarIds: calendarIds ?? null }),
  refreshEvents: (start, end) => invoke("refresh_events", { start, end }),
  createEvent: (draft) => invoke("create_event", { draft }),
  updateEvent: (eventId, draft) => invoke("update_event", { eventId, draft }),
  deleteEvent: (eventId) => invoke("delete_event", { eventId }),
  respondToEvent: (eventId, response, comment, sendResponse) =>
    invoke("respond_to_event", { eventId, response, comment, sendResponse }),

  listContacts: (accountId, text) =>
    invoke("list_contacts", { accountId: accountId ?? null, text: text ?? null }),
  createContact: (draft) => invoke("create_contact", { draft }),
  updateContact: (contactId, draft) => invoke("update_contact", { contactId, draft }),
  deleteContact: (contactId) => invoke("delete_contact", { contactId }),

  listTaskLists: () => invoke("list_task_lists"),
  listTasks: (listId, includeCompleted) => invoke("list_tasks", { listId, includeCompleted }),
  createTask: (draft) => invoke("create_task", { draft }),
  updateTask: (taskId, patch) => invoke("update_task", { taskId, patch }),
  deleteTask: (taskId) => invoke("delete_task", { taskId }),

  onDataChanged: (handler) => subscribe<DataChangedEvent>("data-changed", handler),
  onSyncStatus: (handler) => subscribe<SyncStatus>("sync-status", handler),
};
