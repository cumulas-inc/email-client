import { type KeyboardEvent, type PointerEvent, useEffect, useRef, useState } from "react";

interface SplitterProps {
  /** "vertical": a vertical bar that resizes widths; "horizontal" resizes heights. */
  orientation: "vertical" | "horizontal";
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
  label: string;
}

/** The laid-out size of the pane before the splitter (0 when nothing is laid out, as in tests). */
function paneSize(splitter: Element | null, vertical: boolean): number {
  const pane = splitter?.previousElementSibling;
  if (!(pane instanceof HTMLElement)) return 0;
  return vertical ? pane.offsetWidth : pane.offsetHeight;
}

/** The pane's CSS min-width (or min-height), which can hold it above `min`. */
function paneFloor(splitter: Element | null, vertical: boolean): number {
  const pane = splitter?.previousElementSibling;
  if (!(pane instanceof HTMLElement)) return 0;
  const style = getComputedStyle(pane);
  return parseFloat(vertical ? style.minWidth : style.minHeight) || 0;
}

/** Draggable (and keyboard-adjustable) divider between two panes. */
export function Splitter({ orientation, value, min, max, onChange, label }: SplitterProps) {
  const ref = useRef<HTMLDivElement>(null);
  const drag = useRef<{
    start: number;
    shown: number;
    saved: number;
    root: HTMLElement | null;
  } | null>(null);
  const vertical = orientation === "vertical";

  // A small window can squeeze the pane below the size the user chose
  // (`value`), and a CSS min-width can hold it above. The splitter reports
  // the size on screen. Shrinking starts from the smaller of the two sizes,
  // so the pane moves at once; growing starts from the larger, so a squeezed
  // pane that cannot grow keeps the size the user chose for wider windows.
  const [shown, setShown] = useState(0);
  const [floor, setFloor] = useState(0);
  useEffect(() => {
    const pane = ref.current?.previousElementSibling;
    if (!pane || typeof ResizeObserver !== "function") return;
    const observer = new ResizeObserver(() => {
      setShown(paneSize(ref.current, vertical));
      setFloor(paneFloor(ref.current, vertical));
    });
    observer.observe(pane);
    return () => observer.disconnect();
  }, [vertical]);
  const current = () => paneSize(ref.current, vertical) || value;

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    const root = e.currentTarget.closest<HTMLElement>(".mail");
    // Iframes would swallow pointer events while dragging over them.
    root?.classList.add(vertical ? "is-resizing-x" : "is-resizing-y");
    drag.current = {
      start: vertical ? e.clientX : e.clientY,
      shown: current(),
      saved: value,
      root,
    };
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const delta = (vertical ? e.clientX : e.clientY) - d.start;
    // Measured against the sizes when the drag began, not the live ones. Back
    // where it began, the size the user chose stands.
    if (delta < 0) onChange(Math.min(d.shown, d.saved) + delta);
    else if (delta > 0) onChange(Math.max(d.shown, d.saved) + delta);
    else onChange(d.saved);
  };

  const end = () => {
    drag.current?.root?.classList.remove("is-resizing-x", "is-resizing-y");
    drag.current = null;
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 48 : 16;
    const less = vertical ? "ArrowLeft" : "ArrowUp";
    const more = vertical ? "ArrowRight" : "ArrowDown";
    if (e.key === less) onChange(Math.min(current(), value) - step);
    else if (e.key === more) onChange(Math.max(current(), value) + step);
    else if (e.key === "Home") onChange(min);
    else if (e.key === "End") onChange(max);
    else return;
    e.preventDefault();
  };

  return (
    <div
      ref={ref}
      role="separator"
      aria-orientation={orientation}
      aria-label={label}
      aria-valuenow={shown || value}
      // The least it can be at this window size.
      aria-valuemin={Math.min(Math.max(min, floor), max)}
      aria-valuemax={max}
      tabIndex={0}
      className={`mail-splitter mail-splitter--${orientation}`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={end}
      onPointerCancel={end}
      onLostPointerCapture={end}
      onKeyDown={onKeyDown}
    />
  );
}
