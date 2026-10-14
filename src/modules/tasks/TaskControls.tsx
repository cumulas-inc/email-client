import { clsx } from "clsx";
import { Star } from "lucide-react";
import type { MouseEvent } from "react";

import type { Task } from "../../api/types";

interface ControlProps {
  task: Task;
  /**
   * Called on click. Rows in a task list leave it out: the list handles
   * clicks on `data-action` buttons itself, so rows need no callbacks and
   * stay memoized.
   */
  onToggle?: () => void;
  /** Keep out of the tab order (rows are navigated as a listbox instead). */
  tabbable?: boolean;
}

function clickHandler(onToggle: (() => void) | undefined) {
  if (!onToggle) return undefined;
  return (e: MouseEvent) => {
    e.stopPropagation();
    onToggle();
  };
}

/** Round completion checkbox: teal fill with a check that draws in when done. */
export function TaskCheckbox({
  task,
  onToggle,
  size = "md",
  tabbable = true,
}: ControlProps & { size?: "md" | "lg" }) {
  const checked = task.status === "completed";
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      aria-label={`Completed: ${task.title}`}
      title={checked ? "Mark as not completed" : "Mark as completed"}
      tabIndex={tabbable ? undefined : -1}
      className={clsx("tasks-check", size === "lg" && "tasks-check--lg")}
      data-action="complete"
      onClick={clickHandler(onToggle)}
    >
      <svg viewBox="0 0 20 20" aria-hidden="true">
        <path d="M5.75 10.25l2.75 2.75 5.75-6" />
      </svg>
    </button>
  );
}

/** The star on a task row. Starred tasks collect in the Starred list. */
export function StarToggle({ task, onToggle, tabbable = true }: ControlProps) {
  const starred = task.importance === "high";
  return (
    <button
      type="button"
      className="icon-btn icon-btn--sm tasks-star"
      aria-pressed={starred}
      aria-label={`Starred: ${task.title}`}
      title={starred ? "Remove star" : "Star"}
      tabIndex={tabbable ? undefined : -1}
      data-action="star"
      onClick={clickHandler(onToggle)}
    >
      <Star size={16} strokeWidth={1.75} fill={starred ? "currentColor" : "none"} />
    </button>
  );
}

/** The star in the details pane: "Star" or "Starred", with its state. */
export function StarField({ task, onToggle }: Omit<ControlProps, "tabbable">) {
  const starred = task.importance === "high";
  return (
    <button
      type="button"
      className="tasks-star-field"
      aria-pressed={starred}
      aria-label="Starred"
      title={starred ? "Remove star" : "Star this task"}
      onClick={onToggle}
    >
      <Star size={16} strokeWidth={1.75} fill={starred ? "currentColor" : "none"} />
      {starred ? "Starred" : "Star"}
    </button>
  );
}
