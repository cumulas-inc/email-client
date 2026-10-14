import { clsx } from "clsx";
import { CalendarDays, NotebookText } from "lucide-react";
import { useEffect, useId, type KeyboardEvent, type Ref } from "react";

import type { Task } from "../../api/types";
import { listboxTarget, scrollIdIntoView } from "../../components/listboxKeys";
import { StarToggle, TaskCheckbox } from "./TaskControls";
import { dueLabel } from "./taskUtils";

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

/**
 * Tasks as a listbox of tiles: one tab stop, ↑/↓ to select (the details
 * pane follows), Space to complete, Enter to edit, Delete to delete.
 */
export function TaskListbox({
  label,
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
    } else if (e.key === "Enter") {
      e.preventDefault();
      actions.onOpen(current);
    } else if (e.key === "Delete") {
      e.preventDefault();
      actions.onDelete(current);
    }
  };

  const renderRow = (task: Task) => {
    const done = task.status === "completed";
    const starred = task.importance === "high";
    const due = task.dueDate ? dueLabel(task.dueDate, now) : null;
    const listTag = listTags?.get(task.listId) ?? null;
    const name = [
      task.title,
      done ? "completed" : null,
      starred ? "starred" : null,
      due ? `due ${due.text}${due.overdue && !done ? ", overdue" : ""}` : null,
      task.notes ? "has notes" : null,
      listTag ? `in ${listTag.name}` : null,
    ]
      .filter(Boolean)
      .join(", ");
    return (
      <div
        key={task.id}
        id={optionId(task.id)}
        role="option"
        aria-selected={task.id === selectedId}
        aria-label={name}
        className={clsx(
          "tasks-row",
          done && "tasks-row--done",
          settling?.has(task.id) && "tasks-row--settling",
        )}
        onClick={() => actions.onSelect(task)}
      >
        <TaskCheckbox
          task={task}
          tabbable={false}
          onToggle={() => actions.onToggleComplete(task)}
        />
        <span className="tasks-row__title" title={task.title}>
          {task.title}
        </span>
        {(listTag || due || task.notes) && (
          <span className="tasks-row__meta">
            {listTag && (
              <span className="tasks-row__list" title={listTag.name}>
                <span
                  className="tasks-row__list-dot"
                  style={{ background: listTag.color }}
                  aria-hidden="true"
                />
                <span className="tasks-row__list-name">{listTag.name}</span>
              </span>
            )}
            {/* Notes and due date keep their slots, so they line up from row to row. */}
            <span className="tasks-row__note" title={task.notes ? "Has notes" : undefined}>
              {task.notes && <NotebookText size={14} strokeWidth={1.75} aria-hidden="true" />}
            </span>
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
        <StarToggle task={task} tabbable={false} onToggle={() => actions.onToggleStar(task)} />
      </div>
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
                {group.label} <span className="tasks-group__count tnum">{group.tasks.length}</span>
              </div>
              {group.tasks.map(renderRow)}
            </div>
          );
        })}
      </div>
    </>
  );
}
