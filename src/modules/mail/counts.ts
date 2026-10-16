import { appLocale } from "../../lib/datetime";

/** "48,210": the whole number, grouped, for tooltips and screen readers. */
export function exactCount(n: number): string {
  return new Intl.NumberFormat(appLocale()).format(n);
}
