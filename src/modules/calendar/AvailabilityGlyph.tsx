import { House } from "lucide-react";

import type { CalendarEvent } from "../../api/types";
import { displayShowAs } from "./eventText";

/**
 * A small house on events marked "Working elsewhere", in place of the
 * calendar's stripe. Other availabilities show in the fill (outlined for
 * free, hatched for tentative). The event's accessible name says it in words.
 */
export function AvailabilityGlyph({
  event,
  className,
  size = 13,
}: {
  event: CalendarEvent;
  className: string;
  size?: number;
}) {
  if (displayShowAs(event) !== "workingElsewhere") return null;
  return <House size={size} strokeWidth={2} className={className} aria-hidden="true" />;
}
