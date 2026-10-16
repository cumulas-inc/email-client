import { useAppStore } from "./store";

export const OFFERED_KEY = "cumulas.offers.startAtLogin";

/**
 * Offers starting at login (StartAtLoginOffer), once per computer: called
 * when the first account has been added. Remembered in localStorage, like
 * other per-device choices.
 */
export function offerStartAtLoginOnce(): void {
  try {
    if (window.localStorage.getItem(OFFERED_KEY)) return;
    window.localStorage.setItem(OFFERED_KEY, "offered");
  } catch {
    // Without storage, it's offered this once.
  }
  useAppStore.getState().setStartAtLoginOffer(true);
}
