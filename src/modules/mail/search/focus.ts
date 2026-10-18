/** The search box's id: Ctrl+K, Ctrl+E and Ctrl+Shift+F focus it (App.tsx). */
export const SEARCH_INPUT_ID = "global-search";

/**
 * Puts the keyboard in the search box with the caret at `caret` (the end by
 * default). The box's value must be the current text already (flushSync).
 */
export function focusSearch(caret?: number): void {
  const input = document.getElementById(SEARCH_INPUT_ID);
  if (!(input instanceof HTMLInputElement)) return;
  const at = caret ?? input.value.length;
  input.setSelectionRange(at, at);
  input.focus();
  input.setSelectionRange(at, at);
}
