import { clsx } from "clsx";
import { CalendarDays, NotebookText } from "lucide-react";
import {
  memo,
  useEffect,
  useId,
  useRef,
  type KeyboardEvent,
  type MouseEvent,
  type Ref,
} from "react";

import type { Task } from "../../api/types";
import { listboxTarget, scrollIdIntoView } from "../../components/listboxKeys";
import { Count } from "./Count";
import { titleWhenCut } from "./fullText";
import { StarToggle, TaskCheckbox } from "./TaskControls";
import { SETTLE_MS, dueLabel, parseDue, toDateString } from "./taskUtils";

export interface TaskGroup {
  key: string;
  /** Group header; null for an unlabeled group. */
  label: string | null;
  tasks: Task[];
}

/** Which list a task is in, shown on rows in smart lists. */
export interface ListTag {
  /** "Tasks", or "Tasks · Alex Morgan" when two accounts have a list of that name. */
  name: string;
  /** The account's color. */
  color: string;
}

export interface TaskActions {
  onSelect: (task: Task) => void;
  onToggleComplete: (task: Task) => void;
  onToggleStar: (task: Task) => void;
  onDelete: (task: Task) => void;
  /** Enter: move focus to the details pane. */
  onOpen: (task: Task) => void;
}

interface TaskListboxProps extends TaskActions {
  label: string;
  /** Open tasks, or the Completed section: where a toggled task leaves the list. */
  kind: "open" | "completed";
  groups: TaskGroup[];
  selectedId: string | null;
  /** List tags by list id, shown on rows in smart lists. */
  listTags: Map<string, ListTag> | null;
  now: Date;
  id?: string;
  listRef?: Ref<HTMLDivElement>;
  /** Tasks that were just completed and are about to move away. */
  settling?: Set<string>;
}

/** The last part of a completed task's settle, in which its row closes up. */
const COLLAPSE_MS = 180;

