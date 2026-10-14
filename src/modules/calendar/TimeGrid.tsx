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
import { appLocale, formatDate, formatHour, formatTime, formatTimeSpan } from "../../lib/datetime";
import { atMinutes, eventRange, minutesOfDay } from "./dates";
import { EventBar } from "./EventBar";
import { type EventSelectProps, eventClasses, eventStyle, selectHandlers } from "./eventProps";
import {
  SHOW_AS_LABELS,
  displayShowAs,
  eventAriaLabel,
  eventTimeRange,
  eventTitle,
  formatClock,
} from "./eventText";
import {
  type PlacedSegment,
  cellLanes,
  laneTable,
  layoutDay,
  rowItems,
  timedSegmentsForDay,
} from "./layout";

/** Height of one hour in pixels (two 30-minute slots). */
export const HOUR_HEIGHT = 56;
const SLOT_MINUTES = 30;
const SLOTS = (24 * 60) / SLOT_MINUTES;
const SLOT_HEIGHT = (HOUR_HEIGHT * SLOT_MINUTES) / 60;
/** Shortest drawn event, in minutes, so a one-line title always fits. */
const MIN_MINUTES = 18;
const WORK_START = 8 * 60;
const WORK_END = 17 * 60;
const SCROLL_TO_MINUTES = 8 * 60;
/** Height of a line of event text. */
const LINE_HEIGHT = 16;
/** Events keep this much of the column free on the left and right. */
const INSET_LEFT = 4;
const INSET_RIGHT = 5;
/** Vertical gap between back-to-back events. */
const EVENT_GAP = 2;
/** Hour labels this close to the now pill make way for it. */
const NOW_PILL_CLEARANCE = 18;

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

/** "EDT", or "GMT+2" where the zone has no common abbreviation. */
function timeZoneLabel(now: Date): string {
  try {
    const parts = new Intl.DateTimeFormat(appLocale(), { timeZoneName: "short" }).formatToParts(
      now,
    );
    return parts.find((p) => p.type === "timeZoneName")?.value ?? "";
  } catch {
    return "";
  }
}

/** Reference day for formatting times of day. */
const DAY_ZERO = new Date(2000, 0, 1);

