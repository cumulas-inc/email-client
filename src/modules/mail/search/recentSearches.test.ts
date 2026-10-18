import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  MAX_RECENT_SEARCHES,
  RECENT_SEARCHES_KEY,
  addRecentSearch,
  clearRecentSearches,
  removeRecentSearch,
  useRecentSearches,
} from "./recentSearches";

afterEach(() => {
  vi.restoreAllMocks();
  window.localStorage.clear();
});

const stored = () => JSON.parse(window.localStorage.getItem(RECENT_SEARCHES_KEY) ?? "[]");

describe("recent searches", () => {
  it("keeps the newest first, once each whatever the case, eight at most", () => {
    const { result } = renderHook(() => useRecentSearches());
    expect(result.current).toEqual([]);
    act(() => {
      addRecentSearch("  lunch ");
      addRecentSearch("from:megan");
      addRecentSearch("LUNCH");
    });
    expect(result.current).toEqual(["LUNCH", "from:megan"]);
    expect(stored()).toEqual(["LUNCH", "from:megan"]);
    act(() => {
      for (let i = 0; i < 10; i++) addRecentSearch(`word${i}`);
    });
    expect(result.current).toHaveLength(MAX_RECENT_SEARCHES);
    expect(result.current[0]).toBe("word9");
  });

  it("keeps only searches that look for something, and at most 1,024 characters", () => {
    const { result } = renderHook(() => useRecentSearches());
    act(() => {
      addRecentSearch("");
      addRecentSearch("in:inbox");
      addRecentSearch("is:starred");
      addRecentSearch(`${"x".repeat(2000)}`);
    });
    expect(result.current).toEqual(["x".repeat(1024)]);
  });

  it("removes one, or all", () => {
    const { result } = renderHook(() => useRecentSearches());
    act(() => {
      addRecentSearch("a1");
      addRecentSearch("b2");
      removeRecentSearch("A1");
    });
    expect(result.current).toEqual(["b2"]);
    act(() => clearRecentSearches());
    expect(result.current).toEqual([]);
    expect(window.localStorage.getItem(RECENT_SEARCHES_KEY)).toBeNull();
  });

  it("reads what storage holds, and ignores what isn't a list of searches", () => {
    window.localStorage.setItem(RECENT_SEARCHES_KEY, '["saved", 3, null, "kept"]');
    const { result, rerender } = renderHook(() => useRecentSearches());
    expect(result.current).toEqual(["saved", "kept"]);
    window.localStorage.setItem(RECENT_SEARCHES_KEY, "{not json");
    rerender();
    expect(result.current).toEqual([]);
  });

  it("works without storage, for this session", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    const { result } = renderHook(() => useRecentSearches());
    expect(result.current).toEqual([]);
    act(() => addRecentSearch("lunch"));
    expect(result.current).toEqual(["lunch"]);
  });
});