function prefersReducedMotion(): boolean {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

interface TaskRowProps {
  task: Task;
  optionId: string;
  selected: boolean;
  settling: boolean;
  listName: string | null;
  listColor: string | null;
  /** Today as "YYYY-MM-DD", so rows only re-render when a task changes, not every second. */
  today: string;
}

/**
 * One task tile. It takes no callbacks (the listbox handles clicks on its
 * `data-action` buttons), so moving the selection re-renders two rows, not
 * the whole list.
 */
const TaskRow = memo(function TaskRow({
  task,
  optionId,
  selected,
  settling,
  listName,
  listColor,
  today,
}: TaskRowProps) {
  const ref = useRef<HTMLDivElement>(null);
  const done = task.status === "completed";
  const starred = task.importance === "high";
  const due = task.dueDate ? dueLabel(task.dueDate, parseDue(today)) : null;

  // A task checked off closes its gap before it moves to Completed, so the
  // rows below slide up instead of jumping.
  useEffect(() => {
    const el = ref.current;
    if (!settling || !el || typeof el.animate !== "function" || prefersReducedMotion()) return;
    const anim = el.animate(
      [
        { height: `${el.offsetHeight}px` },
        { height: "0px", minHeight: "0px", paddingTop: "0px", paddingBottom: "0px", opacity: 0 },
      ],
      {
        duration: COLLAPSE_MS,
        delay: SETTLE_MS - COLLAPSE_MS - 30,
        easing: "cubic-bezier(0.77, 0, 0.175, 1)",
        fill: "forwards",
      },
    );
    return () => anim.cancel();
  }, [settling]);

  const name = [
    task.title,
    done ? "completed" : null,
    starred ? "starred" : null,
    due ? `due ${due.text}${due.overdue && !done ? ", overdue" : ""}` : null,
    task.notes ? "has notes" : null,
    listName ? `in ${listName}` : null,
  ]
    .filter(Boolean)
    .join(", ");

  return (
    <div
      ref={ref}
      id={optionId}
      role="option"
      aria-selected={selected}
      aria-label={name}
      data-task-id={task.id}
      className={clsx("tasks-row", done && "tasks-row--done", settling && "tasks-row--settling")}
    >
      <TaskCheckbox task={task} tabbable={false} />
      <span className="tasks-row__title">
        <span className="tasks-row__text" dir="auto" onMouseEnter={titleWhenCut}>
          {task.title}
        </span>
        {task.notes && (
          <NotebookText
            className="tasks-row__note"
            size={14}
            strokeWidth={1.75}
            aria-hidden="true"
          />
        )}
      </span>
      {(listName || due) && (
        <span className="tasks-row__meta">
          {listName && (
            <span className="tasks-row__list">
              <span
                className="tasks-row__list-dot"
                style={{ background: listColor ?? undefined }}
                aria-hidden="true"
              />
              <span className="tasks-row__list-name" dir="auto" onMouseEnter={titleWhenCut}>
                {listName}
              </span>
            </span>
          )}
          {/* The due date keeps its slot, so list names line up from row to row. */}
          <span className="tasks-row__due">
            {due && (
              <span
                className={clsx(
                  "tasks-due",
                  due.overdue && !done && "tasks-due--overdue",
                  due.today && !done && "tasks-due--today",
                )}
              >
                <CalendarDays size={13} strokeWidth={1.9} aria-hidden="true" />
                {due.text}
              </span>
            )}
          </span>
        </span>
      )}
      <StarToggle task={task} tabbable={false} />
    </div>
  );
});

/**
 * Tasks as a listbox of tiles: one tab stop, ↑/↓ to select (the details
 * pane follows), Space to complete, Enter to edit, Delete to delete.
 */
export function TaskListbox({
  label,
  kind,
  groups,
  selectedId,
  listTags,
  now,
  id,
  listRef,
  settling,
  ...actions
}: TaskListboxProps) {
  const baseId = useId();
  const hintId = `${baseId}-hint`;
  const ordered = groups.flatMap((g) => g.tasks);
  const index = ordered.findIndex((t) => t.id === selectedId);
  const optionId = (taskId: string) => `${baseId}-task-${taskId}`;
  const activeId = index >= 0 && selectedId ? optionId(selectedId) : undefined;
  const today = toDateString(now);

  useEffect(() => {
    scrollIdIntoView(activeId);
  }, [activeId]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const target = listboxTarget(e.key, index, ordered.length);
    if (target !== null) {
      e.preventDefault();
      actions.onSelect(ordered[target]!);
      return;
    }
    const current = ordered[index];
    if (!current) return;
    if (e.key === " ") {
      e.preventDefault();
      actions.onToggleComplete(current);
      // The task is about to leave this list (to Completed, or back to the
      // open tasks): keep the keyboard where it was, on the next task.
      const leaves = kind === "completed" || current.status !== "completed";
      const neighbor = leaves ? (ordered[index + 1] ?? ordered[index - 1]) : undefined;
      if (neighbor) actions.onSelect(neighbor);
    } else if (e.key === "Enter") {
      e.preventDefault();
      actions.onOpen(current);
    } else if (e.key === "Delete") {
      e.preventDefault();
      actions.onDelete(current);
    }
  };

  // One handler for every row: the checkbox and star carry a data-action.
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    const taskId = target.closest<HTMLElement>("[data-task-id]")?.dataset.taskId;
    const task = taskId ? ordered.find((t) => t.id === taskId) : undefined;
    if (!task) return;
    const action = target.closest<HTMLElement>("[data-action]")?.dataset.action;
    if (action === "complete") actions.onToggleComplete(task);
    else if (action === "star") actions.onToggleStar(task);
    else actions.onSelect(task);
  };

  const renderRow = (task: Task) => {
    const tag = listTags?.get(task.listId) ?? null;
    return (
      <TaskRow
        key={task.id}
        task={task}
        optionId={optionId(task.id)}
        selected={task.id === selectedId}
        settling={settling?.has(task.id) ?? false}
        listName={tag?.name ?? null}
        listColor={tag?.color ?? null}
        today={today}
      />
    );
  };

  return (
    <>
      <span id={hintId} className="visually-hidden">
        Space completes the selected task, Enter edits it, Delete deletes it.
      </span>
      <div
        ref={listRef}
        id={id}
        role="listbox"
        aria-label={label}
        aria-activedescendant={activeId}
        aria-describedby={hintId}
        tabIndex={0}
        className="tasks-listbox"
        onKeyDown={onKeyDown}
        onClick={onClick}
      >
        {groups.map((group) => {
          if (group.label === null) return group.tasks.map(renderRow);
          const headerId = `${baseId}-group-${group.key}`;
          return (
            <div
              key={group.key}
              role="group"
              aria-labelledby={headerId}
              className={clsx("tasks-group", `tasks-group--${group.key}`)}
            >
              <div id={headerId} className="tasks-group__header">
                {group.label} <Count n={group.tasks.length} className="tasks-group__count" />
              </div>
              {group.tasks.map(renderRow)}
            </div>
          );
        })}
      </div>
    </>
  );
}
