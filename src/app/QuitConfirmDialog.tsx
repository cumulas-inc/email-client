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
      onCancel={() => setOpen(false)}
      onConfirm={() => {
        setOpen(false);
        backend.quitApp().catch((e: unknown) => toast.error(e));
      }}
    />
  );
}
