import type { ReactNode } from "react";

import type { Backend } from "./backend";
import { BackendContext } from "./context";

export function BackendProvider({ backend, children }: { backend: Backend; children: ReactNode }) {
  return <BackendContext.Provider value={backend}>{children}</BackendContext.Provider>;
}
