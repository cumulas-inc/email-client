import clsx from "clsx";
import { Check, ChevronDown } from "lucide-react";
import {
  type KeyboardEvent,
  memo,
  type MouseEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { flushSync } from "react-dom";

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

/** The nearest ancestor that scrolls `el`, if any. */
function scrollParent(el: Element): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const { overflowY } = getComputedStyle(p);
    const scrolls = overflowY === "auto" || overflowY === "scroll";
    if (scrolls && p.scrollHeight > p.clientHeight) return p;
  }
  return null;
}

const MARGIN = 8;
const GAP = 4;
const MAX_HEIGHT = 320;
/** An option's height (`.font-picker__option`). */
const ROW = 32;
/** Room for four options and the list's padding, which the list wants below the input. */
const MIN_BELOW = 4 * ROW + 8;
/**
 * Installed fonts rendered past each end of the list's visible part: enough
 * that a fast fling in WebKitGTK lands on rows that are already painted.
 */
const OVERSCAN = 100;
/**
 * Installed fonts past each end of the view shown in their own face. While
 * the list scrolls, rows newly in view use the interface font: resolving a
 * face per row as it mounts is what left a fast scroll blank.
 */
const FACE_MARGIN = 8;
/** How long the list is still before the rows in view take their faces. */
const SETTLE_MS = 100;
/**
 * Rows given their faces per frame, so a scroll that starts again isn't held
 * up behind a batch of them.
 */
const FACE_STEP = 2;
/** Installed fonts rendered, and shown in their faces, before the list is measured. */
const FIRST_RANGE: [number, number] = [0, MAX_HEIGHT / ROW + OVERSCAN];
const FIRST_FACED: [number, number] = [0, MAX_HEIGHT / ROW + FACE_MARGIN];

interface OptionRowProps<V> {
  id: string;
  /** Its place among the options shown. */
  index: number;
  option: FontOption<V>;
  /** Highlighted. */
  active: boolean;
  /** The current choice. */
  current: boolean;
  /** Shown in its own face (not while the list scrolls past it). */
  face?: boolean;
  /** Its place among the installed fonts shown, which aren't all rendered. */
  setSize?: number;
  posInSet?: number;
  onHover: (index: number) => void;
  onPick: (option: FontOption<V>) => void;
}

function OptionRowBase<V>({
  id,
  index,
  option,
  active,
  current,
  face = true,
  setSize,
  posInSet,
  onHover,
  onPick,
}: OptionRowProps<V>) {
  return (
    <div
      id={id}
      role="option"
      aria-selected={active}
      aria-setsize={setSize}
      aria-posinset={posInSet}
      data-current={current || undefined}
      className="font-picker__option"
      style={face ? { fontFamily: option.fontFamily } : undefined}
      onMouseMove={(e) => {
        // WebKit also sends one, without movement, when the list (or the
        // panel, making room for it) scrolls under a pointer that's still.
        const moved = e.movementX !== 0 || e.movementY !== 0;
        if (!active && moved) onHover(index);
      }}
      onClick={() => onPick(option)}
    >
      <span className="font-picker__name">{option.label}</span>
      <Check size={16} className="font-picker__check" aria-hidden="true" />
    </div>
  );
}

/** Memoized, so moving the highlight re-renders two rows rather than all of them. */
const OptionRow = memo(OptionRowBase) as typeof OptionRowBase;

