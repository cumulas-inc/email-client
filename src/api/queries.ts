// Shared React Query keys and hooks. Every key starts with a DataKind so a
// backend "data-changed" event can invalidate exactly the affected queries.

import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";

import type { Backend } from "./backend";
import { useBackend } from "./context";
import type {
  AccountSignature,
  ActiveReminder,
  BackgroundState,
  DataKind,
  ReplyKind,
  Settings,
  SnoozeFor,
} from "./types";

export const qk = {
  root: (kind: DataKind) => [kind] as const,

  accounts: () => ["accounts", "list"] as const,
  settings: () => ["accounts", "settings"] as const,
  fonts: () => ["accounts", "fonts"] as const,
  appInfo: () => ["accounts", "app-info"] as const,
  syncStatuses: () => ["accounts", "sync-statuses"] as const,
  pushStatuses: () => ["accounts", "push-statuses"] as const,
  backgroundState: () => ["accounts", "background"] as const,

  folders: (accountId?: string) => ["mail", "folders", accountId ?? "all"] as const,
  messages: (folderId: string, filter: string) => ["mail", "messages", folderId, filter] as const,
  message: (messageId: string, allowRemote: boolean) =>
    ["mail", "message", messageId, allowRemote] as const,
  search: (text: string) => ["mail", "search", text] as const,
  suggestions: (text: string) => ["contacts", "suggest", text] as const,

  calendars: () => ["calendar", "calendars"] as const,
  events: (start: string, end: string) => ["calendar", "events", start, end] as const,

  contacts: (accountId?: string, text?: string) =>
    ["contacts", "list", accountId ?? "all", text ?? ""] as const,

  /** Not under a DataKind: the reminders-changed event keeps it up to date. */
  reminders: () => ["reminders", "active"] as const,

  accountSignature: (accountId: string) => ["signatures", "account", accountId] as const,
  composeSignature: (accountId: string, replyKind: ReplyKind | null) =>
    ["signatures", "compose", accountId, replyKind ?? "new"] as const,

  taskLists: () => ["tasks", "lists"] as const,
  tasks: (listId: string | null, includeCompleted: boolean) =>
    ["tasks", "items", listId ?? "all", includeCompleted] as const,
};

export function useAccounts() {
  const backend = useBackend();
  return useQuery({ queryKey: qk.accounts(), queryFn: () => backend.listAccounts() });
}

export function useAppInfo() {
  const backend = useBackend();
  return useQuery({
    queryKey: qk.appInfo(),
    queryFn: () => backend.getAppInfo(),
    staleTime: Infinity,
  });
}

export function useSettings() {
  const backend = useBackend();
  return useQuery({ queryKey: qk.settings(), queryFn: () => backend.getSettings() });
}

/** Installed font families; the list only changes when the app restarts. */
export function useFonts() {
  const backend = useBackend();
  return useQuery({
    queryKey: qk.fonts(),
    queryFn: () => backend.listFonts(),
    staleTime: Infinity,
  });
}

export function useUpdateSettings() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (settings: Settings) => backend.updateSettings(settings),
    onSuccess: (settings) => client.setQueryData(qk.settings(), settings),
  });
}

export function useSyncStatuses() {
  const backend = useBackend();
  return useQuery({ queryKey: qk.syncStatuses(), queryFn: () => backend.getSyncStatuses() });
}

export function usePushStatuses() {
  const backend = useBackend();
  return useQuery({ queryKey: qk.pushStatuses(), queryFn: () => backend.getPushStatuses() });
}

/** Running in the background, starting at login, the tray and notifications. */
export function useBackgroundState() {
  const backend = useBackend();
  return useQuery({ queryKey: qk.backgroundState(), queryFn: () => backend.getBackgroundState() });
}

/** Keeps the background state, and the settings it mirrors, in step with the backend. */
export function applyBackgroundState(client: QueryClient, state: BackgroundState): void {
  client.setQueryData(qk.backgroundState(), state);
  client.setQueryData<Settings>(qk.settings(), (old) =>
    old
      ? {
          ...old,
          runInBackground: state.runInBackground,
          startAtLogin: state.startAtLogin,
          startHidden: state.startHidden,
        }
      : old,
  );
}

export function useSetRunInBackground() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (enabled: boolean) => backend.setRunInBackground(enabled),
    onSuccess: (state) => applyBackgroundState(client, state),
    onError: () => void client.invalidateQueries({ queryKey: qk.backgroundState() }),
  });
}

export function useSetStartAtLogin() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ enabled, hidden }: { enabled: boolean; hidden: boolean }) =>
      backend.setStartAtLogin(enabled, hidden),
    onSuccess: (state) => applyBackgroundState(client, state),
    onError: () => void client.invalidateQueries({ queryKey: qk.backgroundState() }),
  });
}

/** The reminders due now, or snoozed; kept current by the reminders-changed event. */
export function useActiveReminders() {
  const backend = useBackend();
  return useQuery({
    queryKey: qk.reminders(),
    queryFn: () => backend.listActiveReminders(),
    staleTime: Infinity,
  });
}

// Snoozing and dismissing change the reminders through the reminders-changed
// event, which the backend sends as the change is made. Their answers, the
// reminders then active, aren't put in the cache: they come back only after
// the server has been told, by when a newer event (a reminder that came due
// meanwhile) may have replaced that list.

export function useSnoozeReminder() {
  const backend = useBackend();
  return useMutation({
    mutationFn: ({ key, snooze }: { key: string; snooze: SnoozeFor }) =>
      backend.snoozeReminder(key, snooze),
  });
}

export function useDismissReminders() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (keys: string[]) => backend.dismissReminders(keys),
    onMutate: (keys: string[]) => {
      // Gone at once; the reminders-changed event replaces this.
      client.setQueryData<ActiveReminder[]>(qk.reminders(), (old) =>
        old?.filter((r) => !keys.includes(r.key)),
      );
    },
    onError: () => void client.invalidateQueries({ queryKey: qk.reminders() }),
  });
}

// --- Signatures -------------------------------------------------------------------

export function useAccountSignature(accountId: string) {
  const backend = useBackend();
  return useQuery({
    queryKey: qk.accountSignature(accountId),
    queryFn: () => backend.getAccountSignature(accountId),
  });
}

/** The signature a message from the account starts with; also for fetchQuery. */
export function composeSignatureQuery(
  backend: Backend,
  accountId: string,
  replyKind: ReplyKind | null,
) {
  return {
    queryKey: qk.composeSignature(accountId, replyKind),
    queryFn: () => backend.getComposeSignature(accountId, replyKind),
  };
}

export function useComposeSignature(
  accountId: string,
  replyKind: ReplyKind | null,
  enabled = true,
) {
  const backend = useBackend();
  return useQuery({ ...composeSignatureQuery(backend, accountId, replyKind), enabled });
}

function storeSignature(client: QueryClient, signature: AccountSignature) {
  client.setQueryData(qk.accountSignature(signature.accountId), signature);
}

export function useUpdateOwnSignature() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      accountId,
      html,
      onReplies,
    }: {
      accountId: string;
      html: string;
      onReplies: boolean;
    }) => backend.updateOwnSignature(accountId, html, onReplies),
    onSuccess: (signature) => storeSignature(client, signature),
  });
}

export function useCheckOrganizationSignature() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => backend.checkOrganizationSignature(accountId),
    onSuccess: (signature) => storeSignature(client, signature),
  });
}
