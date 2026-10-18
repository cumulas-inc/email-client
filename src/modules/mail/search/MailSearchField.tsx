import clsx from "clsx";
import { AtSign, CalendarDays, Folder, History, ListFilter, Search, Trash2, X } from "lucide-react";
import {
  type KeyboardEvent,
  type SyntheticEvent,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { useAppStore } from "../../../app/store";
import { useAccounts } from "../../../api/queries";
import { Avatar } from "../../../components/Avatar";
import { useDebounced } from "../hooks";
import { useFolders, useSuggestions } from "../queries";
import { SEARCH_INPUT_ID } from "./focus";
import {
  addRecentSearch,
  clearRecentSearches,
  removeRecentSearch,
  useRecentSearches,
} from "./recentSearches";
import { type Suggestion, buildSuggestions, peopleQuery } from "./suggestions";

const LISTBOX_ID = "mail-search-suggestions";
const optionId = (index: number) => `mail-search-option-${index}`;

function OptionIcon({ option }: { option: Suggestion }) {
  const props = { size: 16, "aria-hidden": true as const };
  switch (option.kind) {
    case "person":
      return <Avatar name={option.label} colorKey={option.detail ?? option.label} size={24} />;
    case "recent":
      return <History {...props} />;
    case "clearRecent":
      return <Trash2 {...props} />;
    case "folder":
      return <Folder {...props} />;
    case "account":
      return <AtSign {...props} />;
    case "date":
    case "hint":
      return <CalendarDays {...props} />;
    default:
      return <ListFilter {...props} />;
  }
}

/**
 * Mail's search box. As you type it suggests recent searches, people,
 * filters, folders, accounts and dates for the word at the caret; a pick
 * edits the text, as typing would. Ctrl+K, Ctrl+E or Ctrl+Shift+F jumps
 * here; Escape closes the suggestions, then clears the box.
 */
export function MailSearchField() {
  const search = useAppStore((s) => s.search);
  const setSearch = useAppStore((s) => s.setSearch);
  const inputRef = useRef<HTMLInputElement>(null);
  const [focused, setFocused] = useState(false);
  // Closed by Escape, Enter, Tab or a pick, until the next key, input or focus.
  const [dismissed, setDismissed] = useState(false);
  const [caret, setCaret] = useState(0);
  // The option the arrows reached, for the options it was among: none at first.
  const [active, setActive] = useState<{ key: string; index: number } | null>(null);
  const [status, setStatus] = useState("");
  // Where the caret goes once a pick's text is in the box.
  const pendingCaret = useRef<number | null>(null);

  const recent = useRecentSearches();
  const { data: folders = [] } = useFolders();
  const { data: accounts = [] } = useAccounts();
  const open = focused && !dismissed;
  const at = Math.min(caret, search.length);
  const peopleText = peopleQuery(search, at);
  const askPeople = useDebounced(open ? peopleText : null, 150);
  const { data: people = [] } = useSuggestions(askPeople ?? "", { allowEmpty: askPeople !== null });

  const groups = useMemo(
    () =>
      open
        ? buildSuggestions({
            text: search,
            caret: at,
            recent,
            people: peopleText === null ? [] : people,
            folders,
            accounts,
          })
        : [],
    [open, search, at, recent, peopleText, people, folders, accounts],
  );
  const options = useMemo(() => groups.flatMap((g) => g.options), [groups]);
  const optionsKey = options.map((o) => o.id).join("\n");
  const activeIndex = active?.key === optionsKey ? active.index : null;
  const expanded = open && options.length > 0;

  useLayoutEffect(() => {
    const input = inputRef.current;
    if (pendingCaret.current === null || !input) return;
    input.setSelectionRange(pendingCaret.current, pendingCaret.current);
    pendingCaret.current = null;
  });

  const readCaret = (e: SyntheticEvent<HTMLInputElement>) =>
    setCaret(e.currentTarget.selectionStart ?? e.currentTarget.value.length);

  const pick = (option: Suggestion) => {
    if (option.disabled) return;
    const result = option.apply(search, at);
    setActive(null);
    setStatus("");
    if (result === "clearRecent") {
      clearRecentSearches();
      setDismissed(true);
      return;
    }
    if (option.kind === "recent") addRecentSearch(result.text);
    pendingCaret.current = result.caret;
    setCaret(result.caret);
    setSearch(result.text);
    // An operator waiting for its value keeps the suggestions for it.
    setDismissed(!result.awaitsValue);
  };

  /** The next option the arrows reach from `from`, skipping what can't be picked. */
  const step = (from: number | null, by: 1 | -1): number | null => {
    const count = options.length;
    let index = from ?? (by === 1 ? -1 : count);
    for (let tries = 0; tries < count; tries++) {
      index = (index + by + count) % count;
      if (!options[index]!.disabled) return index;
    }
    return null;
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp": {
        if (e.ctrlKey || e.metaKey) return;
        e.preventDefault();
        if (!expanded || e.altKey) {
          setDismissed(false);
          setActive(null);
          return;
        }
        const next = step(activeIndex, e.key === "ArrowDown" ? 1 : -1);
        setActive(next === null ? null : { key: optionsKey, index: next });
        break;
      }
      case "Enter":
        if (expanded && activeIndex !== null) {
          e.preventDefault();
          pick(options[activeIndex]!);
        } else {
          setDismissed(true);
          setActive(null);
          addRecentSearch(search);
        }
        break;
      case "Escape":
        if (expanded) {
          e.preventDefault();
          e.stopPropagation();
          setDismissed(true);
          setActive(null);
        } else {
          setSearch("");
          inputRef.current?.blur();
        }
        break;
      case "Tab":
        setDismissed(true);
        setActive(null);
        break;
      case "Delete": {
        const option = activeIndex === null ? null : options[activeIndex];
        if (!e.shiftKey || !expanded || option?.kind !== "recent") return;
        e.preventDefault();
        removeRecentSearch(option.label);
        setActive(null);
        setStatus(`Removed “${option.label}” from recent searches.`);
        break;
      }
    }
  };

  return (
    <div className="search-field mail-search-field" role="search">
      <Search size={16} aria-hidden="true" className="search-field__icon" />
      <input
        ref={inputRef}
        id={SEARCH_INPUT_ID}
        type="search"
        dir="auto"
        placeholder="Search mail"
        aria-label="Search mail"
        role="combobox"
        aria-expanded={expanded}
        aria-controls={LISTBOX_ID}
        aria-autocomplete="list"
        aria-activedescendant={expanded && activeIndex !== null ? optionId(activeIndex) : undefined}
        autoComplete="off"
        spellCheck={false}
        value={search}
        onChange={(e) => {
          setSearch(e.target.value);
          readCaret(e);
          setDismissed(false);
          setActive(null);
          setStatus("");
        }}
        onSelect={readCaret}
        onKeyDown={onKeyDown}
        onFocus={(e) => {
          setFocused(true);
          setDismissed(false);
          readCaret(e);
        }}
        onBlur={() => {
          setFocused(false);
          setActive(null);
        }}
      />
      {search ? (
        <button
          type="button"
          className="icon-btn icon-btn--sm"
          aria-label="Clear"
          title="Clear (Esc)"
          onClick={() => {
            setSearch("");
            inputRef.current?.focus();
          }}
        >
          <X size={14} />
        </button>
      ) : (
        <span className="kbd" aria-hidden="true">
          Ctrl K
        </span>
      )}
      {expanded && (
        <ul
          id={LISTBOX_ID}
          role="listbox"
          aria-label="Search suggestions"
          className="mail-suggest mail-search-suggest"
        >
          {groups.map((group) => {
            const headingId = `mail-search-group-${group.id}`;
            return (
              <li key={group.id} role="presentation" className="mail-search-suggest__group">
                <div id={headingId} className="mail-suggest__heading" aria-hidden="true">
                  {group.heading}
                </div>
                <ul role="group" aria-labelledby={headingId} className="mail-search-suggest__list">
                  {group.options.map((option) => {
                    const index = options.indexOf(option);
                    return (
                      <li
                        key={option.id}
                        id={optionId(index)}
                        role="option"
                        aria-selected={index === activeIndex}
                        aria-disabled={option.disabled || undefined}
                        aria-label={option.ariaLabel}
                        className={clsx(
                          "mail-suggest__item",
                          option.disabled && "is-disabled",
                          option.kind === "clearRecent" && "mail-search-suggest__clear",
                        )}
                        // The keyboard stays in the box.
                        onMouseDown={(e) => e.preventDefault()}
                        onMouseEnter={() =>
                          !option.disabled && setActive({ key: optionsKey, index })
                        }
                        onClick={() => pick(option)}
                      >
                        <span className="mail-search-suggest__icon">
                          <OptionIcon option={option} />
                        </span>
                        <span className="mail-suggest__text">
                          <span className="mail-suggest__name" dir="auto">
                            {option.label}
                          </span>
                          {option.detail && (
                            <span className="mail-suggest__address">{option.detail}</span>
                          )}
                        </span>
                      </li>
                    );
                  })}
                </ul>
              </li>
            );
          })}
        </ul>
      )}
      <p className="visually-hidden" role="status">
        {status}
      </p>
    </div>
  );
}
