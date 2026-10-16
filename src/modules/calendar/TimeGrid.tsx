import clsx from "clsx";
import { addDays, format, isSameDay, isWeekend } from "date-fns";
import { ChevronDown, ChevronUp } from "lucide-react";
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
import { formatCount } from "../../lib/format";
import { AvailabilityGlyph } from "./AvailabilityGlyph";
import { atMinutes, eventRange, minutesOfDay } from "./dates";
import { EventBar } from "./EventBar";
import { type EventSelectProps, eventClasses, eventStyle, selectHandlers } from "./eventProps";
import {
  SHOW_AS_LABELS,
  displayShowAs,
  eventAriaLabel,
  eventTimeRange,
  eventTitle,
  formatCompactTime,
  formatCompactTimeRange,
} from "./eventText";
import { useElementWidth } from "./hooks";
import {
  type OverflowGroup,
  type PlacedSegment,
  cellLanes,
  fitColumns,
  hiddenItems,
  laneTable,
  layoutDay,
  limitColumns,
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
/** Width of the hour gutter and the breathing room after the last day. */
const GUTTER = 64;
const GRID_END = 12;
/** The "+N" chip beside a crowded cluster, and the gap before it. */
const CHIP_WIDTH = 28;
const CHIP_GAP = 2;
/**
 * Overlapping events narrower than `minWidth` would only show slivers of
 * their titles: what doesn't fit goes behind the chip, which opens the day,
 * and the events still shown get `shownWidth` for a word or two per line.
 */
const FIT = { minWidth: 48, shownWidth: 72, chipWidth: CHIP_WIDTH + CHIP_GAP };
/** All-day rows shown before the rest go behind "+N more". */
const ALL_DAY_LINES = 3;

/**
 * About how wide a short time is, in ems of its font, erring a little wide:
 * tabular digits are 0.6em, AM/PM letters three quarters of one. The CSS
 * falls back from the range to the start, and from the start to nothing,
 * by comparing these with the width of the event.
 */
function timeWidthEm(text: string): number {
  let em = 0;
  for (const ch of text) {
    if (/\d/.test(ch)) em += 0.6;
    else if (/[:.\u2009]/.test(ch)) em += 0.26;
    else if (/\s/.test(ch)) em += 0.28;
    else if (ch === "–") em += 0.5;
    else if (/[A-Za-z]/.test(ch)) em += 0.75;
    else em += 1;
  }
  return Math.round(em * 100) / 100;
}

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

/**
 * Horizontal placement inside a day column, leaving the column's insets free
 * and `reserve` pixels on the right for "+N" chips.
 */
function columnPlacement(
  column: number,
  span: number,
  columns: number,
  reserve = 0,
): CSSProperties {
  const inner = `(100% - ${INSET_LEFT + INSET_RIGHT + reserve}px)`;
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
  const bodyRef = useRef<HTMLDivElement>(null);
  const bodyWidth = useElementWidth(bodyRef);
  const [allDayOpen, setAllDayOpen] = useState(false);
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

  // Room for events in one day column (each column has a 1px border). Before
  // the grid is measured, and in day view, every event gets its column.
  const eventsWidth =
    bodyWidth > 0
      ? (bodyWidth - GUTTER - GRID_END) / days.length - 1 - INSET_LEFT - INSET_RIGHT
      : 0;
  const limit = onOpenDay !== undefined && eventsWidth > 0;
  const columns = useMemo(
    () =>
      days.map((day) =>
        limitColumns(
          layoutDay(timedSegmentsForDay(events, day), MIN_MINUTES),
          (n) => (limit ? fitColumns(eventsWidth, n, FIT) : n),
          MIN_MINUTES,
        ),
      ),
    [events, days, limit, eventsWidth],
  );

  // A crowded all-day lane shows three rows, and "+N more" below them on the
  // days with more, until it's opened.
  const allDayLanes = Math.max(0, ...allDay.map((lanes) => lanes.length));
  const allDayCollapsible = allDayLanes > ALL_DAY_LINES;
  const allDayVisible = allDayCollapsible && !allDayOpen ? ALL_DAY_LINES : Infinity;

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
    <div
      className="cal-tg"
      style={
        {
          "--cal-days": days.length,
          "--tg-gutter": `${GUTTER}px`,
          "--tg-end": `${GRID_END}px`,
        } as CSSProperties
      }
    >
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
        <div className={clsx("cal-tg__row cal-tg__allday", allDayOpen && "cal-tg__allday--open")}>
          <div className="cal-tg__gutter cal-tg__allday-label">
            All day
            {allDayCollapsible && (
              <button
                type="button"
                className="icon-btn icon-btn--sm cal-tg__allday-toggle"
                aria-expanded={allDayOpen}
                aria-label={allDayOpen ? "Show fewer all-day events" : "Show all all-day events"}
                title={allDayOpen ? "Show fewer" : "Show all"}
                onClick={() => setAllDayOpen((open) => !open)}
              >
                {allDayOpen ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
              </button>
            )}
          </div>
          {days.map((day, col) => {
            const hidden = hiddenItems(allDay, col, allDayVisible);
            return (
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
                {cellLanes(allDay, col, () => allDayVisible).map(({ lane, piece }) => {
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
                {hidden.length > 0 && (
                  <button
                    type="button"
                    className="cal-more-link cal-tg__allday-more"
                    aria-label={`${hidden.length} more all-day event${hidden.length === 1 ? "" : "s"} on ${formatDate(day, "weekdayLong")}`}
                    // Where these events are shown (see drawnEvent).
                    data-event-ids={hidden.map((e) => e.id).join(" ")}
                    onClick={(e) => {
                      e.stopPropagation();
                      setAllDayOpen(true);
                    }}
                  >
                    +{formatCount(hidden.length)} more
                  </button>
                )}
              </div>
            );
          })}
        </div>
      </div>

      <div className="cal-tg__scroll" ref={scrollRef}>
        <div
          className="cal-tg__row cal-tg__body"
          ref={bodyRef}
          style={{ height: 24 * HOUR_HEIGHT }}
        >
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
                {columns[col]!.placed.map((placed) => (
                  <TimedEvent
                    key={placed.key}
                    placed={placed}
                    calendar={calendars.get(placed.item.calendarId)}
                    selected={selectedId === placed.item.id}
                    onSelect={onSelect}
                  />
                ))}
                {onOpenDay &&
                  columns[col]!.overflow.map((group) => (
                    <MoreChip key={group.key} group={group} day={day} onOpen={onOpenDay} />
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

/** "+3": the events of a crowded cluster that didn't fit; opens the day. */
function MoreChip({
  group,
  day,
  onOpen,
}: {
  group: OverflowGroup<CalendarEvent>;
  day: Date;
  onOpen: (day: Date) => void;
}) {
  const count = group.items.length;
  const height = ((group.end - group.start) / 60) * HOUR_HEIGHT - EVENT_GAP;
  return (
    <button
      type="button"
      className="cal-ev-more tnum"
      style={{
        top: (group.start / 60) * HOUR_HEIGHT + EVENT_GAP / 2,
        height: Math.max(height, 20),
        right: INSET_RIGHT,
        width: CHIP_WIDTH,
      }}
      aria-label={`${count} more event${count === 1 ? "" : "s"} at ${formatTime(
        atMinutes(day, group.start),
      )} on ${formatDate(day, "weekdayLong")}`}
      title={group.items.map(eventTitle).join("\n")}
      // Where these events are shown (see drawnEvent).
      data-event-ids={group.items.map((e) => e.id).join(" ")}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation();
        onOpen(day);
      }}
    >
      +{formatCount(count)}
    </button>
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
  const shortRange = formatCompactTimeRange(range.start, range.end);
  const shortStart = formatCompactTime(range.start);
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
        "--ev-height": `${height}px`,
        ...columnPlacement(
          placed.column,
          placed.span,
          placed.columns,
          placed.overflowed ? CHIP_WIDTH + CHIP_GAP : 0,
        ),
        "--title-lines": titleLines,
      } as CSSProperties)}
      data-event-id={event.id}
      aria-label={eventAriaLabel(event, calendar?.name)}
      aria-haspopup="dialog"
      aria-expanded={selected}
      {...selectHandlers(event, onSelect)}
    >
      <span className="cal-ev__title" dir="auto">
        <AvailabilityGlyph event={event} className="cal-ev__glyph" />
        {eventTitle(event)}
      </span>
      {compact ? (
        !placed.continuesBefore && (
          <span className="cal-ev__meta tnum">
            <span className="cal-ev__time-full">{formatTime(range.start)}</span>
            <span className="cal-ev__time-short">{shortStart}</span>
          </span>
        )
      ) : (
        <>
          <span
            className="cal-ev__meta tnum"
            style={
              {
                "--range-width": `${timeWidthEm(shortRange)}em`,
                "--start-width": `${timeWidthEm(shortStart)}em`,
              } as CSSProperties
            }
          >
            {/* calendar.css shows one of the three by the width of the event. */}
            <span className="cal-ev__time-full">{eventTimeRange(event)}</span>
            <span className="cal-ev__time-short">{shortRange}</span>
            <span className="cal-ev__time-start">{shortStart}</span>
            {statusText && <span className="cal-ev__status">{statusText}</span>}
          </span>
          {showLocation && (
            <span className="cal-ev__meta" dir="auto">
              {event.location}
            </span>
          )}
        </>
      )}
    </button>
  );
}
