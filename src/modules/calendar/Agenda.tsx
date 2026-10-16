import clsx from "clsx";
import { addDays } from "date-fns";
import { useId, useMemo } from "react";

import type { Calendar } from "../../api/types";
import { formatDate } from "../../lib/datetime";
import { formatCount } from "../../lib/format";
import { tintStyle } from "./colors";
import { eventRange, isAllDayLike } from "./dates";
import type { EventSelectProps } from "./eventProps";
import { eventTimeRange, eventTitle, isStruck } from "./eventText";
import { useEventsInRange } from "./queries";

/** Events listed before the "more" link; the sidebar has room for three. */
const MAX_ITEMS = 3;

interface AgendaProps {
  /** Local midnight of today. */
  today: Date;
  hidden: ReadonlySet<string>;
  calendars: ReadonlyMap<string, Calendar>;
  selectedId: string | null;
  onSelect: EventSelectProps["onSelect"];
  onOpenDay: (day: Date) => void;
}

/** What tomorrow holds, in the sidebar under the calendars. */
export function Agenda({ today, hidden, calendars, selectedId, onSelect, onOpenDay }: AgendaProps) {
  const headingId = useId();
  const start = useMemo(() => addDays(today, 1), [today]);
  const end = useMemo(() => addDays(today, 2), [today]);
  const query = useEventsInRange(start, end);

  const events = useMemo(() => {
    const visible = (query.data ?? []).filter((e) => {
      // Canceled and declined events aren't part of the day.
      if (hidden.has(e.calendarId) || isStruck(e)) return false;
      // The query may still hold the previous range while it refetches.
      const range = eventRange(e);
      if (range.start >= end || range.end <= start) return false;
      // An all-day event that is already under way (a trip, a launch week)
      // isn't news about tomorrow.
      return !(isAllDayLike(e) && range.start < start);
    });
    // All-day events first, then by start time.
    return visible.sort((a, b) => {
      const allDay = Number(isAllDayLike(b)) - Number(isAllDayLike(a));
      return allDay || eventRange(a).start.getTime() - eventRange(b).start.getTime();
    });
  }, [query.data, hidden, start, end]);

  if (!query.isSuccess) return null;
  const shown = events.slice(0, MAX_ITEMS);
  const more = events.length - shown.length;

  return (
    <section className="cal-agenda" aria-labelledby={headingId}>
      <h2 id={headingId} className="cal-section-label">
        Tomorrow · {formatDate(start, "weekdayShort")}
      </h2>
      {events.length === 0 ? (
        <p className="cal-agenda__empty">Nothing scheduled</p>
      ) : (
        <ul className="cal-agenda__list">
          {shown.map((event) => (
            <li key={event.id}>
              <button
                type="button"
                className={clsx(
                  "cal-agenda__item cal-tint",
                  selectedId === event.id && "cal-agenda__item--selected",
                )}
                style={tintStyle(calendars.get(event.calendarId)?.color)}
                aria-haspopup="dialog"
                aria-expanded={selectedId === event.id}
                onClick={(e) => onSelect(event, e.currentTarget)}
              >
                <span className="cal-agenda__title" dir="auto">
                  {eventTitle(event)}
                </span>
                <span className="cal-agenda__time tnum">{eventTimeRange(event)}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {more > 0 && (
        <button
          type="button"
          className="cal-more-link cal-agenda__more"
          aria-label={`${more} more event${more === 1 ? "" : "s"} tomorrow`}
          onClick={() => onOpenDay(start)}
        >
          +{formatCount(more)} more
        </button>
      )}
    </section>
  );
}
