import { CircleAlert, RefreshCw, Search, X } from "lucide-react";
import { useRef } from "react";

import { useBackend } from "../api/context";
import { useAccounts, useSyncStatuses } from "../api/queries";
import { useAppStore } from "./store";

function SyncButton() {
  const backend = useBackend();
  const { data: statuses = [] } = useSyncStatuses();
  const syncing = statuses.some((s) => s.state === "syncing");
  const errors = statuses.filter((s) => s.state === "error");
  const message = syncing
    ? (statuses.find((s) => s.state === "syncing")?.message ?? "Syncing…")
    : errors.length > 0
      ? (errors[0]?.message ?? "Sync failed")
      : "Sync all accounts";
  return (
    <button
      type="button"
      className="titlebar__button"
      title={message}
      aria-label={message}
      onClick={() => void backend.syncAll()}
      disabled={syncing}
    >
      {errors.length > 0 && !syncing ? (
        <CircleAlert size={18} />
      ) : (
        <RefreshCw size={18} className={syncing ? "spin" : undefined} />
      )}
    </button>
  );
}

export function TitleBar() {
  const { module, search, setSearch, setModule } = useAppStore();
  const { data: accounts = [] } = useAccounts();
  const inputRef = useRef<HTMLInputElement>(null);
  const placeholder =
    module === "people" ? "Search people" : module === "tasks" ? "Search tasks" : "Search mail";

  return (
    <header className="titlebar">
      <div className="titlebar__brand">
        <img src="/icon.svg" alt="" width={20} height={20} />
        <span>Cumulas Mail</span>
      </div>
      {accounts.length > 0 && (
        <div className="titlebar__search" role="search">
          <Search size={16} aria-hidden="true" />
          <input
            ref={inputRef}
            id="global-search"
            type="search"
            placeholder={placeholder}
            aria-label={placeholder}
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              if (module === "calendar") setModule("mail");
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setSearch("");
                inputRef.current?.blur();
              }
            }}
          />
          {search && (
            <button
              type="button"
              className="titlebar__clear"
              aria-label="Clear search"
              onClick={() => setSearch("")}
            >
              <X size={14} />
            </button>
          )}
        </div>
      )}
      <div className="titlebar__actions">{accounts.length > 0 && <SyncButton />}</div>
    </header>
  );
}
