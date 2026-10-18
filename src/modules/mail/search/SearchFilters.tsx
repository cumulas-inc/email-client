import clsx from "clsx";
import { addDays, format } from "date-fns";
import { ChevronDown, X } from "lucide-react";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";

import { useAppStore } from "../../../app/store";
import { useAccounts } from "../../../api/queries";
import { Menu, type MenuItem } from "../../../components/Menu";
import { formatDate, formatDateRange, weekStartsOn } from "../../../lib/datetime";
import {
  type Clause,
  clearFilters,
  findClauses,
  formatValue,
  hasFilters,
  isAccountClause,
  isDateClause,
  isFlaggedClause,
  isFromClause,
  isHasClause,
  isInClause,
  isReadClause,
  removeClauses,
  setClause,
} from "../../../lib/search/edit";
import {
  DATE_KEYWORDS,
  type DateClause,
  type PlainDay,
  accountMatches,
  folderRoleOf,
  localMidnight,
  parseSearch,
  resolveDates,
  scopeKey,
} from "../../../lib/search/query";
import { useMailStore } from "../mailStore";
import { useFolders } from "../queries";
import { DateRangeDialog } from "./DateRangeDialog";
import { focusSearch } from "./focus";
import { DATE_LABELS, FOLDER_ROLES } from "./suggestions";

const DATE_CHIP_ID = "mail-search-date";

/** "Oct 1", or "Oct 1, 2025" in another year. */
function dayLabel(day: PlainDay): string {
  const date = localMidnight(day);
  return formatDate(date, date.getFullYear() === new Date().getFullYear() ? "short" : "shortYear");
}

function rangeLabel(first: PlainDay, last: PlainDay): string {
  const a = localMidnight(first);
  const b = localMidnight(last);
  const year = new Date().getFullYear();
  return formatDateRange(
    a,
    b,
    a.getFullYear() === year && b.getFullYear() === year ? "short" : "shortYear",
  );
}

/** A clause's value as a phrase in a label: a day, or a date word lowercased. */
function specWords(clause: DateClause): string {
  switch (clause.spec.kind) {
    case "day":
      return dayLabel(clause.spec.day);
    case "keyword":
      return DATE_LABELS[clause.spec.keyword].toLowerCase();
    case "range":
      return "";
  }
}

/** What the Date filter says for the search's dates. */
function dateLabel(dates: DateClause[]): string {
  if (dates.length === 0) return "Date";
  if (dates.length === 1) {
    const [clause] = dates as [DateClause];
    const { spec } = clause;
    if (clause.op === "received") {
      if (spec.kind === "keyword") return DATE_LABELS[spec.keyword];
      if (spec.kind === "day") return dayLabel(spec.day);
      if (spec.first && spec.last) return rangeLabel(spec.first, spec.last);
      if (spec.first) return `Since ${dayLabel(spec.first)}`;
      if (spec.last) return `Until ${dayLabel(spec.last)}`;
    }
    if (clause.op === "after") return `Since ${specWords(clause)}`;
    if (clause.op === "before") return `Before ${specWords(clause)}`;
  }
  if (dates.length === 2) {
    const after = dates.find((d) => d.op === "after");
    const before = dates.find((d) => d.op === "before");
    if (after?.spec.kind === "day" && before?.spec.kind === "day") {
      // The days both take in: from the first through the one before the second.
      const last = format(addDays(localMidnight(before.spec.day), -1), "yyyy-MM-dd");
      if (last >= after.spec.day) return rangeLabel(after.spec.day, last);
    }
  }
  return "Custom dates";
}

/** A filter that is on or off. */
function Toggle({
  label,
  pressed,
  onToggle,
}: {
  label: string;
  pressed: boolean;
  onToggle: (on: boolean) => void;
}) {
  return (
    <button
      type="button"
      className={clsx("mail-search-chip", pressed && "is-set")}
      aria-pressed={pressed}
      onClick={() => onToggle(!pressed)}
    >
      <span className="mail-search-chip__label">{label}</span>
    </button>
  );
}

