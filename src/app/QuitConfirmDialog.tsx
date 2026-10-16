import { useBackend } from "../api/context";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { toast } from "../components/toast";
import { useAppStore } from "./store";

/** Asks before quitting loses an unsent message (see useQuit). */
export function QuitConfirmDialog() {
  const backend = useBackend();
  const open = useAppStore((s) => s.quitConfirmOpen);
  const setOpen = useAppStore((s) => s.setQuitConfirmOpen);
  return (
    <ConfirmDialog
      open={open}
      title="Quit Cumulas Mail?"
      message="Your unsent message will be discarded."
      confirmLabel="Discard and quit"
      onCancel={() => {
        setOpen(false);
        // Tells the desktop the user kept the message (Cancel, Close, Escape
        // or a click outside): otherwise the tray's next Quit would quit at
        // once, as for a page that never answered.
        backend
          .cancelQuit()
          .catch((e: unknown) => console.warn("Couldn't tell the desktop to keep running", e));
      }}
      onConfirm={() => {
        setOpen(false);
        backend.quitApp().catch((e: unknown) => toast.error(e));
      }}
    />
  );
}
