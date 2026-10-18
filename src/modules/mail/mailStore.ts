// UI state of the Mail module. Kept in a store (not component state) so the
// selected folder, selection and an unsent draft survive switching modules.

import { create } from "zustand";

import type {
  AccountKind,
  EmailAddress,
  Importance,
  MessageLabel,
  ProtectAction,
  ProtectionKind,
  ReplyKind,
} from "../../api/types";

export type MailFilter = "all" | "unread" | "flagged";

/** Virtual folder id for the unified "All inboxes" favorite. */
export const ALL_INBOXES = "all-inboxes";

export interface DraftAttachment {
  id: string;
  name: string;
  contentType: string;
  size: number;
  dataBase64: string;
}

/**
 * The person's own version of the signature in a draft, `own` when it
 * started as their own signature rather than their organization's.
 */
export interface SignatureEdit {
  edited: string;
  own: boolean;
}

/**
 * The signature under what's written: "auto" follows the From account's
 * (for a new message or this reply kind, as it changes); an edit is the
 * person's own version of it; "removed" they took it out, or `{ removed }`
 * they took out their edit, which "Add signature" brings back; "off" a
 * message that starts without one (a saved draft, reopened).
 */
export type DraftSignature =
  "auto" | SignatureEdit | "removed" | { removed: SignatureEdit } | "off";

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
  signature: DraftSignature;
  attachments: DraftAttachment[];
  importance: Importance;
  replyToMessageId: string | null;
  replyKind: ReplyKind | null;
  /** Encryption the organization's mail server applies (only offered once checked), or null. */
  protect: ProtectAction | null;
  /** The sensitivity label to apply (an id from the account's labels), or null. */
  labelId: string | null;
  /** The person chose the label (or No label); until then the default or the original's is filled in. */
  labelChosen: boolean;
  /** The label of the message replied to or forwarded. */
  originalLabel: MessageLabel | null;
  /** Where the cursor starts: "to" for new messages, "body" for replies. */
  focus: "to" | "body";
  dirty: boolean;
}

/**
 * A reply or forward that Outlook on the web has to send: the original is
 * encrypted. `protection` is null when only the backend's refusal said so.
 */
export interface ProtectedReply {
  messageId: string;
  kind: ReplyKind;
  accountKind: AccountKind;
  protection: ProtectionKind | null;
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
  /** Messages in the list, including pages not loaded yet. */
  listTotal: number;
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
  /** Reply, Reply all or Forward on an encrypted message: the hand-off to Outlook on the web. */
  protectedReply: ProtectedReply | null;

  selectFolder: (folderId: string) => void;
  /**
   * Shows a message from outside the list (a notification): in its folder,
   * or in All inboxes when that's open and the message is in an Inbox; with
   * a filter that shows it, and open even when the reading pane is off.
   */
  openMessage: (id: string, folderId: string, inInbox: boolean) => void;
  setFilter: (filter: MailFilter) => void;
  select: (id: string, mode?: SelectMode) => void;
  setSelection: (ids: string[], activeId: string | null) => void;
  selectAll: () => void;
  clearSelection: () => void;
  setVisibleIds: (ids: string[], total?: number) => void;
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
  askProtectedReply: (reply: ProtectedReply) => void;
  closeProtectedReply: () => void;
}

let draftSeq = 0;

const INITIAL = {
  folderId: null,
  filter: "all" as MailFilter,
  selectedIds: [],
  activeId: null,
  anchorId: null,
  visibleIds: [],
  listTotal: 0,
  readerOpen: false,
  remoteAllowed: {},
  collapsed: {},
  compose: null,
  composeVisible: false,
  pendingCompose: null,
  protectedReply: null,
} satisfies Partial<MailState>;

export const useMailStore = create<MailState>((set, get) => ({
  ...INITIAL,

  selectFolder: (folderId) =>
    set({ folderId, selectedIds: [], activeId: null, anchorId: null, readerOpen: false }),

  openMessage: (id, folderId, inInbox) => {
    const s = get();
    if (s.folderId !== folderId && !(inInbox && s.folderId === ALL_INBOXES)) {
      s.selectFolder(folderId);
    }
    // New mail is unread and not flagged yet.
    set((state) => ({
      filter: state.filter === "flagged" ? "all" : state.filter,
      selectedIds: [id],
      activeId: id,
      anchorId: id,
      composeVisible: false,
      readerOpen: true,
    }));
  },

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

  setVisibleIds: (visibleIds, total) =>
    set({ visibleIds, listTotal: Math.max(total ?? 0, visibleIds.length) }),

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

  askProtectedReply: (protectedReply) => set({ protectedReply }),

  closeProtectedReply: () => set({ protectedReply: null }),
}));

/** Restores the initial state (tests). */
export function resetMailStore(): void {
  useMailStore.setState({ ...INITIAL });
}
