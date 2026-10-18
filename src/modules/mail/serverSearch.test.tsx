import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

import { BackendProvider } from "../../api/BackendProvider";
import type { Backend } from "../../api/backend";
import { MockBackend } from "../../api/mock";
import type { ServerSearchResult } from "../../api/types";
import { weekStartsOn } from "../../lib/datetime";
import { MAX_SERVER_RESULTS, useServerSearch } from "./serverSearch";
import { deferred } from "./testUtils";

type Call = Parameters<Backend["searchServer"]>;

/** Dates resolve with the week starting where the locale's does. */
const OPTIONS = { weekStartsOn: weekStartsOn() };

/** The hook over the demo's two accounts, with the server search answered by `answer`. */
function setup(answer: (...call: Call) => Promise<ServerSearchResult>, text = "invoice") {
  const backend = new MockBackend();
  const searchServer = vi.fn(answer);
  const cancelServerSearch = vi.fn(async (_searchId: string) => {});
  backend.searchServer = searchServer;
  backend.cancelServerSearch = cancelServerSearch;
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <BackendProvider backend={backend}>
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    </BackendProvider>
  );
  const hook = renderHook(({ text }) => useServerSearch(text), {
    wrapper,
    initialProps: { text },
  });
  /** Waits for the accounts to load and both to be asked, and for their answers. */
  const settled = async () => {
    await waitFor(() => expect(searchServer).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(hook.result.current.searching).toBe(false));
  };
  return { ...hook, searchServer, cancelServerSearch, settled };
}

function done(call: Call, messageIds: string[], hasMore = false): ServerSearchResult {
  const [searchId, , accountId] = call;
  return { searchId, accountId, status: "done", messageIds, added: 0, hasMore, error: null };
}