/** A filter with a menu of choices. */
function MenuChip({
  id,
  label,
  menuLabel,
  set,
  items,
}: {
  id?: string;
  label: string;
  menuLabel: string;
  set: boolean;
  items: MenuItem[];
}) {
  return (
    <Menu
      fixed
      label={menuLabel}
      items={items}
      trigger={(props) => (
        <button
          id={id}
          type="button"
          className={clsx("mail-search-chip", set && "is-set")}
          title={label}
          {...props}
        >
          <span className="mail-search-chip__label">{label}</span>
          <ChevronDown size={14} className="mail-search-chip__caret" aria-hidden="true" />
        </button>
      )}
    />
  );
}

/**
 * The filters above the search results, while there's a search: each shows
 * what the text asks for, and changes the text as typing would.
 */
export function SearchFilters() {
  const search = useAppStore((s) => s.search);
  const setSearch = useAppStore((s) => s.setSearch);
  const folderId = useMailStore((s) => s.folderId);
  const { data: folders = [] } = useFolders();
  const { data: accounts = [] } = useAccounts();
  const query = useMemo(() => parseSearch(search), [search]);
  const clauses = useMemo(() => findClauses(search), [search]);
  const [rangeOpen, setRangeOpen] = useState(false);

  // The custom range's dialog gone, the keyboard goes back to the Date filter
  // (a closed Dialog renders nothing, so nothing else would put it there).
  const rangeWasOpen = useRef(false);
  useEffect(() => {
    if (rangeWasOpen.current && !rangeOpen) document.getElementById(DATE_CHIP_ID)?.focus();
    rangeWasOpen.current = rangeOpen;
  }, [rangeOpen]);

  const valuesOf = (test: (c: Clause) => boolean) =>
    clauses.filter((c) => test(c) && c.value.trim()).map((c) => c.value.trim());

  // From.
  const from = valuesOf(isFromClause);
  const askFrom = () => {
    const dangling = clauses.find((c) => isFromClause(c) && !c.value);
    const next = dangling ? search : `${search.trimEnd()} from:`.trimStart();
    flushSync(() => setSearch(next));
    focusSearch(dangling ? dangling.end : next.length);
  };
  const editFrom = () => {
    const clause = clauses.find((c) => isFromClause(c) && c.value);
    focusSearch(clause?.end);
  };

  // Dates.
  const dates = query.dates;
  const only = dates.length === 1 ? dates[0]! : null;
  const keyword =
    only?.op === "received" && only.spec.kind === "keyword" ? only.spec.keyword : null;
  const dateItems: MenuItem[] = [
    {
      label: "Any time",
      radio: true,
      checked: dates.length === 0,
      onSelect: () => setSearch(removeClauses(search, isDateClause)),
    },
    ...DATE_KEYWORDS.map((k) => ({
      label: DATE_LABELS[k],
      radio: true,
      checked: keyword === k,
      onSelect: () => setSearch(setClause(search, isDateClause, `received:${formatValue(k)}`)),
    })),
    {
      label: "Custom range…",
      radio: true,
      separator: true,
      checked: dates.length > 0 && !keyword,
      onSelect: () => setRangeOpen(true),
    },
  ];

  // Folders: the folder pane's own (unless a well-known one), then the well-known ones.
  const inValues = query.folders;
  const onlyIn = inValues.length === 1 ? inValues[0]! : null;
  const current = folders.find((f) => f.id === folderId);
  const currentFolder = current && current.role === "other" ? current : null;
  const roles = FOLDER_ROLES.filter(([role]) => folders.some((f) => f.role === role));
  const folderItems: MenuItem[] = [
    {
      label: "All folders",
      radio: true,
      checked: inValues.length === 0,
      onSelect: () => setSearch(removeClauses(search, isInClause)),
    },
  ];
  if (currentFolder) {
    folderItems.push({
      label: `Current folder (${currentFolder.name})`,
      radio: true,
      checked: onlyIn !== null && onlyIn === scopeKey(currentFolder.name),
      onSelect: () =>
        setSearch(setClause(search, isInClause, `in:${formatValue(currentFolder.name)}`)),
    });
  }
  for (const [role, label] of roles) {
    folderItems.push({
      label,
      radio: true,
      checked: onlyIn !== null && folderRoleOf(onlyIn) === role,
      onSelect: () => setSearch(setClause(search, isInClause, `in:${role}`)),
    });
  }
  const folderChecked = folderItems.find((item, i) => i > 0 && item.checked);
  const folderLabel =
    inValues.length === 0
      ? "Folder"
      : (folderChecked?.label ?? `In: ${valuesOf(isInClause).join(", ")}`);

  // Accounts, by name, or with their address when two share one.
  const accountLabel = (a: (typeof accounts)[number]) =>
    accounts.filter((b) => b.displayName === a.displayName).length > 1
      ? `${a.displayName} (${a.email})`
      : a.displayName || a.email;
  const accountValues = query.accounts;
  const checkedAccount =
    accountValues.length === 1
      ? accounts.find((a) => accountMatches(accountValues[0]!, a))
      : undefined;
  const accountItems: MenuItem[] = [
    {
      label: "All accounts",
      radio: true,
      checked: accountValues.length === 0,
      onSelect: () => setSearch(removeClauses(search, isAccountClause)),
    },
    ...accounts.map((a) => ({
      label: accountLabel(a),
      radio: true,
      checked: a === checkedAccount,
      onSelect: () =>
        setSearch(setClause(search, isAccountClause, `account:${formatValue(a.email)}`)),
    })),
  ];
  const accountChipLabel =
    accountValues.length === 0
      ? "Account"
      : checkedAccount
        ? accountLabel(checkedAccount)
        : `Account: ${valuesOf(isAccountClause).join(", ")}`;

  let fromChip: ReactNode;
  if (from.length > 0) {
    const label = `From: ${from.join(", ")}`;
    fromChip = (
      <span className="mail-search-chip mail-search-chip--split is-set">
        <button type="button" className="mail-search-chip__main" title={label} onClick={editFrom}>
          <span className="mail-search-chip__label">{label}</span>
        </button>
        <button
          type="button"
          className="mail-search-chip__remove"
          aria-label="Remove From filter"
          title="Remove"
          onClick={() => setSearch(removeClauses(search, isFromClause))}
        >
          <X size={13} aria-hidden="true" />
        </button>
      </span>
    );
  } else {
    fromChip = (
      <button type="button" className="mail-search-chip" onClick={askFrom}>
        <span className="mail-search-chip__label">From</span>
      </button>
    );
  }

  return (
    <div role="group" aria-label="Search filters" className="mail-search-filters">
      {fromChip}
      <MenuChip
        id={DATE_CHIP_ID}
        label={dateLabel(dates)}
        menuLabel="Date"
        set={dates.length > 0}
        items={dateItems}
      />
      <Toggle
        label="Has attachment"
        pressed={query.hasAttachment}
        onToggle={(on) =>
          setSearch(
            on
              ? setClause(search, isHasClause, "has:attachment")
              : removeClauses(search, isHasClause),
          )
        }
      />
      <Toggle
        label="Unread"
        pressed={query.read === false}
        onToggle={(on) =>
          setSearch(
            on ? setClause(search, isReadClause, "is:unread") : removeClauses(search, isReadClause),
          )
        }
      />
      <Toggle
        label="Flagged"
        pressed={query.flagged}
        onToggle={(on) =>
          setSearch(
            on
              ? setClause(search, isFlaggedClause, "is:flagged")
              : removeClauses(search, isFlaggedClause),
          )
        }
      />
      <MenuChip
        label={folderLabel}
        menuLabel="Folder"
        set={inValues.length > 0}
        items={folderItems}
      />
      {accounts.length > 1 && (
        <MenuChip
          label={accountChipLabel}
          menuLabel="Account"
          set={accountValues.length > 0}
          items={accountItems}
        />
      )}
      {hasFilters(search) && (
        <button
          type="button"
          className="btn btn--subtle btn--sm mail-search-filters__clear"
          onClick={() => setSearch(clearFilters(search))}
        >
          Clear filters
        </button>
      )}
      {rangeOpen && (
        <DateRangeDialog
          initial={resolveDates(query, new Date(), weekStartsOn())}
          onApply={(clause) => {
            setSearch(setClause(search, isDateClause, clause));
            setRangeOpen(false);
          }}
          onClose={() => setRangeOpen(false)}
        />
      )}
    </div>
  );
}
