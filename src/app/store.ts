import { create } from "zustand";

import type { EmailAddress } from "../api/types";

export type Module = "mail" | "calendar" | "people" | "tasks";

export type SettingsSection = "accounts" | "general" | "mail" | "about";

/** A request to open the mail composer, possibly from another module. */
export interface ComposeRequest {
  to?: EmailAddress[];
  cc?: EmailAddress[];
  subject?: string;
  bodyHtml?: string;
  /** Account to send from; defaults to the current account. */
  accountId?: string;
}

interface AppState {
  module: Module;
  setModule: (module: Module) => void;

  /** Global search text from the title bar (mail search). */
  search: string;
  setSearch: (search: string) => void;

  settingsOpen: SettingsSection | null;
  openSettings: (section?: SettingsSection) => void;
  closeSettings: () => void;

  addAccountOpen: boolean;
  setAddAccountOpen: (open: boolean) => void;

  composeRequest: ComposeRequest | null;
  /** Switches to Mail and opens a new message. */
  requestCompose: (request: ComposeRequest) => void;
  clearComposeRequest: () => void;
}

export const useAppStore = create<AppState>((set) => ({
  module: "mail",
  setModule: (module) => set({ module }),

  search: "",
  setSearch: (search) => set({ search }),

  settingsOpen: null,
  openSettings: (section = "accounts") => set({ settingsOpen: section }),
  closeSettings: () => set({ settingsOpen: null }),

  addAccountOpen: false,
  setAddAccountOpen: (addAccountOpen) => set({ addAccountOpen }),

  composeRequest: null,
  requestCompose: (composeRequest) => set({ composeRequest, module: "mail" }),
  clearComposeRequest: () => set({ composeRequest: null }),
}));
