import { X } from "lucide-react";
import {
  type ClipboardEvent,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
  useId,
  useImperativeHandle,
  useRef,
  useState,
} from "react";

import type { EmailAddress } from "../../api/types";
import { Avatar } from "../../components/Avatar";
import { displayName } from "../../lib/format";
import { parseAddressList, sameAddress, uniqueAddresses } from "./draft";
import { useDebounced } from "./hooks";
import { useSuggestions } from "./queries";

export interface RecipientFieldHandle {
  /** Turns typed text into chips. Returns the full list, or null if the text is invalid. */
  commit: () => EmailAddress[] | null;
  focus: () => void;
}

interface RecipientFieldProps {
  label: string;
  value: EmailAddress[];
  onChange: (value: EmailAddress[]) => void;
  /** An error from the form (e.g. "Add at least one recipient"). */
  error?: string | null;
  autoFocus?: boolean;
  trailing?: ReactNode;
  ref?: Ref<RecipientFieldHandle>;
}

export function RecipientField({
  label,
  value,
  onChange,
  error: formError,
  autoFocus,
  trailing,
  ref,
}: RecipientFieldProps) {
  const inputId = useId();
  const listId = useId();
  const errorId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const [text, setText] = useState("");
  const [parseError, setParseError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [highlight, setHighlight] = useState(0);

  const query = useDebounced(text.trim(), 150);
  const { data: suggestions = [] } = useSuggestions(open ? query : "");
  const options = suggestions.filter((s) => !value.some((v) => sameAddress(v, s))).slice(0, 8);
  const showList = open && text.trim().length > 0 && query.length > 0 && options.length > 0;
  const active = Math.min(highlight, Math.max(0, options.length - 1));
  const error = parseError ?? formError ?? null;

  /** Adds the typed (or pasted) addresses; keeps invalid ones in the input. */
  const commitText = (raw: string): EmailAddress[] | null => {
    const { valid, invalid } = parseAddressList(raw);
    const next = valid.length > 0 ? uniqueAddresses([...value, ...valid]) : value;
    if (valid.length > 0) onChange(next);
    if (invalid.length > 0) {
      setText(invalid.join("; "));
      setParseError(
        invalid.length === 1
          ? `“${invalid[0]}” isn't a valid email address.`
          : `${invalid.length} entries aren't valid email addresses.`,
      );
      return null;
    }
    setText("");
    setParseError(null);
    return next;
  };

  useImperativeHandle(ref, () => ({
    commit: () => (text.trim() ? commitText(text) : value),
    focus: () => inputRef.current?.focus(),
  }));

  const pick = (address: EmailAddress) => {
    onChange(uniqueAddresses([...value, address]));
    setText("");
    setParseError(null);
    setOpen(false);
    setHighlight(0);
    inputRef.current?.focus();
  };

  const remove = (index: number) => {
    onChange(value.filter((_, i) => i !== index));
    inputRef.current?.focus();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    switch (e.key) {
      case "ArrowDown":
        if (!showList) return;
        e.preventDefault();
        setHighlight((active + 1) % options.length);
        break;
      case "ArrowUp":
        if (!showList) return;
        e.preventDefault();
        setHighlight((active - 1 + options.length) % options.length);
        break;
      case "Enter":
        if (showList) {
          e.preventDefault();
          pick(options[active]!);
        } else if (text.trim()) {
          e.preventDefault();
          commitText(text);
        }
        break;
      case "Tab":
        if (showList && !e.shiftKey) {
          e.preventDefault();
          pick(options[active]!);
        } else if (text.trim()) {
          commitText(text);
        }
        break;
      case ",":
      case ";":
        if (text.trim()) {
          e.preventDefault();
          commitText(text);
        }
        break;
      case "Backspace":
        if (!text && value.length > 0) {
          e.preventDefault();
          onChange(value.slice(0, -1));
        }
        break;
      case "Escape":
        if (showList) {
          e.preventDefault();
          e.stopPropagation();
          setOpen(false);
        }
        break;
    }
  };

  const onPaste = (e: ClipboardEvent<HTMLInputElement>) => {
    const pasted = e.clipboardData.getData("text");
    if (!/[,;\n]/.test(pasted) && !pasted.includes("@")) return;
    e.preventDefault();
    commitText(`${text}${pasted}`);
    setOpen(false);
  };

  return (
    <div className={`mail-recipients${error ? " has-error" : ""}`}>
      <div className="mail-compose__field" onClick={() => inputRef.current?.focus()}>
        <label htmlFor={inputId} className="mail-compose__label">
          {label}
        </label>
        <div className="mail-recipients__box">
          {value.map((a, i) => (
            <span key={`${a.address}-${i}`} className="mail-chip" title={a.address}>
              <span className="mail-chip__text">{displayName(a)}</span>
              <button
                type="button"
                className="mail-chip__remove"
                aria-label={`Remove ${displayName(a)}`}
                tabIndex={-1}
                onClick={(e) => {
                  e.stopPropagation();
                  remove(i);
                }}
              >
                <X size={12} />
              </button>
            </span>
          ))}
          <input
            ref={inputRef}
            id={inputId}
            className="mail-recipients__input"
            role="combobox"
            aria-expanded={showList}
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={showList ? `${listId}-${active}` : undefined}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? errorId : undefined}
            autoComplete="off"
            spellCheck={false}
            autoFocus={autoFocus}
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setParseError(null);
              setOpen(true);
              setHighlight(0);
            }}
            onKeyDown={onKeyDown}
            onPaste={onPaste}
            onFocus={() => setOpen(true)}
            onBlur={() => {
              setOpen(false);
              if (text.trim()) commitText(text);
            }}
          />
        </div>
        {trailing}
      </div>
      {showList && (
        <ul id={listId} role="listbox" aria-label="Suggested contacts" className="mail-suggest">
          {options.map((s, i) => (
            <li
              key={s.address}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === active}
              className="mail-suggest__item"
              // Keep focus in the input so blur doesn't commit half-typed text.
              onMouseDown={(e) => e.preventDefault()}
              onMouseEnter={() => setHighlight(i)}
              onClick={() => pick(s)}
            >
              <Avatar name={displayName(s)} colorKey={s.address} size={28} />
              <span className="mail-suggest__text">
                <span className="mail-suggest__name">{s.name ?? s.address}</span>
                {s.name && <span className="mail-suggest__address">{s.address}</span>}
              </span>
            </li>
          ))}
        </ul>
      )}
      {error && (
        <p id={errorId} className="mail-compose__error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
