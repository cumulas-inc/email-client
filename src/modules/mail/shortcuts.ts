// Keyboard shortcuts for the Mail module, the ones people know from other
// desktop mail programs: Ctrl+Q marks read, as in Outlook (so quitting has
// no shortcut), and Q and U as in Outlook on the web. The app shell owns
// Ctrl+1–4 (modules), Ctrl+E / Ctrl+Shift+F (search) and Ctrl+, (Settings).

import { useEffect } from "react";

import type { ReadingPane } from "../../api/types";
import type { MailActions } from "./actions";
import { useMailStore } from "./mailStore";

export const SHORTCUTS: { keys: string; action: string }[] = [
  { keys: "Ctrl+N", action: "New message" },
  { keys: "Ctrl+R", action: "Reply" },
  { keys: "Ctrl+Shift+R", action: "Reply all" },
  { keys: "Ctrl+F", action: "Forward" },
  { keys: "Ctrl+Q / Q", action: "Mark as read" },
  { keys: "Ctrl+U / U", action: "Mark as unread" },
  { keys: "Insert / Ctrl+Shift+G", action: "Flag or unflag" },
  { keys: "Delete", action: "Delete" },
  { keys: "↑ ↓ / J K", action: "Previous / next message" },
  { keys: "Ctrl+Enter", action: "Send (in the composer)" },
];

function isEditable(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    (el as HTMLElement).isContentEditable === true
  );
}

export function useMailShortcuts(actions: MailActions, readingPane: ReadingPane): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.altKey) return;
      const target = e.target instanceof Element ? e.target : null;
      // Dialogs and menus handle their own keys.
      if (target?.closest("dialog, [role=menu]")) return;
      const editing = isEditable(target);
      const inComposer = !!target?.closest(".mail-compose");
      const ctrl = e.ctrlKey || e.metaKey;
      const key = e.key.toLowerCase();
      const store = useMailStore.getState();
      const run = (fn: () => void) => {
        e.preventDefault();
        fn();
      };

      if (ctrl) {
        if (key === "n" && !e.shiftKey) {
          if (!inComposer) run(() => actions.newMessage());
          return;
        }
        if (editing || inComposer) return;
        const active = store.activeId;
        if (key === "r") {
          if (active) run(() => void actions.reply(active, e.shiftKey ? "replyAll" : "reply"));
          else e.preventDefault(); // Never reload the window.
        } else if (key === "f" && !e.shiftKey) {
          if (active) run(() => void actions.reply(active, "forward"));
        } else if (key === "q" && !e.shiftKey) {
          run(() => actions.markRead(actions.targetIds(), true));
        } else if (key === "u" && !e.shiftKey) {
          run(() => actions.markRead(actions.targetIds(), false));
        } else if (key === "g" && e.shiftKey) {
          run(() => actions.toggleFlag(actions.targetIds()));
        }
        return;
      }

      // Typing, and keys inside the folder tree, belong to those controls.
      if (editing || inComposer || target?.closest("[role=tree]")) return;

      switch (e.key) {
        case "q":
          run(() => actions.markRead(actions.targetIds(), true));
          break;
        case "u":
          run(() => actions.markRead(actions.targetIds(), false));
          break;
        case "Insert":
          run(() => actions.toggleFlag(actions.targetIds()));
          break;
        case "Delete":
          run(() => actions.remove(actions.targetIds()));
          break;
        case "j":
        case "J":
          run(() => store.moveActive(1, e.shiftKey));
          break;
        case "k":
        case "K":
          run(() => store.moveActive(-1, e.shiftKey));
          break;
        case "Escape":
          if (readingPane === "off" && store.readerOpen && !store.composeVisible) {
            run(() => store.setReaderOpen(false));
          }
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [actions, readingPane]);
}
