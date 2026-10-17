import { useEffect } from "react";

import type { Backend } from "../api/backend";
import { useBackend } from "../api/context";
import { useAppInfo } from "../api/queries";
import type { RestartPhase } from "../api/types";
import { toast } from "../components/toast";

/** What the card and Settings → About say about an installed update. */
export function installedText(restart: RestartPhase): string {
  return restart === "failed"
    ? "Couldn't restart. Try again, or quit Cumulas Mail and open it again."
    : "Restart Cumulas Mail to use the new version.";
}

/** Pages (one per backend) that said they were updated. */
const announced = new WeakSet<Backend>();

/** Says once, after a restart into an update, that it worked. */
export function useUpdatedToast(): void {
  const backend = useBackend();
  const { data: info } = useAppInfo();
  useEffect(() => {
    if (!info?.restartedToUpdate || announced.has(backend)) return;
    announced.add(backend);
    toast.success(`Updated to version ${info.version}.`);
  }, [backend, info]);
}
