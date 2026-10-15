import { clsx } from "clsx";
import { Bell, ChevronDown, Trash2, X } from "lucide-react";
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from "react";

import type { Account, Task, TaskList, TaskPatch } from "../../api/types";
import { StarField, TaskCheckbox } from "./TaskControls";
import {
  dueLabel,
  isValidDateString,
  longDate,
  parseDue,
  quickDueDates,
  quickReminders,
  reminderFromInputs,
  reminderLabel,
  shortDate,
  toDateString,
  toTimeString,
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

/** Grows a textarea to fit its content (while `enabled`; otherwise CSS sizes it). */
function useAutoSize(ref: RefObject<HTMLTextAreaElement | null>, value: string, enabled = true) {
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (!enabled) {
      el.style.height = "";
      el.scrollTop = 0;
      return;
    }
    el.style.height = "auto";
    if (el.scrollHeight > 0) el.style.height = `${el.scrollHeight}px`;
  }, [ref, value, enabled]);
}

/** Lines a task title shows before it is clamped (see .tasks-title__text). */
const TITLE_LINES = 3;

/** Whether the text in `ref` runs past TITLE_LINES lines, re-checked as the pane resizes. */
function useLongerThanClamp(ref: RefObject<HTMLElement | null>, text: string) {
  const [longer, setLonger] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      const style = getComputedStyle(el);
      const padding = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
      const lines = (el.scrollHeight - padding) / parseFloat(style.lineHeight);
      setLonger(lines > TITLE_LINES + 0.5);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref, text]);
  return longer;
}

/**
 * The details card. It stays mounted while the selection moves from task to
 * task, so it only animates in when it opens, and only when `enter` is set
 * (not when Tasks opens with a task already showing).
 */
export function TaskDetailsPane({
  children,
  enter,
  onClose,
}: {
  children: ReactNode;
  enter: boolean;
  onClose: () => void;
}) {
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === "Escape" && !e.defaultPrevented) {
      e.preventDefault();
      // Blur first so a note being typed is saved.
      (document.activeElement as HTMLElement | null)?.blur();
      onClose();
    }
  };
  return (
    <aside
      className={clsx("tasks-details card", enter && "tasks-details--enter")}
      aria-label="Task details"
      onKeyDown={onKeyDown}
    >
      {children}
    </aside>
  );
}

/**
 * When to be reminded: quick picks (later today, tomorrow, next week), a
 * date and a time, and Remove. A complete date commits at once, as the due
 * date does; the time, once it's whole.
 */
