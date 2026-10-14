import type { ReactNode } from "react";

import { Dialog } from "./Dialog";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  message: ReactNode;
  /** Label of the confirming button (default "Delete"). */
  confirmLabel?: string;
  /** Disables the buttons while the action runs. */
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/** Marks an element as the one <dialog>.showModal() focuses first. */
function autofocus(el: HTMLElement | null) {
  el?.setAttribute("autofocus", "");
}

/**
 * Asks the user to confirm an action, typically a deletion. Focus starts on
 * Cancel so an accidental Enter does not destroy anything.
 */
export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel = "Delete",
  busy = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  return (
    <Dialog
      open={open}
      onClose={onCancel}
      title={title}
      width={420}
      footer={
        <>
          <button type="button" className="btn" ref={autofocus} onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button type="button" className="btn btn--primary" onClick={onConfirm} disabled={busy}>
            {confirmLabel}
          </button>
        </>
      }
    >
      <p>{message}</p>
    </Dialog>
  );
}
