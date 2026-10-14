// Calendar view preferences kept in localStorage (per device, not synced).

import { type CalendarView, VIEWS } from "./layout";

export const VIEW_KEY = "cumulas.calendar.view";
export const HIDDEN_KEY = "cumulas.calendar.hiddenCalendars";

function read(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Storage may be unavailable (private mode, quota); preferences are optional.
  }
}

export function loadView(): CalendarView {
  const value = read(VIEW_KEY);
  return VIEWS.includes(value as CalendarView) ? (value as CalendarView) : "workweek";
}

export function saveView(view: CalendarView): void {
  write(VIEW_KEY, view);
}

export function loadHiddenCalendars(): string[] {
  try {
    const parsed: unknown = JSON.parse(read(HIDDEN_KEY) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === "string") : [];
  } catch {
    return [];
  }
}

export function saveHiddenCalendars(ids: Iterable<string>): void {
  write(HIDDEN_KEY, JSON.stringify([...ids].sort()));
}
