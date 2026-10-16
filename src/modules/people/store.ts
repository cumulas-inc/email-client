import { create } from "zustand";

interface PeopleState {
  /** Selected contact; when null or filtered out the first contact is shown. */
  selectedId: string | null;
  /** Account filter; null shows every account. */
  accountId: string | null;
  select: (id: string | null) => void;
  setAccountId: (accountId: string | null) => void;
}

/** UI state of the Contacts module, kept across module switches. */
export const usePeopleStore = create<PeopleState>((set) => ({
  selectedId: null,
  accountId: null,
  select: (selectedId) => set({ selectedId }),
  setAccountId: (accountId) => set({ accountId }),
}));
