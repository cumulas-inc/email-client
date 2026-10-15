// Shared React Query keys and hooks. Every key starts with a DataKind so a
// backend "data-changed" event can invalidate exactly the affected queries.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useBackend } from "./context";
import type { DataKind, Settings } from "./types";

export const qk = {
  root: (kind: DataKind) => [kind] as const,

  accounts: () => ["accounts", "list"] as const,
  settings: () => ["accounts", "settings"] as const,
  fonts: () => ["accounts", "fonts"] as const,
  appInfo: () => ["accounts", "app-info"] as const,
  syncStatuses: () => ["accounts", "sync-statuses"] as const,
  pushStatuses: () => ["accounts", "push-statuses"] as const,

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
