import clsx from "clsx";
import { addDays, format, isSameDay, isWeekend } from "date-fns";
import {
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import type { Calendar, CalendarEvent } from "../../api/types";
import { atMinutes, minutesOfDay } from "./dates";
import { EventBar } from "./EventBar";
import { type EventSelectProps, eventClasses, eventStyle, selectHandlers } from "./eventProps";
import { eventAriaLabel, eventTimeRange, eventTitle } from "./eventText";
import {
  type PlacedSegment,
  cellLanes,
  laneTable,
  layoutDay,
  rowItems,
  timedSegmentsForDay,
} from "./layout";

/** Height of one hour in pixels (two 30-minute slots). */
export const HOUR_HEIGHT = 48;
const SLOT_MINUTES = 30;
const SLOTS = (24 * 60) / SLOT_MINUTES;
const SLOT_HEIGHT = (HOUR_HEIGHT * SLOT_MINUTES) / 60;
/** Shortest drawn event, in minutes, so a title always fits. */
const MIN_MINUTES = 22;
const WORK_START = 8 * 60;
const WORK_END = 17 * 60;
const SCROLL_TO_MINUTES = 8 * 60;
const LINE_HEIGHT = 16;

interface TimeGridProps {
  days: Date[];
  events: CalendarEvent[];
  calendars: ReadonlyMap<string, Calendar>;
  now: Date;
  selectedId: string | null;
  onSelect: EventSelectProps["onSelect"];
  onCreate: (start: Date, end: Date, allDay: boolean) => void;
  /** Open a single day (from its header); omitted in day view. */
  onOpenDay?: (day: Date) => void;
}

interface DragState {
  col: number;
  from: number;
  to: number;
}

function timeZoneLabel(now: Date): string {
  try {
    const parts = new Intl.DateTimeFormat(undefined, { timeZoneName: "shortOffset" }).formatToParts(
      now,
    );
    return parts.find((p) => p.type === "timeZoneName")?.value ?? "";
  } catch {
    return "";
  }
}

/** "HH:mm" for minutes since midnight. */
function clockLabel(minutes: number): string {
  const h = Math.floor(minutes / 60) % 24;
  return `${String(h).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
}

/** Day, work week and week views: a 24-hour grid with an all-day lane. */
export function TimeGrid({
  days,
  events,
  calendars,
  now,
  selectedId,
  onSelect,
  onCreate,
  onOpenDay,
}: TimeGridProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const headRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<DragState | null>(null);
  const stopDragRef = useRef<(() => void) | null>(null);
  const [drag, setDrag] = useState<DragState | null>(null);

  // Stop listening if the grid goes away in the middle of a drag.
  useEffect(() => () => stopDragRef.current?.(), []);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = (SCROLL_TO_MINUTES / 60) * HOUR_HEIGHT - 8;
  }, []);

  // Keep the header columns aligned with the body when it shows a scrollbar.
  useEffect(() => {
    const scroll = scrollRef.current;
    const head = headRef.current;
    if (!scroll || !head) return;
    const align = () => {
      head.style.paddingRight = `${scroll.offsetWidth - scroll.clientWidth}px`;
    };
    align();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(align);
    observer.observe(scroll);
    return () => observer.disconnect();
  }, []);

  const allDay = useMemo(() => {
    const placed = rowItems(events, days, { includeTimed: false });
    return laneTable(placed, days.length);
  }, [events, days]);

  const columns = useMemo(
    () => days.map((day) => layoutDay(timedSegmentsForDay(events, day), MIN_MINUTES)),
    [events, days],
  );

  const startDrag = (e: ReactPointerEvent<HTMLDivElement>, col: number) => {
    if (e.button !== 0) return;
    const slot = (e.target as HTMLElement).closest<HTMLElement>("[data-slot]");
    if (!slot) return;
    e.preventDefault();
    const index = Number(slot.dataset.slot);
    const column = e.currentTarget;
    const state = { col, from: index, to: index };
    dragRef.current = state;
    setDrag(state);

    const onMove = (ev: PointerEvent) => {
      const current = dragRef.current;
      if (!current) return;
      const rect = column.getBoundingClientRect();
      if (rect.height === 0) return;
      const to = Math.min(
        SLOTS - 1,
        Math.max(0, Math.floor((ev.clientY - rect.top) / SLOT_HEIGHT)),
      );
      if (to !== current.to) {
        dragRef.current = { ...current, to };
        setDrag(dragRef.current);
      }
    };
    const stop = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      stopDragRef.current = null;
    };
    const onUp = () => {
      stop();
      const current = dragRef.current;
      dragRef.current = null;
      setDrag(null);
      const day = current ? days[current.col] : undefined;
      if (!current || !day) return;
      const first = Math.min(current.from, current.to);
      const last = Math.max(current.from, current.to);
      onCreate(
        atMinutes(day, first * SLOT_MINUTES),
        atMinutes(day, (last + 1) * SLOT_MINUTES),
        false,
      );
    };
    const onCancel = () => {
      stop();
      dragRef.current = null;
      setDrag(null);
    };
    stopDragRef.current?.();
    stopDragRef.current = stop;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
  };

  const wide = days.length <= 5;
  const nowMinutes = minutesOfDay(now);

  return (
    <div className="cal-tg" style={{ "--cal-days": days.length } as CSSProperties}>
      <div className="cal-tg__head" ref={headRef}>
        <div className="cal-tg__row cal-tg__days">
          <div className="cal-tg__gutter cal-tg__tz" title="Time zone">
            {timeZoneLabel(now)}
          </div>
          {days.map((day) => {
            const today = isSameDay(day, now);
            const label = (
              <>
                <span className="cal-tg__daynum">{format(day, "d")}</span>
                <span className="cal-tg__dayname">{format(day, wide ? "EEEE" : "EEE")}</span>
              </>
            );
            return (
              <div
                key={day.getTime()}
                className={clsx("cal-tg__dayhead", today && "cal-tg__dayhead--today")}
                aria-current={today ? "date" : undefined}
              >
                {onOpenDay ? (
                  <button
                    type="button"
                    className="cal-tg__daybtn"
                    aria-label={format(day, "EEEE, d MMMM yyyy")}
                    title="Open this day"
                    onClick={() => onOpenDay(day)}
                  >
                    {label}
                  </button>
                ) : (
                  <div className="cal-tg__daybtn">{label}</div>
                )}
              </div>
            );
          })}
        </div>
        <div className="cal-tg__row cal-tg__allday">
          <div className="cal-tg__gutter cal-tg__allday-label">All day</div>
          {days.map((day, col) => (
            <div
              key={day.getTime()}
              className={clsx("cal-tg__allday-cell", isWeekend(day) && "cal-tg__allday-cell--off")}
              role="group"
              aria-label={`All-day events, ${format(day, "EEEE d MMMM")}`}
              onClick={() => onCreate(day, addDays(day, 1), true)}
            >
              {cellLanes(allDay, col, () => Infinity).map(({ lane, piece }) =>
                piece ? (
                  <EventBar
                    key={piece.placed.key}
                    event={piece.placed.item}
                    span={piece.span}
                    joinsBefore={piece.joinsBefore}
                    joinsAfter={piece.joinsAfter}
                    calendar={calendars.get(piece.placed.item.calendarId)}
                    selected={selectedId === piece.placed.item.id}
                    onSelect={onSelect}
                  />
                ) : (
                  <div key={`gap-${lane}`} className="cal-bar-gap" aria-hidden="true" />
                ),
              )}
            </div>
          ))}
        </div>
      </div>

      <div className="cal-tg__scroll" ref={scrollRef}>
        <div className="cal-tg__row cal-tg__body" style={{ height: 24 * HOUR_HEIGHT }}>
          <div className="cal-tg__gutter cal-tg__hours" aria-hidden="true">
            {Array.from({ length: 23 }, (_, i) => i + 1).map((hour) => (
              <span key={hour} className="cal-tg__hour" style={{ top: hour * HOUR_HEIGHT }}>
                {clockLabel(hour * 60)}
              </span>
            ))}
          </div>
          {days.map((day, col) => {
            const today = isSameDay(day, now);
            const off = isWeekend(day);
            return (
              <div
                key={day.getTime()}
                className="cal-tg__col"
                role="group"
                aria-label={format(day, "EEEE d MMMM")}
                onPointerDown={(e) => startDrag(e, col)}
              >
                {Array.from({ length: SLOTS }, (_, i) => {
                  const minutes = i * SLOT_MINUTES;
                  return (
                    <div
                      key={i}
                      className={clsx(
                        "cal-tg__slot",
                        i % 2 === 1 && "cal-tg__slot--half",
                        (off || minutes < WORK_START || minutes >= WORK_END) && "cal-tg__slot--off",
                      )}
                      data-slot={i}
                      data-time={`${format(day, "yyyy-MM-dd")}T${clockLabel(minutes)}`}
                      aria-hidden="true"
                    />
                  );
                })}
                {columns[col]!.map((placed) => (
                  <TimedEvent
                    key={placed.key}
                    placed={placed}
                    calendar={calendars.get(placed.item.calendarId)}
                    selected={selectedId === placed.item.id}
                    onSelect={onSelect}
                  />
                ))}
                {drag && drag.col === col && <DragGhost drag={drag} />}
                {today && (
                  <div
                    className="cal-now"
                    style={{ top: (nowMinutes / 60) * HOUR_HEIGHT }}
                    aria-hidden="true"
                  />
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function DragGhost({ drag }: { drag: DragState }) {
  const first = Math.min(drag.from, drag.to);
  const last = Math.max(drag.from, drag.to);
  return (
    <div
      className="cal-ghost"
      style={{ top: first * SLOT_HEIGHT, height: (last - first + 1) * SLOT_HEIGHT }}
      aria-hidden="true"
    >
      {clockLabel(first * SLOT_MINUTES)} – {clockLabel((last + 1) * SLOT_MINUTES)}
    </div>
  );
}

interface TimedEventProps extends EventSelectProps {
  placed: PlacedSegment<CalendarEvent>;
}

function TimedEvent({ placed, calendar, selected, onSelect }: TimedEventProps) {
  const event = placed.item;
  const minutes = Math.max(placed.end - placed.start, MIN_MINUTES);
  const height = (minutes / 60) * HOUR_HEIGHT;
  // Text lines that fit: title first, then the time, then the location.
  const lines = Math.max(1, Math.floor((height - 5) / LINE_HEIGHT));
  const compact = lines < 2;
  const showLocation = !!event.location && lines >= 3;
  const titleLines = Math.max(1, Math.min(3, lines - 1 - (showLocation ? 1 : 0)));
  const left = (placed.column / placed.columns) * 100;
  const width = (placed.span / placed.columns) * 100;
  return (
    <button
      type="button"
      className={clsx(
        "cal-ev",
        eventClasses(event, selected),
        compact && "cal-ev--compact",
        placed.continuesBefore && "cal-ev--joins-before",
        placed.continuesAfter && "cal-ev--joins-after",
      )}
      style={eventStyle(calendar, {
        top: (placed.start / 60) * HOUR_HEIGHT,
        height: height - 1,
        left: `${left}%`,
        width: `calc(${width}% - ${placed.column + placed.span < placed.columns ? 2 : 8}px)`,
        "--title-lines": titleLines,
      } as CSSProperties)}
      data-event-id={event.id}
      aria-label={eventAriaLabel(event, calendar?.name)}
      aria-haspopup="dialog"
      aria-expanded={selected}
      {...selectHandlers(event, onSelect)}
    >
      <span className="cal-ev__title">{eventTitle(event)}</span>
      {compact ? (
        event.location && <span className="cal-ev__meta"> {event.location}</span>
      ) : (
        <>
          <span className="cal-ev__meta">{eventTimeRange(event)}</span>
          {showLocation && <span className="cal-ev__meta">{event.location}</span>}
        </>
      )}
    </button>
  );
}
