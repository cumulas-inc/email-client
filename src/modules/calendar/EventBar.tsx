import clsx from "clsx";
import { format } from "date-fns";
import type { CSSProperties } from "react";

import type { CalendarEvent } from "../../api/types";
import { eventRange, isAllDayLike } from "./dates";
import { type EventSelectProps, eventClasses, eventStyle, selectHandlers } from "./eventProps";
import { eventAriaLabel, eventTitle } from "./eventText";

interface EventBarProps extends EventSelectProps {
  event: CalendarEvent;
  /** Columns the bar covers in its row. */
  span: number;
  joinsBefore: boolean;
  joinsAfter: boolean;
}

/** An all-day or multi-day event drawn as a bar across day cells. */
export function EventBar({
  event,
  span,
  joinsBefore,
  joinsAfter,
  calendar,
  selected,
  onSelect,
}: EventBarProps) {
  const timed = !isAllDayLike(event);
  return (
    <button
      type="button"
      className={clsx(
        "cal-bar",
        eventClasses(event, selected),
        joinsBefore && "cal-bar--joins-before",
        joinsAfter && "cal-bar--joins-after",
      )}
      style={eventStyle(calendar, { "--span": span } as CSSProperties)}
      data-event-id={event.id}
      aria-label={eventAriaLabel(event, calendar?.name)}
      aria-haspopup="dialog"
      aria-expanded={selected}
      {...selectHandlers(event, onSelect)}
    >
      {timed && !joinsBefore && (
        <span className="cal-bar__time">{format(eventRange(event).start, "HH:mm")}</span>
      )}
      <span className="cal-bar__title">{eventTitle(event)}</span>
    </button>
  );
}
