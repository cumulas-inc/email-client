import { CalendarCheck, ChevronLeft, ChevronRight } from "lucide-react";

import { Spinner } from "../../components/Spinner";
import { type CalendarView, VIEWS, VIEW_LABELS, stepLabel } from "./layout";

interface ToolbarProps {
  view: CalendarView;
  title: string;
  busy: boolean;
  onToday: () => void;
  onStep: (direction: 1 | -1) => void;
  onView: (view: CalendarView) => void;
}

export function Toolbar({ view, title, busy, onToday, onStep, onView }: ToolbarProps) {
  return (
    <div className="toolbar cal-toolbar">
      <button type="button" className="btn btn--subtle" onClick={onToday} title="Go to today (T)">
        <CalendarCheck size={16} aria-hidden="true" />
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
          <ChevronLeft size={18} />
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={stepLabel(view, 1)}
          title={`${stepLabel(view, 1)} (Alt+→)`}
          onClick={() => onStep(1)}
        >
          <ChevronRight size={18} />
        </button>
      </div>
      <h1 className="cal-toolbar__title" aria-live="polite">
        {title}
      </h1>
      {busy && <Spinner size={16} label="Updating calendar" />}
      <div className="toolbar__spacer" />
      <div className="cal-views" role="group" aria-label="Calendar view">
        {VIEWS.map((v, i) => (
          <button
            key={v}
            type="button"
            className="cal-views__button"
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
