// Pure layout math for the calendar views: visible ranges, titles, the
// side-by-side layout of overlapping events and lanes for multi-day bars.

import {
  addDays,
  addMonths,
  addWeeks,
  getISOWeek,
  startOfDay,
  startOfMonth,
  startOfWeek,
} from "date-fns";

import type { CalendarEvent } from "../../api/types";
import { appLocale, formatDate, formatDateRange, weekStartsOn } from "../../lib/datetime";
import { type LocalRange, eventRange, isAllDayLike, minutesOfDay } from "./dates";

export type CalendarView = "day" | "workweek" | "week" | "month";

export const VIEWS: CalendarView[] = ["day", "workweek", "week", "month"];

export const VIEW_LABELS: Record<CalendarView, string> = {
  day: "Day",
  workweek: "Work week",
  week: "Week",
  month: "Month",
};

export const MINUTES_PER_DAY = 24 * 60;

// --- Ranges & navigation -----------------------------------------------------

export interface ViewRange {
  /** Local midnight of the first visible day. */
  start: Date;
  /** Local midnight after the last visible day (exclusive). */
  end: Date;
  days: Date[];
}

function daysFrom(start: Date, count: number): Date[] {
  return Array.from({ length: count }, (_, i) => addDays(start, i));
}

export function viewRange(view: CalendarView, anchor: Date): ViewRange {
  let start: Date;
  let count: number;
  switch (view) {
    case "day":
      start = startOfDay(anchor);
      count = 1;
      break;
    case "workweek":
      start = startOfWorkWeek(anchor);
      count = 5;
      break;
    case "week":
      start = startOfWeek(anchor, { weekStartsOn: weekStartsOn() });
      count = 7;
      break;
    case "month":
      start = startOfWeek(startOfMonth(anchor), { weekStartsOn: weekStartsOn() });
      count = 42;
      break;
  }
  const days = daysFrom(start, count);
  return { start, end: addDays(start, count), days };
}

/**
 * Monday of the work week to show. Where weeks start on Sunday, a Sunday
 * belongs to the coming work week rather than the one just ended.
 */
function startOfWorkWeek(anchor: Date): Date {
  const day = weekStartsOn() === 0 && anchor.getDay() === 0 ? addDays(anchor, 1) : anchor;
  return startOfWeek(day, { weekStartsOn: 1 });
}

/** The anchor date one step back (-1) or forward (+1). */
export function shiftAnchor(view: CalendarView, anchor: Date, direction: 1 | -1): Date {
  switch (view) {
    case "day":
      return addDays(anchor, direction);
    case "workweek":
    case "week":
      return addWeeks(anchor, direction);
    case "month":
      return addMonths(anchor, direction);
  }
}

/** "Wednesday, October 14, 2026", "October 12 – 16, 2026", "October 2026"… */
export function rangeTitle(view: CalendarView, anchor: Date): string {
  if (view === "day") return formatDate(anchor, "weekdayLongYear");
  if (view === "month") return formatDate(anchor, "monthYear");
  const { days } = viewRange(view, anchor);
  return formatDateRange(days[0]!, days[days.length - 1]!, "longYear");
}

/** Label of the previous/next buttons for a view. */
export function stepLabel(view: CalendarView, direction: 1 | -1): string {
  const unit = view === "day" ? "day" : view === "month" ? "month" : "week";
  return `${direction < 0 ? "Previous" : "Next"} ${unit}`;
}

/**
 * Whether people in the app's locale refer to weeks by number. Locales that
 * follow ISO 8601 weeks (Monday first, the first week has four or more days),
 * as most of Europe does, use them; the US and most others don't.
 */
export function usesWeekNumbers(): boolean {
  try {
    const locale = new Intl.Locale(appLocale()) as Intl.Locale & {
      getWeekInfo?: () => { firstDay: number; minimalDays: number };
      weekInfo?: { firstDay: number; minimalDays: number };
    };
    const info = locale.getWeekInfo?.() ?? locale.weekInfo;
    return !!info && info.firstDay === 1 && info.minimalDays === 4;
  } catch {
    return false;
  }
}

/** "Week 42" for day and week views where the locale numbers weeks, else null. */
export function weekLabel(view: CalendarView, anchor: Date): string | null {
  if (view === "month" || !usesWeekNumbers()) return null;
  return `Week ${getISOWeek(viewRange(view, anchor).start)}`;
}

export function chunk<T>(items: T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < items.length; i += size) out.push(items.slice(i, i + size));
  return out;
}

// --- Timed events: side-by-side layout ---------------------------------------

export interface TimedSegment<T> {
  item: T;
  key: string;
  /** Minutes since the start of the day, 0–1440. */
  start: number;
  end: number;
  /** The event started on an earlier day. */
  continuesBefore: boolean;
  /** The event ends on a later day. */
  continuesAfter: boolean;
}

