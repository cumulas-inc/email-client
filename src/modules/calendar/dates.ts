// Pure date helpers for the calendar. Everything shown to the user is in the
// local time zone; the backend stores UTC instants, and all-day events as
// midnight UTC of the first day up to midnight UTC of the day after the last.

import { addDays, addMinutes, format, isValid, startOfDay } from "date-fns";

import type { CalendarEvent } from "../../api/types";

const MINUTE = 60_000;
const DAY = 24 * 60 * MINUTE;

type EventTimesLike = Pick<CalendarEvent, "start" | "end" | "isAllDay">;

// --- All-day representation -------------------------------------------------

/** Midnight UTC of a local calendar day: how all-day boundaries are stored. */
export function toAllDayIso(day: Date): string {
  return new Date(Date.UTC(day.getFullYear(), day.getMonth(), day.getDate())).toISOString();
}

/** The local calendar day (at local midnight) a stored all-day boundary means. */
export function fromAllDayIso(iso: string): Date {
  const d = new Date(iso);
  return new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate());
}

export interface LocalRange {
  start: Date;
  /** Exclusive. */
  end: Date;
}

/**
 * The event's span in local time. All-day events cover whole local days
 * (local midnight to local midnight) whatever the time zone.
 */
export function eventRange(event: EventTimesLike): LocalRange {
  if (event.isAllDay) {
    const start = fromAllDayIso(event.start);
    const end = fromAllDayIso(event.end);
    return { start, end: end > start ? end : addDays(start, 1) };
  }
  const start = new Date(event.start);
  const end = new Date(event.end);
  return { start, end: end > start ? end : start };
}

/** All-day events and timed events of 24 hours or more go in the all-day lane. */
export function isAllDayLike(event: EventTimesLike): boolean {
  if (event.isAllDay) return true;
  return new Date(event.end).getTime() - new Date(event.start).getTime() >= DAY;
}

/**
 * ISO instants whose *local* reading gives the event's displayed bounds, for
 * formatters such as `formatTimeRange` that read dates in local time.
 */
export function displayInstants(event: EventTimesLike): { start: string; end: string } {
  const { start, end } = eventRange(event);
  return { start: start.toISOString(), end: end.toISOString() };
}

/** Minutes since local midnight on the wall clock. */
export function minutesOfDay(d: Date): number {
  return d.getHours() * 60 + d.getMinutes();
}

/** The wall-clock time `minutes` after midnight on `day` (1440 = next midnight). */
export function atMinutes(day: Date, minutes: number): Date {
  return new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, minutes);
}

// --- Form values -------------------------------------------------------------

/** Start/end as edited in forms: local dates and wall-clock times. */
export interface EventTimes {
  allDay: boolean;
  /** "yyyy-MM-dd" */
  startDate: string;
  /** "HH:mm" (kept while all-day so toggling back restores it) */
  startTime: string;
  /** "yyyy-MM-dd"; for all-day events the last day, inclusive. */
  endDate: string;
  endTime: string;
}

export function formatDateInput(d: Date): string {
  return format(d, "yyyy-MM-dd");
}

export function formatTimeInput(d: Date): string {
  return format(d, "HH:mm");
}

/** Local midnight of a "yyyy-MM-dd" string, or null when malformed. */
export function parseDateInput(value: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]) - 1, Number(m[3])];
  const date = new Date(y, mo, d);
  if (!isValid(date) || date.getMonth() !== mo || date.getDate() !== d) return null;
  return date;
}

/** Minutes since midnight of a "HH:mm" string, or null when malformed. */
export function parseTimeInput(value: string): number | null {
  const m = /^(\d{1,2}):(\d{2})/.exec(value);
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  if (h > 23 || min > 59) return null;
  return h * 60 + min;
}

/** A local instant from a date and a wall-clock time. */
export function combineDateTime(date: string, time: string): Date | null {
  const day = parseDateInput(date);
  const minutes = parseTimeInput(time);
  if (!day || minutes === null) return null;
  return new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, minutes);
}

export function timesFromRange(start: Date, end: Date, allDay = false): EventTimes {
  if (allDay) {
    const first = startOfDay(start);
    const last = addDays(startOfDay(end), -1);
    return {
      allDay: true,
      startDate: formatDateInput(first),
      startTime: "09:00",
      endDate: formatDateInput(last < first ? first : last),
      endTime: "09:30",
    };
  }
  return {
    allDay: false,
    startDate: formatDateInput(start),
    startTime: formatTimeInput(start),
    endDate: formatDateInput(end),
    endTime: formatTimeInput(end),
  };
}

export function timesFromEvent(event: EventTimesLike): EventTimes {
  const { start, end } = eventRange(event);
  return timesFromRange(start, end, event.isAllDay);
}

