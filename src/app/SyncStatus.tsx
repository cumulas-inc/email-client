import { CircleAlert, CloudCheck, RefreshCw } from "lucide-react";

import { useBackend } from "../api/context";
import { useAccounts, useSyncStatuses } from "../api/queries";
import { formatTime } from "../lib/datetime";

/** One quiet line: synced, syncing or what went wrong, with a sync button. */
export function SyncStatus() {
  const backend = useBackend();
  const { data: accounts = [] } = useAccounts();
  const { data: statuses = [] } = useSyncStatuses();
  const syncing = statuses.find((s) => s.state === "syncing");
  const failed = statuses.find((s) => s.state === "error");

  const times = statuses
    .map((s) => s.lastSyncedAt)
    .concat(accounts.map((a) => a.lastSyncedAt))
    .filter((t): t is string => !!t)
    .sort();
  const last = times.length > 0 ? times[times.length - 1]! : null;

  let icon = <CloudCheck size={14} aria-hidden="true" />;
  let text: string;
  if (syncing) {
    icon = <RefreshCw size={14} className="spin" aria-hidden="true" />;
    text = syncing.message ?? "Syncing…";
  } else if (failed) {
    icon = <CircleAlert size={14} aria-hidden="true" />;
    text = failed.message ?? "Sync failed";
  } else {
    text = accounts.length > 1 ? "All accounts synced" : "Synced";
  }

  return (
    <div className={`sync-status${failed && !syncing ? " sync-status--error" : ""}`}>
      {icon}
      <span className="sync-status__text" title={text}>
        {text}
        {!syncing && !failed && last && (
          <span className="sync-status__time"> · {formatTime(new Date(last))}</span>
        )}
      </span>
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        title="Sync all accounts"
        aria-label="Sync all accounts"
        disabled={!!syncing}
        onClick={() => void backend.syncAll()}
      >
        <RefreshCw size={14} />
      </button>
    </div>
  );
}
