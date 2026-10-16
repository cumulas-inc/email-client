// Mail commands shared by the list, the reading pane and keyboard shortcuts.
// The returned object is stable: it reads current data from the query cache
// and the mail store at call time, so memoized rows don't re-render.

import { useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";

import { useBackend } from "../../api/context";
import { qk } from "../../api/queries";
import type { Account, Folder, ReplyKind } from "../../api/types";
import { toast, useToasts } from "../../components/toast";
import { type DraftInput, newDraft, parseMailto, replyDraft, textToHtml } from "./draft";
import { ALL_INBOXES, useMailStore } from "./mailStore";
import {
  fetchMessage,
  findSummaries,
  trashFor,
  useArchiveMessages,
  useDeleteMessages,
  useMoveMessages,
  useSetFlagged,
  useSetRead,
} from "./queries";

export interface MailActions {
  /** The selection, or the open message when nothing else is selected. */
  targetIds: () => string[];
  markRead: (ids: string[], read: boolean) => void;
  /** Flags the messages, or unflags them when all are flagged already. */
  toggleFlag: (ids: string[], flagged?: boolean) => void;
  remove: (ids: string[]) => void;
  archive: (ids: string[]) => void;
  move: (ids: string[], folderId: string) => void;
  reply: (messageId: string, kind: ReplyKind) => Promise<void>;
  newMessage: (init?: Partial<DraftInput>) => void;
  /** Opens a saved draft in the composer. */
  editDraft: (messageId: string) => Promise<void>;
  openMailto: (url: string) => void;
}

function plural(n: number, one: string, many = `${one}s`): string {
  return n === 1 ? `1 ${one}` : `${n} ${many}`;
}

/** Moves the selection to the next message after `ids` leave the list. */
function selectAfterRemoval(ids: string[]) {
  const { visibleIds, activeId, selectedIds, setSelection } = useMailStore.getState();
  const removed = new Set(ids);
  if (!activeId || !removed.has(activeId)) {
    if (selectedIds.some((id) => removed.has(id))) {
      setSelection(
        selectedIds.filter((id) => !removed.has(id)),
        activeId && !removed.has(activeId) ? activeId : null,
      );
    }
    return;
  }
  const positions = [...ids, activeId].map((id) => visibleIds.indexOf(id)).filter((i) => i >= 0);
  const last = positions.length > 0 ? Math.max(...positions) : -1;
  const next =
    visibleIds.slice(last + 1).find((id) => !removed.has(id)) ??
    visibleIds
      .slice(0, Math.max(0, last))
      .reverse()
      .find((id) => !removed.has(id)) ??
    null;
  setSelection(next ? [next] : [], next);
}

export function useMailActions(): MailActions {
  const backend = useBackend();
  const client = useQueryClient();
  const { mutate: setRead } = useSetRead();
  const { mutate: setFlagged } = useSetFlagged();
  const { mutate: moveMessages } = useMoveMessages();
  const { mutate: archiveMessages } = useArchiveMessages();
  const { mutate: deleteMessages } = useDeleteMessages();

  return useMemo(() => {
    const folders = () => client.getQueryData<Folder[]>(qk.folders()) ?? [];
    const accounts = () => client.getQueryData<Account[]>(qk.accounts()) ?? [];

    /** Where each message is now, grouped by folder (captured before a move). */
    const originsOf = (ids: string[]) => {
      const map = new Map<string, string[]>();
      for (const m of findSummaries(client, ids)) {
        map.set(m.folderId, [...(map.get(m.folderId) ?? []), m.id]);
      }
      return map;
    };

    /** Confirms a move with an "Undo" action that moves the messages back. */
    const undoToast = (message: string, origins: Map<string, string[]>) =>
      useToasts.getState().push({
        kind: "info",
        message,
        action:
          origins.size > 0
            ? {
                label: "Undo",
                onClick: () => {
                  for (const [folderId, ids] of origins) moveMessages({ ids, folderId });
                },
              }
            : undefined,
      });

    const currentAccountId = () => {
      const { folderId } = useMailStore.getState();
      const folder =
        folderId && folderId !== ALL_INBOXES ? folders().find((f) => f.id === folderId) : null;
      return folder?.accountId ?? accounts()[0]?.id ?? null;
    };

    const newMessage = (init: Partial<DraftInput> = {}) => {
      const accountId = init.accountId ?? currentAccountId();
      if (!accountId) {
        toast.error("Add an account before writing a message.");
        return;
      }
      useMailStore.getState().startCompose(newDraft(accountId, init));
    };

    return {
      targetIds: () => {
        const { selectedIds, activeId } = useMailStore.getState();
        if (selectedIds.length > 0) return selectedIds;
        return activeId ? [activeId] : [];
      },

      markRead: (ids, read) => {
        if (ids.length > 0) setRead({ ids, read });
      },

      toggleFlag: (ids, flagged) => {
        if (ids.length === 0) return;
        const known = findSummaries(client, ids);
        const value = flagged ?? !(known.length > 0 && known.every((m) => m.isFlagged));
        setFlagged({ ids, flagged: value });
      },

      remove: (ids) => {
        if (ids.length === 0) return;
        const all = folders();
        const permanent = findSummaries(client, ids).every((m) => trashFor(all, m) === null);
        const origins = originsOf(ids);
        selectAfterRemoval(ids);
        deleteMessages(
          { ids },
          {
            onSuccess: () => {
              if (permanent) toast.success(`Permanently deleted ${plural(ids.length, "item")}`);
              else undoToast(`Deleted ${plural(ids.length, "item")}`, origins);
            },
          },
        );
      },

      archive: (ids) => {
        if (ids.length === 0) return;
        const origins = originsOf(ids);
        selectAfterRemoval(ids);
        archiveMessages(
          { ids },
          { onSuccess: () => undoToast(`Archived ${plural(ids.length, "item")}`, origins) },
        );
      },

      move: (ids, folderId) => {
        const origins = originsOf(ids);
        origins.delete(folderId);
        if (origins.size === 0) return;
        const name = folders().find((f) => f.id === folderId)?.name ?? "folder";
        selectAfterRemoval(ids);
        moveMessages(
          { ids, folderId },
          { onSuccess: () => undoToast(`Moved ${plural(ids.length, "item")} to ${name}`, origins) },
        );
      },

      reply: async (messageId, kind) => {
        try {
          const original = await fetchMessage(client, backend, messageId);
          useMailStore.getState().startCompose(replyDraft(original, kind, accounts()));
        } catch (e) {
          toast.error(e);
        }
      },

      newMessage,

      editDraft: async (messageId) => {
        try {
          const draft = await fetchMessage(client, backend, messageId);
          newMessage({
            accountId: draft.accountId,
            to: draft.to,
            cc: draft.cc,
            bcc: draft.bcc,
            subject: draft.subject,
            bodyHtml: draft.bodyHtml ?? textToHtml(draft.bodyText ?? ""),
            importance: draft.importance,
            // A saved draft has whatever signature it was written with.
            signature: "off",
          });
        } catch (e) {
          toast.error(e);
        }
      },

      openMailto: (url) => {
        const init = parseMailto(url);
        if (init) newMessage(init);
      },
    };
  }, [backend, client, setRead, setFlagged, moveMessages, archiveMessages, deleteMessages]);
}
