// Account mutations for the settings and add-account flows. Keys come from
// the shared `qk` so backend "data-changed" events keep them fresh.

import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";

import { useBackend } from "../../api/context";
import { qk } from "../../api/queries";
import type {
  Account,
  DataKind,
  ImapAccountUpdate,
  NewImapAccount,
  SyncStatus,
} from "../../api/types";

const ALL_KINDS: DataKind[] = ["accounts", "mail", "calendar", "contacts", "tasks"];

/** Adding or removing an account changes every kind of data. */
function refreshEverything(client: QueryClient) {
  for (const kind of ALL_KINDS) void client.invalidateQueries({ queryKey: qk.root(kind) });
}

function upsertAccount(client: QueryClient, account: Account) {
  client.setQueryData<Account[]>(qk.accounts(), (old) => {
    if (!old) return [account];
    return old.some((a) => a.id === account.id)
      ? old.map((a) => (a.id === account.id ? account : a))
      : [...old, account];
  });
}

export function useAddImapAccount() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: NewImapAccount) => backend.addImapAccount(input),
    onSuccess: (account) => {
      upsertAccount(client, account);
      refreshEverything(client);
    },
  });
}

export function useAddMicrosoftAccount() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (loginHint?: string) => backend.addMicrosoftAccount(loginHint),
    onSuccess: (account) => {
      upsertAccount(client, account);
      refreshEverything(client);
    },
  });
}

interface AccountUpdate {
  accountId: string;
  displayName?: string;
  color?: string;
}

export function useUpdateAccount() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ accountId, displayName, color }: AccountUpdate) =>
      backend.updateAccount(accountId, displayName, color),
    // Show the new name or colour straight away; roll back if it fails.
    onMutate: async ({ accountId, displayName, color }) => {
      await client.cancelQueries({ queryKey: qk.accounts() });
      const previous = client.getQueryData<Account[]>(qk.accounts());
      client.setQueryData<Account[]>(qk.accounts(), (old) =>
        old?.map((a) =>
          a.id === accountId
            ? {
                ...a,
                ...(displayName?.trim() ? { displayName: displayName.trim() } : {}),
                ...(color ? { color } : {}),
              }
            : a,
        ),
      );
      return { previous };
    },
    onError: (_error, _vars, context) => {
      if (context?.previous) client.setQueryData(qk.accounts(), context.previous);
    },
    onSuccess: (account) => upsertAccount(client, account),
  });
}

export function useUpdateImapAccount() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ accountId, update }: { accountId: string; update: ImapAccountUpdate }) =>
      backend.updateImapAccount(accountId, update),
    onSuccess: (account) => {
      upsertAccount(client, account);
      // The check succeeded, so the account's sync error is gone.
      client.setQueryData<SyncStatus[]>(qk.syncStatuses(), (old) =>
        old?.map((s) =>
          s.accountId === account.id && s.state === "error"
            ? { ...s, state: "idle", message: null }
            : s,
        ),
      );
      void client.invalidateQueries({ queryKey: qk.syncStatuses() });
    },
  });
}

export function useRemoveAccount() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => backend.removeAccount(accountId),
    onSuccess: (_result, accountId) => {
      client.setQueryData<Account[]>(qk.accounts(), (old) =>
        old?.filter((a) => a.id !== accountId),
      );
      refreshEverything(client);
    },
  });
}

export function useSyncAccount() {
  const backend = useBackend();
  return useMutation({ mutationFn: (accountId: string) => backend.syncAccount(accountId) });
}
