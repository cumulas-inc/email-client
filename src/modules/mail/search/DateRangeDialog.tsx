import { addDays, format } from "date-fns";
import { useId, useState } from "react";

import { Dialog } from "../../../components/Dialog";
import type { ResolvedDates } from "../../../lib/search/query";

interface DateRangeDialogProps {
  /** The days the search's dates leave now, to start from. */
  initial: ResolvedDates;
  /** A `received:` clause for the days chosen. */
  onApply: (clause: string) => void;
  onClose: () => void;
}

const day = (date: Date | null) => (date ? format(date, "yyyy-MM-dd") : "");

/** A range of days for the search: from a day, up to one, or both. */
export function DateRangeDialog({ initial, onApply, onClose }: DateRangeDialogProps) {
  const errorId = useId();
  const [from, setFrom] = useState(day(initial.since));
  // The last day the range takes in: the one before its end.
  const [to, setTo] = useState(day(initial.before ? addDays(initial.before, -1) : null));
  const [error, setError] = useState<string | null>(null);

  const apply = () => {
    if (!from && !to) setError("Choose a start date, an end date or both.");
    else if (from && to && to < from) setError("The end date is before the start date.");
    else onApply(`received:${from}..${to}`);
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title="Custom date range"
      width={380}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="button" className="btn btn--primary" onClick={apply}>
            Apply
          </button>
        </>
      }
    >
      <form
        className="mail-date-range"
        onSubmit={(e) => {
          e.preventDefault();
          apply();
        }}
      >
        <label className="field">
          <span className="field__label">From</span>
          <input
            type="date"
            className="input"
            value={from}
            aria-describedby={error ? errorId : undefined}
            onChange={(e) => {
              setFrom(e.target.value);
              setError(null);
            }}
          />
        </label>
        <label className="field">
          <span className="field__label">To</span>
          <input
            type="date"
            className="input"
            value={to}
            min={from || undefined}
            aria-describedby={error ? errorId : undefined}
            onChange={(e) => {
              setTo(e.target.value);
              setError(null);
            }}
          />
        </label>
        {error && (
          <p id={errorId} className="field__error" role="alert">
            {error}
          </p>
        )}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}
