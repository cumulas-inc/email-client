import { isTauri } from "@tauri-apps/api/core";

import type { Backend } from "./backend";
import { MockBackend } from "./mock";
import { tauriBackend } from "./tauri";

export type { Backend } from "./backend";
export * from "./types";
export { errorMessage, hasErrorCode, isBackendError } from "./errors";

/**
 * The Rust backend inside the desktop app; the in-memory demo backend in a
 * plain browser (`pnpm dev`). Add `?empty` to the URL to start the demo with
 * no accounts, or `?worst` to start it with worst-case data for stress-testing
 * the UI.
 */
export function createBackend(): Backend {
  if (isTauri()) return tauriBackend;
  const params = new URLSearchParams(window.location.search);
  return new MockBackend({
    latency: 80,
    empty: params.has("empty"),
    worst: params.has("worst"),
  });
}