describe("useServerSearch", () => {
  it("asks every account at once, gathers what they find once each, and asks for more", async () => {
    const { result, searchServer, settled } = setup(async (...call) =>
      call[2] === "acct-work" ? done(call, ["a", "b"], !call[3]) : done(call, ["b", "c"]),
    );
    await settled();
    expect(searchServer.mock.calls.map((c) => [c[1], c[2], c[3]])).toEqual([
      ["invoice", "acct-work", false],
      ["invoice", "acct-home", false],
    ]);
    const searchId = searchServer.mock.calls[0]![0];
    expect(searchServer.mock.calls[1]![0]).toBe(searchId);
    expect(result.current).toMatchObject({ text: "invoice", searchId, ids: ["a", "b", "c"] });
    const settledAs = { status: "done", retrying: false, loadingMore: false, error: null };
    expect(result.current.accounts).toEqual([
      { accountId: "acct-work", ...settledAs, hasMore: true },
      { accountId: "acct-home", ...settledAs, hasMore: false },
    ]);

    act(() => result.current.showMore());
    expect(result.current.accounts[0]!.loadingMore).toBe(true);
    expect(result.current.searching).toBe(true);
    await waitFor(() => expect(result.current.searching).toBe(false));
    // Only the account with more was asked, in the same search.
    expect(searchServer.mock.calls.slice(2)).toEqual([
      [searchId, "invoice", "acct-work", true, OPTIONS],
    ]);
    expect(result.current.accounts[0]!.hasMore).toBe(false);
  });

  it("ignores answers to a search the text has moved on from, and cancels it", async () => {
    const pending: { call: Call; answer: (r: ServerSearchResult) => void }[] = [];
    const { result, rerender, searchServer, cancelServerSearch } = setup((...call) => {
      const answer = deferred<ServerSearchResult>();
      pending.push({ call, answer: answer.resolve });
      return answer.promise;
    });
    await waitFor(() => expect(searchServer).toHaveBeenCalledTimes(2));
    const first = searchServer.mock.calls[0]![0];
    rerender({ text: "invoices" });
    expect(cancelServerSearch).toHaveBeenCalledWith(first);
    const second = searchServer.mock.calls[2]![0];
    expect(second).not.toBe(first);
    expect(result.current).toMatchObject({ text: "invoices", searchId: second, ids: [] });

    // A server that answers the old search anyway is not listened to.
    await act(async () => {
      for (const p of pending.slice(0, 2)) p.answer(done(p.call, ["old"]));
    });
    expect(result.current.ids).toEqual([]);
    expect(result.current.searching).toBe(true);
    await act(async () => {
      for (const p of pending.slice(2)) p.answer(done(p.call, ["new"]));
    });
    expect(result.current.ids).toEqual(["new"]);
    expect(result.current.searching).toBe(false);
  });

  it("reports a failure per account and tries that account again in the same search", async () => {
    let offline = true;
    const { result, searchServer, settled } = setup(async (...call) => {
      if (call[2] !== "acct-home") return done(call, ["a"]);
      if (!offline) return done(call, ["h"]);
      return {
        ...done(call, []),
        status: "failed",
        error: { code: "network", message: "Could not reach imap.morgan-family.example." },
      };
    });
    await settled();
    expect(result.current.ids).toEqual(["a"]);
    expect(result.current.accounts[1]).toMatchObject({
      status: "failed",
      error: { code: "network" },
    });

    offline = false;
    act(() => result.current.retry("acct-home"));
    // The failure stays, as what is being tried again, until the answer.
    expect(result.current.accounts[1]).toMatchObject({
      status: "searching",
      retrying: true,
      error: { code: "network" },
    });
    await waitFor(() => expect(result.current.searching).toBe(false));
    expect(searchServer.mock.calls.at(-1)).toEqual([
      searchServer.mock.calls[0]![0],
      "invoice",
      "acct-home",
      false,
      OPTIONS,
    ]);
    expect(result.current.ids).toEqual(["a", "h"]);
    expect(result.current.accounts[1]).toMatchObject({
      status: "done",
      retrying: false,
      error: null,
    });
  });

  it("turns a rejection into a failure, and settles a search canceled from elsewhere", async () => {
    const { result, settled } = setup(async (...call) => {
      if (call[2] === "acct-home") throw { code: "notFound", message: "account not found" };
      return { ...done(call, []), status: "cancelled" };
    });
    await settled();
    expect(result.current.accounts).toEqual([
      {
        accountId: "acct-work",
        status: "done",
        retrying: false,
        hasMore: false,
        loadingMore: false,
        error: null,
      },
      {
        accountId: "acct-home",
        status: "failed",
        retrying: false,
        hasMore: false,
        loadingMore: false,
        error: { code: "notFound", message: "account not found" },
      },
    ]);
  });

  it("asks for no more pages once as many hits have come as the list shows", async () => {
    const many = Array.from({ length: MAX_SERVER_RESULTS }, (_, i) => `m${i}`);
    const { result, searchServer, settled } = setup(async (...call) =>
      call[2] === "acct-work" ? done(call, many, true) : done(call, []),
    );
    await settled();
    expect(result.current.ids).toHaveLength(MAX_SERVER_RESULTS);
    expect(result.current.full).toBe(true);
    expect(result.current.accounts[0]!.hasMore).toBe(true);
    act(() => result.current.showMore());
    expect(result.current.searching).toBe(false);
    expect(searchServer).toHaveBeenCalledTimes(2);
  });

  it("asks only the accounts account: names, with the week starting where the locale's does", async () => {
    const { result, searchServer } = setup(
      async (...call) => done(call, []),
      "invoice account:alex@morgan-family.example",
    );
    await waitFor(() => expect(searchServer).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(result.current.searching).toBe(false));
    expect(searchServer.mock.calls[0]!.slice(1)).toEqual([
      "invoice account:alex@morgan-family.example",
      "acct-home",
      false,
      OPTIONS,
    ]);
    expect(result.current.accounts.map((a) => a.accountId)).toEqual(["acct-home"]);
  });

  it("asks no account when account: names none of them", async () => {
    const { result, searchServer } = setup(async (...call) => done(call, []), "invoice account:x");
    // The accounts load meanwhile; still nothing is asked.
    await act(async () => {});
    await act(async () => {});
    expect(searchServer).not.toHaveBeenCalled();
    expect(result.current).toMatchObject({ searchId: null, accounts: [], searching: false });
  });

  it("leaves single letters to this device, and cancels when the text clears or it unmounts", async () => {
    const { result, rerender, unmount, searchServer, cancelServerSearch } = setup(
      () => new Promise<ServerSearchResult>(() => {}),
      "a",
    );
    // The accounts load meanwhile; still nothing is asked.
    await act(async () => {});
    await act(async () => {});
    expect(searchServer).not.toHaveBeenCalled();
    expect(result.current).toMatchObject({ text: "a", searchId: null, searching: false });
    // Nor for several single letters, or one after an operator.
    for (const text of ["a b c", "from:a"]) {
      rerender({ text });
      await act(async () => {});
      expect(searchServer).not.toHaveBeenCalled();
    }

    rerender({ text: "ab" });
    await waitFor(() => expect(searchServer).toHaveBeenCalledTimes(2));
    const first = searchServer.mock.calls[0]![0];
    rerender({ text: "" });
    expect(cancelServerSearch).toHaveBeenLastCalledWith(first);
    expect(result.current).toMatchObject({ text: "", searchId: null, accounts: [] });

    rerender({ text: "abc" });
    await waitFor(() => expect(searchServer).toHaveBeenCalledTimes(4));
    const second = searchServer.mock.calls[2]![0];
    unmount();
    expect(cancelServerSearch).toHaveBeenLastCalledWith(second);
  });
});
