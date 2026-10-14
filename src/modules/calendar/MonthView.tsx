import clsx from "clsx";
import { addDays, format, isSameDay, isSameMonth, isWeekend } from "date-fns";
import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";

import type { Calendar, CalendarEvent } from "../../api/types";
import { eventRange } from "./dates";
import { EventBar } from "./EventBar";
import { type EventSelectProps, eventClasses, eventStyle, selectHandlers } from "./eventProps";
import { eventAriaLabel, eventTitle } from "./eventText";
import { useElementHeight } from "./hooks";
import { cellLanes, chunk, hiddenCount, isBar, laneTable, rowItems, visibleLanes } from "./layout";

/** Lines per day when the grid has not been measured (tests, first paint). */
export const DEFAULT_MONTH_LINES = 3;
const LINE_HEIGHT = 22;
const DAY_HEADER_HEIGHT = 30;

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
      ? Math.max(1, Math.floor((bodyHeight / weeks.length - DAY_HEADER_HEIGHT - 4) / LINE_HEIGHT))
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
    <div className="cal-month" role="grid" aria-label={format(anchor, "MMMM yyyy")}>
      <div className="cal-month__head" role="rowgroup">
        <div className="cal-month__headrow" role="row">
          {weeks[0]!.map((d) => (
            <div key={d.getTime()} className="cal-month__weekday" role="columnheader">
              <abbr title={format(d, "EEEE")}>{format(d, "EEE")}</abbr>
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
          const visible = (col: number) => visibleLanes(table[col]?.length ?? 0, maxLines);
          return (
            <div key={week[0]!.getTime()} className="cal-month__week" role="row">
              {week.map((day, col) => {
                const isToday = isSameDay(day, today);
                const more = hiddenCount(table, col, visible(col));
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
                        className="cal-month__daynum"
                        tabIndex={tabbable && isSameDay(day, tabbable) ? 0 : -1}
                        aria-label={`${format(day, "EEEE, d MMMM yyyy")}, ${
                          dayEvents === 1 ? "1 event" : `${dayEvents} events`
                        }`}
                        title="Open this day"
                        onClick={(e) => {
                          e.stopPropagation();
                          onOpenDay(day);
                        }}
                        onFocus={() => setFocused(day)}
                      >
                        {format(day, day.getDate() === 1 ? "d MMM" : "d")}
                      </button>
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
                      {more > 0 && (
                        <button
                          type="button"
                          className="cal-month__more"
                          aria-label={`${more} more event${more === 1 ? "" : "s"} on ${format(day, "EEEE d MMMM")}`}
                          onClick={(e) => {
                            e.stopPropagation();
                            onOpenDay(day);
                          }}
                        >
                          +{more} more
                        </button>
                      )}
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
      <span className="cal-entry__dot" aria-hidden="true" />
      <span className="cal-entry__time">{format(eventRange(event).start, "HH:mm")}</span>
      <span className="cal-entry__title">{eventTitle(event)}</span>
    </button>
  );
}
