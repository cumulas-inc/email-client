// Labels and descriptions of events shared by the views and the details card.

import { format } from "date-fns";

import type { CalendarEvent, ResponseStatus, ShowAs } from "../../api/types";
import { appLocale, formatDate, formatDateRange } from "../../lib/datetime";
import { formatTimeRange } from "../../lib/format";
import { eventRange } from "./dates";

export const SHOW_AS_LABELS: Record<ShowAs, string> = {
  free: "Free",
  workingElsewhere: "Working elsewhere",
  tentative: "Tentative",
  busy: "Busy",
  outOfOffice: "Out of office",
};

export const RESPONSE_LABELS: Record<ResponseStatus, string> = {
  none: "No response needed",
  organizer: "Organizer",
  accepted: "Accepted",
  tentative: "Tentative",
  declined: "Declined",
  notResponded: "Not responded",
};

/** Title as shown on the grid, with a "Canceled: " prefix for canceled events. */
export function eventTitle(event: CalendarEvent): string {
  const subject = event.subject.trim() || "(No title)";
  return event.isCancelled && !/^cancel+ed:/i.test(subject) ? `Canceled: ${subject}` : subject;
}

/** Struck-through on the grid. */
export function isStruck(event: CalendarEvent): boolean {
  return event.isCancelled || event.responseStatus === "declined";
}

/** An invitation the user can respond to. */
export function isInvitation(event: CalendarEvent): boolean {
  return event.responseStatus !== "organizer" && !event.isCancelled;
}

/** Invitations still waiting for an answer are drawn like tentative events. */
export function displayShowAs(event: CalendarEvent): ShowAs {
  if (event.responseStatus === "notResponded" && event.showAs === "busy") return "tentative";
  return event.showAs;
}

/** Time range for display, all-day events read as local days. */
export function eventTimeRange(event: CalendarEvent): string {
  return formatTimeRange(event.start, event.end, event.isAllDay);
}

const clockFormats = new Map<string, Intl.DateTimeFormat>();

function clockParts(date: Date): Intl.DateTimeFormatPart[] {
  const locale = appLocale();
  let f = clockFormats.get(locale);
  if (!f) {
    f = new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit" });
    clockFormats.set(locale, f);
  }
  return f.formatToParts(date);
}

/**
 * A time of day for a narrow event. With a 12-hour clock the minutes go
 * when they are zero, and AM/PM is left off unless `period` asks for it
 * ("10:30", "11", "12 PM"). A 24-hour clock keeps its usual form ("15:00").
 */
function compactClock(date: Date, period: boolean): string {
  const parts = clockParts(date);
  const twelveHour = parts.some((p) => p.type === "dayPeriod");
  const onTheHour = twelveHour && parts.find((p) => p.type === "minute")?.value === "00";
  return parts
    .filter((p, i) => {
      if (p.type === "dayPeriod") return period;
      if (!onTheHour) return true;
      // The minutes, and the ":" in front of them.
      return p.type !== "minute" && !(p.type === "literal" && parts[i + 1]?.type === "minute");
    })
    .map((p) => p.value)
    .join("")
    .replace(/[\s\u202f\u2009\u00a0]+/g, " ")
    .trim();
}

/** "10:30 AM", "11 AM" (or "15:00") for an event too narrow for its range. */
export function formatCompactTime(date: Date): string {
  return compactClock(date, true);
}

/** "AM" or "PM" (as the locale writes it), or null with a 24-hour clock. */
function dayPeriod(date: Date): string | null {
  return clockParts(date).find((p) => p.type === "dayPeriod")?.value ?? null;
}

/**
 * "10:30 – 11 PM" for narrow events: AM/PM once, on the end, when both
 * times share it, and on both when they don't ("11 AM – 12 PM"), so the
 * start never reads as the wrong half of the day. 24-hour clocks stay as
 * they are ("10:30 – 12:00"). Thin spaces keep the short form short.
 */
export function formatCompactTimeRange(start: Date, end: Date): string {
  const both = dayPeriod(start) !== dayPeriod(end);
  return `${compactClock(start, both)}\u2009\u2013\u2009${compactClock(end, true)}`;
}

/** "Wednesday, October 14, 2026", or "Wed, Oct 14 – Fri, Oct 16, 2026" for long events. */
export function eventDateLine(event: CalendarEvent): string {
  const { start, end } = eventRange(event);
  const last = new Date(end.getTime() - 1);
  if (format(start, "yyyy-MM-dd") === format(last, "yyyy-MM-dd") || !event.isAllDay) {
    return formatDate(start, "weekdayLongYear");
  }
  return formatDateRange(start, last, "weekdayShortYear");
}

/** Spoken summary of an event for its button. */
export function eventAriaLabel(event: CalendarEvent, calendarName?: string): string {
  const parts = [eventTitle(event)];
  const { start, end } = eventRange(event);
  if (event.isAllDay) {
    const last = new Date(end.getTime() - 1);
    const sameDay = format(start, "yyyy-MM-dd") === format(last, "yyyy-MM-dd");
    parts.push(
      sameDay
        ? `${formatDate(start, "weekdayLong")}, all day`
        : `${formatDate(start, "weekdayLong")} to ${formatDate(last, "weekdayLong")}, all day`,
    );
  } else {
    parts.push(`${formatDate(start, "weekdayLong")}, ${eventTimeRange(event)}`);
  }
  if (event.location) parts.push(event.location);
  if (calendarName) parts.push(`${calendarName} calendar`);
  parts.push(SHOW_AS_LABELS[displayShowAs(event)]);
  if (event.responseStatus === "notResponded") parts.push("Not responded");
  if (event.responseStatus === "declined") parts.push("Declined");
  if (event.isRecurring) parts.push("Recurring");
  return parts.join(", ");
}
