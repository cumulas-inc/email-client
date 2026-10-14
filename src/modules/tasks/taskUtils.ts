import {
  addDays,
  differenceInCalendarDays,
  endOfWeek,
  format,
  isSameYear,
  isValid,
  nextMonday,
  parseISO,
} from "date-fns";

import type { Account, Task, TaskList, TaskPatch } from "../../api/types";
import { formatDate, weekStartsOn } from "../../lib/datetime";

// --- Views -------------------------------------------------------------------

export type SmartListId = "starred" | "scheduled" | "all";

export type TaskView = { kind: "smart"; id: SmartListId } | { kind: "list"; id: string };

export const SMART_LISTS: { id: SmartListId; label: string }[] = [
  { id: "starred", label: "Starred" },
  { id: "scheduled", label: "Scheduled" },
  { id: "all", label: "All tasks" },
];

/**
 * How long a just-completed task stays in the open list: its check mark
 * draws in, then its row closes up and it moves to Completed.
 */
export const SETTLE_MS = 450;

export function isCompleted(task: Task): boolean {
  return task.status === "completed";
}

/** Whether a task belongs in a view (completed tasks included). */
export function inView(task: Task, view: TaskView): boolean {
  if (view.kind === "list") return task.listId === view.id;
  switch (view.id) {
    case "starred":
      return task.importance === "high";
    case "scheduled":
      return task.dueDate !== null;
    case "all":
      return true;
  }
}

/** Lists in display order: by account, the default list first, then by name. */
export function sortTaskLists(lists: TaskList[], accounts: Account[]): TaskList[] {
  const rank = (l: TaskList) => {
    const i = accounts.findIndex((a) => a.id === l.accountId);
    return i < 0 ? accounts.length : i;
  };
  return [...lists].sort(
    (a, b) =>
      rank(a) - rank(b) ||
      Number(b.isDefault) - Number(a.isDefault) ||
      a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
  );
}

/** Where a task typed into a smart list goes: the first account's default list. */
export function defaultTaskList(lists: TaskList[], accounts: Account[]): TaskList | null {
  const sorted = sortTaskLists(lists, accounts);
  const first = accounts[0];
  return (
    sorted.find((l) => l.accountId === first?.id && l.isDefault) ??
    sorted.find((l) => l.accountId === first?.id) ??
    sorted.find((l) => l.isDefault) ??
    sorted[0] ??
    null
  );
}

/** Case-insensitive match of every search word against title and notes. */
export function matchesSearch(task: Task, search: string): boolean {
  const terms = search.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return true;
  const haystack = `${task.title} ${task.notes ?? ""}`.toLowerCase();
  return terms.every((t) => haystack.includes(t));
}

// --- Due dates -------------------------------------------------------------------

/** A "YYYY-MM-DD" due date as a local calendar day. */
export function parseDue(dueDate: string): Date {
  return parseISO(dueDate);
}

export function toDateString(date: Date): string {
  return format(date, "yyyy-MM-dd");
}

export function isValidDateString(value: string): boolean {
  return /^\d{4}-\d{2}-\d{2}$/.test(value) && isValid(parseISO(value));
}

export type DueGroup = "overdue" | "today" | "tomorrow" | "thisWeek" | "later";

export const DUE_GROUPS: { id: DueGroup; label: string }[] = [
  { id: "overdue", label: "Overdue" },
  { id: "today", label: "Today" },
  { id: "tomorrow", label: "Tomorrow" },
  { id: "thisWeek", label: "This week" },
  { id: "later", label: "Later" },
];

/** Scheduled group of a due date. "This week" runs to Sunday (weeks start on Monday). */
export function dueGroup(dueDate: string, now = new Date()): DueGroup {
  const due = parseDue(dueDate);
  const days = differenceInCalendarDays(due, now);
  if (days < 0) return "overdue";
  if (days === 0) return "today";
  if (days === 1) return "tomorrow";
  if (due <= endOfWeek(now, { weekStartsOn: weekStartsOn() })) return "thisWeek";
  return "later";
}

export interface DueLabel {
  text: string;
  overdue: boolean;
  today: boolean;
}

/** Short due label: "Today", "Tomorrow", "Yesterday", "Friday" or "Wed, Oct 28". */
export function dueLabel(dueDate: string, now = new Date()): DueLabel {
  const due = parseDue(dueDate);
  const days = differenceInCalendarDays(due, now);
  let text: string;
  if (days === 0) text = "Today";
  else if (days === 1) text = "Tomorrow";
  else if (days === -1) text = "Yesterday";
  else if (days > 1 && days < 7) text = formatDate(due, "weekdayName");
  else text = formatDate(due, isSameYear(due, now) ? "weekdayShort" : "weekdayShortYear");
  return { text, overdue: days < 0, today: days === 0 };
}

/** Long form for the details pane: "Wednesday, October 14" (+ year if needed). */
export function longDate(date: Date, now = new Date()): string {
  return formatDate(date, isSameYear(date, now) ? "weekdayLong" : "weekdayLongYear");
}

/** "Wed, Oct 14" (+ year if needed) for created / completed stamps. */
export function shortDate(iso: string, now = new Date()): string {
  const d = new Date(iso);
  return formatDate(d, isSameYear(d, now) ? "weekdayShort" : "weekdayShortYear");
}

export interface QuickDue {
  label: string;
  date: string;
}

/** Quick picks for the due date: Today, Tomorrow, Next week (next Monday). */
export function quickDueDates(now = new Date()): QuickDue[] {
  return [
    { label: "Today", date: toDateString(now) },
    { label: "Tomorrow", date: toDateString(addDays(now, 1)) },
    { label: "Next week", date: toDateString(nextMonday(now)) },
  ];
}

// --- Optimistic updates -------------------------------------------------------------

/** Applies a patch the way the backend does, for optimistic updates. */
export function applyTaskPatch(task: Task, patch: TaskPatch, now = new Date()): Task {
  const t = { ...task };
  if (patch.title !== undefined) t.title = patch.title.trim();
  if (patch.clearNotes) t.notes = null;
  else if (patch.notes !== undefined) t.notes = patch.notes;
  if (patch.importance) t.importance = patch.importance;
  if (patch.clearDueDate) t.dueDate = null;
  else if (patch.dueDate) t.dueDate = patch.dueDate;
  if (patch.status) {
    if (patch.status === "completed" && t.status !== "completed") {
      t.completedAt = now.toISOString();
    } else if (patch.status !== "completed") {
      t.completedAt = null;
    }
    t.status = patch.status;
  }
  t.updatedAt = now.toISOString();
  return t;
}
