// Readable tints derived from a calendar's color (which comes from data, so
// it cannot be a design token). Each color yields a light and a dark theme
// variant; calendar.css picks one with the same selectors as tokens.css.

import type { CSSProperties } from "react";

type Rgb = [number, number, number];

/** Surface colors of the two themes (tokens.css --bg-surface). */
const LIGHT_SURFACE: Rgb = [255, 255, 255];
const DARK_SURFACE: Rgb = [31, 31, 31];
const BLACK: Rgb = [0, 0, 0];
const WHITE: Rgb = [255, 255, 255];
const FALLBACK = "#0f6cbd";

/** Minimum contrast of event text on its tint (WCAG AA is 4.5). */
export const TEXT_CONTRAST = 5.5;

export function parseColor(value: string): Rgb | null {
  const hex = value.trim().replace(/^#/, "");
  if (/^[0-9a-f]{3}$/i.test(hex)) {
    return [0, 1, 2].map((i) => parseInt(hex[i]! + hex[i]!, 16)) as Rgb;
  }
  if (/^[0-9a-f]{6}([0-9a-f]{2})?$/i.test(hex)) {
    return [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Rgb;
  }
  return null;
}

export function toHex([r, g, b]: Rgb): string {
  return `#${[r, g, b].map((v) => Math.round(v).toString(16).padStart(2, "0")).join("")}`;
}

/** `amount` of `a` mixed into `b`. */
function mix(a: Rgb, b: Rgb, amount: number): Rgb {
  return [0, 1, 2].map((i) => a[i]! * amount + b[i]! * (1 - amount)) as Rgb;
}

function luminance(rgb: Rgb): number {
  const [r, g, b] = rgb.map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as Rgb;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(a: string | Rgb, b: string | Rgb): number {
  const ra = typeof a === "string" ? (parseColor(a) ?? BLACK) : a;
  const rb = typeof b === "string" ? (parseColor(b) ?? BLACK) : b;
  const [hi, lo] = [luminance(ra), luminance(rb)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/**
 * Moves `color` toward `target` until it contrasts enough with every
 * background (or returns `target`).
 */
function readable(color: Rgb, target: Rgb, backgrounds: Rgb[], minimum: number, start = 0): Rgb {
  for (let t = start; t < 1; t += 0.05) {
    const candidate = mix(target, color, t);
    if (backgrounds.every((bg) => contrast(candidate, bg) >= minimum)) return candidate;
  }
  return target;
}

/** `amount` of the color on the surface, reduced until `text` stays readable on it. */
function tintBackground(base: Rgb, surface: Rgb, amount: number, text: Rgb, minimum: number): Rgb {
  for (let a = amount; a > 0; a -= 0.02) {
    const bg = mix(base, surface, a);
    if (contrast(text, bg) >= minimum) return bg;
  }
  return surface;
}

export interface Tint {
  /** Event background. */
  bg: string;
  bgHover: string;
  /** Text on `bg`. */
  fg: string;
  /** Accent bar, swatch and border. */
  bar: string;
  /** Background of a selected event, with `onSolid` text. */
  solid: string;
  onSolid: string;
}

export interface CalendarTints {
  light: Tint;
  dark: Tint;
}

function tint(base: Rgb, surface: Rgb, dark: boolean): Tint {
  const text = dark ? WHITE : BLACK;
  // In dark mode the accent must stand out from the dark surface.
  const bar = dark ? readable(base, WHITE, [surface], 3) : readable(base, BLACK, [surface], 1.6);
  const bg = tintBackground(base, surface, dark ? 0.3 : 0.16, text, dark ? 8 : 9);
  const bgHover = tintBackground(base, surface, dark ? 0.4 : 0.26, text, dark ? 7 : 8);
  const fg = readable(base, text, [bg, bgHover], TEXT_CONTRAST, 0.35);
  // Selected events: the calendar color, darkened until white text reads well.
  const solid = readable(dark ? mix(bar, base, 0.5) : base, BLACK, [WHITE], 4.6);
  return {
    bg: toHex(bg),
    bgHover: toHex(bgHover),
    fg: toHex(fg),
    bar: toHex(bar),
    solid: toHex(solid),
    onSolid: toHex(WHITE),
  };
}

const cache = new Map<string, CalendarTints>();

export function calendarTints(color: string | null | undefined): CalendarTints {
  const key = color ?? FALLBACK;
  const cached = cache.get(key);
  if (cached) return cached;
  const base = parseColor(key) ?? parseColor(FALLBACK)!;
  const tints = { light: tint(base, LIGHT_SURFACE, false), dark: tint(base, DARK_SURFACE, true) };
  cache.set(key, tints);
  return tints;
}

/** CSS custom properties consumed by the `.cal-tint` class. */
export function tintStyle(color: string | null | undefined): CSSProperties {
  const { light, dark } = calendarTints(color);
  return {
    "--tint-l-bg": light.bg,
    "--tint-l-bg-hover": light.bgHover,
    "--tint-l-fg": light.fg,
    "--tint-l-bar": light.bar,
    "--tint-l-solid": light.solid,
    "--tint-l-on-solid": light.onSolid,
    "--tint-d-bg": dark.bg,
    "--tint-d-bg-hover": dark.bgHover,
    "--tint-d-fg": dark.fg,
    "--tint-d-bar": dark.bar,
    "--tint-d-solid": dark.solid,
    "--tint-d-on-solid": dark.onSolid,
  } as CSSProperties;
}
