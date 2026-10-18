import {
  type RefObject,
  useCallback,
  useEffect,
  useLayoutEffect,
  useState,
  useSyncExternalStore,
} from "react";

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

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/**
 * A size the user set, persisted in localStorage and clamped to [min, max];
 * null until they set one. Only a change is stored, so a default can still
 * change later for everyone who never moved it.
 */
export function usePersistentSize(
  key: string,
  min: number,
  max: number,
): [number | null, (value: number) => void] {
  const [value, setValue] = useState<number | null>(() => {
    const stored = Number(readStorage(key) ?? NaN);
    return Number.isFinite(stored) && stored > 0 ? clamp(stored, min, max) : null;
  });
  const update = useCallback(
    (v: number) => {
      const next = clamp(Math.round(v), min, max);
      writeStorage(key, String(next));
      setValue(next);
    },
    [key, min, max],
  );
  return [value, update];
}

/** A number persisted in localStorage, clamped to [min, max]. */
export function usePersistentNumber(
  key: string,
  fallback: number,
  min: number,
  max: number,
): [number, (value: number) => void] {
  const [value, update] = usePersistentSize(key, min, max);
  return [value ?? clamp(fallback, min, max), update];
}

/** The element's height, kept current while `enabled` (0 until measured). */
export function useHeight(ref: RefObject<HTMLElement | null>, enabled: boolean): number {
  const [height, setHeight] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!enabled || !el) return;
    const measure = () => setHeight(el.clientHeight);
    measure();
    if (typeof ResizeObserver !== "function") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref, enabled]);
  return height;
}

/**
 * A string preference persisted in localStorage. A stored value that isn't
 * one of `allowed` (written by another version, or by hand) reads as
 * `fallback`.
 */
export function usePersistentString<T extends string>(
  key: string,
  fallback: T,
  allowed: readonly T[],
): [T, (value: T) => void] {
  const [value, setValue] = useState<T>(() => {
    const stored = readStorage(key);
    return allowed.find((v) => v === stored) ?? fallback;
  });
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