export interface PlacedSegment<T> extends TimedSegment<T> {
  /** Zero-based column within its overlap cluster. */
  column: number;
  /** Number of columns in the cluster. */
  columns: number;
  /** Number of columns the event may stretch over (≥ 1). */
  span: number;
}

/** The parts of timed events that fall on `day`, in wall-clock minutes. */
export function timedSegmentsForDay(
  events: CalendarEvent[],
  day: Date,
): TimedSegment<CalendarEvent>[] {
  const dayStart = startOfDay(day);
  const dayEnd = addDays(dayStart, 1);
  const out: TimedSegment<CalendarEvent>[] = [];
  for (const event of events) {
    if (isAllDayLike(event)) continue;
    const { start, end } = eventRange(event);
    const zeroLength = end.getTime() === start.getTime();
    const intersects = zeroLength
      ? start >= dayStart && start < dayEnd
      : start < dayEnd && end > dayStart;
    if (!intersects) continue;
    const s = start <= dayStart ? 0 : minutesOfDay(start);
    const e = end >= dayEnd ? MINUTES_PER_DAY : minutesOfDay(end);
    out.push({
      item: event,
      key: event.id,
      start: s,
      end: Math.max(e, s),
      continuesBefore: start < dayStart,
      continuesAfter: end > dayEnd,
    });
  }
  return out;
}

/**
 * Lays out one day's timed segments: transitively overlapping events form a
 * cluster that is split into equal columns; each event takes the first free
 * column and then stretches right over columns it does not collide with.
 * `minDuration` (minutes) is the shortest visual height, so very short events
 * still get room for their title.
 */
export function layoutDay<T>(segments: TimedSegment<T>[], minDuration = 0): PlacedSegment<T>[] {
  const sorted = [...segments].sort(
    (a, b) => a.start - b.start || b.end - a.end || a.key.localeCompare(b.key),
  );
  const out: PlacedSegment<T>[] = [];
  type Entry = { seg: TimedSegment<T>; column: number; top: number; bottom: number };
  let cluster: Entry[] = [];
  let columnEnds: number[] = [];
  let clusterEnd = -Infinity;

  const flush = () => {
    const columns = columnEnds.length;
    for (const entry of cluster) {
      let span = 1;
      for (let col = entry.column + 1; col < columns; col++) {
        const blocked = cluster.some(
          (o) => o.column === col && o.top < entry.bottom && entry.top < o.bottom,
        );
        if (blocked) break;
        span += 1;
      }
      out.push({ ...entry.seg, column: entry.column, columns, span });
    }
    cluster = [];
    columnEnds = [];
    clusterEnd = -Infinity;
  };

  for (const seg of sorted) {
    const top = seg.start;
    const bottom = Math.max(seg.end, seg.start + minDuration);
    if (top >= clusterEnd) flush();
    let column = columnEnds.findIndex((end) => end <= top);
    if (column === -1) {
      column = columnEnds.length;
      columnEnds.push(bottom);
    } else {
      columnEnds[column] = bottom;
    }
    cluster.push({ seg, column, top, bottom });
    clusterEnd = Math.max(clusterEnd, bottom);
  }
  flush();
  return out;
}

// --- Multi-day bars: lanes ---------------------------------------------------

export interface SpanItem<T> {
  item: T;
  key: string;
  /** First and last column (inclusive) the item covers in this row. */
  startCol: number;
  endCol: number;
  continuesBefore: boolean;
  continuesAfter: boolean;
}

export interface PlacedSpan<T> extends SpanItem<T> {
  lane: number;
}

/** Columns of `days` (consecutive local days) that a range covers, or null. */
export function spanForDays(
  range: LocalRange,
  days: Date[],
): Pick<SpanItem<unknown>, "startCol" | "endCol" | "continuesBefore" | "continuesAfter"> | null {
  if (days.length === 0) return null;
  const first = days[0]!;
  const after = addDays(days[days.length - 1]!, 1);
  const zeroLength = range.end.getTime() === range.start.getTime();
  const intersects = zeroLength
    ? range.start >= first && range.start < after
    : range.start < after && range.end > first;
  if (!intersects) return null;
  let startCol = -1;
  let endCol = -1;
  days.forEach((day, i) => {
    const next = addDays(day, 1);
    const covers = zeroLength
      ? range.start >= day && range.start < next
      : range.start < next && range.end > day;
    if (covers) {
      if (startCol < 0) startCol = i;
      endCol = i;
    }
  });
  if (startCol < 0) return null;
  return {
    startCol,
    endCol,
    continuesBefore: range.start < first,
    continuesAfter: range.end > after,
  };
}

/**
 * Assigns each item the lowest lane free over all its columns. Items are
 * placed in the given priority order after sorting by first column and
 * length, so callers put multi-day bars before single-day items.
 */
