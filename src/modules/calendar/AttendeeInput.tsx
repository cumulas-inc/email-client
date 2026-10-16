import { X } from "lucide-react";
import { type KeyboardEvent, useId, useRef, useState } from "react";

import type { EmailAddress } from "../../api/types";
import { Avatar } from "../../components/Avatar";
import { displayName } from "../../lib/format";
import { useDebounced } from "./hooks";
import { parseAddress } from "./model";
import { useAddressSuggestions } from "./queries";

interface AttendeeInputProps {
  id?: string;
  value: EmailAddress[];
  onChange: (value: EmailAddress[]) => void;
  placeholder?: string;
}

function same(a: EmailAddress, b: EmailAddress): boolean {
  return a.address.toLowerCase() === b.address.toLowerCase();
}

/** Recipient chips with autocomplete from contacts and recent senders. */
export function AttendeeInput({ id, value, onChange, placeholder }: AttendeeInputProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();
  const [text, setText] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const query = useDebounced(text, 150);
  const { data: suggestions = [] } = useAddressSuggestions(query);
  const options = text.trim() ? suggestions.filter((s) => !value.some((v) => same(v, s))) : [];
  const showList = open && options.length > 0;
  const activeIndex = Math.min(active, options.length - 1);

  const add = (address: EmailAddress) => {
    if (!value.some((v) => same(v, address))) onChange([...value, address]);
    setText("");
    setActive(0);
    setError(null);
  };

  /** Adds the typed text as an address; returns whether it was valid. */
  const commitText = (): boolean => {
    if (!text.trim()) return false;
    const parsed = parseAddress(text);
    if (!parsed) {
      setError(`“${text.trim()}” isn't a valid email address.`);
      return false;
    }
    add(parsed);
    return true;
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    switch (e.key) {
      case "ArrowDown":
        if (options.length === 0) return;
        e.preventDefault();
        setOpen(true);
        setActive((activeIndex + 1) % options.length);
        break;
      case "ArrowUp":
        if (options.length === 0) return;
        e.preventDefault();
        setOpen(true);
        setActive((activeIndex - 1 + options.length) % options.length);
        break;
      case "Enter": {
        if (showList) {
          e.preventDefault();
          add(options[activeIndex]!);
        } else if (text.trim()) {
          e.preventDefault();
          commitText();
        }
        break;
      }
      case ",":
      case ";":
        if (text.trim()) {
          e.preventDefault();
          commitText();
        }
        break;
      case "Backspace":
        if (!text && value.length > 0) onChange(value.slice(0, -1));
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

  return (
    <div className="cal-attendees-field">
      <div className="cal-attendees" onClick={() => inputRef.current?.focus()}>
        <ul className="cal-attendees__chips" aria-label="Attendees">
          {value.map((a) => (
            <li key={a.address} className="cal-chip" title={a.address}>
              <span className="cal-chip__name" dir="auto">
                {displayName(a)}
              </span>
              <button
                type="button"
                className="cal-chip__remove"
                aria-label={`Remove ${displayName(a)}`}
                onClick={(e) => {
                  e.stopPropagation();
                  onChange(value.filter((v) => !same(v, a)));
                  inputRef.current?.focus();
                }}
              >
                <X size={12} />
              </button>
            </li>
          ))}
        </ul>
        <input
          ref={inputRef}
          id={id}
          className="cal-attendees__input"
          type="text"
          role="combobox"
          autoComplete="off"
          aria-autocomplete="list"
          aria-expanded={showList}
          aria-controls={listId}
          aria-activedescendant={showList ? `${listId}-${activeIndex}` : undefined}
          aria-invalid={error ? true : undefined}
          placeholder={value.length === 0 ? placeholder : undefined}
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setOpen(true);
            setActive(0);
            setError(null);
          }}
          onKeyDown={onKeyDown}
          onFocus={() => setOpen(true)}
          onBlur={() => {
            setOpen(false);
            if (text.trim() && parseAddress(text)) commitText();
          }}
        />
      </div>
      {showList && (
        <ul id={listId} role="listbox" className="cal-suggest" aria-label="Suggestions">
          {options.map((o, i) => (
            <li
              key={o.address}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === activeIndex}
              className="cal-suggest__option"
              onMouseDown={(e) => {
                e.preventDefault();
                add(o);
              }}
              onMouseEnter={() => setActive(i)}
            >
              <Avatar name={displayName(o)} colorKey={o.address} size={28} />
              <span className="cal-suggest__text">
                <span className="cal-suggest__name" dir="auto">
                  {displayName(o)}
                </span>
                {o.name && <span className="cal-suggest__address">{o.address}</span>}
              </span>
            </li>
          ))}
        </ul>
      )}
      {error && (
        <p className="field__error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
