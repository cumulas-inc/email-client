import type { MouseEvent } from "react";

import { appLocale } from "../../lib/datetime";

/**
 * Hover handler that shows an element's whole text as a tooltip, but only
 * while it is cut off (by an ellipsis or a line clamp), so names that fit
 * don't get a tooltip repeating them.
 */
export function titleWhenCut(e: MouseEvent<HTMLElement>) {
  const el = e.currentTarget;
  const cut = el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1;
  if (cut) el.title = el.textContent ?? "";
  else el.removeAttribute("title");
}

const exactFormats = new Map<string, Intl.NumberFormat>();

/** "1,204" for screen readers and tooltips, where the count isn't shortened. */
export function exactCount(n: number): string {
  const locale = appLocale();
  let format = exactFormats.get(locale);
  if (!format) {
    format = new Intl.NumberFormat(locale);
    exactFormats.set(locale, format);
  }
  return format.format(n);
}
