// What the Reminders panel says about each reminder, and the snooze choices.

import { differenceInCalendarDays, startOfDay } from "date-fns";
import { create } from "zustand";

import type { ActiveReminder, SnoozeFor } from "../api/types";
import { formatDate, formatTime, formatTimeSpan } from "../lib/datetime";

export function isDue(reminder: ActiveReminder): boolean {
  return reminder.snoozedUntil === null;
}

/** "5 min", "1 h 20 min", "2 h", "3 days". */
export function durationText(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60_000));
  if (minutes < 60) return `${minutes} min`;
  if (minutes < 24 * 60) {
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    return m === 0 ? `${h} h` : `${h} h ${m} min`;
  }
  const days = Math.round(minutes / (24 * 60));
  return days === 1 ? "1 day" : `${days} days`;
}

export type ReminderTone = "soon" | "overdue" | "snoozed" | "missed" | "allDay";

/** All-day events start at local midnight and last whole days. */
function isAllDay(reminder: ActiveReminder): boolean {
  if (reminder.startsAt === null || reminder.endsAt === null) return false;
  const start = new Date(reminder.startsAt);
  const end = new Date(reminder.endsAt);
  return (
    start.getTime() === startOfDay(start).getTime() &&
    end.getTime() === startOfDay(end).getTime() &&
    end > start
  );
}

/** "", "tomorrow at " or "Wed, Oct 14 at ". */
function dayPrefix(date: Date, now: Date): string {
  const days = differenceInCalendarDays(date, now);
  if (days === 0) return "";
  if (days === 1) return "tomorrow at ";
  return `${formatDate(date, "weekdayShort")} at `;
}

/**
 * When, against now: "in 5 min", "12 min overdue", "Now", "Today, all day",
 * "Snoozed until 10:45 AM", "Missed" or "Ended"; with a tone for its color.
 */
export function reminderTime(
  reminder: ActiveReminder,
  now: Date,
): { text: string; tone: ReminderTone } {
  if (reminder.snoozedUntil !== null) {
    const until = new Date(reminder.snoozedUntil);
    return {
      text: `Snoozed until ${dayPrefix(until, now)}${formatTime(until)}`,
      tone: "snoozed",
    };
  }
  if (reminder.missed) return { text: "Missed", tone: "missed" };
  if (reminder.kind === "event" && reminder.endsAt && new Date(reminder.endsAt) <= now) {
    return { text: "Ended", tone: "missed" };
  }
  if (reminder.kind === "event" && reminder.startsAt && isAllDay(reminder)) {
    const start = new Date(reminder.startsAt);
    const days = differenceInCalendarDays(start, now);
    const text =
      days <= 0
        ? "Today, all day"
        : days === 1
          ? "Tomorrow, all day"
          : `${formatDate(start, "weekdayShort")}, all day`;
    return { text, tone: "allDay" };
  }
  const at = new Date(reminder.startsAt ?? reminder.fireAt);
  const ms = at.getTime() - now.getTime();
  if (ms > 0) return { text: `in ${durationText(ms)}`, tone: "soon" };
  if (-ms < 60_000) return { text: "Now", tone: "overdue" };
  return { text: `${durationText(-ms)} overdue`, tone: "overdue" };
}

/** The rest of the line: "10:30 AM – 12:00 PM · Room 3.01", or a task's "Reminder at 10:12 AM". */
export function reminderDetail(reminder: ActiveReminder, now: Date): string {
  if (reminder.kind === "task")
    return `Task · reminder at ${formatTime(new Date(reminder.fireAt))}`;
  const parts: string[] = [];
  if (reminder.startsAt && reminder.endsAt && !isAllDay(reminder)) {
    const start = new Date(reminder.startsAt);
    const day = differenceInCalendarDays(start, now) === 0 ? "" : formatDate(start, "weekdayShort");
    const span = formatTimeSpan(start, new Date(reminder.endsAt));
    parts.push(day ? `${day}, ${span}` : span);
  }
  if (reminder.location?.trim()) parts.push(reminder.location.trim());
  return parts.join(" · ");
}

/** An address the Join button can open: a web address over https. */
export function joinUrl(reminder: ActiveReminder): string | null {
  const url = reminder.onlineMeetingUrl?.trim();
  if (!url) return null;
  try {
    const parsed = new URL(url);
    return parsed.protocol === "https:" && parsed.hostname ? url : null;
  } catch {
    return null;
  }
}

export interface SnoozeChoice {
  label: string;
  snooze: SnoozeFor;
  disabled: boolean;
}

const MINUTES = [5, 10, 15, 30];

/** 5, 10, 15 and 30 minutes, an hour, a day, and (for an event that starts later) 5 minutes before it starts. */
export function snoozeChoices(reminder: ActiveReminder, now: Date): SnoozeChoice[] {
  const choices: SnoozeChoice[] = [
    ...MINUTES.map((m) => ({ label: `${m} minutes`, snooze: { minutes: m }, disabled: false })),
    { label: "1 hour", snooze: { minutes: 60 }, disabled: false },
    { label: "1 day", snooze: { minutes: 24 * 60 }, disabled: false },
  ];
  if (reminder.kind === "event" && reminder.startsAt && !isAllDay(reminder)) {
    const until = new Date(new Date(reminder.startsAt).getTime() - 5 * 60_000);
    choices.push({
      label: "5 minutes before start",
      snooze: { until: until.toISOString() },
      // A minute's leeway: it must still be ahead once it reaches the app.
      disabled: until.getTime() - now.getTime() < 60_000,
    });
  }
  return choices;
}

/** What a screen reader hears when reminders come due. */
export function announcement(newlyDue: ActiveReminder[], now: Date): string {
  if (newlyDue.length === 0) return "";
  if (newlyDue.length > 3) return `${newlyDue.length} reminders are due.`;
  return newlyDue
    .map((r) => `Reminder: ${r.title.trim() || "(no title)"}, ${reminderTime(r, now).text}.`)
    .join(" ");
}

/** The latest announcement, for the header's live region. */
export const useReminderAnnouncer = create<{
  text: string;
  /** Changes with each announcement, so the same words are announced again. */
  seq: number;
  announce: (text: string) => void;
}>((set) => ({
  text: "",
  seq: 0,
  announce: (text) => set((s) => ({ text, seq: s.seq + 1 })),
}));
