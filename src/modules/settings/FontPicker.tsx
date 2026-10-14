import clsx from "clsx";
import { Check, ChevronDown } from "lucide-react";
import {
  type KeyboardEvent,
  type MouseEvent,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { scrollIdIntoView } from "../../components/listboxKeys";
import { fontKey, SANS_STACK, withFamily } from "../../lib/fonts";

export interface FontOption<V> {
  /** Unique among the options: `fontKey` of its value. */
  key: string;
  label: string;
  value: V;
  /** The `font-family` the label is shown in (escaped already). */
  fontFamily: string;
}

interface FontPickerProps<V> {
  /** The input's id, for the setting's <label>. */
  id: string;
  /** The setting's label, which also names the list. */
  labelledBy: string;
  describedBy?: string;
  /** The built-in choices, listed first. */
  presets: FontOption<V>[];
  /** Installed families; undefined while they load. */
  families: string[] | undefined;
  familiesFailed?: boolean;
  /** The value for an installed family. */
  familyValue: (family: string) => V;
  /** The key of the current choice. */
  selectedKey: string;
  /** What the input shows for the current choice. */
  selectedLabel: string;
  onChange: (value: V) => void;
  /** The highlighted option while the list is open (null when it closes), for a preview. */
  onPreview?: (value: V | null) => void;
}

/** Case- and accent-insensitive form for matching. */
function fold(text: string): string {
  return text.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();
}

/** Options whose label has every word of the query; names starting with it first. */
function filterOptions<V>(options: FontOption<V>[], query: string): FontOption<V>[] {
  const q = fold(query.trim());
  if (!q) return options;
  const words = q.split(/\s+/);
  const matches = options.filter((o) => {
    const label = fold(o.label);
    return words.every((w) => label.includes(w));
  });
  const first = matches.filter((o) => fold(o.label).startsWith(q));
  return [...first, ...matches.filter((o) => !first.includes(o))];
}

/** A rendered element's CSS zoom (the browser demo zooms with CSS; the desktop app doesn't). */
function cssZoom(el: Element): number {
  const zoom = (el as Element & { currentCSSZoom?: number }).currentCSSZoom;
  return typeof zoom === "number" && zoom > 0 ? zoom : 1;
}

const MARGIN = 8;
const GAP = 4;
const MAX_HEIGHT = 320;

/**
 * Picks a font: an input that filters a list of the built-in choices and the
 * installed families (the combobox pattern with list autocomplete). Each
 * family is shown in its own face. The list sits in the top layer where
 * popovers are supported, so the settings panel's scrolling can't clip it.
 */
export function FontPicker<V>({
  id,
  labelledBy,
  describedBy,
  presets,
  families,
  familiesFailed,
  familyValue,
  selectedKey,
  selectedLabel,
  onChange,
  onPreview,
}: FontPickerProps<V>) {
  const listId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  // What was typed since the list opened; null shows every option.
  const [query, setQuery] = useState<string | null>(null);
  const [active, setActive] = useState(0);

  const installed = useMemo(
    () =>
      (families ?? []).map((family): FontOption<V> => ({
        key: fontKey({ kind: "family", family }),
        label: family,
        value: familyValue(family),
        fontFamily: withFamily(family, SANS_STACK),
      })),
    [families, familyValue],
  );
  const shownPresets = useMemo(() => filterOptions(presets, query ?? ""), [presets, query]);
  const shownInstalled = useMemo(() => filterOptions(installed, query ?? ""), [installed, query]);
  const options = useMemo(
    () => [...shownPresets, ...shownInstalled],
    [shownPresets, shownInstalled],
  );
  const current = Math.min(active, options.length - 1);
  const activeOption = open && current >= 0 ? options[current] : undefined;
  const optionId = (i: number) => `${listId}-${i}`;

  const show = () => {
    if (open) return;
    const selected = options.findIndex((o) => o.key === selectedKey);
    setActive(Math.max(0, selected));
    setQuery(null);
    setOpen(true);
    // Typing replaces the current name instead of adding to it.
    inputRef.current?.select();
  };

  const close = () => {
    setOpen(false);
    setQuery(null);
  };

  const choose = (option: FontOption<V>) => {
    close();
    if (option.key !== selectedKey) onChange(option.value);
  };

  const move = (index: number) => {
    setActive(index);
    scrollIdIntoView(optionId(index));
  };

  // Tell the preview what's highlighted.
  const previewKey = activeOption?.key ?? null;
  const previewRef = useRef({ onPreview, activeOption });
  useEffect(() => {
    previewRef.current = { onPreview, activeOption };
  });
  useEffect(() => {
    const { onPreview: notify, activeOption: option } = previewRef.current;
    notify?.(previewKey === null || !option ? null : option.value);
  }, [previewKey]);

  // The current choice in view when the list opens.
  useEffect(() => {
    if (open) scrollIdIntoView(optionId(current));
    // Only on opening; arrow keys scroll as they move.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Place the list under the input (or above it, when there's more room).
  const place = useCallback(() => {
    const popup = popupRef.current;
    const input = inputRef.current;
    if (!popup || !input) return;
    const r = input.getBoundingClientRect();
    const zoom = cssZoom(popup);
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const width = Math.min(Math.max(r.width, 280 * zoom), vw - 2 * MARGIN);
    const left = Math.min(Math.max(r.left, MARGIN), vw - MARGIN - width);
    const below = vh - r.bottom - GAP - MARGIN;
    const above = r.top - GAP - MARGIN;
    const downward = below >= Math.min(MAX_HEIGHT, 200) * zoom || below >= above;
    const room = downward ? below : above;
    const s = popup.style;
    s.left = `${left / zoom}px`;
    s.width = `${width / zoom}px`;
    s.maxHeight = `${Math.min(MAX_HEIGHT, room / zoom)}px`;
    s.top = downward ? `${(r.bottom + GAP) / zoom}px` : "auto";
    s.bottom = downward ? "auto" : `${(vh - r.top + GAP) / zoom}px`;
    popup.dataset.side = downward ? "bottom" : "top";
  }, []);

  // In the top layer where popovers are supported, so the dialog's
  // scrolling panel can't clip it; it follows the input as that scrolls.
  useLayoutEffect(() => {
    const popup = popupRef.current;
    if (!open || !popup) return;
    const inTopLayer = typeof popup.showPopover === "function";
    if (inTopLayer) {
      try {
        popup.popover = "manual";
        popup.showPopover();
      } catch {
        // Not in the top layer; it still opens in place.
      }
    }
    window.addEventListener("resize", place);
    document.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      document.removeEventListener("scroll", place, true);
      if (inTopLayer && popup.matches(":popover-open")) popup.hidePopover();
    };
  }, [open, place]);

  // And again after every render, in case the page moved the input.
  useLayoutEffect(() => {
    if (open) place();
  });

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const last = options.length - 1;
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp":
        e.preventDefault();
        if (!open) show();
        else if (last >= 0) {
          const step = e.key === "ArrowDown" ? 1 : -1;
          move(Math.min(Math.max(current + step, 0), last));
        }
        break;
      case "PageDown":
      case "PageUp":
        if (!open || last < 0) return;
        e.preventDefault();
        move(Math.min(Math.max(current + (e.key === "PageDown" ? 10 : -10), 0), last));
        break;
      case "Enter":
        if (!open) return;
        e.preventDefault();
        if (activeOption) choose(activeOption);
        break;
      case "Escape":
        if (!open) return;
        // Closes the list, not the dialog around it.
        e.preventDefault();
        e.stopPropagation();
        close();
        break;
      case "Tab":
        if (open) close();
        break;
    }
  };

  const typed = query !== null && query.trim() !== "";
  let note = "";
  if (open && !families && !familiesFailed) note = "Looking for installed fonts…";
  else if (open && familiesFailed) note = "Couldn't list the installed fonts.";
  else if (open && typed && options.length === 0) note = `No font matches “${query.trim()}”.`;

  const optionProps = (option: FontOption<V>, index: number) => ({
    id: optionId(index),
    role: "option" as const,
    "aria-selected": index === current,
    "data-current": option.key === selectedKey || undefined,
    className: "font-picker__option",
    style: { fontFamily: option.fontFamily },
    onMouseMove: () => index !== current && setActive(index),
    onClick: () => choose(option),
  });

  return (
    <div className="font-picker">
      <input
        ref={inputRef}
        id={id}
        className="input font-picker__input"
        role="combobox"
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={activeOption ? optionId(current) : undefined}
        aria-describedby={describedBy}
        autoComplete="off"
        spellCheck={false}
        value={open && query !== null ? query : selectedLabel}
        onChange={(e) => {
          if (!open) setOpen(true);
          setQuery(e.target.value);
          setActive(0);
        }}
        onClick={show}
        onKeyDown={onKeyDown}
        onBlur={close}
      />
      <ChevronDown size={16} className="font-picker__chevron" aria-hidden="true" />
      {open && (
        <div
          ref={popupRef}
          className="font-picker__popup"
          // Keeps focus in the input, so a click on an option isn't a blur first.
          onMouseDown={(e: MouseEvent) => e.preventDefault()}
        >
          <div
            id={listId}
            role="listbox"
            aria-labelledby={labelledBy}
            className="font-picker__list"
          >
            {shownPresets.map((option, i) => (
              <div key={option.key} {...optionProps(option, i)}>
                <span className="font-picker__name">{option.label}</span>
                <Check size={16} className="font-picker__check" aria-hidden="true" />
              </div>
            ))}
            {shownInstalled.length > 0 && (
              <div role="group" aria-labelledby={`${listId}-installed`}>
                <div
                  id={`${listId}-installed`}
                  role="presentation"
                  className={clsx(
                    "font-picker__group",
                    shownPresets.length > 0 && "font-picker__group--rule",
                  )}
                >
                  Installed fonts
                </div>
                {shownInstalled.map((option, i) => (
                  <div key={option.key} {...optionProps(option, shownPresets.length + i)}>
                    <span className="font-picker__name">{option.label}</span>
                    <Check size={16} className="font-picker__check" aria-hidden="true" />
                  </div>
                ))}
              </div>
            )}
          </div>
          <p className="font-picker__note" role="status">
            {note}
          </p>
        </div>
      )}
    </div>
  );
}
