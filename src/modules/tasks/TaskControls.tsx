import { clsx } from "clsx";
import { Star } from "lucide-react";
import { useState, type MouseEvent } from "react";

import type { Task } from "../../api/types";

interface ControlProps {
  task: Task;
  onToggle: () => void;
  size?: "md" | "lg";
  /** Keep out of the tab order (rows are navigated as a listbox instead). */
  tabbable?: boolean;
}

/** Round completion checkbox; the check mark draws in when the task is completed. */
export function TaskCheckbox({ task, onToggle, size = "md", tabbable = true }: ControlProps) {
  const checked = task.status === "completed";
  // Only animate after a click, not when a completed task first renders.
  const [animate, setAnimate] = useState(false);
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      aria-label={`Completed: ${task.title}`}
      title={checked ? "Mark as not completed" : "Mark as completed"}
      tabIndex={tabbable ? undefined : -1}
      className={clsx("tasks-check", `tasks-check--${size}`, animate && "tasks-check--animate")}
      onClick={(e: MouseEvent) => {
        e.stopPropagation();
        setAnimate(!checked);
        onToggle();
      }}
    >
      <svg viewBox="0 0 20 20" aria-hidden="true">
        <path d="M5.5 10.5l3 3 6-6.5" />
      </svg>
    </button>
  );
}

/** Star toggle for importance. */
export function ImportanceStar({ task, onToggle, size = "md", tabbable = true }: ControlProps) {
  const important = task.importance === "high";
  return (
    <button
      type="button"
      className={clsx("icon-btn tasks-star", size === "lg" ? null : "icon-btn--sm")}
      aria-pressed={important}
      aria-label={`Important: ${task.title}`}
      title={important ? "Remove importance" : "Mark as important"}
      tabIndex={tabbable ? undefined : -1}
      onClick={(e: MouseEvent) => {
        e.stopPropagation();
        onToggle();
      }}
    >
      <Star size={size === "lg" ? 20 : 17} fill={important ? "currentColor" : "none"} />
    </button>
  );
}