/** "HH:mm" for minutes since midnight (machine-readable, for data attributes). */
function clockLabel(minutes: number): string {
  const h = Math.floor(minutes / 60) % 24;
  return `${String(h).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
}

/** Horizontal placement inside a day column, leaving the column's insets free. */
function columnPlacement(column: number, span: number, columns: number): CSSProperties {
  const inner = `(100% - ${INSET_LEFT + INSET_RIGHT}px)`;
  const last = column + span >= columns;
  return {
    left: `calc(${INSET_LEFT}px + ${inner} * ${column / columns})`,
    width: `calc(${inner} * ${span / columns} - ${last ? 0 : 2}px)`,
  };
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
    if (el) el.scrollTop = (SCROLL_TO_MINUTES / 60) * HOUR_HEIGHT - 10;
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

  const todayCol = days.findIndex((day) => isSameDay(day, now));
  const nowTop = (minutesOfDay(now) / 60) * HOUR_HEIGHT;

  return (
    <div className="cal-tg" style={{ "--cal-days": days.length } as CSSProperties}>
      <div className="cal-tg__head" ref={headRef}>
        <div className="cal-tg__row cal-tg__days">
          <div className="cal-tg__gutter cal-tg__tz" title="Time zone">
            {timeZoneLabel(now)}
          </div>
          {days.map((day, col) => {
            const today = col === todayCol;
            const label = (
              <>
                <span className="cal-tg__daynum tnum">{formatDate(day, "dayOfMonth")}</span>
                <span className="cal-tg__dayname">{formatDate(day, "weekday")}</span>
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
                    aria-label={formatDate(day, "weekdayLongYear")}
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
              className={clsx(
                "cal-tg__allday-cell",
                isWeekend(day) && "cal-tg__allday-cell--off",
                col === todayCol && "cal-tg__allday-cell--today",
              )}
              role="group"
              aria-label={`All-day events, ${formatDate(day, "weekdayLong")}`}
              onClick={() => onCreate(day, addDays(day, 1), true)}
            >
              {cellLanes(allDay, col, () => Infinity).map(({ lane, piece }) => {
                if (!piece) {
                  return <div key={`gap-${lane}`} className="cal-bar-gap" aria-hidden="true" />;
                }
                const calendar = calendars.get(piece.placed.item.calendarId);
                return (
                  <EventBar
                    key={piece.placed.key}
                    event={piece.placed.item}
                    span={piece.span}
                    joinsBefore={piece.joinsBefore}
                    joinsAfter={piece.joinsAfter}
                    // Long bars have room to say which calendar they're from.
                    detail={piece.span > 1 ? calendar?.name : undefined}
                    calendar={calendar}
                    selected={selectedId === piece.placed.item.id}
                    onSelect={onSelect}
                  />
                );
              })}
            </div>
          ))}
        </div>
      </div>

      <div className="cal-tg__scroll" ref={scrollRef}>
        <div className="cal-tg__row cal-tg__body" style={{ height: 24 * HOUR_HEIGHT }}>
          <div className="cal-tg__gutter cal-tg__hours" aria-hidden="true">
            {Array.from({ length: 23 }, (_, i) => i + 1).map((hour) => {
              const top = hour * HOUR_HEIGHT;
              const clear = todayCol >= 0 && Math.abs(top - nowTop) < NOW_PILL_CLEARANCE;
              return (
                <span
                  key={hour}
                  className={clsx("cal-tg__hour", clear && "cal-tg__hour--clear")}
                  style={{ top }}
                >
                  {formatHour(new Date(2000, 0, 1, hour))}
                </span>
              );
            })}
            {todayCol >= 0 && (
              <span className="cal-now-pill tnum" style={{ top: nowTop }}>
                {formatTime(now)}
              </span>
            )}
          </div>
          {todayCol >= 0 && days.length > 1 && (
            <div className="cal-now-rule" style={{ top: nowTop }} aria-hidden="true" />
          )}
          {days.map((day, col) => {
            const off = isWeekend(day);
            return (
              <div
                key={day.getTime()}
                className={clsx("cal-tg__col", col === todayCol && "cal-tg__col--today")}
                role="group"
                aria-label={formatDate(day, "weekdayLong")}
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
                {col === todayCol && (
                  <div className="cal-now" style={{ top: nowTop }} aria-hidden="true" />
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
      style={{
        top: first * SLOT_HEIGHT + 1,
        height: (last - first + 1) * SLOT_HEIGHT - EVENT_GAP,
        ...columnPlacement(0, 1, 1),
      }}
      aria-hidden="true"
    >
      {formatTimeSpan(
        atMinutes(DAY_ZERO, first * SLOT_MINUTES),
        atMinutes(DAY_ZERO, (last + 1) * SLOT_MINUTES),
      )}
    </div>
  );
}

interface TimedEventProps extends EventSelectProps {
  placed: PlacedSegment<CalendarEvent>;
}

function TimedEvent({ placed, calendar, selected, onSelect }: TimedEventProps) {
  const event = placed.item;
  const minutes = Math.max(placed.end - placed.start, MIN_MINUTES);
  const height = (minutes / 60) * HOUR_HEIGHT - EVENT_GAP;
  // Lines of text that fit: the title, then the time, then the place. Short
  // events put the title and the start time on one line.
  const lines = Math.max(1, Math.floor((height - 4) / LINE_HEIGHT));
  const compact = lines < 2;
  const tiny = height < 20;
  // In a narrow column the title wraps before the place gets a line.
  let spare = lines - 2;
  let titleLines = 1;
  if (placed.span < placed.columns && spare > 0) {
    titleLines += 1;
    spare -= 1;
  }
  const showLocation = !!event.location && spare > 0;
  if (showLocation) spare -= 1;
  titleLines = Math.min(3, titleLines + Math.max(0, spare));
  const range = eventRange(event);
  const status = displayShowAs(event);
  const statusText = status === "busy" ? "" : ` · ${SHOW_AS_LABELS[status]}`;
  return (
    <button
      type="button"
      className={clsx(
        "cal-ev",
        eventClasses(event, selected),
        compact && "cal-ev--compact",
        tiny && "cal-ev--tiny",
        placed.continuesBefore && "cal-ev--joins-before",
        placed.continuesAfter && "cal-ev--joins-after",
      )}
      style={eventStyle(calendar, {
        top: (placed.start / 60) * HOUR_HEIGHT + EVENT_GAP / 2,
        height,
        ...columnPlacement(placed.column, placed.span, placed.columns),
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
        !placed.continuesBefore && (
          <span className="cal-ev__meta tnum">
            <span className="cal-ev__time-full">{formatTime(range.start)}</span>
            <span className="cal-ev__time-short">{formatClock(range.start)}</span>
          </span>
        )
      ) : (
        <>
          <span className="cal-ev__meta tnum">
            {/* calendar.css shows one of the two by the width of the event. */}
            <span className="cal-ev__time-full">{eventTimeRange(event)}</span>
            <span className="cal-ev__time-short">
              {formatClock(range.start)}
              {/* Thin spaces keep the short form short. */}
              <span className="cal-ev__time-end">
                {"\u2009–\u2009"}
                {formatClock(range.end)}
              </span>
            </span>
            {statusText && <span className="cal-ev__status">{statusText}</span>}
          </span>
          {showLocation && <span className="cal-ev__meta">{event.location}</span>}
        </>
      )}
    </button>
  );
}
