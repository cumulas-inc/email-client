import clsx from "clsx";
import { addDays, isSameDay, isSameMonth, isWeekend } from "date-fns";
import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";

import type { Calendar, CalendarEvent } from "../../api/types";
import { formatDate, formatTime } from "../../lib/datetime";
import { formatCount } from "../../lib/format";
import { AvailabilityGlyph } from "./AvailabilityGlyph";
import { eventRange } from "./dates";
import { EventBar } from "./EventBar";
import { type EventSelectProps, eventClasses, eventStyle, selectHandlers } from "./eventProps";
import { displayShowAs, eventAriaLabel, eventTitle } from "./eventText";
import { useElementHeight } from "./hooks";
import { cellLanes, chunk, hiddenItems, isBar, laneTable, rowItems } from "./layout";

/** Lines per day when the grid has not been measured (tests, first paint). */
export const DEFAULT_MONTH_LINES = 3;
/** An event line: 20px and the 2px gap under it (which also ends the cell). */
const LINE_HEIGHT = 22;
/** The day number and "+N more" share this row, so events get every line below. */
const DAY_HEADER_HEIGHT = 28;
/** Each week row ends in a 1px rule. */
const WEEK_BORDER = 1;

interface MonthViewProps {
  anchor: Date;
  /** The 42 days of the grid. */
  days: Date[];
  events: CalendarEvent[];
  calendars: ReadonlyMap<string, Calendar>;
  today: Date;
  selectedId: string | null;
  onSelect: EventSelectProps["onSelect"];
  onCreate: (start: Date, end: Date, allDay: boolean) => void;
  onOpenDay: (day: Date) => void;
}

const KEY_STEPS: Record<string, number> = {
  ArrowLeft: -1,
  ArrowRight: 1,
  ArrowUp: -7,
  ArrowDown: 7,
};

