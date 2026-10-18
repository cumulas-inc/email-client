// The searches made on this device, newest first, for the search box's
// suggestions. Kept in localStorage, per device like the other view
// preferences; "Clear recent searches" in the suggestions deletes them.

import { useSyncExternalStore } from "react";

import { isEmptySearch, parseSearch } from "../../../lib/search/query";

export const RECENT_SEARCHES_KEY = "cumulas.mail.recentSearches";
export const MAX_RECENT_SEARCHES = 8;
/** Longest search kept, as much as the search box reads. */
const MAX_CHARS = 1024;

function read(): string | null {
  try {
    return window.localStorage.getItem(RECENT_SEARCHES_KEY);
  } catch {
    return null;
  }
}

function write(searches: string[]): void {
  try {
    if (searches.length > 0) {
      window.localStorage.setItem(RECENT_SEARCHES_KEY, JSON.stringify(searches));
    } else window.localStorage.removeItem(RECENT_SEARCHES_KEY);
  } catch {
    // Storage may be unavailable (private mode, quota); they're only suggestions.
  }
}

function parse(raw: string | null): string[] {
  try {
    const parsed: unknown = JSON.parse(raw ?? "[]");
    return Array.isArray(parsed)
      ? parsed.filter((s): s is string => typeof s === "string").slice(0, MAX_RECENT_SEARCHES)
      : [];
  } catch {
    return [];
  }
}

// What storage held when last read, and its list: the same list until it changes.
let lastRaw: string | null = null;
let current: string[] = [];
const listeners = new Set<() => void>();

function snapshot(): string[] {
  const raw = read();
  if (raw !== lastRaw) {
    lastRaw = raw;
    current = parse(raw);
  }
  return current;
}

function set(searches: string[]): void {
  write(searches);
  lastRaw = read();
  current = searches;
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** The recent searches, newest first. */
export function useRecentSearches(): string[] {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/** Puts a search first (once, whatever its case); only one that looks for something. */
export function addRecentSearch(text: string): void {
  const search = Array.from(text.trim()).slice(0, MAX_CHARS).join("").trim();
  if (!search || isEmptySearch(parseSearch(search))) return;
  const others = snapshot().filter((s) => !same(s, search));
  set([search, ...others].slice(0, MAX_RECENT_SEARCHES));
}

export function removeRecentSearch(text: string): void {
  set(snapshot().filter((s) => !same(s, text)));
}

export function clearRecentSearches(): void {
  set([]);
}
