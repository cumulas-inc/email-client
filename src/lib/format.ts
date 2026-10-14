import { differenceInCalendarDays, isSameDay, isSameYear } from "date-fns";

import type { EmailAddress } from "../api/types";
import {
  appLocale,
  formatDate,
  formatDateRange,
  formatDateTime,
  formatTime,
  formatTimeSpan,
} from "./datetime";

/** Name to show for an address: the display name, else the address. */
export function displayName(addr: EmailAddress | null | undefined): string {
  if (!addr) return "(no sender)";
  return addr.name?.trim() || addr.address;
}

/** "Name <address>" or just the address. */
export function fullAddress(addr: EmailAddress): string {
  return addr.name ? `${addr.name} <${addr.address}>` : addr.address;
}

// Name suffixes that don't make initials: "Maximilian Montgomery III" is "MM".
const NAME_SUFFIX = /^(?:jr|sr|ii|iii|iv|phd|md|esq)$/i;

/**
 * Up to two letters for an avatar: "PR" for Priya Raman, "SE" for
 * servicedesk@…, "נכ" for נועה כהן. "?" when there are no letters to use (a
 * phone number, emoji, "(no sender)"), so callers can show a generic person.
 */
export function initials(nameOrAddress: string): string {
  const text = nameOrAddress
    .normalize("NFKC")
    .replace(/<[^>]*>/g, "")
    .replace(/\([^)]*\)/g, " ")
    .trim();
  const base = text.includes("@") && !/\s/.test(text) ? text.split("@")[0]! : text;
  const words = base
    .split(/[\s._-]+/)
    .map((w) => w.replace(/[^\p{L}\p{N}]/gu, ""))
    .filter((w) => /\p{L}/u.test(w) && !NAME_SUFFIX.test(w));
  if (words.length === 0) return "?";
  // Chinese and Japanese names: the first character, as their apps show it.
  if (CJK.test(words[0]![0]!)) return Array.from(words[0]!)[0]!;
  const first = letters(words[0]!);
  if (words.length === 1) return first.slice(0, 2).join("").toLocaleUpperCase();
  return (first[0]! + letters(words[words.length - 1]!)[0]!).toLocaleUpperCase();
}

const CJK = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/u;
// Thai and Lao vowels written before their consonant: "ใจดี" starts with จ.
const LEADING_VOWEL = /^[\u0E40-\u0E44\u0EC0-\u0EC4]/;

function letters(word: string): string[] {
  return Array.from(word.replace(LEADING_VOWEL, "") || word);
}

/** The letters on an account's badge: "AM" for Alex Morgan, "W" for Work. */
export function accountMark(name: string): string {
  const words = name
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .split(/\s+/)
    .filter(Boolean);
  const first = words[0]?.[0] ?? "?";
  const last = words.length > 1 ? (words[words.length - 1]?.[0] ?? "") : "";
  return (first + last).toLocaleUpperCase();
}

export const AVATAR_HUES = [
  "teal",
  "rose",
  "sky",
  "amber",
  "violet",
  "green",
  "orange",
  "slate",
] as const;

export type AvatarHue = (typeof AVATAR_HUES)[number];

/** Stable avatar hue for a person, derived from their address. */
export function hueFor(key: string): AvatarHue {
  let hash = 0;
  for (const ch of key.toLowerCase()) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  return AVATAR_HUES[Math.abs(hash) % AVATAR_HUES.length]!;
}

/**
 * A count for badges and headers: "1,204" with the locale's grouping, and
 * compact from ten thousand on ("12K") so badges stay small. Callers put the
 * exact number in a tooltip when it's shortened.
 */
const countFormats = new Map<string, Intl.NumberFormat>();

function countFormat(compact: boolean): Intl.NumberFormat {
  const key = `${appLocale()}|${compact}`;
  let f = countFormats.get(key);
  if (!f) {
    f = new Intl.NumberFormat(
      appLocale(),
      compact ? { notation: "compact", maximumFractionDigits: 0 } : undefined,
    );
    countFormats.set(key, f);
  }
  return f;
}

export function formatCount(n: number): string {
  return countFormat(n >= 10_000).format(n);
}

/** Compact timestamp for lists: "3:05 PM", "Mon 3:05 PM", "Oct 9", "Oct 9, 2025". */
export function formatListDate(isoDate: string, now = new Date()): string {
  const d = new Date(isoDate);
  const days = differenceInCalendarDays(now, d);
  if (days === 0) return formatTime(d);
  if (days > 0 && days < 7) return `${formatDate(d, "weekday")} ${formatTime(d)}`;
  if (isSameYear(d, now)) return formatDate(d, "short");
  return formatDate(d, "shortYear");
}

/** Full timestamp for message headers: "Wed, Oct 14, 2026, 3:05 PM". */
export function formatFullDate(isoDate: string): string {
  return formatDateTime(new Date(isoDate));
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
  if (isSameYear(d, now)) return formatDate(d, "month");
  return formatDate(d, "monthYear");
}

/** The local calendar day an all-day boundary (stored as midnight UTC) means. */
function allDayToLocal(iso: string): Date {
  const d = new Date(iso);
  return new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate());
}

/**
 * "9:30 – 10:00 AM", "All day", "Oct 8 – 11". Timed events are shown in
 * local time; all-day events use the stored dates (midnight UTC of the first
 * day to midnight UTC of the day after the last), whatever the time zone.
 */
export function formatTimeRange(startIso: string, endIso: string, allDay: boolean): string {
  if (allDay) {
    const start = allDayToLocal(startIso);
    const end = allDayToLocal(endIso);
    const lastDay = end > start ? new Date(end.getTime() - 1) : start;
    return isSameDay(start, lastDay) ? "All day" : formatDateRange(start, lastDay, "short");
  }
  return formatTimeSpan(new Date(startIso), new Date(endIso));
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
