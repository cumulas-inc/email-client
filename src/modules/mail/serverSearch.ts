// Searching each account's whole mailbox on its server, beside the instant
// search of what is on this device. Kept out of React Query on purpose: the
// "data-changed" events every sync sends would run it again, asking every
// server once more. What a server search finds is stored on the device, so
// the local search returns it (with `includeIds`) like any other message.

import { useEffect, useMemo, useSyncExternalStore } from "react";

import type { Backend } from "../../api/backend";
import { useBackend } from "../../api/context";
import { errorMessage, isBackendError } from "../../api/errors";
import { isNarrowEnoughForServers, parseSearch } from "../../api/mock/searchQuery";
import { useAccounts } from "../../api/queries";
import type { BackendError, ServerSearchResult } from "../../api/types";

/**
 * Whether the servers are asked for `text`: not for single letters ("a b",
 * "from:a"), which match most of a mailbox. The core decides the same way.
 */
export function wantsServerSearch(text: string): boolean {
  return isNarrowEnoughForServers(parseSearch(text));
}

/**
 * Most server hits the list shows: the core includes the first 1,000 ids in
 * a local search, so pages beyond them would add nothing.
 */
export const MAX_SERVER_RESULTS = 1000;

export interface AccountSearch {
  accountId: string;
  /** "searching" while the first page is on its way (again, after "Try again"). */
  status: "searching" | "done" | "failed";
  /** Searching again after failing: `error` is the failure, until the answer. */
  retrying: boolean;
  /** The server has another page ("Show more results"). */
  hasMore: boolean;
  loadingMore: boolean;
  error: BackendError | null;
}

export interface ServerSearchState {
  /** The text these results are for. */
  text: string;
  searchId: string | null;
  accounts: AccountSearch[];
  /** Every message found so far, once each, in the order they came. */
  ids: string[];
}

export interface ServerSearch extends ServerSearchState {
  /** Some account is searching or loading more. */
  searching: boolean;
  /** As many hits as the list shows have come: no more pages are asked for. */
  full: boolean;
  /** Asks every account that has more for its next page. */
  showMore(): void;
  /** Asks a failed account for the first page again, in the same search. */
  retry(accountId: string): void;
}

let fallbackIds = 0;

/** A fresh id for a search: starting one cancels the one before. */
function newSearchId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    try {
      return crypto.randomUUID();
    } catch {
      // Not a secure context: the counter below will do.
    }
  }
  fallbackIds += 1;
  return `search-${Date.now().toString(36)}-${fallbackIds}`;
}

function asBackendError(error: unknown): BackendError {
  return isBackendError(error) ? error : { code: "internal", message: errorMessage(error) };
}

/** The server searches of one search box, as an external store for useSyncExternalStore. */
class ServerSearchStore {
  private state: ServerSearchState = { text: "", searchId: null, accounts: [], ids: [] };
  private listeners = new Set<() => void>();

  constructor(private readonly backend: Backend) {}

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getSnapshot = () => this.state;

  private set(state: ServerSearchState) {
    this.state = state;
    this.listeners.forEach((listener) => listener());
  }

  private update(accountId: string, change: Partial<AccountSearch>, ids: string[] = []) {
    const seen = new Set(this.state.ids);
    const added: string[] = [];
    for (const id of ids) {
      if (seen.has(id)) continue;
      seen.add(id);
      added.push(id);
    }
    this.set({
      ...this.state,
      accounts: this.state.accounts.map((a) =>
        a.accountId === accountId ? { ...a, ...change } : a,
      ),
      ids: added.length > 0 ? [...this.state.ids, ...added] : this.state.ids,
    });
  }

  /** Searches every account for `text`, canceling the search before. */
  start(text: string, accountIds: string[]): void {
    this.stop();
    if (!wantsServerSearch(text) || accountIds.length === 0) {
      this.set({ text, searchId: null, accounts: [], ids: [] });
      return;
    }
    const searchId = newSearchId();
    this.set({
      text,
      searchId,
      ids: [],
      accounts: accountIds.map((accountId) => ({
        accountId,
        status: "searching",
        retrying: false,
        hasMore: false,
        loadingMore: false,
        error: null,
      })),
    });
    for (const accountId of accountIds) this.page(accountId, false);
  }

  /** Cancels the search, if one is running; its answers are ignored from now on. */
  stop(): void {
    const { searchId, text } = this.state;
    if (!searchId) return;
    this.set({ text, searchId: null, accounts: [], ids: [] });
    this.backend.cancelServerSearch(searchId).catch(() => {
      // Nothing to do: its answers are ignored anyway.
    });
  }

  showMore = (): void => {
    if (this.state.ids.length >= MAX_SERVER_RESULTS) return;
    for (const a of this.state.accounts) {
      if (a.status === "done" && a.hasMore && !a.loadingMore) {
        this.update(a.accountId, { loadingMore: true });
        this.page(a.accountId, true);
      }
    }
  };

  retry = (accountId: string): void => {
    const account = this.state.accounts.find((a) => a.accountId === accountId);
    if (account?.status !== "failed") return;
    this.update(accountId, { status: "searching", retrying: true, hasMore: false });
    this.page(accountId, false);
  };

  private page(accountId: string, more: boolean): void {
    const { searchId, text } = this.state;
    if (!searchId) return;
    this.backend.searchServer(searchId, text, accountId, more).then(
      (result) => this.settle(searchId, result),
      // Only an account removed meanwhile, or a bad id, rejects.
      (error: unknown) =>
        this.settle(searchId, {
          searchId,
          accountId,
          status: "failed",
          messageIds: [],
          added: 0,
          hasMore: false,
          error: asBackendError(error),
        }),
    );
  }

  private settle(searchId: string, result: ServerSearchResult): void {
    // An answer to a search that was replaced or stopped.
    if (this.state.searchId !== searchId) return;
    const { accountId } = result;
    switch (result.status) {
      case "done":
        this.update(
          accountId,
          {
            status: "done",
            retrying: false,
            hasMore: result.hasMore,
            loadingMore: false,
            error: null,
          },
          result.messageIds,
        );
        break;
      case "failed":
        this.update(accountId, {
          status: "failed",
          retrying: false,
          hasMore: false,
          loadingMore: false,
          error: result.error ?? { code: "internal", message: "Something went wrong." },
        });
        break;
      case "cancelled":
        // Canceled from elsewhere while still ours: settle with what came,
        // rather than search forever.
        this.update(accountId, {
          status: "done",
          retrying: false,
          hasMore: false,
          loadingMore: false,
          error: null,
        });
        break;
    }
  }
}

/**
 * Searches every account's whole mailbox on its server for `text`: the first
 * page of each at once, then more on request. A new text cancels the search
 * before; so do an empty text and unmounting.
 */
export function useServerSearch(text: string): ServerSearch {
  const backend = useBackend();
  const { data: accounts } = useAccounts();
  // A string, so a refetched but unchanged account list doesn't search again.
  const accountKey = (accounts ?? []).map((a) => a.id).join("\n");
  const store = useMemo(() => new ServerSearchStore(backend), [backend]);
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot);

  useEffect(() => {
    store.start(text, accountKey ? accountKey.split("\n") : []);
    return () => store.stop();
  }, [store, text, accountKey]);

  return useMemo(
    () => ({
      ...state,
      searching: state.accounts.some((a) => a.status === "searching" || a.loadingMore),
      full: state.ids.length >= MAX_SERVER_RESULTS,
      showMore: store.showMore,
      retry: store.retry,
    }),
    [state, store],
  );
}
