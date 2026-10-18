// What the search box offers for what's being typed: recent searches, people,
// filters, folders, accounts and dates. Each option says how it changes the
// text; the search box (MailSearchField) shows them and applies a pick.

import type { Account, EmailAddress, Folder, FolderRole } from "../../../api/types";
import { type Token, formatValue, replaceToken, tokenAt } from "../../../lib/search/edit";
import { DATE_KEYWORDS, type DateKeyword, scopeKey } from "../../../lib/search/query";

export type SuggestionKind =
  "recent" | "clearRecent" | "person" | "filter" | "folder" | "account" | "date" | "hint";

/** The text and caret after a pick; `awaitsValue` keeps the suggestions open for it. */
export interface Applied {
  text: string;
  caret: number;
  awaitsValue: boolean;
}

export interface Suggestion {
  id: string;
  kind: SuggestionKind;
  label: string;
  detail?: string;
  ariaLabel: string;
  /** Shown, never picked (the date hint). */
  disabled?: boolean;
  apply(text: string, caret: number): Applied | "clearRecent";
}

export interface SuggestionGroup {
  id: string;
  heading: string;
  options: Suggestion[];
}

export interface SuggestionInput {
  text: string;
  caret: number;
  /** Recent searches, newest first. */
  recent: readonly string[];
  /** Addresses for the token being typed (useSuggestions), when it asks for people. */
  people: readonly EmailAddress[];
  folders: readonly Folder[];
  accounts: readonly Account[];
}

/** Most options shown in all. */
export const MAX_SUGGESTIONS = 15;

/** Operators, as offered, with what they're for. */
export const OPERATOR_DETAILS: readonly [label: string, detail: string][] = [
  ["from:", "Messages from someone"],
  ["to:", "Messages to someone"],
  ["cc:", "Messages copied to someone"],
  ["subject:", "Words in the subject"],
  ["has:attachment", "Messages with attachments"],
  ["is:unread", "Unread messages"],
  ["is:read", "Read messages"],
  ["is:flagged", "Flagged messages"],
  ["in:", "Messages in a folder"],
  ["account:", "Messages in one account"],
  ["received:", "Messages from a day or a range"],
  ["after:", "Messages on or after a day"],
  ["before:", "Messages before a day"],
];

/** What the empty box offers. */
const EMPTY_BOX_FILTERS = [
  "from:",
  "is:unread",
  "is:flagged",
  "has:attachment",
  "received:",
  "in:",
];

/** Operators whose value is a person. */
export const PERSON_OPERATORS: ReadonlySet<string> = new Set(["from", "to", "cc"]);
const PERSON_WORD: Partial<Record<string, string>> = { from: "From", to: "To", cc: "Cc" };

export const DATE_LABELS: Record<DateKeyword, string> = {
  today: "Today",
  yesterday: "Yesterday",
  "this week": "This week",
  "last week": "Last week",
  "this month": "This month",
  "last month": "Last month",
  "this year": "This year",
  "last year": "Last year",
};

/** The well-known folders, as `in:` names them, in the folder pane's order. */
export const FOLDER_ROLES: readonly [FolderRole, string][] = [
  ["inbox", "Inbox"],
  ["drafts", "Drafts"],
  ["sent", "Sent"],
  ["archive", "Archive"],
  ["junk", "Junk"],
  ["trash", "Trash"],
];

/** Puts `insertion` in place of the token at the caret; a space after it unless it awaits a value. */
function inserting(insertion: string, awaitsValue = insertion.endsWith(":")) {
  return (text: string, caret: number): Applied => ({
    ...replaceToken(text, tokenAt(text, caret), insertion, !awaitsValue),
    awaitsValue,
  });
}

/** The whole text, a recent search. */
function replacingAll(search: string) {
  return (): Applied => ({ text: search, caret: search.length, awaitsValue: false });
}

const contains = (haystack: string, needle: string) =>
  haystack.toLowerCase().includes(needle.toLowerCase());

function filterOption([label, detail]: readonly [string, string]): Suggestion {
  return {
    id: `filter-${label}`,
    kind: "filter",
    label,
    detail,
    ariaLabel: `${label} ${detail}`,
    apply: inserting(label),
  };
}

/** The filters the empty box offers, in this order. */
function emptyBoxFilters(): Suggestion[] {
  return EMPTY_BOX_FILTERS.map((label) =>
    filterOption(OPERATOR_DETAILS.find(([l]) => l === label)!),
  );
}

function filterOptions(names: (label: string) => boolean, max: number): Suggestion[] {
  return OPERATOR_DETAILS.filter(([label]) => names(label))
    .slice(0, max)
    .map(filterOption);
}

