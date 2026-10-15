import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { useBackend } from "../api/context";
import { qk } from "../api/queries";
import type { PushStatus, SyncStatus } from "../api/types";

/** Keeps React Query caches in step with backend change notifications. */
export function useBackendEvents(): void {
  const backend = useBackend();
  const client = useQueryClient();

  useEffect(() => {
    const offData = backend.onDataChanged((event) => {
      for (const kind of event.kinds) {
        void client.invalidateQueries({ queryKey: qk.root(kind) });
      }
    });
    const offSync = backend.onSyncStatus((status) => {
      client.setQueryData<SyncStatus[]>(qk.syncStatuses(), (old = []) => {
        const others = old.filter((s) => s.accountId !== status.accountId);
        return [...others, status];
      });
    });
    const offPush = backend.onPushStatus((status) => {
      client.setQueryData<PushStatus[]>(qk.pushStatuses(), (old) => {
        // Nothing to update until someone asked for the statuses.
        if (!old) return old;
        const others = old.filter((s) => s.accountId !== status.accountId);
        return [...others, status];
      });
    });
    return () => {
      offData();
      offSync();
      offPush();
    };
  }, [backend, client]);
}
