import { Search, X } from "lucide-react";
import { useRef } from "react";

import { useAppStore } from "../app/store";

/** The search box at the top of a list. Ctrl+K or Ctrl+E jumps here;
    Escape clears it. */
export function SearchField({ placeholder }: { placeholder: string }) {
  const search = useAppStore((s) => s.search);
  const setSearch = useAppStore((s) => s.setSearch);
  const inputRef = useRef<HTMLInputElement>(null);
  return (
    <div className="search-field" role="search">
      <Search size={16} aria-hidden="true" className="search-field__icon" />
      <input
        ref={inputRef}
        id="global-search"
        type="search"
        dir="auto"
        placeholder={placeholder}
        aria-label={placeholder}
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            setSearch("");
            inputRef.current?.blur();
          }
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
    </div>
  );
}
