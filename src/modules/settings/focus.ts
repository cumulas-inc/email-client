import { useEffect, useLayoutEffect, useRef, type RefObject } from "react";

/**
 * Moves focus to the first `[data-autofocus]` element inside `ref` whenever
 * `key` changes (e.g. a new step in a dialog). Deferred so it runs after
 * <dialog>.showModal(), which would otherwise focus the close button.
 */
export function useFocusOnChange(ref: RefObject<HTMLElement | null>, key: unknown): void {
  useEffect(() => {
    const timer = window.setTimeout(() => {
      ref.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [ref, key]);
}

/**
 * Returns focus to whatever had it when the component mounted (e.g. the
 * button that opened a dialog) once it unmounts, if that is still on screen.
 * With `preferId`, the element with that id gets it instead, when there is one.
 */
export function useRestoreFocus(preferId?: string): void {
  const previous = useRef<Element | null>(null);
  // Read in a layout effect, before <dialog>.showModal() moves focus…
  useLayoutEffect(() => {
    // Only the first time: StrictMode re-runs effects after focus has moved.
    previous.current ??= document.activeElement;
  }, []);
  // …and restore in a passive cleanup, once the modal has left the page
  // (while it is open, everything outside it is inert).
  useEffect(() => {
    const element = previous.current;
    return () => {
      const restore = () => {
        const preferred = preferId ? document.getElementById(preferId) : null;
        if (preferred && !preferred.matches(":disabled")) preferred.focus();
        else if (element instanceof HTMLElement && element.isConnected) element.focus();
      };
      // The preferred element may only be enabled by what the dialog changed.
      if (preferId) window.setTimeout(restore, 0);
      else restore();
    };
  }, [preferId]);
}

/** Focuses an element by id on the next tick (after a re-render). */
export function focusLater(id: string): void {
  window.setTimeout(() => document.getElementById(id)?.focus(), 0);
}
