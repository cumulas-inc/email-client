import { clsx } from "clsx";
import { CircleArrowUp } from "lucide-react";
import { useId } from "react";

import { useUpdateState } from "../api/queries";
import { useCornerCard } from "./cornerCard";
import { useAppStore } from "./store";
import { installedText } from "./update";
import { useRestart } from "./useQuit";

/**
 * A card in the corner, once per installed version, saying it's there:
 * Restart starts it, and after Later the main menu, Settings → About and the
 * tray still offer it. Not a dialog, and it takes no focus: it mustn't
 * interrupt typing.
 */
export function UpdateNotice() {
  const { data: update } = useUpdateState();
  const dismissed = useAppStore((s) => s.dismissedUpdate);
  const dismiss = useAppStore((s) => s.dismissUpdate);
  const restart = useRestart();
  const titleId = useId();
  const bodyId = useId();
  const installed = update?.kind === "installed" ? update : null;
  const shown = useCornerCard(installed !== null && installed.commit !== dismissed);

  if (!shown || !installed) return null;
  const restarting = installed.restart === "restarting";
  const later = () => dismiss(installed.commit);
  return (
    <section
      className="offer"
      aria-labelledby={titleId}
      aria-describedby={bodyId}
      onKeyDown={(e) => {
        if (e.key === "Escape" && !restarting) {
          e.stopPropagation();
          later();
        }
      }}
    >
      <span className="offer__icon" aria-hidden="true">
        <CircleArrowUp size={18} />
      </span>
      <div className="offer__text">
        <h2 id={titleId} className="offer__title">
          Update installed
        </h2>
        <p id={bodyId} className="offer__body">
          {installedText(installed.restart)}
        </p>
        <div className="offer__actions">
          {/* One button from Restart to Restarting… and back, so it keeps the focus. */}
          <button
            type="button"
            className={clsx("btn btn--primary btn--sm", restarting && "btn--busy")}
            aria-disabled={restarting || undefined}
            onClick={() => {
              if (!restarting) restart();
            }}
          >
            {restarting ? "Restarting…" : "Restart"}
          </button>
          {!restarting && (
            <button type="button" className="btn btn--sm" onClick={later}>
              Later
            </button>
          )}
        </div>
      </div>
    </section>
  );
}
