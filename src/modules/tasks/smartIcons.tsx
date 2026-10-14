import { CalendarDays, Infinity as InfinityIcon, Star } from "lucide-react";

import type { SmartListId } from "./taskUtils";

/** Icon of a smart list, as used in the sidebar and the list header. */
export function smartListIcon(id: SmartListId, size = 18) {
  switch (id) {
    case "important":
      return <Star size={size} />;
    case "planned":
      return <CalendarDays size={size} />;
    case "all":
      return <InfinityIcon size={size} />;
  }
}
