import clsx from "clsx";
import { ChevronLeft, ChevronRight } from "lucide-react";
import type { CSSProperties } from "react";

import type { CalendarEvent } from "../../api/types";
import { formatTime } from "../../lib/datetime";
import { AvailabilityGlyph } from "./AvailabilityGlyph";
import { eventRange, isAllDayLike } from "./dates";
import { type EventSelectProps, eventClasses, eventStyle, selectHandlers } from "./eventProps";
import { eventAriaLabel, eventTitle } from "./eventText";

interface EventBarProps extends EventSelectProps {
  event: CalendarEvent;
  /** Columns the bar covers in its row. */
  span: number;
  joinsBefore: boolean;
  joinsAfter: boolean;
  /** Quiet text after the title, e.g. the calendar's name. */
  detail?: string;
}

/**
 * An all-day or multi-day event drawn as a bar across day cells. An end that
 * carries on past the row (into the next week, or behind "+N more") is
 * squared off with a chevron, inset like any other end.
 */
export function EventBar({
  event,
  span,
  joinsBefore,
  joinsAfter,
  detail,
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
      {joinsBefore && (
        <ChevronLeft size={12} strokeWidth={2.25} className="cal-bar__cont" aria-hidden="true" />
      )}
      <AvailabilityGlyph event={event} className="cal-bar__glyph" />
      {timed && !joinsBefore && (
        <span className="cal-bar__time">{formatTime(eventRange(event).start)}</span>
      )}
      <span className="cal-bar__title" dir="auto">
        {eventTitle(event)}
      </span>
      {detail && (
        <span className="cal-bar__detail">
          · <bdi>{detail}</bdi>
        </span>
      )}
      {joinsAfter && (
        <ChevronRight
          size={12}
          strokeWidth={2.25}
          className="cal-bar__cont cal-bar__cont--after"
          aria-hidden="true"
        />
      )}
    </button>
  );
}