/** Start and end of the form values as instants (all-day: local midnights, end exclusive). */
export function localBounds(t: EventTimes): LocalRange | null {
  if (t.allDay) {
    const first = parseDateInput(t.startDate);
    const last = parseDateInput(t.endDate);
    if (!first || !last) return null;
    return { start: first, end: addDays(last, 1) };
  }
  const start = combineDateTime(t.startDate, t.startTime);
  const end = combineDateTime(t.endDate, t.endTime);
  if (!start || !end) return null;
  return { start, end };
}

/** Problem with the times, phrased for the user, or null when they are fine. */
export function validateTimes(t: EventTimes): string | null {
  const bounds = localBounds(t);
  if (!bounds) return "Enter a valid start and end.";
  if (bounds.end <= bounds.start) return "The end must be after the start.";
  return null;
}

/** The start/end strings to send to the backend. */
export function timesToIso(t: EventTimes): { start: string; end: string } {
  if (t.allDay) {
    const first = parseDateInput(t.startDate);
    const last = parseDateInput(t.endDate);
    if (!first || !last) throw new Error("invalid dates");
    return { start: toAllDayIso(first), end: toAllDayIso(addDays(last, 1)) };
  }
  const bounds = localBounds(t);
  if (!bounds) throw new Error("invalid times");
  return { start: bounds.start.toISOString(), end: bounds.end.toISOString() };
}

/** Switches between all-day and timed, keeping the same days. */
export function setAllDay(t: EventTimes, allDay: boolean): EventTimes {
  if (t.allDay === allDay) return t;
  if (allDay) {
    // A timed event ending exactly at midnight does not occupy its end day.
    let endDate = t.endDate;
    const end = parseDateInput(t.endDate);
    if (end && t.endTime === "00:00" && t.endDate > t.startDate) {
      endDate = formatDateInput(addDays(end, -1));
    }
    if (endDate < t.startDate) endDate = t.startDate;
    return { ...t, allDay: true, endDate };
  }
  let { startTime, endTime } = t;
  if (
    t.startDate === t.endDate &&
    (parseTimeInput(endTime) ?? 0) <= (parseTimeInput(startTime) ?? 0)
  ) {
    startTime = "09:00";
    endTime = "09:30";
  }
  return { ...t, allDay: false, startTime, endTime };
}

/** Moves the start, keeping the duration (like Outlook). */
export function moveStart(t: EventTimes, startDate: string, startTime: string): EventTimes {
  const next = { ...t, startDate, startTime };
  const old = localBounds(t);
  if (!old) return next;
  if (t.allDay) {
    const first = parseDateInput(startDate);
    if (!first) return next;
    const days = Math.round((old.end.getTime() - old.start.getTime()) / DAY);
    return { ...next, endDate: formatDateInput(addDays(first, Math.max(days, 1) - 1)) };
  }
  const start = combineDateTime(startDate, startTime);
  if (!start) return next;
  const minutes = Math.max(0, Math.round((old.end.getTime() - old.start.getTime()) / MINUTE));
  const end = addMinutes(start, minutes);
  return { ...next, endDate: formatDateInput(end), endTime: formatTimeInput(end) };
}

/** Default times for a new event on `day`: the next half hour today, else 09:00. */
export function defaultNewEventTimes(day: Date, now: Date): EventTimes {
  let start: Date;
  if (startOfDay(day).getTime() === startOfDay(now).getTime()) {
    const minutes = minutesOfDay(now);
    const rounded = Math.ceil(minutes / 30) * 30;
    start = addMinutes(startOfDay(now), rounded);
    if (start.getDate() !== now.getDate()) start = addMinutes(startOfDay(now), 23 * 60 + 30);
  } else {
    start = addMinutes(startOfDay(day), 9 * 60);
  }
  return timesFromRange(start, addMinutes(start, 30));
}

// --- Descriptions ------------------------------------------------------------

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** Plain text from the description box as simple HTML paragraphs. */
export function textToHtml(text: string): string {
  return text
    .trim()
    .split(/\n\s*\n/)
    .map((p) => `<p>${escapeHtml(p.trim()).replace(/\n/g, "<br>")}</p>`)
    .join("");
}

const BLOCK_TAGS = new Set([
  "P",
  "DIV",
  "LI",
  "TR",
  "H1",
  "H2",
  "H3",
  "H4",
  "H5",
  "H6",
  "UL",
  "OL",
]);

/** Readable plain text from HTML (for editing an existing description). */
export function htmlToText(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  let out = "";
  const walk = (node: Node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      out += (node.textContent ?? "").replace(/\s+/g, " ");
      return;
    }
    if (!(node instanceof Element)) return;
    if (node.tagName === "SCRIPT" || node.tagName === "STYLE") return;
    if (node.tagName === "BR") {
      out += "\n";
      return;
    }
    const block = BLOCK_TAGS.has(node.tagName);
    if (block && out && !out.endsWith("\n")) out += "\n";
    if (node.tagName === "LI") out += "• ";
    node.childNodes.forEach(walk);
    if (node.tagName === "P") out += "\n\n";
    else if (block && !out.endsWith("\n")) out += "\n";
  };
  doc.body.childNodes.forEach(walk);
  return out
    .split("\n")
    .map((l) => l.trim())
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
