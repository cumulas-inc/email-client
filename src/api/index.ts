import { isTauri } from "@tauri-apps/api/core";

import type { Backend } from "./backend";
import { type DemoPush, MockBackend } from "./mock";
import { tauriBackend } from "./tauri";

export type { Backend } from "./backend";
export * from "./types";
export { errorMessage, hasErrorCode, isBackendError } from "./errors";

const DEMO_PUSH: readonly DemoPush[] = ["consent", "refused", "unreachable"];

/**
 * The Rust backend inside the desktop app; the in-memory demo backend in a
 * plain browser (`pnpm dev`). Add `?empty` to the URL to start the demo with
 * no accounts, or `?worst` to start it with worst-case data for stress-testing
 * the UI. `?push=consent|refused|unreachable` shows the Microsoft account's
 * push in that state, `?notray` a desktop without tray icons, and
 * `?reminders` a task reminder 12 minutes overdue besides the events' own.
 */
export function createBackend(): Backend {
  if (isTauri()) return tauriBackend;
  const params = new URLSearchParams(window.location.search);
  const push = params.get("push") as DemoPush | null;
  return new MockBackend({
    latency: 80,
    empty: params.has("empty"),
    worst: params.has("worst"),
    push: push && DEMO_PUSH.includes(push) ? push : undefined,
    overdueReminder: params.has("reminders"),
    background: params.has("notray") ? { trayAvailable: false } : undefined,
  });
}
