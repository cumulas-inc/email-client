// UI state of the Mail module. Kept in a store (not component state) so the
// selected folder, selection and an unsent draft survive switching modules.

import { create } from "zustand";

import type { EmailAddress, Importance, ReplyKind } from "../../api/types";

export type MailFilter = "all" | "unread" | "flagged";

/** Virtual folder id for the unified "All inboxes" favourite. */
export const ALL_INBOXES = "all-inboxes";

export interface DraftAttachment {
  id: string;
  name: string;
  contentType: string;
  size: number;
  dataBase64: string;
}

export interface ComposeDraft {
  /** Unique per draft; used as React key so the editor remounts. */
  id: number;
  accountId: string;
  to: EmailAddress[];
  cc: EmailAddress[];
  bcc: EmailAddress[];
  showCc: boolean;
  showBcc: boolean;
  subject: string;
  /** HTML of the editable part. */
  bodyHtml: string;
  /** Quoted original (reply/forward), appended below the editor on send. */
  quoteHtml: string | null;
  attachments: DraftAttachment[];
  importance: Importance;
  replyToMessageId: string | null;
  replyKind: ReplyKind | null;
  /** Where the cursor starts: "to" for new messages, "body" for replies. */
  focus: "to" | "body";
  dirty: boolean;
}

export type SelectMode = "single" | "toggle" | "range";

interface MailState {
  folderId: string | null;
  filter: MailFilter;
  /** Messages in the current selection (multi-select). */
  selectedIds: string[];
  /** The message shown in the reading pane, and the list's keyboard cursor. */
  activeId: string | null;
  /** Start of a Shift+click range. */
  anchorId: string | null;
  /** Ids in the order the list currently shows them. */
  visibleIds: string[];
  /** Reading pane "off": the message or composer is shown instead of the list. */
  readerOpen: boolean;
  /** Messages the user allowed remote content for in this session. */
  remoteAllowed: Record<string, true>;
  /** Collapsed tree nodes ("favorites", "account:<id>", "folder:<id>"). */
  collapsed: Record<string, boolean>;

  compose: ComposeDraft | null;
  composeVisible: boolean;
  /** A new draft waiting for the user to confirm discarding the current one. */
  pendingCompose: ComposeDraft | null;

  selectFolder: (folderId: string) => void;
  setFilter: (filter: MailFilter) => void;
  select: (id: string, mode?: SelectMode) => void;
  setSelection: (ids: string[], activeId: string | null) => void;
  selectAll: () => void;
  clearSelection: () => void;
  setVisibleIds: (ids: string[]) => void;
  /** Moves the cursor by `delta` rows (extending the selection with Shift). */
  moveActive: (delta: number, extend?: boolean) => string | null;
  setReaderOpen: (open: boolean) => void;
  allowRemote: (messageId: string) => void;
  toggleCollapsed: (key: string, collapsed?: boolean) => void;

  /** Opens a draft; asks for confirmation when an unsent draft has changes. */
  startCompose: (draft: Omit<ComposeDraft, "id" | "dirty">) => void;
  updateCompose: (patch: Partial<ComposeDraft>) => void;
  showCompose: () => void;
  hideCompose: () => void;
  closeCompose: () => void;
  confirmPendingCompose: () => void;
  cancelPendingCompose: () => void;
}

let draftSeq = 0;

const INITIAL = {
  folderId: null,
  filter: "all" as MailFilter,
  selectedIds: [],
  activeId: null,
  anchorId: null,
  visibleIds: [],
  readerOpen: false,
  remoteAllowed: {},
  collapsed: {},
  compose: null,
  composeVisible: false,
  pendingCompose: null,
} satisfies Partial<MailState>;

export const useMailStore = create<MailState>((set, get) => ({
  ...INITIAL,

  selectFolder: (folderId) =>
    set({ folderId, selectedIds: [], activeId: null, anchorId: null, readerOpen: false }),

  setFilter: (filter) => set({ filter, selectedIds: [], activeId: null, anchorId: null }),

  select: (id, mode = "single") => {
    const s = get();
    if (mode === "toggle") {
      const has = s.selectedIds.includes(id);
      const selectedIds = has ? s.selectedIds.filter((x) => x !== id) : [...s.selectedIds, id];
      set({
        selectedIds,
        activeId: has ? (selectedIds[selectedIds.length - 1] ?? null) : id,
        anchorId: id,
        composeVisible: false,
      });
      return;
    }
    if (mode === "range" && s.anchorId) {
      const a = s.visibleIds.indexOf(s.anchorId);
      const b = s.visibleIds.indexOf(id);
      if (a >= 0 && b >= 0) {
        const [from, to] = a < b ? [a, b] : [b, a];
        set({ selectedIds: s.visibleIds.slice(from, to + 1), activeId: id, composeVisible: false });
        return;
      }
    }
    set({ selectedIds: [id], activeId: id, anchorId: id, composeVisible: false });
  },

  setSelection: (ids, activeId) =>
    set({ selectedIds: ids, activeId, anchorId: activeId, composeVisible: false }),

  selectAll: () => {
    const { visibleIds, activeId } = get();
    set({ selectedIds: [...visibleIds], activeId: activeId ?? visibleIds[0] ?? null });
  },

  clearSelection: () => {
    const { activeId } = get();
    set({ selectedIds: activeId ? [activeId] : [], anchorId: activeId });
  },

  setVisibleIds: (visibleIds) => set({ visibleIds }),

  moveActive: (delta, extend = false) => {
    const { visibleIds, activeId, select } = get();
    if (visibleIds.length === 0) return null;
    const index = activeId ? visibleIds.indexOf(activeId) : -1;
    const target =
      index < 0
        ? delta > 0
          ? 0
          : visibleIds.length - 1
        : Math.min(visibleIds.length - 1, Math.max(0, index + delta));
    const id = visibleIds[target]!;
    select(id, extend ? "range" : "single");
    return id;
  },

  setReaderOpen: (readerOpen) => set({ readerOpen }),

  allowRemote: (messageId) =>
    set((s) => ({ remoteAllowed: { ...s.remoteAllowed, [messageId]: true } })),

  toggleCollapsed: (key, collapsed) =>
    set((s) => ({ collapsed: { ...s.collapsed, [key]: collapsed ?? !s.collapsed[key] } })),

  startCompose: (input) => {
    draftSeq += 1;
    const draft: ComposeDraft = { ...input, id: draftSeq, dirty: false };
    const current = get().compose;
    if (current?.dirty) {
      set({ pendingCompose: draft, composeVisible: true, readerOpen: true });
    } else {
      set({ compose: draft, composeVisible: true, readerOpen: true });
    }
  },

  updateCompose: (patch) =>
    set((s) => (s.compose ? { compose: { ...s.compose, dirty: true, ...patch } } : {})),

  showCompose: () => set((s) => (s.compose ? { composeVisible: true, readerOpen: true } : {})),

  hideCompose: () => set({ composeVisible: false }),

  closeCompose: () =>
    set((s) => ({
      compose: null,
      composeVisible: false,
      // With the reading pane off, return to the list unless a message is open.
      readerOpen: s.readerOpen && s.activeId !== null,
    })),

  confirmPendingCompose: () =>
    set((s) =>
      s.pendingCompose
        ? {
            compose: s.pendingCompose,
            pendingCompose: null,
            composeVisible: true,
            readerOpen: true,
          }
        : {},
    ),

  cancelPendingCompose: () => set({ pendingCompose: null }),
}));

/** Restores the initial state (tests). */
export function resetMailStore(): void {
  useMailStore.setState({ ...INITIAL });
}
