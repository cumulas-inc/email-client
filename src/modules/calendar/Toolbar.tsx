import clsx from "clsx";
import { ChevronLeft, ChevronRight } from "lucide-react";

import { Spinner } from "../../components/Spinner";
import { type CalendarView, VIEWS, VIEW_LABELS, stepLabel } from "./layout";

interface ToolbarProps {
  view: CalendarView;
  title: string;
  /** The same range in fewer characters, for narrow windows. */
  shortTitle: string;
  /** Use the short title as soon as the toolbar narrows, not only when it's at its narrowest. */
  shortenEarly: boolean;
  /** "Week 42" where the locale numbers its weeks, else null. */
  weekLabel: string | null;
  busy: boolean;
  onToday: () => void;
  onStep: (direction: 1 | -1) => void;
  onView: (view: CalendarView) => void;
}

export function Toolbar({
  view,
  title,
  shortTitle,
  shortenEarly,
  weekLabel,
  busy,
  onToday,
  onStep,
  onView,
}: ToolbarProps) {
  return (
    <div className="toolbar cal-toolbar">
      <button
        type="button"
        className="btn cal-toolbar__today"
        onClick={onToday}
        title="Go to today (T)"
      >
        Today
      </button>
      <div className="cal-toolbar__steps">
        <button
          type="button"
          className="icon-btn"
          aria-label={stepLabel(view, -1)}
          title={`${stepLabel(view, -1)} (Alt+←)`}
          onClick={() => onStep(-1)}
        >
          <ChevronLeft size={18} strokeWidth={1.75} />
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={stepLabel(view, 1)}
          title={`${stepLabel(view, 1)} (Alt+→)`}
          onClick={() => onStep(1)}
        >
          <ChevronRight size={18} strokeWidth={1.75} />
        </button>
      </div>
      <h1
        className="cal-toolbar__title"
        aria-live="polite"
        data-shorten={shortenEarly ? "early" : undefined}
      >
        {/* calendar.css shows one of the two by the width of the toolbar. */}
        <span className="cal-toolbar__title-long">{title}</span>
        {shortTitle !== title && (
          <span className="cal-toolbar__title-short" aria-hidden="true">
            {shortTitle}
          </span>
        )}
      </h1>
      {weekLabel && <span className="cal-toolbar__week tnum">{weekLabel}</span>}
      {busy && <Spinner size={16} label="Updating calendar" />}
      <div className="toolbar__spacer" />
      <div className="segmented cal-views" role="group" aria-label="Calendar view">
        {VIEWS.map((v, i) => (
          <button
            key={v}
            type="button"
            className={clsx(
              "segmented__option cal-views__button",
              view === v && "segmented__option--selected",
            )}
            aria-pressed={view === v}
            title={`${VIEW_LABELS[v]} (Ctrl+Alt+${i + 1})`}
            onClick={() => onView(v)}
          >
            {VIEW_LABELS[v]}
          </button>
        ))}
      </div>
    </div>
  );
}
