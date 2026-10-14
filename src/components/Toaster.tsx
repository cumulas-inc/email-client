import { CircleAlert, CircleCheck, Info, X } from "lucide-react";

import { useToasts } from "./toast";

export function Toaster() {
  const { toasts, dismiss } = useToasts();
  return (
    <div className="toaster" role="region" aria-label="Notifications" aria-live="polite">
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
