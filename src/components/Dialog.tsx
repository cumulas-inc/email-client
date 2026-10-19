import { X } from "lucide-react";
import { useEffect, useId, useRef, type ReactNode } from "react";

import { useModals } from "./modals";

interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
  /** The title's id, for dialogs that move focus to it as their content changes. */
  titleId?: string;
  /** False while what the dialog started can't be left halfway: Escape and Close do nothing. */
  canClose?: boolean;
  /**
   * Whether a click on the backdrop closes it. False for a dialog that holds
   * something a stray click mustn't end, such as a sign-in.
   */
  closeOnBackdrop?: boolean;
}

/** Modal dialog built on <dialog>: focus trapping and Escape come for free. */
export function Dialog({
  open,
  onClose,
  title,
  children,
  footer,
  width = 480,
  titleId: givenTitleId,
  canClose = true,
  closeOnBackdrop = true,
}: DialogProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const ownTitleId = useId();
  const titleId = givenTitleId ?? ownTitleId;

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      // jsdom lacks showModal; fall back to the open attribute.
      if (typeof dialog.showModal === "function") dialog.showModal();
      else dialog.setAttribute("open", "");
    } else if (!open && dialog.open) {
      if (typeof dialog.close === "function") dialog.close();
      else dialog.removeAttribute("open");
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    useModals.setState((s) => ({ open: s.open + 1 }));
    return () => useModals.setState((s) => ({ open: s.open - 1 }));
  }, [open]);

  if (!open) return null;

  return (
    <dialog
      ref={ref}
      className="dialog"
      aria-labelledby={titleId}
      style={{ width }}
      onCancel={(e) => {
        e.preventDefault();
        // React passes the event on to a dialog this one opened from (Settings),
        // which Escape here mustn't close as well.
        e.stopPropagation();
        if (canClose) onClose();
      }}
      onClose={(e) => {
        // Closed by the browser itself: a second Escape can't be refused.
        // Open again while it can't close; otherwise its owner closes it.
        // Like cancel, React passes it on to a dialog this one opened from.
        e.stopPropagation();
        const dialog = ref.current;
        if (!canClose) {
          if (dialog && !dialog.open && typeof dialog.showModal === "function") dialog.showModal();
        } else {
          onClose();
        }
      }}
      onMouseDown={(e) => {
        // Click on the backdrop closes.
        if (e.target === ref.current && canClose && closeOnBackdrop) onClose();
      }}
    >
      <div className="dialog__header">
        <h2 id={titleId} className="dialog__title" tabIndex={givenTitleId ? -1 : undefined}>
          {title}
        </h2>
        <button
          type="button"
          className="icon-btn"
          aria-label="Close"
          disabled={!canClose}
          onClick={onClose}
        >
          <X size={18} />
        </button>
      </div>
      <div className="dialog__body">{children}</div>
      {footer && <div className="dialog__footer">{footer}</div>}
    </dialog>
  );
}
