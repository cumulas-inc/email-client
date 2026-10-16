import { useCallback, useEffect } from "react";

import { useBackend } from "../api/context";
import { toast } from "../components/toast";
import { useMailStore } from "../modules/mail/mailStore";
import { useAppStore } from "./store";

/**
 * Quits the app, even while running in the background is on (the main menu,
 * the tray). An unsent message with changes is lost on quitting, so that
 * asks first (QuitConfirmDialog).
 */
export function useQuit(): () => void {
  const backend = useBackend();
  return useCallback(() => {
    if (useMailStore.getState().compose?.dirty) {
      useAppStore.getState().setQuitConfirmOpen(true);
      return;
    }
    backend.quitApp().catch((e: unknown) => toast.error(e));
  }, [backend]);
}

/**
 * Quits when the tray asks, the same way. The desktop shows the window if
 * the page asks first, and quits anyway on a second Quit while it asks;
 * QuitConfirmDialog tells it when the user keeps the message instead.
 */
export function useQuitRequests(): void {
  const backend = useBackend();
  const quit = useQuit();
  useEffect(() => backend.onQuitRequested(quit), [backend, quit]);
}
