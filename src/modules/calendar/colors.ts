// Readable tints derived from a calendar's color (which comes from data, so
// it cannot be a design token). Each color yields a light and a dark theme
// variant; calendar.css picks one with the same selectors as tokens.css.
//
// Tints are built in OKLCH: every calendar gets the same lightness steps
// whatever its hue, so a gold and a sky calendar look equally quiet, and the
// hue of the calendar color is kept. Contrast is then checked in sRGB and the
// lightness nudged until text and stripes stay readable. Neutral colors lean
// toward the warm paper of the theme instead of turning a cold grey.

import type { CSSProperties } from "react";

type Rgb = [number, number, number];
/** OKLab: lightness 0–1 and the a/b opponent axes. */
type Lab = [number, number, number];

const BLACK: Rgb = [0, 0, 0];
const WHITE: Rgb = [255, 255, 255];
const FALLBACK = "#0b7a75";

/** Card surfaces of the two themes (tokens.css --bg-surface). */
export const LIGHT_SURFACE = "#ffffff";
export const DARK_SURFACE = "#22211e";
/** Warm neutrals that low-chroma tints lean toward (--bg-field, --bg-surface). */
const LIGHT_PAPER = "#f5f4f1";
const DARK_PAPER = DARK_SURFACE;

/** Minimum contrast of event titles on their tint (WCAG AA is 4.5). */
export const TEXT_CONTRAST = 5.5;
/** Minimum contrast of secondary event text (times, places). */
export const META_CONTRAST = 4.5;
/** Minimum contrast of stripes, swatches and outlines (WCAG non-text). */
export const GRAPHIC_CONTRAST = 3;

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
  return `#${[r, g, b]
    .map((v) =>
      Math.round(Math.min(255, Math.max(0, v)))
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

function toLinear(v: number): number {
  const c = v / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function fromLinear(v: number): number {
  const c = v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055;
  return c * 255;
}

function luminance(rgb: Rgb): number {
  const [r, g, b] = rgb.map(toLinear) as Rgb;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(a: string | Rgb, b: string | Rgb): number {
  const ra = typeof a === "string" ? (parseColor(a) ?? BLACK) : a;
  const rb = typeof b === "string" ? (parseColor(b) ?? BLACK) : b;
  const [hi, lo] = [luminance(ra), luminance(rb)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

// --- OKLab ------------------------------------------------------------------

function rgbToLab(rgb: Rgb): Lab {
  const [r, g, b] = rgb.map(toLinear) as Rgb;
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

/** Linear sRGB of an OKLab color (may be out of gamut). */
function labToLinear([L, a, b]: Lab): Rgb {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

const inGamut = (linear: Rgb) => linear.every((v) => v >= -0.0005 && v <= 1.0005);

/** Lightness, chroma and hue (degrees) of a color. */
export function oklch(color: string | Rgb): [number, number, number] {
  const rgb = typeof color === "string" ? (parseColor(color) ?? BLACK) : color;
  const [L, a, b] = rgbToLab(rgb);
  const hue = (Math.atan2(b, a) * 180) / Math.PI;
  return [L, Math.hypot(a, b), (hue + 360) % 360];
}

/**
 * The sRGB color with lightness `L` and chroma `C` toward `hue`, offset by
 * the warm `paper` neutral. Chroma is reduced until the color fits in sRGB,
 * which keeps the hue instead of clipping toward another one.
 */
function lch(L: number, C: number, hue: number, paper: Lab): Rgb {
  const h = (hue * Math.PI) / 180;
  const at = (c: number): Lab => [L, paper[1] + c * Math.cos(h), paper[2] + c * Math.sin(h)];
  let lo = 0;
  let hi = C;
  if (!inGamut(labToLinear(at(hi)))) {
    for (let i = 0; i < 16; i++) {
      const mid = (lo + hi) / 2;
      if (inGamut(labToLinear(at(mid)))) lo = mid;
      else hi = mid;
    }
    hi = lo;
  }
  // Rounded, so contrast is checked on the color that is finally used.
  return labToLinear(at(hi)).map((v) => Math.round(fromLinear(Math.min(1, Math.max(0, v))))) as Rgb;
}

/**
 * Steps the lightness of an LCH color (darker for `direction` -1, lighter for
 * +1) until it contrasts enough with every background.
 */
function readableLch(
  L: number,
  C: number,
  hue: number,
  paper: Lab,
  backgrounds: Rgb[],
  minimum: number,
  direction: 1 | -1,
): Rgb {
  for (let l = L; l >= 0 && l <= 1; l += direction * 0.01) {
    const candidate = lch(l, C, hue, paper);
    if (backgrounds.every((bg) => contrast(candidate, bg) >= minimum)) return candidate;
  }
  return direction < 0 ? BLACK : WHITE;
}

// --- Tints ------------------------------------------------------------------

export interface Tint {
  /** Event background. */
  bg: string;
  bgHover: string;
  /** Titles on `bg`: a deep shade of the calendar color. */
  fg: string;
  /** Times and places on `bg`. */
  sub: string;
  /** Inset stripe, outlines, rings and dots. */
  bar: string;
  /** Filled swatches (checkboxes, badges), with `onSolid` marks. */
  solid: string;
  onSolid: string;
}

export interface CalendarTints {
  light: Tint;
  dark: Tint;
}

const NO_PAPER: Lab = [0, 0, 0];

function paperOf(hex: string): Lab {
  const [, a, b] = rgbToLab(parseColor(hex)!);
  return [0, a, b];
}

function lightTint(base: Rgb): Tint {
  const [L, C, h] = oklch(base);
  const paper = paperOf(LIGHT_PAPER);
  const surface = parseColor(LIGHT_SURFACE)!;
  const bg = lch(0.945, Math.min(C * 0.2, 0.032), h, paper);
  const bgHover = lch(0.912, Math.min(C * 0.28, 0.046), h, paper);
  const grounds = [surface, bg, bgHover];
  // The calendar color itself when it shows up on white, else a deeper one.
  const bar = grounds.every((g) => contrast(base, g) >= GRAPHIC_CONTRAST)
    ? base
    : readableLch(L, C, h, NO_PAPER, grounds, GRAPHIC_CONTRAST, -1);
  const fg = readableLch(
    Math.min(L, 0.42),
    Math.min(C * 0.8, 0.11),
    h,
    NO_PAPER,
    grounds,
    TEXT_CONTRAST,
    -1,
  );
  const sub = readableLch(
    Math.min(L, 0.5),
    Math.min(C * 0.7, 0.09),
    h,
    NO_PAPER,
    grounds,
    META_CONTRAST,
    -1,
  );
  const solid =
    contrast(base, WHITE) >= META_CONTRAST
      ? base
      : readableLch(L, C, h, NO_PAPER, [WHITE], META_CONTRAST, -1);
  return finish({ bg, bgHover, fg, sub, bar, solid, onSolid: WHITE });
}

function darkTint(base: Rgb): Tint {
  const [L, C, h] = oklch(base);
  const paper = paperOf(DARK_PAPER);
  const surface = parseColor(DARK_SURFACE)!;
  const bg = lch(0.3, Math.min(C * 0.34, 0.036), h, paper);
  const bgHover = lch(0.335, Math.min(C * 0.42, 0.046), h, paper);
  const grounds = [surface, bg, bgHover];
  // A lighter cut of the color so the stripe stands out from the dark card.
  const bar = readableLch(
    Math.max(L, 0.7),
    Math.min(C * 1.15, 0.15),
    h,
    NO_PAPER,
    grounds,
    GRAPHIC_CONTRAST,
    1,
  );
  const fg = readableLch(0.9, Math.min(C * 0.48, 0.06), h, NO_PAPER, grounds, TEXT_CONTRAST, 1);
  const sub = readableLch(0.77, Math.min(C * 0.66, 0.085), h, NO_PAPER, grounds, META_CONTRAST, 1);
  // Filled swatches take the bright cut with a dark mark, as dark text reads
  // better on it than white.
  const ink = lch(0.22, Math.min(C * 0.25, 0.025), h, paper);
  const onSolid = contrast(bar, ink) >= META_CONTRAST ? ink : WHITE;
  const solid =
    contrast(bar, onSolid) >= META_CONTRAST
      ? bar
      : readableLch(L, C, h, NO_PAPER, [WHITE], META_CONTRAST, -1);
  return finish({ bg, bgHover, fg, sub, bar, solid, onSolid });
}

function finish(t: Record<keyof Tint, Rgb>): Tint {
  return Object.fromEntries(Object.entries(t).map(([k, v]) => [k, toHex(v)])) as unknown as Tint;
}

const cache = new Map<string, CalendarTints>();

export function calendarTints(color: string | null | undefined): CalendarTints {
  const key = color ?? FALLBACK;
  const cached = cache.get(key);
  if (cached) return cached;
  const base = parseColor(key) ?? parseColor(FALLBACK)!;
  const tints = { light: lightTint(base), dark: darkTint(base) };
  cache.set(key, tints);
  return tints;
}

const TINT_KEYS: [keyof Tint, string][] = [
  ["bg", "bg"],
  ["bgHover", "bg-hover"],
  ["fg", "fg"],
  ["sub", "sub"],
  ["bar", "bar"],
  ["solid", "solid"],
  ["onSolid", "on-solid"],
];

/** CSS custom properties consumed by the `.cal-tint` class. */
export function tintStyle(color: string | null | undefined): CSSProperties {
  const { light, dark } = calendarTints(color);
  const style: Record<string, string> = {};
  for (const [key, name] of TINT_KEYS) {
    style[`--tint-l-${name}`] = light[key];
    style[`--tint-d-${name}`] = dark[key];
  }
  return style as CSSProperties;
}
