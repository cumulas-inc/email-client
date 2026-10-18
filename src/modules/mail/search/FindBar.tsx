import { ChevronDown, ChevronUp } from "lucide-react";
import { useEffect } from "react";

interface FindBarProps {
  /** Matches marked in the message. */
  count: number;
  /** The match shown, or null before the first step. */
  current: number | null;
  onShow: (index: number) => void;
}

/** The match one step on (or back) from `current`, around the ends. */
function step(current: number | null, count: number, by: 1 | -1): number {
  if (current === null) return by === 1 ? 0 : count - 1;
  return (current + by + count) % count;
}

/**
 * The search's matches in the open message, and buttons (F3, Shift+F3) to
 * step through them, around from the last to the first. It sticks to the
 * top of the reader as the message scrolls under it.
 */
export function FindBar({ count, current, onShow }: FindBarProps) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "F3" || e.defaultPrevented || e.ctrlKey || e.altKey || e.metaKey) return;
      e.preventDefault();
      onShow(step(current, count, e.shiftKey ? -1 : 1));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [count, current, onShow]);

  const position = current === null ? null : `Match ${current + 1} of ${count}`;
  return (
    <div className="mail-findbar" role="group" aria-label="Matches in this message">
      <span className="mail-findbar__count tnum" aria-hidden={position ? true : undefined}>
        {position ?? (count === 1 ? "1 match" : `${count} matches`)}
      </span>
      <p className="visually-hidden" role="status">
        {position ?? ""}
      </p>
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        aria-label="Previous match"
        title="Previous match (Shift+F3)"
        onClick={() => onShow(step(current, count, -1))}
      >
        <ChevronUp size={15} aria-hidden="true" />
      </button>
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        aria-label="Next match"
        title="Next match (F3)"
        onClick={() => onShow(step(current, count, 1))}
      >
        <ChevronDown size={15} aria-hidden="true" />
      </button>
    </div>
  );
}
