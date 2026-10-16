// The settings dialog saves every change straight away; this tracks those
// saves for the small "Saving…" / "Saved" indicator in its header.

import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { create } from "zustand";

import { errorMessage } from "../../api/errors";
import { qk, useUpdateSettings } from "../../api/queries";
import type { Settings } from "../../api/types";

type SaveState = "idle" | "saving" | "saved" | "error";

interface SaveStatus {
  state: SaveState;
  /** What the indicator says of a failed save. */
  error: string | null;
  pending: number;
}

export const useSaveStatus = create<SaveStatus>(() => ({ state: "idle", error: null, pending: 0 }));

let fadeTimer: ReturnType<typeof setTimeout> | undefined;

/**
 * Runs a save and reflects it in the indicator. Rethrows failures. The
 * indicator says one as "Couldn't save: {reason}", unless `describe`, asked
 * as it fails, says something else.
 */
export async function trackSave<T>(
  work: Promise<T>,
  describe?: (e: unknown) => string | undefined,
): Promise<T> {
  clearTimeout(fadeTimer);
  useSaveStatus.setState((s) => ({ state: "saving", error: null, pending: s.pending + 1 }));
  try {
    const result = await work;
    useSaveStatus.setState((s) => {
      const pending = Math.max(0, s.pending - 1);
      return pending > 0
        ? { pending }
        : { pending, state: s.state === "error" ? "error" : "saved" };
    });
    if (useSaveStatus.getState().state === "saved") {
      fadeTimer = setTimeout(() => useSaveStatus.setState({ state: "idle" }), 2500);
    }
    return result;
  } catch (e) {
    useSaveStatus.setState((s) => ({
      pending: Math.max(0, s.pending - 1),
      state: "error",
      error: describe?.(e) ?? `Couldn't save: ${errorMessage(e)}`,
    }));
    throw e;
  }
}

export function resetSaveStatus(): void {
  clearTimeout(fadeTimer);
  useSaveStatus.setState({ state: "idle", error: null, pending: 0 });
}

/**
 * Returns `save(patch)`: applies the change to the cached settings at once
 * (so the theme, reading pane… update immediately) and persists it.
 * Resolves to false if the backend rejected it; the cache is then refreshed.
 */
export function useSaveSettings() {
  const client = useQueryClient();
  const { mutateAsync } = useUpdateSettings();
  return useCallback(
    async (patch: Partial<Settings>): Promise<boolean> => {
      const current = client.getQueryData<Settings>(qk.settings());
      if (!current) return false;
      const next = { ...current, ...patch };
      client.setQueryData(qk.settings(), next);
      try {
        await trackSave(mutateAsync(next));
        return true;
      } catch {
        void client.invalidateQueries({ queryKey: qk.settings() });
        return false;
      }
    },
    [client, mutateAsync],
  );
}
