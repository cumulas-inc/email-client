import { CircleAlert, CircleCheck, Info, X } from "lucide-react";
import { useLayoutEffect, useRef } from "react";

import { useToasts } from "./toast";

export function Toaster() {
  const { toasts, dismiss } = useToasts();
  const ref = useRef<HTMLDivElement>(null);
  // Where popovers are supported the toaster is a manual one: showing it
  // again whenever a toast arrives puts it at the top of the top layer,
  // above any open dialog. Elsewhere it stays an ordinary fixed element.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || typeof el.showPopover !== "function") return;
    try {
      if (!el.popover) el.popover = "manual";
      if (el.matches(":popover-open")) el.hidePopover();
      if (toasts.length > 0) el.showPopover();
    } catch {
      el.removeAttribute("popover");
    }
  }, [toasts.length]);
  return (
    <div ref={ref} className="toaster" role="region" aria-label="Notifications" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`toast toast--${t.kind}`}
          role={t.kind === "error" ? "alert" : "status"}
        >
          {t.kind === "error" ? (
            <CircleAlert size={18} />
          ) : t.kind === "success" ? (
            <CircleCheck size={18} />
          ) : (
            <Info size={18} />
          )}
          <span className="toast__message">{t.message}</span>
          {t.action && (
            <button
              type="button"
              className="btn btn--subtle btn--sm"
              onClick={() => {
                t.action?.onClick();
                dismiss(t.id);
              }}
            >
              {t.action.label}
            </button>
          )}
          <button
            type="button"
            className="icon-btn icon-btn--sm"
            aria-label="Dismiss"
            onClick={() => dismiss(t.id)}
          >
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
