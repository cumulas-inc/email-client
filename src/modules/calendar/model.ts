// Non-visual helpers about calendars and accounts.

import type { Account, Calendar, EmailAddress } from "../../api/types";

export interface CalendarGroup {
  account: Account | null;
  accountId: string;
  calendars: Calendar[];
}

/** Calendars grouped by account, in account order, default calendar first. */
export function groupCalendars(accounts: Account[], calendars: Calendar[]): CalendarGroup[] {
  const groups = new Map<string, CalendarGroup>();
  for (const account of accounts) {
    groups.set(account.id, { account, accountId: account.id, calendars: [] });
  }
  for (const calendar of calendars) {
    let group = groups.get(calendar.accountId);
    if (!group) {
      group = { account: null, accountId: calendar.accountId, calendars: [] };
      groups.set(calendar.accountId, group);
    }
    group.calendars.push(calendar);
  }
  for (const group of groups.values()) {
    group.calendars.sort((a, b) => Number(b.isDefault) - Number(a.isDefault));
  }
  return [...groups.values()].filter((g) => g.calendars.length > 0);
}

/** The calendar new events go to: the first account's default writable one. */
export function defaultCalendar(
  accounts: Account[],
  calendars: Calendar[],
  hidden: ReadonlySet<string> = new Set(),
): Calendar | null {
  const writable = groupCalendars(accounts, calendars)
    .flatMap((g) => g.calendars)
    .filter((c) => c.canEdit);
  return (
    writable.find((c) => c.isDefault && !hidden.has(c.id)) ??
    writable.find((c) => !hidden.has(c.id)) ??
    writable[0] ??
    null
  );
}

const EMAIL = /^[^\s@<>,;]+@[^\s@<>,;]+\.[^\s@<>,;]+$/;

/** "Name <address>" or a bare address, or null when it is not an address. */
export function parseAddress(text: string): EmailAddress | null {
  const trimmed = text
    .trim()
    .replace(/[,;]+$/, "")
    .trim();
  const named = /^(.*?)\s*<([^>]+)>$/.exec(trimmed);
  if (named) {
    const address = named[2]!.trim();
    if (!EMAIL.test(address)) return null;
    const name = named[1]!.trim().replace(/^"(.*)"$/, "$1");
    return { name: name || null, address };
  }
  return EMAIL.test(trimmed) ? { name: null, address: trimmed } : null;
}
