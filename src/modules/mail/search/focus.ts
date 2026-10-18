/** The search box's id: Ctrl+K, Ctrl+E and Ctrl+Shift+F focus it (App.tsx). */
export const SEARCH_INPUT_ID = "global-search";

/** Set while focusSearch focuses the box quietly (see isQuietFocus). */
let quietFocus = false;

/**
 * Puts the keyboard in the search box with the caret at `caret` (the end by
 * default). The box's value must be the current text already (flushSync).
 * `quiet` is for focus the user didn't ask for (a control that went away
 * hands it on): the suggestions stay closed until the next key.
 */
export function focusSearch(caret?: number, { quiet = false }: { quiet?: boolean } = {}): void {
  const input = document.getElementById(SEARCH_INPUT_ID);
  if (!(input instanceof HTMLInputElement)) return;
  const at = caret ?? input.value.length;
  input.setSelectionRange(at, at);
  quietFocus = quiet;
  try {
    input.focus();
  } finally {
    quietFocus = false;
  }
  input.setSelectionRange(at, at);
}

/**
 * Whether the keyboard lost its place: it is on the page itself, or on an
 * element taken out of it (as a control that went away leaves it).
 */
export function focusLost(): boolean {
  const active = document.activeElement;
  return !active || active === document.body || !active.isConnected;
}

/** Whether the focus the box is getting now is focusSearch's, quietly. */
export function isQuietFocus(): boolean {
  return quietFocus;
}