function ReminderField({
  task,
  now,
  onUpdate,
}: {
  task: Task;
  now: Date;
  onUpdate: (patch: TaskPatch) => void;
}) {
  const id = useId();
  const current = task.reminderAt ? new Date(task.reminderAt) : null;
  // Drafts while a field has focus; otherwise the reminder's own values.
  const [date, setDate] = useState<string | null>(null);
  const [time, setTime] = useState<string | null>(null);
  const dateValue = date ?? (current ? toDateString(current) : "");
  const timeValue = time ?? (current ? toTimeString(current) : "");
  const done = task.status === "completed";
  const past = current !== null && current <= now && !done;

  const setReminder = (at: Date | null) => {
    if (at === null) {
      if (task.reminderAt) onUpdate({ clearReminder: true });
    } else if (!current || at.getTime() !== current.getTime()) {
      onUpdate({ reminderAt: at.toISOString() });
    }
  };

  const commitInputs = (nextDate: string, nextTime: string) => {
    const at = reminderFromInputs(nextDate, nextTime, now);
    if (at) setReminder(at);
  };

  return (
    <div className="tasks-field">
      <div className="tasks-field__head">
        <label className="tasks-field__label" htmlFor={`${id}-date`}>
          Reminder
        </label>
        <span
          className={clsx(
            "tasks-field__value",
            !current && "tasks-field__value--empty",
            current && !past && !done && "tasks-field__value--today",
          )}
        >
          {current && (
            <Bell size={12} strokeWidth={2} aria-hidden="true" className="tasks-field__icon" />
          )}
          {current ? reminderLabel(current, now) : "None"}
        </span>
      </div>
      <div className="tasks-quick" role="group" aria-label="Quick reminders">
        {quickReminders(now).map((q) => (
          <button
            key={q.label}
            type="button"
            className="tasks-quick__option"
            aria-pressed={current?.getTime() === q.at.getTime()}
            aria-label={`Remind ${q.label.toLowerCase()}`}
            title={reminderLabel(q.at, now)}
            onClick={() => setReminder(q.at)}
          >
            {q.label}
          </button>
        ))}
      </div>
      <div className="tasks-details__date-row tasks-details__reminder-row">
        <input
          id={`${id}-date`}
          type="date"
          className="input tasks-details__date"
          aria-label="Reminder date"
          value={dateValue}
          onFocus={() => setDate(dateValue)}
          onChange={(e) => {
            const value = e.target.value;
            setDate(value);
            if (isValidDateString(value) && Number(value.slice(0, 4)) >= 1900) {
              commitInputs(value, timeValue);
            }
          }}
          onBlur={() => setDate(null)}
        />
        <input
          type="time"
          className="input tasks-details__time"
          aria-label="Reminder time"
          value={timeValue}
          onFocus={() => setTime(timeValue)}
          onChange={(e) => {
            const value = e.target.value;
            setTime(value);
            if (/^\d{2}:\d{2}$/.test(value)) commitInputs(dateValue, value);
          }}
          onBlur={() => setTime(null)}
        />
        {current && (
          <button
            type="button"
            className="btn btn--subtle btn--sm tasks-details__remove"
            aria-label="Remove reminder"
            title="Remove reminder"
            onClick={() => setReminder(null)}
          >
            <X size={14} strokeWidth={2} aria-hidden="true" />
            Remove
          </button>
        )}
      </div>
    </div>
  );
}

