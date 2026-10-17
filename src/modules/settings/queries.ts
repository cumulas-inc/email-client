// Account mutations for the settings and add-account flows. Keys come from
// the shared `qk` so backend "data-changed" events keep them fresh.

import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";

import { useBackend } from "../../api/context";
import { qk } from "../../api/queries";
import type {
  Account,
  DataKind,
  ImapAccountUpdate,
  NewImapAccount,
  PermissionRequest,
  SyncStatus,
} from "../../api/types";
import { noteFeatureTurnedOn } from "./featureToasts";

const ALL_KINDS: DataKind[] = ["accounts", "mail", "calendar", "contacts", "tasks", "signatures"];

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
    mutationFn: ({
      loginHint,
      permissions,
    }: {
      loginHint?: string;
      permissions?: PermissionRequest;
    }) => backend.addMicrosoftAccount(loginHint, permissions),
    onSuccess: (account, { permissions }) => {
      // Before the account changes anywhere, so its own toast is the only one.
      if (permissions?.kind === "feature") noteFeatureTurnedOn(account.id, permissions.feature);
      // A sign-in checks refused features again, as Check again does: the
      // notice going away says one is back, a sync that's refused again says so.
      const before = client
        .getQueryData<Account[]>(qk.accounts())
        ?.find((a) => a.id === account.id);
      for (const f of before?.refusedFeatures ?? []) noteFeatureTurnedOn(account.id, f);
      upsertAccount(client, account);
      refreshEverything(client);
    },
  });
}

/** Where an administrator approves the account's organization (Ask IT). */
export function useAdminConsentLink(accountId: string, enabled = true) {
  const backend = useBackend();
  return useQuery({
    queryKey: qk.adminConsent(accountId),
    queryFn: () => backend.microsoftAdminConsent(accountId),
    enabled,
    retry: false,
  });
}

/** "Check again": asks Microsoft 365 again for the features it refused. */
export function useRetryMicrosoftFeatures() {
  const backend = useBackend();
  return useMutation({
    mutationFn: (accountId: string) => backend.retryMicrosoftFeatures(accountId),
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
    // Show the new name or color straight away; roll back if it fails.
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
