// React Query hooks for the Mail module. Every key comes from `qk`, so it
// starts with "mail" (or "contacts" for address suggestions) and backend
// "data-changed" events invalidate it.

import {
  type InfiniteData,
  type QueryClient,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import type { Backend } from "../../api/backend";
import { useBackend } from "../../api/context";
import { qk } from "../../api/queries";
import type {
  Folder,
  Message,
  MessagePage,
  MessageSummary,
  OutgoingMessage,
  SearchHit,
  SearchSort,
} from "../../api/types";
import { toast } from "../../components/toast";
import { weekStartsOn } from "../../lib/datetime";
import { ALL_INBOXES, type MailFilter } from "./mailStore";

export const PAGE_SIZE = 50;

/** Offset into a folder, or per-folder offsets for the unified inbox. */
export type PageParam = number | Record<string, number>;

export interface ListPage extends MessagePage {
  next: PageParam | null;
}

export type ListData = InfiniteData<ListPage, PageParam>;

const messagesRoot = ["mail", "messages"] as const;
const searchRoot = ["mail", "search"] as const;
const messageRoot = ["mail", "message"] as const;
const foldersRoot = ["mail", "folders"] as const;

// --- Queries -------------------------------------------------------------------

export function useFolders() {
  const backend = useBackend();
  return useQuery({ queryKey: qk.folders(), queryFn: () => backend.listFolders() });
}

function foldersQuery(backend: Backend) {
  return { queryKey: qk.folders(), queryFn: () => backend.listFolders() };
}

/** Newest-first merge of every account's inbox, paged by per-folder offsets. */
async function fetchUnified(
  backend: Backend,
  client: QueryClient,
  offsets: Record<string, number>,
  filter: MailFilter,
): Promise<ListPage> {
  const folders = await client.ensureQueryData(foldersQuery(backend));
  const inboxes = folders.filter((f) => f.role === "inbox");
  const pages = await Promise.all(
    inboxes.map((f) =>
      backend.listMessages(f.id, {
        offset: offsets[f.id] ?? 0,
        limit: PAGE_SIZE,
        unreadOnly: filter === "unread",
        flaggedOnly: filter === "flagged",
      }),
    ),
  );
  const merged = pages
    .flatMap((p) => p.items)
    .sort((a, b) => b.receivedAt.localeCompare(a.receivedAt))
    .slice(0, PAGE_SIZE);
  const next: Record<string, number> = {};
  let more = false;
  inboxes.forEach((f, i) => {
    const consumed = merged.filter((m) => m.folderId === f.id).length;
    next[f.id] = (offsets[f.id] ?? 0) + consumed;
    if (next[f.id]! < (pages[i]?.total ?? 0)) more = true;
  });
  const total = pages.reduce((sum, p) => sum + p.total, 0);
  return { items: merged, total, next: more ? next : null };
}

export function useMessageList(folderId: string | null, filter: MailFilter) {
  const backend = useBackend();
  const client = useQueryClient();
  return useInfiniteQuery({
    queryKey: qk.messages(folderId ?? "none", filter),
    enabled: folderId !== null,
    initialPageParam: (folderId === ALL_INBOXES ? {} : 0) as PageParam,
    queryFn: async ({ pageParam }): Promise<ListPage> => {
      if (folderId === ALL_INBOXES) {
        return fetchUnified(backend, client, pageParam as Record<string, number>, filter);
      }
      const offset = pageParam as number;
      const page = await backend.listMessages(folderId!, {
        offset,
        limit: PAGE_SIZE,
        unreadOnly: filter === "unread",
        flaggedOnly: filter === "flagged",
      });
      const end = offset + page.items.length;
      return { ...page, next: end < page.total && page.items.length > 0 ? end : null };
    },
    getNextPageParam: (last) => last.next ?? undefined,
  });
}

export function useMessage(messageId: string | null, allowRemote: boolean) {
  const backend = useBackend();
  return useQuery({
    queryKey: qk.message(messageId ?? "none", allowRemote),
    queryFn: () => backend.getMessage(messageId!, allowRemote),
    enabled: messageId !== null,
    // Keep showing the same message while it reloads with remote content.
    placeholderData: (previous) => (previous?.id === messageId ? previous : undefined),
  });
}

/** Loads a full message (for replies), from the cache when possible. */
export function fetchMessage(client: QueryClient, backend: Backend, messageId: string) {
  const cached =
    client.getQueryData<Message>(qk.message(messageId, true)) ??
    client.getQueryData<Message>(qk.message(messageId, false));
  if (cached) return Promise.resolve(cached);
  return client.fetchQuery({
    queryKey: qk.message(messageId, false),
    queryFn: () => backend.getMessage(messageId, false),
  });
}

/**
 * The messages on this device matching `text`, with `includeIds` (what the
 * server searches found, stored on the device now) among them, in `sort`'s
 * order. `revision` changes whenever `includeIds` does (the search they
 * came from and how many), so each list of hits is its own query; the last
 * results stay on screen while the next ones load.
 */
export function useSearch(
  text: string,
  includeIds: string[],
  revision: string,
  sort: SearchSort = "relevance",
) {
  const backend = useBackend();
  return useQuery({
    queryKey: qk.search(text, revision, sort),
    queryFn: () =>
      backend.searchMessages(text, undefined, includeIds, { sort, weekStartsOn: weekStartsOn() }),
    enabled: text.length > 0,
    placeholderData: (previous) => previous,
  });
}

/**
 * Addresses to offer for `text`; with `allowEmpty`, an empty text asks too
 * (the people who wrote most lately).
 */
export function useSuggestions(text: string, { allowEmpty = false } = {}) {
  const backend = useBackend();
  return useQuery({
    queryKey: qk.suggestions(text),
    queryFn: () => backend.suggestAddresses(text),
    enabled: text.length > 0 || allowEmpty,
    staleTime: 60_000,
    placeholderData: (previous) => previous,
  });
}

// --- Optimistic cache updates ----------------------------------------------------

/**
 * Returns the patched summary, or null to remove it. `where` says which cache
 * is being patched: folder lists, search results or full messages.
 */
type Patch = (m: MessageSummary, where: "list" | "search" | "message") => MessageSummary | null;

type Snapshot = [readonly unknown[], unknown][];

function snapshot(client: QueryClient): Snapshot {
  return client.getQueriesData({ queryKey: qk.root("mail") });
}

function restore(client: QueryClient, snap: Snapshot): void {
  for (const [key, data] of snap) client.setQueryData(key, data);
}

/** Every summary the caches know for the given ids (lists, searches, messages). */
export function findSummaries(client: QueryClient, ids: Iterable<string>): MessageSummary[] {
  const wanted = new Set(ids);
  const found = new Map<string, MessageSummary>();
  const take = (m: MessageSummary) => {
    if (wanted.has(m.id) && !found.has(m.id)) found.set(m.id, m);
  };
  for (const [, data] of client.getQueriesData<ListData>({ queryKey: messagesRoot })) {
    data?.pages.forEach((p) => p.items.forEach(take));
  }
  for (const [, data] of client.getQueriesData<MessageSummary[]>({ queryKey: searchRoot })) {
    data?.forEach(take);
  }
  for (const [, data] of client.getQueriesData<Message>({ queryKey: messageRoot })) {
    if (data) take(data);
  }
  return [...found.values()];
}

function patchMessages(client: QueryClient, ids: Set<string>, patch: Patch): void {
  client.setQueriesData<ListData>({ queryKey: messagesRoot }, (data) => {
    if (!data?.pages) return data;
    let removed = 0;
    let changed = false;
    const pages = data.pages.map((page) => {
      if (!page.items.some((m) => ids.has(m.id))) return page;
      changed = true;
      const items: MessageSummary[] = [];
      for (const m of page.items) {
        if (!ids.has(m.id)) items.push(m);
        else {
          const next = patch(m, "list");
          if (next) items.push(next);
          else removed += 1;
        }
      }
      return { ...page, items };
    });
    if (!changed) return data;
    return {
      ...data,
      pages: pages.map((p) => (removed ? { ...p, total: Math.max(0, p.total - removed) } : p)),
    };
  });
  client.setQueriesData<SearchHit[]>({ queryKey: searchRoot }, (data) => {
    if (!data?.some((m) => ids.has(m.id))) return data;
    // A hit keeps its snippet.
    return data.flatMap((m) => {
      if (!ids.has(m.id)) return [m];
      const next = patch(m, "search");
      return next ? [{ ...m, ...next }] : [];
    });
  });
  client.setQueriesData<Message>({ queryKey: messageRoot }, (data) => {
    if (!data || !ids.has(data.id)) return data;
    const next = patch(data, "message");
    return next ? { ...data, ...next } : data;
  });
}

function patchFolderCounts(
  client: QueryClient,
  deltas: Map<string, { unread: number; total: number }>,
): void {
  if (deltas.size === 0) return;
  client.setQueriesData<Folder[]>({ queryKey: foldersRoot }, (data) =>
    data?.map((f) => {
      const d = deltas.get(f.id);
      if (!d) return f;
      return {
        ...f,
        unreadCount: Math.max(0, f.unreadCount + d.unread),
        totalCount: Math.max(0, f.totalCount + d.total),
      };
    }),
  );
}

function addDelta(
  deltas: Map<string, { unread: number; total: number }>,
  folderId: string,
  unread: number,
  total: number,
) {
  const d = deltas.get(folderId) ?? { unread: 0, total: 0 };
  d.unread += unread;
  d.total += total;
  deltas.set(folderId, d);
}

function cachedFolders(client: QueryClient): Folder[] {
  return client.getQueryData<Folder[]>(qk.folders()) ?? [];
}

/** Destination of a delete: the account's trash, or null when deleting permanently. */
export function trashFor(folders: Folder[], m: MessageSummary): Folder | null {
  const current = folders.find((f) => f.id === m.folderId);
  if (current && (current.role === "trash" || current.role === "junk")) return null;
  return folders.find((f) => f.accountId === m.accountId && f.role === "trash") ?? null;
}

/** Moves summaries between folders in the caches (destination null = gone). */
function applyMove(
  client: QueryClient,
  ids: string[],
  destinationFor: (m: MessageSummary) => string | null,
) {
  const deltas = new Map<string, { unread: number; total: number }>();
  for (const m of findSummaries(client, ids)) {
    const dest = destinationFor(m);
    if (dest === m.folderId) continue;
    const unread = m.isRead ? 0 : 1;
    addDelta(deltas, m.folderId, -unread, -1);
    if (dest) addDelta(deltas, dest, unread, 1);
  }
  patchFolderCounts(client, deltas);
  patchMessages(client, new Set(ids), (m, where) => {
    const dest = destinationFor(m);
    if (dest === m.folderId) return m;
    // Folder lists lose the message; searches span folders and keep it.
    if (where === "list" || dest === null) return null;
    return { ...m, folderId: dest };
  });
}

// --- Mutations -------------------------------------------------------------------

interface Context {
  snap: Snapshot;
}

const MAIL_MUTATION = ["mail", "change"] as const;

function useOptimisticMutation<V>(
  mutationFn: (vars: V) => Promise<void>,
  apply: (client: QueryClient, vars: V) => void,
) {
  const client = useQueryClient();
  return useMutation<void, unknown, V, Context>({
    mutationKey: MAIL_MUTATION,
    mutationFn,
    onMutate: async (vars) => {
      await client.cancelQueries({ queryKey: qk.root("mail") });
      const snap = snapshot(client);
      apply(client, vars);
      return { snap };
    },
    onError: (error, _vars, context) => {
      if (context) restore(client, context.snap);
      toast.error(error);
    },
    onSettled: () => {
      // Refetching while another change is in flight would undo its optimistic update.
      if (client.isMutating({ mutationKey: MAIL_MUTATION }) <= 1) {
        return client.invalidateQueries({ queryKey: qk.root("mail") });
      }
    },
  });
}

export function useSetRead() {
  const backend = useBackend();
  return useOptimisticMutation<{ ids: string[]; read: boolean }>(
    ({ ids, read }) => backend.setRead(ids, read),
    (client, { ids, read }) => {
      const deltas = new Map<string, { unread: number; total: number }>();
      for (const m of findSummaries(client, ids)) {
        if (m.isRead !== read) addDelta(deltas, m.folderId, read ? -1 : 1, 0);
      }
      patchFolderCounts(client, deltas);
      patchMessages(client, new Set(ids), (m) => ({ ...m, isRead: read }));
    },
  );
}

export function useSetFlagged() {
  const backend = useBackend();
  return useOptimisticMutation<{ ids: string[]; flagged: boolean }>(
    ({ ids, flagged }) => backend.setFlagged(ids, flagged),
    (client, { ids, flagged }) =>
      patchMessages(client, new Set(ids), (m) => ({ ...m, isFlagged: flagged })),
  );
}

export function useMoveMessages() {
  const backend = useBackend();
  return useOptimisticMutation<{ ids: string[]; folderId: string }>(
    ({ ids, folderId }) => backend.moveMessages(ids, folderId),
    (client, { ids, folderId }) => applyMove(client, ids, () => folderId),
  );
}

export function useArchiveMessages() {
  const backend = useBackend();
  return useOptimisticMutation<{ ids: string[] }>(
    ({ ids }) => backend.archiveMessages(ids),
    (client, { ids }) => {
      const folders = cachedFolders(client);
      applyMove(
        client,
        ids,
        (m) =>
          folders.find((f) => f.accountId === m.accountId && f.role === "archive")?.id ??
          m.folderId,
      );
    },
  );
}

export function useDeleteMessages() {
  const backend = useBackend();
  return useOptimisticMutation<{ ids: string[] }>(
    ({ ids }) => backend.deleteMessages(ids),
    (client, { ids }) => {
      const folders = cachedFolders(client);
      applyMove(client, ids, (m) => trashFor(folders, m)?.id ?? null);
    },
  );
}

export function useSendMessage() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (message: OutgoingMessage) => backend.sendMessage(message),
    onSettled: () => client.invalidateQueries({ queryKey: qk.root("mail") }),
  });
}
