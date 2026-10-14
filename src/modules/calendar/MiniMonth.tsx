import clsx from "clsx";
import {
  addDays,
  addMonths,
  format,
  isSameDay,
  isSameMonth,
  startOfDay,
  startOfMonth,
} from "date-fns";
import { ChevronDown, ChevronUp } from "lucide-react";
import { type KeyboardEvent, useEffect, useRef, useState } from "react";

import { chunk, viewRange } from "./layout";

interface MiniMonthProps {
  /** The date the main view is showing. */
  anchor: Date;
  /** Visible range of the main view (end exclusive). */
  rangeStart: Date;
  rangeEnd: Date;
  today: Date;
  onSelect: (day: Date) => void;
}

const KEY_STEPS: Record<string, number> = {
  ArrowLeft: -1,
  ArrowRight: 1,
  ArrowUp: -7,
  ArrowDown: 7,
};

/** Small month calendar for jumping to a date. Arrow keys move between days. */
export function MiniMonth({ anchor, rangeStart, rangeEnd, today, onSelect }: MiniMonthProps) {
  const [month, setMonth] = useState(() => startOfMonth(anchor));
  const [focused, setFocused] = useState(() => startOfDay(anchor));
  const [shownAnchor, setShownAnchor] = useState(anchor);
  const gridRef = useRef<HTMLTableElement>(null);
  const moveFocus = useRef(false);

  // Follow the main view when it moves to another date.
  if (shownAnchor.getTime() !== anchor.getTime()) {
    setShownAnchor(anchor);
    setFocused(startOfDay(anchor));
    if (!isSameMonth(anchor, month)) setMonth(startOfMonth(anchor));
  }

  useEffect(() => {
    if (!moveFocus.current) return;
    moveFocus.current = false;
    gridRef.current?.querySelector<HTMLButtonElement>('button[tabindex="0"]')?.focus();
  }, [focused, month]);

  const { days } = viewRange("month", month);
  const weeks = chunk(days, 7);
  const tabbable = isSameMonth(focused, month) ? focused : month;

  const onKeyDown = (e: KeyboardEvent<HTMLTableElement>) => {
    const step = KEY_STEPS[e.key];
    let next: Date | null = null;
    if (step !== undefined) next = addDays(tabbable, step);
    else if (e.key === "PageUp") next = addMonths(tabbable, -1);
    else if (e.key === "PageDown") next = addMonths(tabbable, 1);
    if (!next) return;
    e.preventDefault();
    e.stopPropagation();
    moveFocus.current = true;
    setFocused(next);
    if (!isSameMonth(next, month)) setMonth(startOfMonth(next));
  };

  return (
    <div className="cal-mini">
      <div className="cal-mini__header">
        <span className="cal-mini__title" aria-live="polite">
          {format(month, "MMMM yyyy")}
        </span>
        <button
          type="button"
          className="icon-btn icon-btn--sm"
          aria-label="Go to previous month"
          title="Previous month"
          onClick={() => setMonth((m) => addMonths(m, -1))}
        >
          <ChevronUp size={16} />
        </button>
        <button
          type="button"
          className="icon-btn icon-btn--sm"
          aria-label="Go to next month"
          title="Next month"
          onClick={() => setMonth((m) => addMonths(m, 1))}
        >
          <ChevronDown size={16} />
        </button>
      </div>
      <table
        ref={gridRef}
        role="grid"
        className="cal-mini__grid"
        aria-label={`Date navigator, ${format(month, "MMMM yyyy")}`}
        onKeyDown={onKeyDown}
      >
        <thead>
          <tr>
            {weeks[0]!.map((d) => (
              <th key={d.getTime()} scope="col" className="cal-mini__weekday">
                <abbr title={format(d, "EEEE")}>{format(d, "EEEEE")}</abbr>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {weeks.map((week) => (
            <tr key={week[0]!.getTime()}>
              {week.map((day) => {
                const inRange = day >= rangeStart && day < rangeEnd;
                const isToday = isSameDay(day, today);
                const tabIndex = isSameDay(day, tabbable) ? 0 : -1;
                return (
                  <td key={day.getTime()} role="gridcell" aria-selected={inRange}>
                    <button
                      type="button"
                      tabIndex={tabIndex}
                      className={clsx("cal-mini__day", {
                        "cal-mini__day--outside": !isSameMonth(day, month),
                        "cal-mini__day--in-range": inRange,
                        "cal-mini__day--range-start": inRange && isSameDay(day, rangeStart),
                        "cal-mini__day--range-end": inRange && isSameDay(addDays(day, 1), rangeEnd),
                        "cal-mini__day--today": isToday,
                        "cal-mini__day--anchor": isSameDay(day, anchor),
                      })}
                      aria-label={format(day, "EEEE, d MMMM yyyy")}
                      aria-current={isToday ? "date" : undefined}
                      onClick={() => {
                        setFocused(day);
                        onSelect(day);
                      }}
                    >
                      <span>{format(day, "d")}</span>
                    </button>
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
