import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { useBackend } from "../api/context";
import { applyBackgroundState, qk } from "../api/queries";
import type { PushStatus, SyncStatus } from "../api/types";
import { followNavigation } from "./navigation";
import { announcement, useReminderAnnouncer } from "./reminders";

/**
 * Keeps React Query caches in step with backend change notifications,
 * follows the desktop's requests to show a message, an event or a task,
 * and announces reminders as they come due.
 */
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
    const offBackground = backend.onBackgroundState((state) => applyBackgroundState(client, state));
    const offReminders = backend.onRemindersChanged((event) => {
      client.setQueryData(qk.reminders(), event.active);
      const text = announcement(event.newlyDue, new Date());
      if (text) useReminderAnnouncer.getState().announce(text);
    });
    const offNavigate = backend.onNavigate((to) => void followNavigation(to, backend, client));
    // A notification clicked while the app ran without a window opened it
    // for this. (Followed even if this effect is torn down meanwhile, as in
    // React's development double run: it was taken, and the stores are
    // global.)
    void backend.takePendingNavigation().then((to) => {
      if (to) void followNavigation(to, backend, client);
    });
    return () => {
      offData();
      offSync();
      offPush();
      offBackground();
      offReminders();
      offNavigate();
    };
  }, [backend, client]);
}
