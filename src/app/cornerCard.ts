import { useState } from "react";

import { useModals } from "../components/modals";
import { useToasts } from "../components/toast";

/**
 * Whether a card in the corner (StartAtLoginOffer, UpdateNotice) shows: it's
 * `open`, and no dialog covers it. It arrives after the toasts, which would
 * cover its buttons in a narrow window; once there, it stays for later toasts,
 * until a dialog covers it, and then comes back after the toasts again.
 */
export function useCornerCard(open: boolean): boolean {
  const covered = useModals((s) => s.open > 0);
  const toasting = useToasts((s) => s.toasts.length > 0);
  const [arrived, setArrived] = useState(false);
  const shown = open && !covered && (arrived || !toasting);
  if (shown && !arrived) setArrived(true);
  else if (arrived && (!open || covered)) setArrived(false);
  return shown;
}