function personOptions(
  people: readonly EmailAddress[],
  operator: string,
  max: number,
): Suggestion[] {
  const seen = new Set<string>();
  return people
    .filter((p) => {
      const key = p.address.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .slice(0, max)
    .map((p) => {
      const word = PERSON_WORD[operator] ?? "From";
      // The address beside the name; alone, it is the label.
      return {
        id: `person-${p.address.toLowerCase()}`,
        kind: "person" as const,
        label: p.name || p.address,
        detail: p.name ? p.address : undefined,
        ariaLabel: p.name ? `${word} ${p.name}, ${p.address}` : `${word} ${p.address}`,
        apply: inserting(`${operator}:${formatValue(p.address)}`, false),
      };
    });
}

function recentOptions(searches: readonly string[]): Suggestion[] {
  return searches.map((search) => ({
    id: `recent-${search}`,
    kind: "recent" as const,
    label: search,
    ariaLabel: search,
    apply: replacingAll(search),
  }));
}

const CLEAR_RECENT: Suggestion = {
  id: "clear-recent",
  kind: "clearRecent",
  label: "Clear recent searches",
  ariaLabel: "Clear recent searches",
  apply: () => "clearRecent",
};

/** The folders `in:value` could name: roles present first, then other folders' names. */
function folderOptions(value: string, folders: readonly Folder[]): Suggestion[] {
  const roles = FOLDER_ROLES.filter(
    ([role, label]) =>
      folders.some((f) => f.role === role) && (contains(label, value) || contains(role, value)),
  ).map(([role, label]) => ({
    id: `folder-role-${role}`,
    kind: "folder" as const,
    label,
    ariaLabel: `In ${label}`,
    apply: inserting(`in:${role}`, false),
  }));
  const seen = new Set<string>();
  const named: Suggestion[] = [];
  for (const f of folders) {
    const key = scopeKey(f.name);
    if (f.role !== "other" || !key || seen.has(key) || !contains(f.name, value)) continue;
    seen.add(key);
    named.push({
      id: `folder-${key}`,
      kind: "folder",
      label: f.name,
      ariaLabel: `In ${f.name}`,
      apply: inserting(`in:${formatValue(f.name)}`, false),
    });
  }
  return [...roles, ...named].slice(0, 8);
}

function accountOptions(value: string, accounts: readonly Account[]): Suggestion[] {
  return accounts
    .filter((a) => contains(a.displayName, value) || contains(a.email, value))
    .map((a) => ({
      id: `account-${a.id}`,
      kind: "account" as const,
      label: a.displayName || a.email,
      detail: a.displayName ? a.email : undefined,
      ariaLabel: a.displayName ? `Account ${a.displayName}, ${a.email}` : `Account ${a.email}`,
      apply: inserting(`account:${formatValue(a.email)}`, false),
    }));
}

function dateOptions(operator: string, value: string): Suggestion[] {
  const typed = value.toLowerCase().replace(/\s+/g, " ").trimStart();
  const options: Suggestion[] = DATE_KEYWORDS.filter((k) => k.startsWith(typed)).map((k) => {
    const clause = `${operator}:${formatValue(k)}`;
    return {
      id: `date-${operator}-${k}`,
      kind: "date",
      label: clause,
      detail: DATE_LABELS[k],
      ariaLabel: `${clause} ${DATE_LABELS[k]}`,
      apply: inserting(clause, false),
    };
  });
  if (operator === "received") {
    options.push({
      id: "date-hint",
      kind: "hint",
      label: "Or a date: YYYY-MM-DD",
      ariaLabel: "Or a date: YYYY-MM-DD",
      disabled: true,
      apply: (text, caret) => ({ text, caret, awaitsValue: true }),
    });
  }
  return options;
}

/** Whether the token at the caret asks for people, and for which text. */
export function peopleQuery(text: string, caret: number): string | null {
  const token: Token = tokenAt(text, caret);
  if (token.operator) return PERSON_OPERATORS.has(token.operator) ? token.value : null;
  return token.value && !token.quoted ? token.value : null;
}

/** The options for the text and the caret, in groups. */
export function buildSuggestions({
  text,
  caret,
  recent,
  people,
  folders,
  accounts,
}: SuggestionInput): SuggestionGroup[] {
  const groups: SuggestionGroup[] = [];
  const add = (id: string, heading: string, options: Suggestion[]) => {
    if (options.length > 0) groups.push({ id, heading, options });
  };
  const token = tokenAt(text, caret);
  const { operator, value } = token;

  if (!text.trim()) {
    const searches = recentOptions(recent.slice(0, 8));
    add("recent", "Recent searches", searches.length > 0 ? [...searches, CLEAR_RECENT] : []);
    add("filters", "Filters", emptyBoxFilters());
  } else if (operator && PERSON_OPERATORS.has(operator)) {
    add("people", "People", personOptions(people, operator, 8));
  } else if (operator === "is" || operator === "has") {
    add(
      "filters",
      "Filters",
      filterOptions((label) => {
        const [name, option] = label.split(":") as [string, string];
        return name === operator && !!option && option.startsWith(value.toLowerCase());
      }, 6),
    );
  } else if (operator === "in") {
    add("folders", "Folders", folderOptions(value, folders));
  } else if (operator === "account") {
    add("accounts", "Accounts", accountOptions(value, accounts));
  } else if (operator === "received" || operator === "after" || operator === "before") {
    add("dates", "Dates", dateOptions(operator, value));
  } else if (!operator && !token.quoted && value) {
    const word = value.toLowerCase();
    add(
      "recent",
      "Recent searches",
      recentOptions(recent.filter((s) => contains(s, word) && !same(s, text)).slice(0, 3)),
    );
    add("people", "People", personOptions(people, "from", 5));
    add(
      "filters",
      "Filters",
      filterOptions((label) => label.startsWith(word), 6),
    );
  } else if (!operator && !value) {
    // After a space: what can narrow the search further.
    add("filters", "Filters", emptyBoxFilters());
  }
  return cap(groups, MAX_SUGGESTIONS);
}

const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/** At most `max` options in all, the first groups' first. */
function cap(groups: SuggestionGroup[], max: number): SuggestionGroup[] {
  let left = max;
  const out: SuggestionGroup[] = [];
  for (const group of groups) {
    if (left <= 0) break;
    const options = group.options.slice(0, left);
    left -= options.length;
    out.push({ ...group, options });
  }
  return out;
}
