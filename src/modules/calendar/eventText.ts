// Labels and descriptions of events shared by the views and the details card.

import { format } from "date-fns";

import type { CalendarEvent, ResponseStatus, ShowAs } from "../../api/types";
import { formatDate, formatDateRange } from "../../lib/datetime";
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

/** Title as shown on the grid ("Canceled: …" like Outlook). */
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

/** "Pending" invitations are drawn like tentative events (as Outlook does). */
export function displayShowAs(event: CalendarEvent): ShowAs {
  if (event.responseStatus === "notResponded" && event.showAs === "busy") return "tentative";
  return event.showAs;
}

/** Time range for display, all-day events read as local days. */
export function eventTimeRange(event: CalendarEvent): string {
  return formatTimeRange(event.start, event.end, event.isAllDay);
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
