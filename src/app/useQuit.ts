import { useCallback, useEffect } from "react";

import type { Backend } from "../api/backend";
import { useBackend } from "../api/context";
import { toast } from "../components/toast";
import { useMailStore } from "../modules/mail/mailStore";
import { type Leave, useAppStore } from "./store";

/**
 * Quits the app, even while running in the background is on (the main menu,
 * the tray). An unsent message with changes is lost on quitting, so that
 * asks first (QuitConfirmDialog).
 */
export function useQuit(): () => void {
  return useLeave("quit");
}

/**
 * Restarts into the installed update (its card, the main menu, Settings →
 * About, the tray, its notification). That quits too, so it asks first the
 * same way.
 */
export function useRestart(): () => void {
  return useLeave("restart");
}

function useLeave(how: Leave): () => void {
  const backend = useBackend();
  return useCallback(() => {
    if (useMailStore.getState().compose?.dirty) {
      useAppStore.getState().setLeaveConfirm(how);
      return;
    }
    leaveNow(backend, how);
  }, [backend, how]);
}

/** Quits or restarts without asking; a restart that fails says why, and the app keeps running. */
export function leaveNow(backend: Backend, how: Leave): void {
  const left = how === "quit" ? backend.quitApp() : backend.restartApp();
  left.catch((e: unknown) => toast.error(e));
}

/**
 * Quits when the tray asks, and restarts when the tray or a notification
 * asks, the same ways. The desktop shows the window if the page asks first,
 * and leaves anyway when asked again while it asks; QuitConfirmDialog tells
 * it when the user keeps the message instead.
 */
export function useQuitRequests(): void {
  const backend = useBackend();
  const quit = useQuit();
  const restart = useRestart();
  useEffect(() => backend.onQuitRequested(quit), [backend, quit]);
  useEffect(() => backend.onRestartRequested(restart), [backend, restart]);
}
