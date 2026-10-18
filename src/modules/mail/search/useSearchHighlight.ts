import { useMemo } from "react";

import { useAppStore } from "../../../app/store";
import { type HitPatterns, hitPatterns } from "../../../lib/search/highlight";
import { parseSearch } from "../../../lib/search/query";
import { useDebounced } from "../hooks";

/**
 * What to mark in the open message: the words of the search box's text, or
 * null when it has none. The same words typed again are the same object, so
 * nothing downstream redoes its work.
 */
export function useSearchHighlight(): HitPatterns | null {
  const text = useDebounced(
    useAppStore((s) => s.search.trim()),
    150,
  );
  const key = useMemo(() => {
    const patterns = text ? hitPatterns(parseSearch(text)) : null;
    return patterns ? JSON.stringify(patterns) : "";
  }, [text]);
  return useMemo(() => (key ? (JSON.parse(key) as HitPatterns) : null), [key]);
}