/** Contents of the details card: title, completion, due date, reminder, list, star, notes. */
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
  const titleTextRef = useRef<HTMLParagraphElement>(null);
  // The title is a draft (non-null) exactly while its field has focus.
  const editingTitle = title !== null;
  const [titleExpanded, setTitleExpanded] = useState(false);
  const titleIsLong = useLongerThanClamp(titleTextRef, titleValue);
  useAutoSize(titleRef, titleValue, editingTitle);
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

  const dueInfo = task.dueDate ? dueLabel(task.dueDate, now) : null;
  const quick = quickDueDates(now);

  return (
    <div className="tasks-details__scroll">
      <div className="tasks-details__head">
        <TaskCheckbox task={task} size="lg" onToggle={onToggleComplete} />
        <div className="tasks-details__title-col">
          {/*
            The title reads as a serif heading clamped to a few lines; the
            field over it takes the click and shows the whole title while
            it's being edited.
          */}
          <div
            className={clsx(
              "tasks-title",
              editingTitle && "tasks-title--editing",
              titleExpanded && "tasks-title--expanded",
              done && "tasks-title--done",
            )}
          >
            <p ref={titleTextRef} className="tasks-title__text" dir="auto" aria-hidden="true">
              {titleValue}
            </p>
            <textarea
              ref={titleRef}
              className="tasks-title__input"
              dir="auto"
              aria-label="Task title"
              title={titleIsLong && !titleExpanded && !editingTitle ? task.title : undefined}
              rows={1}
              value={titleValue}
              onFocus={() => setTitle(task.title)}
              onChange={(e) => setTitle(e.target.value.replace(/\n/g, " "))}
              onBlur={commitTitle}
              onKeyDown={onTitleKeyDown}
            />
          </div>
          {titleIsLong && !editingTitle && (
            <button
              type="button"
              className="tasks-title__more"
              aria-expanded={titleExpanded}
              onClick={() => setTitleExpanded((x) => !x)}
            >
              {titleExpanded ? "Show less" : "Show full title"}
              <ChevronDown size={14} strokeWidth={2} aria-hidden="true" />
            </button>
          )}
        </div>
        <div className="tasks-details__actions">
          <button
            type="button"
            className="icon-btn tasks-details__delete"
            aria-label="Delete task"
            title="Delete task"
            onClick={onDelete}
          >
            <Trash2 size={18} strokeWidth={1.75} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="icon-btn"
            aria-label="Close details"
            title="Close details (Esc)"
            onClick={onClose}
          >
            <X size={18} strokeWidth={1.75} aria-hidden="true" />
          </button>
        </div>
      </div>

      <div className="tasks-field">
        <div className="tasks-field__head">
          <label className="tasks-field__label" htmlFor={`${id}-date`}>
            Due date
          </label>
          <span
            className={clsx(
              "tasks-field__value",
              !task.dueDate && "tasks-field__value--empty",
              dueInfo?.overdue && !done && "tasks-field__value--overdue",
              dueInfo?.today && !done && "tasks-field__value--today",
            )}
          >
            {task.dueDate ? longDate(parseDue(task.dueDate), now) : "None"}
            {dueInfo?.overdue && !done && " · Overdue"}
            {dueInfo?.today && !done && " · Today"}
          </span>
        </div>
        <div className="tasks-quick" role="group" aria-label="Quick due dates">
          {quick.map((q) => (
            <button
              key={q.label}
              type="button"
              className="tasks-quick__option"
              aria-pressed={task.dueDate === q.date}
              onClick={() => setDueDate(q.date)}
            >
              {q.label}
            </button>
          ))}
        </div>
        <div className="tasks-details__date-row">
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
              aria-label="Remove due date"
              title="Remove due date"
              onClick={() => setDueDate(null)}
            >
              <X size={14} strokeWidth={2} aria-hidden="true" />
              Remove
            </button>
          )}
        </div>
      </div>

      <ReminderField task={task} now={now} onUpdate={onUpdate} />

      {list && (
        <div className="tasks-field tasks-field--row">
          <span className="tasks-field__label">List</span>
          <p className="tasks-field__list">
            <span
              className="tasks-field__dot"
              style={account ? { background: account.color } : undefined}
              aria-hidden="true"
            />
            <span className="tasks-field__list-text">
              <span className="tasks-field__list-name" dir="auto">
                {list.name}
              </span>{" "}
              {account && (
                <span
                  className="tasks-field__list-account"
                  title={`${account.displayName} (${account.email})`}
                >
                  {account.email}
                </span>
              )}
            </span>
          </p>
        </div>
      )}

      <div className="tasks-field tasks-field--row">
        <span className="tasks-field__label">Star</span>
        <StarField
          task={task}
          onToggle={() => onUpdate({ importance: task.importance === "high" ? "normal" : "high" })}
        />
      </div>

      <div className="tasks-field tasks-field--notes">
        <label className="tasks-field__label" htmlFor={`${id}-notes`}>
          Notes
        </label>
        <textarea
          id={`${id}-notes`}
          ref={notesRef}
          className="textarea tasks-details__notes"
          dir="auto"
          placeholder="Add a note"
          value={notesValue}
          onFocus={() => setNotes(task.notes ?? "")}
          onChange={(e) => setNotes(e.target.value)}
          onBlur={commitNotes}
        />
      </div>

      {/* Quiet metadata under the notes: "Created Oct 8 · Completed Oct 14". */}
      <p className="tasks-details__stamp">
        {[
          `Created ${shortDate(task.createdAt, now)}`,
          done && task.completedAt && `Completed ${shortDate(task.completedAt, now)}`,
        ]
          .filter(Boolean)
          .join(" · ")}
      </p>
    </div>
  );
}
