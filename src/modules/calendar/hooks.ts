import { type RefObject, useEffect, useState } from "react";

/** The current time, updated at the start of every minute. */
export function useNow(intervalMs = 60_000): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const schedule = () => {
      timer = setTimeout(
        () => {
          setNow(new Date());
          schedule();
        },
        intervalMs - (Date.now() % intervalMs),
      );
    };
    schedule();
    return () => clearTimeout(timer);
  }, [intervalMs]);
  return now;
}

/** Content-box size of an element, tracked with ResizeObserver (0 until measured). */
function useElementSize(ref: RefObject<HTMLElement | null>, dimension: "width" | "height"): number {
  const [size, setSize] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry) setSize(entry.contentRect[dimension]);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref, dimension]);
  return size;
}

/** Height of an element, tracked with ResizeObserver (0 until measured). */
export function useElementHeight(ref: RefObject<HTMLElement | null>): number {
  return useElementSize(ref, "height");
}

/** Width of an element, tracked with ResizeObserver (0 until measured). */
export function useElementWidth(ref: RefObject<HTMLElement | null>): number {
  return useElementSize(ref, "width");
}

/** `value`, but only after it has stopped changing for `delayMs`. */
export function useDebounced<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(timer);
  }, [value, delayMs]);
  return debounced;
}

/** True when keyboard focus is in a text field (so letter shortcuts must not fire). */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag !== "INPUT") return false;
  const type = (target as HTMLInputElement).type;
  return !["checkbox", "radio", "button", "submit", "reset", "range", "color"].includes(type);
}
