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
 * no accounts.
 */
export function createBackend(): Backend {
  if (isTauri()) return tauriBackend;
  const params = new URLSearchParams(window.location.search);
  return new MockBackend({ latency: 80, empty: params.has("empty") });
}
