import { create } from "zustand";

import type { EmailAddress } from "../api/types";

export type Module = "mail" | "calendar" | "people" | "tasks";

export type SettingsSection =
  "accounts" | "general" | "mail" | "signatures" | "microsoft" | "about";

/** The ways of leaving the app: both lose an unsent message. */
export type Leave = "quit" | "restart";

/** Where the update whose card was put off is kept, per device. */
export const DISMISSED_UPDATE_KEY = "cumulas.update.dismissed";

function storedDismissedUpdate(): string | null {
  try {
    return window.localStorage.getItem(DISMISSED_UPDATE_KEY);
  } catch {
    return null;
  }
}

/** A request to show an event in Calendar (a reminder was clicked). */
export interface CalendarRequest {
  eventId: string;
  /** The event's start (ISO 8601); for an all-day event, midnight UTC of its first day. */
  start: string;
  isAllDay: boolean;
}

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

  /** Asking whether to quit (or restart into an update) and lose an unsent message. */
  leaveConfirm: Leave | null;
  setLeaveConfirm: (leave: Leave | null) => void;

  /**
   * The installed update whose card was put off (Later), by its commit: the
   * card comes back only for another one. Remembered per device.
   */
  dismissedUpdate: string | null;
  dismissUpdate: (commit: string) => void;

  /** The one-time offer to start at login, after the first account was added. */
  startAtLoginOffer: boolean;
  setStartAtLoginOffer: (open: boolean) => void;

  /** The header's Reminders panel is open. */
  remindersOpen: boolean;
  setRemindersOpen: (open: boolean) => void;

  calendarRequest: CalendarRequest | null;
  /** Switches to Calendar and shows the event. */
  requestCalendarEvent: (request: CalendarRequest) => void;
  clearCalendarRequest: () => void;
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

  leaveConfirm: null,
  setLeaveConfirm: (leaveConfirm) => set({ leaveConfirm }),

  dismissedUpdate: storedDismissedUpdate(),
  dismissUpdate: (commit) => {
    try {
      window.localStorage.setItem(DISMISSED_UPDATE_KEY, commit);
    } catch {
      // Without storage, it's put off for this run.
    }
    set({ dismissedUpdate: commit });
  },

  startAtLoginOffer: false,
  setStartAtLoginOffer: (startAtLoginOffer) => set({ startAtLoginOffer }),

  remindersOpen: false,
  setRemindersOpen: (remindersOpen) => set({ remindersOpen }),

  calendarRequest: null,
  requestCalendarEvent: (calendarRequest) =>
    set((state) => ({
      calendarRequest,
      module: "calendar",
      search: state.module === "calendar" ? state.search : "",
    })),
  clearCalendarRequest: () => set({ calendarRequest: null }),
}));
