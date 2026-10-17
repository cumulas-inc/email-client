import { create } from "zustand";

/**
 * How many modal dialogs are open (Dialog counts itself): cards in a corner
 * of the page wait for them, since a modal dialog makes the rest of the page
 * inert.
 */
export const useModals = create<{ open: number }>(() => ({ open: 0 }));
