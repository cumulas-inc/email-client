// React Query hooks for contacts. Keys come from `qk` and start with the
// "contacts" DataKind, so backend data-changed events refresh them.

import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useBackend } from "../../api/context";
import { qk } from "../../api/queries";
import type { ContactDraft } from "../../api/types";

/** Contacts sorted by name, optionally limited to one account and a search text. */
export function useContacts(accountId: string | null, text: string) {
  const backend = useBackend();
  const search = text.trim();
  return useQuery({
    queryKey: qk.contacts(accountId ?? undefined, search),
    queryFn: () => backend.listContacts(accountId ?? undefined, search || undefined),
    // Keep showing the previous results while a new search runs.
    placeholderData: keepPreviousData,
  });
}

/**
 * Refreshes every contacts query. Returned from onSuccess so `mutateAsync`
 * resolves only once the lists show the change.
 */
function useInvalidateContacts() {
  const client = useQueryClient();
  return () => client.invalidateQueries({ queryKey: qk.root("contacts") });
}

export function useCreateContact() {
  const backend = useBackend();
  const invalidate = useInvalidateContacts();
  return useMutation({
    mutationFn: (draft: ContactDraft) => backend.createContact(draft),
    onSuccess: () => invalidate(),
  });
}

export function useUpdateContact() {
  const backend = useBackend();
  const invalidate = useInvalidateContacts();
  return useMutation({
    mutationFn: ({ id, draft }: { id: string; draft: ContactDraft }) =>
      backend.updateContact(id, draft),
    onSuccess: () => invalidate(),
  });
}

export function useDeleteContact() {
  const backend = useBackend();
  const invalidate = useInvalidateContacts();
  return useMutation({
    mutationFn: (id: string) => backend.deleteContact(id),
    onSuccess: () => invalidate(),
  });
}
