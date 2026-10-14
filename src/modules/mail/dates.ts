// Dates as the message list and reading pane show them. Mail dates come from
// the sender's clock: some are missing (sent as the Unix epoch, 1 Jan 1970)
// and some are years ahead.

import { differenceInCalendarDays } from "date-fns";

import { formatDate } from "../../lib/datetime";
import { dateGroup, formatFullDate, formatListDate } from "../../lib/format";

/** Within a day of the Unix epoch, in any time zone: a date that was never set. */
const EPOCH_SLACK_MS = 36 * 60 * 60 * 1000;

/** The message's date, or null when it has none (missing, invalid or epoch 0). */
export function messageDate(iso: string | null | undefined): Date | null {
  if (!iso) return null;
  const d = new Date(iso);
  const t = d.getTime();
  if (Number.isNaN(t) || Math.abs(t) < EPOCH_SLACK_MS) return null;
  return d;
}

/** Dated after today: the sender's clock is wrong (or it was scheduled). */
export function isFutureDated(date: Date, now = new Date()): boolean {
  return differenceInCalendarDays(date, now) > 0;
}

/** The list's date column: "3:05 PM", "Oct 9", "Jan 19, 2038"; "" without a date. */
export function listDate(iso: string, now = new Date()): string {
  const d = messageDate(iso);
  if (!d) return "";
  if (isFutureDated(d, now)) return formatDate(d, "shortYear");
  return formatListDate(iso, now);
}

export const NO_DATE_GROUP = "No date";
export const FUTURE_GROUP = "Dated in the future";

/** The group a message is listed under: "Today", "March", "No date"… */
export function listGroup(iso: string, now = new Date()): string {
  const d = messageDate(iso);
  if (!d) return NO_DATE_GROUP;
  if (isFutureDated(d, now)) return FUTURE_GROUP;
  return dateGroup(iso, now);
}

/** The reading pane's date, or null when the message has none. */
export function fullDate(iso: string): string | null {
  return messageDate(iso) ? formatFullDate(iso) : null;
}

export const FUTURE_NOTE = "This message is dated in the future. The sender's clock may be wrong.";
