// Organization setup's queries and mutations. Nothing of setup is kept
// across openings of the dialog (gcTime 0): each opening asks again, and the
// sign-in itself lives only in the backend's memory.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useBackend } from "../../../api/context";
import { qk } from "../../../api/queries";
import type { OrgSetupChoices, OrgSetupScriptRequest } from "../../../api/types";

/** Under "protection", like the info: making changes refreshes an open script too. */
function scriptKey(accountId: string, request: OrgSetupScriptRequest) {
  return [...qk.root("protection"), "org-setup-script", accountId, request] as const;
}

/** Whether automatic setup is available, and signed in, for the account's organization. */
export function useOrgSetupInfo(accountId: string, enabled = true) {
  const backend = useBackend();
  return useQuery({
    queryKey: qk.orgSetupInfo(accountId),
    queryFn: () => backend.getOrgSetupInfo(accountId),
    enabled,
    gcTime: 0,
  });
}

/** Signs an administrator in with the setup app; the organization's info then says who. */
export function useOrgSetupSignIn() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => backend.orgSetupSignIn(accountId),
    // A failed sign-in ends the one there was too.
    onSettled: (_admin, _error, accountId) =>
      void client.invalidateQueries({ queryKey: qk.orgSetupInfo(accountId) }),
    gcTime: 0,
  });
}

/** Reads the organization's setup: a mutation, as reading is what the administrator asks for. */
export function useReadOrgSetup() {
  const backend = useBackend();
  return useMutation({
    mutationFn: (accountId: string) => backend.readOrgSetup(accountId),
    gcTime: 0,
  });
}

/** The changes the choices make, from the last read. */
export function usePlanOrgSetup() {
  const backend = useBackend();
  return useMutation({
    mutationFn: ({ accountId, choices }: { accountId: string; choices: OrgSetupChoices }) =>
      backend.planOrgSetup(accountId, choices),
    gcTime: 0,
  });
}

/**
 * Makes the confirmed changes. The backend's protection event refreshes
 * people's checks; the organization's info (when it changed, whether still
 * signed in) is asked for again here as well.
 */
export function useApplyOrgSetup() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      accountId,
      choices,
      confirmed,
    }: {
      accountId: string;
      choices: OrgSetupChoices;
      confirmed: string[];
    }) => backend.applyOrgSetup(accountId, choices, confirmed),
    onSettled: (_result, _error, { accountId }) =>
      void client.invalidateQueries({ queryKey: qk.orgSetupInfo(accountId) }),
    gcTime: 0,
  });
}

/** The PowerShell script and the portal steps for `request`, filled in; works without signing in. */
export function useOrgSetupScript(
  accountId: string,
  request: OrgSetupScriptRequest,
  enabled = true,
) {
  const backend = useBackend();
  return useQuery({
    queryKey: scriptKey(accountId, request),
    queryFn: () => backend.getOrgSetupScript(accountId, request),
    enabled,
    gcTime: 0,
  });
}
