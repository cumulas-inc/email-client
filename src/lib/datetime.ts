// Locale-aware date and time formatting. Everything follows the system locale
// (the webview reports it as navigator.language), so Americans see
// "Oct 14, 3:05 PM" and weeks starting on Sunday while, say, Germans keep
// "14. Okt., 15:05" and Monday. American English when the locale is unknown.

import type { Day } from "date-fns";

const FALLBACK_LOCALE = "en-US";

function detectLocale(): string {
  const requested = typeof navigator !== "undefined" ? navigator.language : undefined;
  try {
    return Intl.getCanonicalLocales(requested || FALLBACK_LOCALE)[0] ?? FALLBACK_LOCALE;
  } catch {
    return FALLBACK_LOCALE;
  }
}

let locale = detectLocale();

export function appLocale(): string {
  return locale;
}

/** For tests: format as if the system locale were `tag`. */
export function setLocaleForTesting(tag: string): void {
  locale = tag;
  formatters.clear();
}

const formatters = new Map<string, Intl.DateTimeFormat>();

function formatter(options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = `${locale}|${JSON.stringify(options)}`;
  let f = formatters.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(locale, options);
    formatters.set(key, f);
  }
  return f;
}

/**
 * ICU separates "3:05" and "PM", or the two sides of a range, with narrow
 * and thin spaces; plain spaces keep output predictable for comparisons.
 */
function tidy(text: string): string {
  return text.replace(/[\u202f\u2009\u00a0]/g, " ");
}

const PRESETS = {
  /** "Oct 14" */
  short: { month: "short", day: "numeric" },
  /** "Oct 14, 2026" */
  shortYear: { month: "short", day: "numeric", year: "numeric" },
  /** "October 14" */
  long: { month: "long", day: "numeric" },
  /** "October 14, 2026" */
  longYear: { month: "long", day: "numeric", year: "numeric" },
  /** "Wed, Oct 14" */
  weekdayShort: { weekday: "short", month: "short", day: "numeric" },
  /** "Wed, Oct 14, 2026" */
  weekdayShortYear: { weekday: "short", month: "short", day: "numeric", year: "numeric" },
  /** "Wednesday, October 14" */
  weekdayLong: { weekday: "long", month: "long", day: "numeric" },
  /** "Wednesday, October 14, 2026" */
  weekdayLongYear: { weekday: "long", month: "long", day: "numeric", year: "numeric" },
  /** "October 2026" */
  monthYear: { month: "long", year: "numeric" },
  /** "October" */
  month: { month: "long" },
  /** "Wed" */
  weekday: { weekday: "short" },
  /** "Wednesday" */
  weekdayName: { weekday: "long" },
  /** "W" */
  weekdayNarrow: { weekday: "narrow" },
  /** "14" */
  dayOfMonth: { day: "numeric" },
} satisfies Record<string, Intl.DateTimeFormatOptions>;

export type DateStyle = keyof typeof PRESETS;

export function formatDate(date: Date, style: DateStyle): string {
  return tidy(formatter(PRESETS[style]).format(date));
}

/** "Oct 12 – 16, 2026", "Oct 30 – Nov 3", merging the parts the dates share. */
export function formatDateRange(start: Date, end: Date, style: DateStyle): string {
  return tidy(formatter(PRESETS[style]).formatRange(start, end));
}

const TIME: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };

/** "3:05 PM" or "15:05". */
export function formatTime(date: Date): string {
  return tidy(formatter(TIME).format(date));
}

/** "9:00 – 10:30 AM", or across days "Oct 14, 9:00 PM – Oct 15, 1:00 AM". */
export function formatTimeSpan(start: Date, end: Date): string {
  const sameDay =
    start.getFullYear() === end.getFullYear() &&
    start.getMonth() === end.getMonth() &&
    start.getDate() === end.getDate();
  const options = sameDay ? TIME : { ...TIME, month: "short", day: "numeric" };
  return tidy(formatter(options as Intl.DateTimeFormatOptions).formatRange(start, end));
}

/** Hour labels for time grids: "8 AM" with a 12-hour clock, "08:00" otherwise. */
export function formatHour(date: Date): string {
  const options: Intl.DateTimeFormatOptions = uses24HourClock()
    ? { hour: "2-digit", minute: "2-digit" }
    : { hour: "numeric" };
  return tidy(formatter(options).format(date));
}

/** "Wed, Oct 14, 2026, 3:05 PM" */
export function formatDateTime(date: Date): string {
  return tidy(
    formatter({ ...PRESETS.weekdayShortYear, ...TIME } as Intl.DateTimeFormatOptions).format(date),
  );
}

export function uses24HourClock(): boolean {
  const cycle = formatter({ hour: "numeric" }).resolvedOptions().hourCycle;
  return cycle === "h23" || cycle === "h24";
}

/** Regions whose weeks start on Sunday, for engines without Intl week info. */
const SUNDAY_FIRST = new Set([
  "US",
  "CA",
  "MX",
  "BR",
  "JP",
  "KR",
  "TW",
  "HK",
  "IL",
  "PH",
  "IN",
  "ZA",
  "SA",
]);

/** First day of the week for the locale, in date-fns numbering (0 = Sunday). */
export function weekStartsOn(): Day {
  try {
    const intlLocale = new Intl.Locale(locale) as Intl.Locale & {
      getWeekInfo?: () => { firstDay: number };
      weekInfo?: { firstDay: number };
    };
    const info = intlLocale.getWeekInfo?.() ?? intlLocale.weekInfo;
    if (info) return (info.firstDay % 7) as Day;
    const region = intlLocale.maximize().region ?? "US";
    return SUNDAY_FIRST.has(region) ? 0 : 1;
  } catch {
    return 0;
  }
}
