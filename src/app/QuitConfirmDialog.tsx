import { useBackend } from "../api/context";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { useAppStore } from "./store";
import { leaveNow } from "./useQuit";

/** Asks before quitting, or restarting into an update, loses an unsent message (see useQuit). */
export function QuitConfirmDialog() {
  const backend = useBackend();
  const leave = useAppStore((s) => s.leaveConfirm);
  const setLeave = useAppStore((s) => s.setLeaveConfirm);
  const restart = leave === "restart";
  return (
    <ConfirmDialog
      open={leave !== null}
      title={restart ? "Restart Cumulas Mail?" : "Quit Cumulas Mail?"}
      message="Your unsent message will be discarded."
      confirmLabel={restart ? "Discard and restart" : "Discard and quit"}
      onCancel={() => {
        setLeave(null);
        // Tells the desktop the user kept the message (Cancel, Close, Escape
        // or a click outside): otherwise the tray's next Quit would quit at
        // once, as for a page that never answered.
        backend
          .cancelQuit()
          .catch((e: unknown) => console.warn("Couldn't tell the desktop to keep running", e));
      }}
      onConfirm={() => {
        setLeave(null);
        if (leave) leaveNow(backend, leave);
      }}
    />
  );
}
