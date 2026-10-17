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
 */
export function useRestoreFocus(): void {
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
      if (element instanceof HTMLElement && element.isConnected) element.focus();
    };
  }, []);
}

/** Holds the keyboard while what it's for is disabled. */
const WAITING_REGION = "aside, section, li, dialog, main, [role='region'], [role='tabpanel']";
/** How long a disabled element is waited for. */
const ENABLED_WITHIN_MS = 30_000;

/**
 * Focuses an element by id on the next tick, once a dialog that did what it
 * was for has closed (after useRestoreFocus). While the element is disabled
 * (it waits for what the dialog changed, such as the calendars the sync
 * after a sign-in brings in), the region around it holds the keyboard and
 * hands it on once it's enabled, unless the keyboard moved meanwhile.
 */
export function focusOnceEnabled(id: string): void {
  window.setTimeout(() => {
    const target = document.getElementById(id);
    if (target) focusOrWait(target);
  }, 0);
}

function focusOrWait(target: HTMLElement): void {
  if (!target.matches(":disabled")) {
    target.focus();
    return;
  }
  const region = target.parentElement?.closest<HTMLElement>(WAITING_REGION);
  if (!region) return;
  if (!region.hasAttribute("tabindex")) {
    region.tabIndex = -1;
    region.addEventListener("blur", () => region.removeAttribute("tabindex"), { once: true });
  }
  region.focus();
  let timer = 0;
  const observer = new MutationObserver(() => {
    if (target.matches(":disabled")) return;
    stop();
    if (document.activeElement === region) target.focus();
  });
  const stop = () => {
    observer.disconnect();
    window.clearTimeout(timer);
  };
  observer.observe(target, { attributes: true, attributeFilter: ["disabled"] });
  timer = window.setTimeout(stop, ENABLED_WITHIN_MS);
}

/** Focuses an element by id on the next tick (after a re-render). */
export function focusLater(id: string): void {
  window.setTimeout(() => document.getElementById(id)?.focus(), 0);
}