export function MonthView({
  anchor,
  days,
  events,
  calendars,
  today,
  selectedId,
  onSelect,
  onCreate,
  onOpenDay,
}: MonthViewProps) {
  const bodyRef = useRef<HTMLDivElement>(null);
  const bodyHeight = useElementHeight(bodyRef);
  const weeks = useMemo(() => chunk(days, 7), [days]);
  const maxLines =
    bodyHeight > 0
      ? Math.max(
          1,
          Math.floor((bodyHeight / weeks.length - WEEK_BORDER - DAY_HEADER_HEIGHT) / LINE_HEIGHT),
        )
      : DEFAULT_MONTH_LINES;

  const [focused, setFocused] = useState<Date>(anchor);
  const [shownAnchor, setShownAnchor] = useState(anchor);
  if (shownAnchor.getTime() !== anchor.getTime()) {
    setShownAnchor(anchor);
    setFocused(anchor);
  }
  const tabbable =
    days.find((d) => isSameDay(d, focused)) ?? days.find((d) => isSameDay(d, anchor));
  const moveFocus = useRef(false);
  useEffect(() => {
    if (!moveFocus.current) return;
    moveFocus.current = false;
    bodyRef.current?.querySelector<HTMLButtonElement>('.cal-month__daynum[tabindex="0"]')?.focus();
  }, [focused]);

  const rows = useMemo(
    () =>
      weeks.map((week) => {
        const table = laneTable(rowItems(events, week, { includeTimed: true }), 7);
        return { week, table };
      }),
    [weeks, events],
  );

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = KEY_STEPS[e.key];
    if (step === undefined || !(e.target as HTMLElement).classList.contains("cal-month__daynum")) {
      return;
    }
    const next = addDays(tabbable ?? anchor, step);
    if (!days.some((d) => isSameDay(d, next))) return;
    e.preventDefault();
    moveFocus.current = true;
    setFocused(next);
  };

  return (
    <div className="cal-month" role="grid" aria-label={formatDate(anchor, "monthYear")}>
      <div className="cal-month__head" role="rowgroup">
        <div className="cal-month__headrow" role="row">
          {weeks[0]!.map((d) => (
            <div key={d.getTime()} className="cal-month__weekday" role="columnheader">
              <abbr title={formatDate(d, "weekdayName")}>{formatDate(d, "weekday")}</abbr>
            </div>
          ))}
        </div>
      </div>
      <div
        className="cal-month__body"
        role="rowgroup"
        ref={bodyRef}
        style={{ gridTemplateRows: `repeat(${weeks.length}, minmax(0, 1fr))` }}
        onKeyDown={onKeyDown}
      >
        {rows.map(({ week, table }) => {
          // Every day shows the same lanes, so a bar across several days is
          // drawn across all of them or counted in "+N more" on each.
          const visible = () => maxLines;
          return (
            <div key={week[0]!.getTime()} className="cal-month__week" role="row">
              {week.map((day, col) => {
                const isToday = isSameDay(day, today);
                const hidden = hiddenItems(table, col, maxLines);
                const more = hidden.length;
                const dayEvents = (table[col] ?? []).filter(Boolean).length;
                return (
                  <div
                    key={day.getTime()}
                    role="gridcell"
                    className={clsx("cal-month__cell", {
                      "cal-month__cell--outside": !isSameMonth(day, anchor),
                      "cal-month__cell--today": isToday,
                      "cal-month__cell--weekend": isWeekend(day),
                    })}
                    aria-current={isToday ? "date" : undefined}
                    onClick={() => onCreate(day, addDays(day, 1), true)}
                  >
                    <div className="cal-month__dayhead">
                      <button
                        type="button"
                        className="cal-month__daynum tnum"
                        tabIndex={tabbable && isSameDay(day, tabbable) ? 0 : -1}
                        aria-label={`${formatDate(day, "weekdayLongYear")}, ${
                          dayEvents === 1 ? "1 event" : `${dayEvents} events`
                        }`}
                        title="Open this day"
                        onClick={(e) => {
                          e.stopPropagation();
                          onOpenDay(day);
                        }}
                        onFocus={() => setFocused(day)}
                      >
                        {formatDate(day, day.getDate() === 1 ? "short" : "dayOfMonth")}
                      </button>
                      {more > 0 && (
                        <button
                          type="button"
                          className="cal-more-link cal-month__more"
                          aria-label={`${more} more event${more === 1 ? "" : "s"} on ${formatDate(day, "weekdayLong")}`}
                          // Where these events are shown (see drawnEvent).
                          data-event-ids={hidden.map((e) => e.id).join(" ")}
                          onClick={(e) => {
                            e.stopPropagation();
                            onOpenDay(day);
                          }}
                        >
                          +{formatCount(more)}
                          <span className="cal-month__more-word"> more</span>
                        </button>
                      )}
                    </div>
                    <div className="cal-month__lanes">
                      {cellLanes(table, col, visible).map(({ lane, piece }) => {
                        if (!piece) {
                          return (
                            <div key={`gap-${lane}`} className="cal-bar-gap" aria-hidden="true" />
                          );
                        }
                        const event = piece.placed.item;
                        const common = {
                          event,
                          calendar: calendars.get(event.calendarId),
                          selected: selectedId === event.id,
                          onSelect,
                        };
                        return isBar(piece.placed) ? (
                          <EventBar
                            key={piece.placed.key}
                            span={piece.span}
                            joinsBefore={piece.joinsBefore}
                            joinsAfter={piece.joinsAfter}
                            {...common}
                          />
                        ) : (
                          <MonthEntry key={piece.placed.key} {...common} />
                        );
                      })}
                    </div>
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>
    </div>
  );
}

interface MonthEntryProps extends EventSelectProps {
  event: CalendarEvent;
}

/** A timed single-day event in a month cell: dot, start time and title. */
function MonthEntry({ event, calendar, selected, onSelect }: MonthEntryProps) {
  return (
    <button
      type="button"
      className={clsx("cal-entry", eventClasses(event, selected))}
      style={eventStyle(calendar)}
      data-event-id={event.id}
      aria-label={eventAriaLabel(event, calendar?.name)}
      aria-haspopup="dialog"
      aria-expanded={selected}
      {...selectHandlers(event, onSelect)}
    >
      {displayShowAs(event) === "workingElsewhere" ? (
        <AvailabilityGlyph event={event} className="cal-entry__glyph" size={12} />
      ) : (
        <span className="cal-entry__dot" aria-hidden="true" />
      )}
      <span className="cal-entry__time tnum">{formatTime(eventRange(event).start)}</span>
      <span className="cal-entry__title" dir="auto">
        {eventTitle(event)}
      </span>
    </button>
  );
}