export function layoutLanes<T>(items: SpanItem<T>[]): PlacedSpan<T>[] {
  const occupied: boolean[][] = [];
  const free = (lane: number, from: number, to: number) => {
    const row = occupied[lane];
    if (!row) return true;
    for (let c = from; c <= to; c++) if (row[c]) return false;
    return true;
  };
  return items.map((item) => {
    let lane = 0;
    while (!free(lane, item.startCol, item.endCol)) lane += 1;
    const row = (occupied[lane] ??= []);
    for (let c = item.startCol; c <= item.endCol; c++) row[c] = true;
    return { ...item, lane };
  });
}

/** Sorts bars by first column, then longest first. */
export function byStartThenLength<T>(a: SpanItem<T>, b: SpanItem<T>): number {
  return a.startCol - b.startCol || b.endCol - b.startCol - (a.endCol - a.startCol);
}

/** Per column, which item occupies each lane. */
export function laneTable<T>(placed: PlacedSpan<T>[], columns: number): (PlacedSpan<T> | null)[][] {
  const table: (PlacedSpan<T> | null)[][] = Array.from({ length: columns }, () => []);
  for (const p of placed) {
    for (let c = p.startCol; c <= p.endCol; c++) {
      const col = table[c];
      if (col) col[p.lane] = p;
    }
  }
  return table.map((col) => Array.from({ length: col.length }, (_, i) => col[i] ?? null));
}

export interface CellLane<T> {
  lane: number;
  /** The item to draw starting in this cell, with how many columns it spans. */
  piece: { placed: PlacedSpan<T>; span: number; joinsBefore: boolean; joinsAfter: boolean } | null;
}

/**
 * The lanes to render in one cell when only `visible(col)` lanes fit in each
 * column. Bars are drawn from the first column where they are visible and
 * stretch over following columns while they stay visible there.
 */
export function cellLanes<T>(
  table: (PlacedSpan<T> | null)[][],
  col: number,
  visible: (col: number) => number,
): CellLane<T>[] {
  const lanes = table[col] ?? [];
  const count = Math.min(lanes.length, visible(col));
  const out: CellLane<T>[] = [];
  for (let lane = 0; lane < count; lane++) {
    const placed = lanes[lane] ?? null;
    if (!placed) {
      out.push({ lane, piece: null });
      continue;
    }
    const drawnBefore =
      col > placed.startCol && table[col - 1]?.[lane] === placed && lane < visible(col - 1);
    if (drawnBefore) {
      out.push({ lane, piece: null });
      continue;
    }
    let span = 1;
    while (
      col + span <= placed.endCol &&
      table[col + span]?.[lane] === placed &&
      lane < visible(col + span)
    ) {
      span += 1;
    }
    out.push({
      lane,
      piece: {
        placed,
        span,
        joinsBefore: col > placed.startCol || placed.continuesBefore,
        joinsAfter: col + span - 1 < placed.endCol || placed.continuesAfter,
      },
    });
  }
  return out;
}

/** Items in a column that do not fit in its visible lanes. */
export function hiddenCount<T>(
  table: (PlacedSpan<T> | null)[][],
  col: number,
  visible: number,
): number {
  const lanes = table[col] ?? [];
  let n = 0;
  for (let lane = visible; lane < lanes.length; lane++) if (lanes[lane]) n += 1;
  return n;
}

/**
 * How many lanes a month cell shows: all of them when they fit in
 * `maxLines`, otherwise one line fewer to make room for "+k more".
 */
export function visibleLanes(laneCount: number, maxLines: number): number {
  return laneCount <= maxLines ? maxLines : Math.max(0, maxLines - 1);
}

/** Bars (all-day and multi-day events) and single-day items for one row of days. */
export function rowItems(
  events: CalendarEvent[],
  days: Date[],
  { includeTimed }: { includeTimed: boolean },
): PlacedSpan<CalendarEvent>[] {
  const bars: SpanItem<CalendarEvent>[] = [];
  const singles: (SpanItem<CalendarEvent> & { start: number })[] = [];
  for (const event of events) {
    const range = eventRange(event);
    const allDayLike = isAllDayLike(event);
    if (!allDayLike && !includeTimed) continue;
    const span = spanForDays(range, days);
    if (!span) continue;
    const item = { item: event, key: event.id, ...span };
    if (
      allDayLike ||
      span.startCol !== span.endCol ||
      span.continuesBefore ||
      span.continuesAfter
    ) {
      bars.push(item);
    } else {
      singles.push({ ...item, start: range.start.getTime() });
    }
  }
  bars.sort(byStartThenLength);
  singles.sort((a, b) => a.startCol - b.startCol || a.start - b.start);
  return layoutLanes<CalendarEvent>([...bars, ...singles]);
}

/** Whether an item drawn in a row is a bar (vs. a single timed entry). */
export function isBar(p: SpanItem<CalendarEvent>): boolean {
  return isAllDayLike(p.item) || p.startCol !== p.endCol || p.continuesBefore || p.continuesAfter;
}
