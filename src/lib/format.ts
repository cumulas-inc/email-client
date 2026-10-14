import { differenceInCalendarDays, format, isSameDay, isSameYear } from "date-fns";

import type { EmailAddress } from "../api/types";

/** Name to show for an address: the display name, else the address. */
export function displayName(addr: EmailAddress | null | undefined): string {
  if (!addr) return "(no sender)";
  return addr.name?.trim() || addr.address;
}

/** "Name <address>" or just the address. */
export function fullAddress(addr: EmailAddress): string {
  return addr.name ? `${addr.name} <${addr.address}>` : addr.address;
}

export function initials(nameOrAddress: string): string {
  const cleaned = nameOrAddress
    .replace(/<.*>/, "")
    .replace(/[^\p{L}\p{N}\s@.]/gu, "")
    .trim();
  const base = cleaned.includes("@") && !cleaned.includes(" ") ? cleaned.split("@")[0]! : cleaned;
  const parts = base.split(/[\s._-]+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0]!.slice(0, 2).toUpperCase();
  return (parts[0]![0]! + parts[parts.length - 1]![0]!).toUpperCase();
}

const AVATAR_COLORS = [
  "#0f6cbd",
  "#c239b3",
  "#038387",
  "#ca5010",
  "#4f6bed",
  "#498205",
  "#8764b8",
  "#d13438",
  "#986f0b",
  "#00727c",
  "#5c2e91",
  "#b146c2",
  "#13a10e",
  "#a4262c",
];

/** Stable colour for a person, derived from their address. */
export function colorFor(key: string): string {
  let hash = 0;
  for (const ch of key.toLowerCase()) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  return AVATAR_COLORS[Math.abs(hash) % AVATAR_COLORS.length]!;
}

/** Compact timestamp for lists: "14:05", "Mon 14:05", "12/09", "12/09/2025". */
export function formatListDate(isoDate: string, now = new Date()): string {
  const d = new Date(isoDate);
  const days = differenceInCalendarDays(now, d);
  if (days === 0) return format(d, "HH:mm");
  if (days > 0 && days < 7) return format(d, "EEE HH:mm");
  if (isSameYear(d, now)) return format(d, "d MMM");
  return format(d, "d MMM yyyy");
}

/** Full timestamp for message headers: "Tue 14/10/2026 14:05". */
export function formatFullDate(isoDate: string): string {
  return format(new Date(isoDate), "EEE d MMM yyyy HH:mm");
}

/** Group label for message lists. */
export function dateGroup(isoDate: string, now = new Date()): string {
  const d = new Date(isoDate);
  const days = differenceInCalendarDays(now, d);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return "This week";
  if (days < 14) return "Last week";
  if (isSameYear(d, now) && d.getMonth() === now.getMonth()) return "Earlier this month";
  if (isSameYear(d, now)) return format(d, "MMMM");
  return format(d, "MMMM yyyy");
}

/** The local calendar day an all-day boundary (stored as midnight UTC) means. */
function allDayToLocal(iso: string): Date {
  const d = new Date(iso);
  return new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate());
}

/**
 * "09:30 – 10:00", "All day", "8 Oct – 11 Oct". Timed events are shown in
 * local time; all-day events use the stored dates (midnight UTC of the first
 * day to midnight UTC of the day after the last), whatever the time zone.
 */
export function formatTimeRange(startIso: string, endIso: string, allDay: boolean): string {
  if (allDay) {
    const start = allDayToLocal(startIso);
    const end = allDayToLocal(endIso);
    const lastDay = end > start ? new Date(end.getTime() - 1) : start;
    return isSameDay(start, lastDay)
      ? "All day"
      : `${format(start, "d MMM")} – ${format(lastDay, "d MMM")}`;
  }
  const start = new Date(startIso);
  const end = new Date(endIso);
  if (isSameDay(start, end)) return `${format(start, "HH:mm")} – ${format(end, "HH:mm")}`;
  return `${format(start, "d MMM HH:mm")} – ${format(end, "d MMM HH:mm")}`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
