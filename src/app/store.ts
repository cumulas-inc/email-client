import { create } from "zustand";

import type { EmailAddress } from "../api/types";

export type Module = "mail" | "calendar" | "people" | "tasks";

export type SettingsSection = "accounts" | "general" | "mail" | "microsoft" | "about";

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

  /** Text in the current module's search box; cleared when switching modules. */
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

  /** Asking whether to quit and lose an unsent message. */
  quitConfirmOpen: boolean;
  setQuitConfirmOpen: (open: boolean) => void;

  /** The one-time offer to start at login, after the first account was added. */
  startAtLoginOffer: boolean;
  setStartAtLoginOffer: (open: boolean) => void;
}

export const useAppStore = create<AppState>((set) => ({
  module: "mail",
  setModule: (module) => set((state) => (state.module === module ? state : { module, search: "" })),

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

  quitConfirmOpen: false,
  setQuitConfirmOpen: (quitConfirmOpen) => set({ quitConfirmOpen }),

  startAtLoginOffer: false,
  setStartAtLoginOffer: (startAtLoginOffer) => set({ startAtLoginOffer }),
}));