/**
 * Picks a font: an input that filters a list of the built-in choices and the
 * installed families (the combobox pattern with list autocomplete). Each
 * family is shown in its own face. The list sits in the top layer where
 * popovers are supported, so the settings panel's scrolling can't clip it.
 * Only the installed fonts in view are rendered, as there can be thousands.
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
  const headingRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  // What was typed since the list opened; null shows every option.
  const [query, setQuery] = useState<string | null>(null);
  // The highlighted option's index, or null for the current choice (which
  // may only be listed once the installed fonts arrive).
  const [active, setActive] = useState<number | null>(null);
  // The installed fonts rendered: those in view, and many either side; and
  // those shown in their own faces: in view when the list was last still.
  const [range, setRange] = useState(FIRST_RANGE);
  const [faced, setFaced] = useState(FIRST_FACED);
  const settleTimer = useRef<number | undefined>(undefined);
  const faceFrame = useRef(0);
  const facedRef = useRef(faced);
  useEffect(() => {
    facedRef.current = faced;
  }, [faced]);

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
  const selected = useMemo(
    () => options.findIndex((o) => o.key === selectedKey),
    [options, selectedKey],
  );
  const current = active === null ? selected : Math.min(active, options.length - 1);
  const activeOption = open && current >= 0 ? options[current] : undefined;

  // An option's id follows the option, not its place in the filtered list,
  // so aria-activedescendant changes whenever the highlight does.
  const ids = useMemo(
    () =>
      new Map([
        ...presets.map((o, i): [string, string] => [o.key, `${listId}-p${i}`]),
        ...installed.map((o, i): [string, string] => [o.key, `${listId}-f${i}`]),
      ]),
    [listId, presets, installed],
  );
  const optionId = (option: FontOption<V>) => ids.get(option.key)!;
  const activeId = activeOption && optionId(activeOption);

  const show = () => {
    if (open) return;
    setActive(null);
    setQuery(null);
    setRange(FIRST_RANGE);
    setFaced(FIRST_FACED);
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

  // Set when the pointer moves the highlight, which is in view already.
  const hovered = useRef(false);

  const move = (index: number) => {
    hovered.current = false;
    setActive(index);
  };

  // Stable, so the rows don't re-render for new callbacks.
  const chooseRef = useRef(choose);
  useEffect(() => {
    chooseRef.current = choose;
  });
  const onHover = useCallback((index: number) => {
    hovered.current = true;
    setActive(index);
  }, []);
  const onPick = useCallback((option: FontOption<V>) => chooseRef.current(option), []);

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

  // Which side of the input the list is on. It's picked as the list opens
  // (and on resize), not as the panel scrolls, so the list doesn't jump
  // from one side to the other under the pointer.
  const downward = useRef(true);

  // Place the list under the input (or above it, when there's no room below).
  const place = useCallback((pickSide = false) => {
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
    // Below, it leaves the preview above the font rows in view. (A pixel's
    // leeway, as the panel scrolls by whole pixels to make that room.)
    if (pickSide) downward.current = below + 1 >= MIN_BELOW * zoom || below >= above;
    const room = Math.max(0, downward.current ? below : above);
    const s = popup.style;
    s.left = `${left / zoom}px`;
    s.width = `${width / zoom}px`;
    s.maxHeight = `${Math.min(MAX_HEIGHT, room / zoom)}px`;
    s.top = downward.current ? `${(r.bottom + GAP) / zoom}px` : "auto";
    s.bottom = downward.current ? "auto" : `${(vh - r.top + GAP) / zoom}px`;
    popup.dataset.side = downward.current ? "bottom" : "top";
  }, []);

  // The installed fonts in view, give or take `margin` rows either side.
  // Measured in layout pixels, which the list's opening scale (and CSS zoom)
  // leave alone. The heading's offsetParent is the list, as that's fixed.
  // Null when nothing is laid out (as in tests).
  const inView = useCallback((margin: number): [number, number] | null => {
    const popup = popupRef.current;
    const heading = headingRef.current;
    if (!popup || !heading || !popup.clientHeight) return null;
    const top = (popup.scrollTop - heading.offsetTop - heading.offsetHeight) / ROW;
    return [
      Math.max(0, Math.floor(top) - margin),
      Math.ceil(top + popup.clientHeight / ROW) + margin,
    ];
  }, []);

  // Which installed fonts to render.
  const measure = useCallback(() => {
    if (!popupRef.current || !headingRef.current) return;
    const [first, last] = inView(OVERSCAN) ?? [0, Infinity];
    setRange((r) => (r[0] === first && r[1] === last ? r : [first, last]));
  }, [inView]);

  // Which to show in their faces, once the list has been still for a moment.
  const settle = useCallback(() => {
    if (!popupRef.current || !headingRef.current) return;
    const [first, last] = inView(FACE_MARGIN) ?? [0, Infinity];
    setFaced((r) => (r[0] === first && r[1] === last ? r : [first, last]));
  }, [inView]);
  // The same once a scroll stops, a few rows a frame: those in view first,
  // then the margin either side. Faces already in place stay.
  const settleGradually = useCallback(() => {
    const target = inView(FACE_MARGIN);
    const view = inView(0);
    if (!target || !view || typeof requestAnimationFrame !== "function") return settle();
    let [lo, hi] = facedRef.current;
    if (hi <= target[0] || lo >= target[1]) [lo, hi] = [view[0], view[0]];
    else [lo, hi] = [Math.max(lo, target[0]), Math.min(hi, target[1])];
    const step = () => {
      if (hi < view[1]) hi = Math.min(hi + FACE_STEP, view[1]);
      else if (lo > view[0]) lo = Math.max(lo - FACE_STEP, view[0]);
      else [lo, hi] = [Math.max(lo - FACE_STEP, target[0]), Math.min(hi + FACE_STEP, target[1])];
      const next: [number, number] = [lo, hi];
      facedRef.current = next;
      setFaced(next);
      if (lo > target[0] || hi < target[1]) faceFrame.current = requestAnimationFrame(step);
    };
    step();
  }, [inView, settle]);
  const stopSettling = () => {
    window.clearTimeout(settleTimer.current);
    if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(faceFrame.current);
  };
  useEffect(() => {
    if (!open) return;
    settle();
    return stopSettling;
  }, [open, settle]);

  // In the top layer where popovers are supported, so the dialog's
  // scrolling panel can't clip it; it follows the input as that scrolls.
  useLayoutEffect(() => {
    const popup = popupRef.current;
    const input = inputRef.current;
    if (!open || !popup || !input) return;
    const inTopLayer = typeof popup.showPopover === "function";
    if (inTopLayer) {
      try {
        popup.popover = "manual";
        popup.showPopover();
      } catch {
        // Not in the top layer; it still opens in place.
      }
    }
    const panel = scrollParent(input);
    if (panel) {
      const zoom = cssZoom(panel);
      const box = panel.getBoundingClientRect();
      // The input in view first: a key pressed in it opens the list again
      // after the panel has scrolled it out (and closed the list). Clear of
      // the panel's padding, where the panel fades its content out.
      const { paddingTop, paddingBottom } = getComputedStyle(panel);
      const top = box.top + (parseFloat(paddingTop) || 0) * zoom;
      const bottom = box.bottom - (parseFloat(paddingBottom) || 0) * zoom;
      let r = input.getBoundingClientRect();
      const out = r.top < top ? r.top - top : Math.max(0, r.bottom - bottom);
      if (out) {
        panel.scrollTop += out / zoom;
        r = input.getBoundingClientRect();
      }
      // Then, short of room below it, scroll the panel to make some, as far
      // as the input stays in view, so the list needn't open over the preview.
      const short = MIN_BELOW * zoom - (window.innerHeight - r.bottom - GAP - MARGIN);
      const by = Math.min(short, r.top - box.top);
      if (by > 0) panel.scrollTop += by / zoom;
    }
    place(true);

    const onResize = () => place(true);
    const onScroll = (e: Event) => {
      // The list's own scrolling is measure()'s.
      if (e.target instanceof Node && popup.contains(e.target)) return;
      // Closes once the panel scrolls the input (half of it) out of view.
      if (e.target instanceof Element && e.target.contains(input)) {
        const box = e.target.getBoundingClientRect();
        const r = input.getBoundingClientRect();
        const middle = (r.top + r.bottom) / 2;
        if (middle < box.top || middle > box.bottom) {
          setOpen(false);
          setQuery(null);
          return;
        }
      }
      place();
    };
    window.addEventListener("resize", onResize);
    document.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("resize", onResize);
      document.removeEventListener("scroll", onScroll, true);
      if (inTopLayer && popup.matches(":popover-open")) popup.hidePopover();
    };
  }, [open, place]);

  // And again after every render, in case the page moved the input; then
  // render the fonts now in view.
  useLayoutEffect(() => {
    if (!open) return;
    place();
    measure();
  });

  // The highlighted option in view as it moves by keyboard, by typing or as
  // the installed fonts arrive (and when the list opens).
  useLayoutEffect(() => {
    if (!open || !activeId) return;
    if (!hovered.current) scrollIdIntoView(activeId);
    hovered.current = false;
    measure();
  }, [open, activeId, measure]);

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
        if (!open) return;
        // Even with nothing to move to, so the panel underneath doesn't scroll
        // the field away (which closes the list and loses the filter).
        e.preventDefault();
        if (last >= 0)
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
  // Screen readers also hear how many fonts match what's typed.
  let status = note;
  if (!status && open && typed) {
    status = options.length === 1 ? "1 font matches." : `${options.length} fonts match.`;
  }

  const rowProps = (option: FontOption<V>, index: number) => ({
    id: optionId(option),
    index,
    option,
    active: index === current,
    current: option.key === selectedKey,
    onHover,
    onPick,
  });

  // The installed fonts in range, and the highlighted one wherever it is,
  // with spacers the height of the rows left out.
  const count = shownInstalled.length;
  const first = Math.min(range[0], count);
  const last = Math.min(range[1], count);
  const highlighted = current - shownPresets.length;
  const rendered: number[] = [];
  if (highlighted >= 0 && highlighted < first) rendered.push(highlighted);
  for (let i = first; i < last; i++) rendered.push(i);
  if (highlighted >= last && highlighted < count) rendered.push(highlighted);
  const installedRows: ReactNode[] = [];
  let next = 0;
  for (const i of [...rendered, count]) {
    if (i > next) {
      installedRows.push(<div key={`gap-${next}`} style={{ height: (i - next) * ROW }} />);
    }
    if (i === count) break;
    const option = shownInstalled[i]!;
    installedRows.push(
      <OptionRow
        key={option.key}
        {...rowProps(option, shownPresets.length + i)}
        face={(i >= faced[0] && i < faced[1]) || i === highlighted}
        setSize={count}
        posInSet={i + 1}
      />,
    );
    next = i + 1;
  }

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
        aria-activedescendant={activeId}
        aria-describedby={describedBy}
        autoComplete="off"
        spellCheck={false}
        value={open && query !== null ? query : selectedLabel}
        onChange={(e) => {
          if (!open) {
            setRange(FIRST_RANGE);
            setFaced(FIRST_FACED);
            setOpen(true);
          }
          setQuery(e.target.value);
          move(0);
        }}
        onClick={show}
        onKeyDown={onKeyDown}
        onBlur={close}
      />
      <ChevronDown size={16} className="font-picker__chevron" aria-hidden="true" />
      {/* Always there, so screen readers announce what changes in it. */}
      <p className="visually-hidden" role="status">
        {status}
      </p>
      {open && (
        <div
          ref={popupRef}
          className="font-picker__popup"
          // Keeps focus in the input, so a click on an option isn't a blur first.
          onMouseDown={(e: MouseEvent) => e.preventDefault()}
          // The DOM has the rows scrolled to before the frame is painted, not
          // after it; the overscan covers what the compositor scrolls ahead.
          onScroll={() => {
            flushSync(measure);
            stopSettling();
            settleTimer.current = window.setTimeout(settleGradually, SETTLE_MS);
          }}
        >
          <div
            id={listId}
            role="listbox"
            aria-labelledby={labelledBy}
            className="font-picker__list"
          >
            {shownPresets.map((option, i) => (
              <OptionRow key={option.key} {...rowProps(option, i)} />
            ))}
            {count > 0 && (
              <div role="group" aria-labelledby={`${listId}-installed`}>
                <div
                  ref={headingRef}
                  id={`${listId}-installed`}
                  role="presentation"
                  className={clsx(
                    "font-picker__group",
                    shownPresets.length > 0 && "font-picker__group--rule",
                  )}
                >
                  Installed fonts
                </div>
                {installedRows}
              </div>
            )}
          </div>
          <p className="font-picker__note" aria-hidden="true">
            {note}
          </p>
        </div>
      )}
    </div>
  );
}
