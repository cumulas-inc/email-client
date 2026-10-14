import { type KeyboardEvent, type PointerEvent, useRef } from "react";

interface SplitterProps {
  /** "vertical": a vertical bar that resizes widths; "horizontal" resizes heights. */
  orientation: "vertical" | "horizontal";
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
  label: string;
}

/** Draggable (and keyboard-adjustable) divider between two panes. */
export function Splitter({ orientation, value, min, max, onChange, label }: SplitterProps) {
  const drag = useRef<{ start: number; value: number; root: HTMLElement | null } | null>(null);
  const vertical = orientation === "vertical";

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    const root = e.currentTarget.closest<HTMLElement>(".mail");
    // Iframes would swallow pointer events while dragging over them.
    root?.classList.add(vertical ? "is-resizing-x" : "is-resizing-y");
    drag.current = { start: vertical ? e.clientX : e.clientY, value, root };
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    onChange(d.value + (vertical ? e.clientX : e.clientY) - d.start);
  };

  const end = () => {
    drag.current?.root?.classList.remove("is-resizing-x", "is-resizing-y");
    drag.current = null;
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 48 : 16;
    const less = vertical ? "ArrowLeft" : "ArrowUp";
    const more = vertical ? "ArrowRight" : "ArrowDown";
    if (e.key === less) onChange(value - step);
    else if (e.key === more) onChange(value + step);
    else if (e.key === "Home") onChange(min);
    else if (e.key === "End") onChange(max);
    else return;
    e.preventDefault();
  };

  return (
    <div
      role="separator"
      aria-orientation={orientation}
      aria-label={label}
      aria-valuenow={value}
      aria-valuemin={min}
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
