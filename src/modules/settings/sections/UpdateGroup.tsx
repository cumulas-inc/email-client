import { clsx } from "clsx";
import { CircleArrowUp } from "lucide-react";
import { useEffect, useId } from "react";

import { useUpdateState } from "../../../api/queries";
import { useAppStore } from "../../../app/store";
import { installedText } from "../../../app/update";
import { useRestart } from "../../../app/useQuit";

/** An installed update, in About, for as long as it isn't running: Restart starts it. */
export function UpdateGroup() {
  const { data: update } = useUpdateState();
  const dismiss = useAppStore((s) => s.dismissUpdate);
  const restart = useRestart();
  const id = useId();
  const commit = update?.kind === "installed" ? update.commit : null;
  // Seen here, it's been seen: the corner's card doesn't come once Settings closes.
  useEffect(() => {
    if (commit) dismiss(commit);
  }, [commit, dismiss]);

  if (update?.kind !== "installed") return null;
  const restarting = update.restart === "restarting";
  return (
    <section className="settings-group" aria-labelledby={`${id}-updates`}>
      <h4 id={`${id}-updates`} className="settings-group__title">
        Updates
      </h4>
      <div className="settings-card">
        <div className="about-row">
          <CircleArrowUp size={18} aria-hidden="true" className="about-row__icon" />
          <div className="about-row__text">
            <p className="about-row__title">Update installed</p>
            <p>{installedText(update.restart)}</p>
          </div>
          {/* One button from Restart to Restarting… and back, so it keeps the focus. */}
          <button
            type="button"
            className={clsx(
              "btn btn--primary btn--sm about-row__action",
              restarting && "btn--busy",
            )}
            aria-disabled={restarting || undefined}
            onClick={() => {
              if (!restarting) restart();
            }}
          >
            {restarting ? "Restarting…" : "Restart"}
          </button>
        </div>
      </div>
    </section>
  );
}
