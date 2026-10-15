import type { NotifyScope } from "../../api/types";

/** The choices for "Notify for", in Settings → General. */
export const NOTIFY_SCOPES: { value: NotifyScope; label: string }[] = [
  { value: "inbox", label: "Inbox" },
  { value: "allFolders", label: "All folders" },
];

/**
 * Where new mail shows a notification, for "when mail arrives in …":
 * `whose` is "your" or "this account's". All folders leaves out the same
 * folders as the alert policy in the core (Outbox isn't one people look at).
 */
export function notifyWhere(scope: NotifyScope, whose: string): string {
  return scope === "inbox"
    ? `${whose} Inbox`
    : `any of ${whose} folders except Junk, Sent, Drafts and Trash`;
}
