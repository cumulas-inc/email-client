import { LogIn } from "lucide-react";
import { useId } from "react";

import { errorMessage } from "../api/errors";
import { useBackgroundState, useSetStartAtLogin, useSettings } from "../api/queries";
import { toast } from "../components/toast";
import { useCornerCard } from "./cornerCard";
import { useAppStore } from "./store";

/**
 * A small card in the corner, not a dialog: the app works the same whichever
 * the user picks, and Settings has the switch.
 */
export function StartAtLoginOffer() {
  const open = useAppStore((s) => s.startAtLoginOffer);
  const close = () => useAppStore.getState().setStartAtLoginOffer(false);
  const { data: settings } = useSettings();
  const { data: background } = useBackgroundState();
  const startAtLogin = useSetStartAtLogin();
  const titleId = useId();
  const bodyId = useId();

  const applies =
    settings !== undefined &&
    !settings.startAtLogin &&
    background !== undefined &&
    background.autostart !== "unsupported";
  // Waits for the add-account dialog and Settings to close, and comes after
  // the toast that says an account is ready (adding a second account from
  // Settings shows another).
  const shown = useCornerCard(open && applies);

  const accept = () => {
    startAtLogin.mutate(
      { enabled: true, hidden: true },
      {
        onSuccess: () => {
          close();
          toast.success("Cumulas Mail will start when you log in.");
        },
        onError: (e) => {
          close();
          toast.error(errorMessage(e));
        },
      },
    );
  };

  if (!shown) return null;
  return (
    <section
      className="offer"
      aria-labelledby={titleId}
      aria-describedby={bodyId}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          close();
        }
      }}
    >
      <span className="offer__icon" aria-hidden="true">
        <LogIn size={18} />
      </span>
      <div className="offer__text">
        <h2 id={titleId} className="offer__title">
          Start Cumulas Mail when you log in?
        </h2>
        <p id={bodyId} className="offer__body">
          It starts without a window and shows new mail as it arrives. You can change this in
          Settings.
        </p>
        <div className="offer__actions">
          <button
            type="button"
            className="btn btn--primary btn--sm"
            disabled={startAtLogin.isPending}
            onClick={accept}
          >
            Start at login
          </button>
          <button type="button" className="btn btn--sm" onClick={close}>
            Not now
          </button>
        </div>
      </div>
    </section>
  );
}
