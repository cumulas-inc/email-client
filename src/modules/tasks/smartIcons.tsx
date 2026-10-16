import { CalendarClock, Layers, Star } from "lucide-react";

import type { SmartListId } from "./taskUtils";

/** Icon of a smart list in the sidebar. */
export function smartListIcon(id: SmartListId, size = 17) {
  switch (id) {
    case "starred":
      return <Star size={size} strokeWidth={1.75} />;
    case "scheduled":
      return <CalendarClock size={size} strokeWidth={1.75} />;
    case "all":
      return <Layers size={size} strokeWidth={1.75} />;
  }
}
