import { useCallback } from "react";

import { useBackend } from "../api/context";
import { toast } from "../components/toast";
import { useMailStore } from "../modules/mail/mailStore";
import { useAppStore } from "./store";

/**
 * Quits the app, even while running in the background is on (Ctrl+Q, the
 * main menu). An unsent message with changes is lost on quitting, so that
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
