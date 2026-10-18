// Account mutations for the settings and add-account flows. Keys come from
// the shared `qk` so backend "data-changed" events keep them fresh.

import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { useBackend } from "../../api/context";
import { qk, storeProtectionSetup } from "../../api/queries";
import type {
  Account,
  CheckItem,
  DataKind,
  ImapAccountUpdate,
  NewImapAccount,
  PermissionRequest,
  SyncStatus,
} from "../../api/types";
import { noteFeatureTurnedOn } from "./featureToasts";

const ALL_KINDS: DataKind[] = [
  "accounts",
  "mail",
  "calendar",
  "contacts",
  "tasks",
  "signatures",
  "protection",
];

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

// --- Protection -------------------------------------------------------------------

/** How often a waiting check looks for its test messages. */
export const CHECK_POLL_MS = 10_000;

/** "Refresh": lists the account's sensitivity labels from Microsoft 365 now. */
export function useRefreshLabels() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => backend.refreshSensitivityLabels(accountId),
    onSuccess: (setup) => storeProtectionSetup(client, setup),
  });
}

/** "Check": sends the account test messages for the items. */
export function useCheckProtection() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ accountId, items }: { accountId: string; items: CheckItem[] }) =>
      backend.checkProtection(accountId, items),
    onSuccess: (setup) => storeProtectionSetup(client, setup),
    // A failed send can still leave earlier items waiting.
    onError: (_error, { accountId }) =>
      void client.invalidateQueries({ queryKey: qk.protectionSetup(accountId) }),
  });
}

/**
 * While the account's check waits for its test messages, looks for them
 * every 10 seconds; stops when it's done or the caller goes.
 */
export function useProtectionCheckPolling(accountId: string, checking: boolean): void {
  const backend = useBackend();
  const client = useQueryClient();
  useEffect(() => {
    if (!checking) return;
    let stopped = false;
    const timer = window.setInterval(() => {
      backend
        .refreshProtectionCheck(accountId)
        .then((setup) => {
          if (!stopped) storeProtectionSetup(client, setup);
        })
        // The next look tries again; the check's own timeout ends it.
        .catch(() => undefined);
    }, CHECK_POLL_MS);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, [backend, client, accountId, checking]);
}
