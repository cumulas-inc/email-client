import { useCallback, useEffect, useState, useSyncExternalStore } from "react";

/** `value`, updated only after it stopped changing for `ms`. */
export function useDebounced<T>(value: T, ms: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return debounced;
}

function readStorage(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStorage(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Storage may be unavailable (private mode); sizes just won't persist.
  }
}

/** A number persisted in localStorage, clamped to [min, max]. */
export function usePersistentNumber(
  key: string,
  fallback: number,
  min: number,
  max: number,
): [number, (value: number) => void] {
  const [value, setValue] = useState(() => {
    const stored = Number(readStorage(key));
    return Math.min(max, Math.max(min, Number.isFinite(stored) && stored > 0 ? stored : fallback));
  });
  useEffect(() => writeStorage(key, String(value)), [key, value]);
  const update = useCallback(
    (v: number) => setValue(Math.min(max, Math.max(min, Math.round(v)))),
    [min, max],
  );
  return [value, update];
}

/** A string preference persisted in localStorage. */
export function usePersistentString<T extends string>(
  key: string,
  fallback: T,
): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(() => (readStorage(key) as T | null) ?? fallback);
  const update = useCallback(
    (v: T) => {
      writeStorage(key, v);
      setValue(v);
    },
    [key],
  );
  return [value, update];
}

function subscribeTheme(onChange: () => void): () => void {
  const media =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: dark)")
      : null;
  media?.addEventListener("change", onChange);
  const observer = typeof MutationObserver === "function" ? new MutationObserver(onChange) : null;
  observer?.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
  });
  return () => {
    media?.removeEventListener("change", onChange);
    observer?.disconnect();
  };
}

function isDarkNow(): boolean {
  const theme = document.documentElement.dataset.theme;
  if (theme === "dark") return true;
  if (theme === "light") return false;
  return typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: dark)").matches
    : false;
}

/** Whether the app currently renders in dark mode. */
export function useIsDark(): boolean {
  return useSyncExternalStore(subscribeTheme, isDarkNow, () => false);
}
