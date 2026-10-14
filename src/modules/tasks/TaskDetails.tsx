import { clsx } from "clsx";
import { CalendarDays, List, PanelRightClose, StickyNote, Trash2, X } from "lucide-react";
import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
} from "react";

import type { Account, Task, TaskList, TaskPatch } from "../../api/types";
import { ImportanceStar, TaskCheckbox } from "./TaskControls";
import {
  dueLabel,
  isValidDateString,
  longDate,
  parseDue,
  quickDueDates,
  shortDate,
} from "./taskUtils";

interface TaskDetailsProps {
  task: Task;
  list: TaskList | undefined;
  account: Account | undefined;
  now: Date;
  titleRef: RefObject<HTMLTextAreaElement | null>;
  onUpdate: (patch: TaskPatch) => void;
  onToggleComplete: () => void;
  onDelete: () => void;
  onClose: () => void;
}

/** Grows a textarea to fit its content. */
function useAutoSize(ref: RefObject<HTMLTextAreaElement | null>, value: string) {
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    if (el.scrollHeight > 0) el.style.height = `${el.scrollHeight}px`;
  }, [ref, value]);
}

/** Right-hand pane: title, completion, importance, due date, notes. */
export function TaskDetails({
  task,
  list,
  account,
  now,
  titleRef,
  onUpdate,
  onToggleComplete,
  onDelete,
  onClose,
}: TaskDetailsProps) {
  const id = useId();
  const done = task.status === "completed";

  // Local drafts while a field has focus; otherwise the task's own values.
  const [title, setTitle] = useState<string | null>(null);
  const [notes, setNotes] = useState<string | null>(null);
  const [due, setDue] = useState<string | null>(null);
  const titleValue = title ?? task.title;
  const notesValue = notes ?? task.notes ?? "";
  const dueValue = due ?? task.dueDate ?? "";
  const notesRef = useRef<HTMLTextAreaElement>(null);
  useAutoSize(titleRef, titleValue);
  useAutoSize(notesRef, notesValue);

  const discardTitle = useRef(false);

  const commitTitle = () => {
    const next = (title ?? task.title).trim();
    setTitle(null);
    if (discardTitle.current) {
      discardTitle.current = false;
      return;
    }
    if (next && next !== task.title) onUpdate({ title: next });
  };

  const commitNotes = () => {
    if (notes === null) return;
    const next = notes;
    setNotes(null);
    if (!next.trim()) {
      if (task.notes) onUpdate({ clearNotes: true });
    } else if (next !== (task.notes ?? "")) {
      onUpdate({ notes: next });
    }
  };

  const setDueDate = (value: string | null) => {
    if (value === null) {
      if (task.dueDate) onUpdate({ clearDueDate: true });
    } else if (value !== task.dueDate) {
      onUpdate({ dueDate: value });
    }
  };

  const onTitleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      e.currentTarget.blur();
    } else if (e.key === "Escape") {
      // Revert and leave the field without saving.
      e.preventDefault();
      e.stopPropagation();
      discardTitle.current = true;
      e.currentTarget.blur();
    }
  };

  const onPaneKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === "Escape" && !e.defaultPrevented) {
      e.preventDefault();
      // Blur first so a note being typed is saved.
      (document.activeElement as HTMLElement | null)?.blur();
      onClose();
    }
  };

  const dueInfo = task.dueDate ? dueLabel(task.dueDate, now) : null;
  const quick = quickDueDates(now);

  return (
    <aside className="tasks-details" aria-label="Task details" onKeyDown={onPaneKeyDown}>
      <div className="tasks-details__scroll">
        <div className="tasks-card tasks-details__title-card">
          <TaskCheckbox task={task} size="lg" onToggle={onToggleComplete} />
          <textarea
            ref={titleRef}
            className={clsx("tasks-details__title", done && "tasks-details__title--done")}
            aria-label="Task title"
            rows={1}
            value={titleValue}
            onFocus={() => setTitle(task.title)}
            onChange={(e) => setTitle(e.target.value.replace(/\n/g, " "))}
            onBlur={commitTitle}
            onKeyDown={onTitleKeyDown}
          />
          <ImportanceStar
            task={task}
            size="lg"
            onToggle={() =>
              onUpdate({ importance: task.importance === "high" ? "normal" : "high" })
            }
          />
        </div>

        <section className="tasks-card tasks-details__due" aria-labelledby={`${id}-due`}>
          <h3 id={`${id}-due`} className="tasks-details__section-title">
            <CalendarDays size={16} aria-hidden="true" />
            <span
              className={clsx(
                dueInfo?.overdue && !done && "tasks-due--overdue",
                dueInfo?.today && !done && "tasks-due--today",
              )}
            >
              {task.dueDate ? `Due ${longDate(parseDue(task.dueDate), now)}` : "Add due date"}
            </span>
          </h3>
          <div className="tasks-details__quick" role="group" aria-label="Quick due dates">
            {quick.map((q) => (
              <button
                key={q.label}
                type="button"
                className="tasks-chip"
                aria-pressed={task.dueDate === q.date}
                onClick={() => setDueDate(q.date)}
              >
                {q.label}
              </button>
            ))}
          </div>
          <div className="tasks-details__date-row">
            <label className="visually-hidden" htmlFor={`${id}-date`}>
              Due date
            </label>
            <input
              id={`${id}-date`}
              type="date"
              className="input tasks-details__date"
              value={dueValue}
              onFocus={() => setDue(task.dueDate ?? "")}
              onChange={(e) => {
                const value = e.target.value;
                setDue(value);
                // Commit complete dates right away; partial typing waits for blur.
                if (isValidDateString(value) && Number(value.slice(0, 4)) >= 1900) {
                  setDueDate(value);
                }
              }}
              onBlur={() => {
                if (due === "") setDueDate(null);
                setDue(null);
              }}
            />
            {task.dueDate && (
              <button
                type="button"
                className="btn btn--subtle btn--sm tasks-details__remove"
                onClick={() => setDueDate(null)}
              >
                <X size={14} aria-hidden="true" />
                Remove due date
              </button>
            )}
          </div>
        </section>

        <section className="tasks-card tasks-details__notes-card">
          <label className="tasks-details__section-title" htmlFor={`${id}-notes`}>
            <StickyNote size={16} aria-hidden="true" />
            Notes
          </label>
          <textarea
            id={`${id}-notes`}
            ref={notesRef}
            className="tasks-details__notes"
            placeholder="Add a note"
            value={notesValue}
            onFocus={() => setNotes(task.notes ?? "")}
            onChange={(e) => setNotes(e.target.value)}
            onBlur={commitNotes}
          />
        </section>

        {list && (
          <p className="tasks-details__list">
            <List size={14} aria-hidden="true" />
            <span>
              In <strong>{list.name}</strong>
              {account && <> · {account.email}</>}
            </span>
          </p>
        )}
      </div>

      <footer className="tasks-details__footer">
        <button
          type="button"
          className="icon-btn"
          aria-label="Close details"
          title="Close details (Esc)"
          onClick={onClose}
        >
          <PanelRightClose size={18} />
        </button>
        <span className="tasks-details__stamp">
          {done && task.completedAt
            ? `Completed ${shortDate(task.completedAt, now)}`
            : `Created ${shortDate(task.createdAt, now)}`}
        </span>
        <button
          type="button"
          className="icon-btn tasks-details__delete"
          aria-label="Delete task"
          title="Delete task"
          onClick={onDelete}
        >
          <Trash2 size={18} />
        </button>
      </footer>
    </aside>
  );
}
