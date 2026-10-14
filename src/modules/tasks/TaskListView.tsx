import { ChevronRight, Plus, SearchX } from "lucide-react";
import { useId, useState, type FormEvent, type ReactNode, type Ref } from "react";

import type { Task } from "../../api/types";
import { EmptyState } from "../../components/EmptyState";
import { PaneSpinner } from "../../components/Spinner";
import { type TaskActions, type TaskGroup, TaskListbox } from "./TaskListbox";

interface TaskListViewProps extends TaskActions {
  title: string;
  subtitle: string | null;
  icon: ReactNode;
  /** Where "Add a task" saves to; null disables adding. */
  addTarget: { name: string } | null;
  /** Shown in the add box in smart lists, e.g. "Tasks · alex@…". */
  addHint: string | null;
  onAdd: (title: string) => Promise<unknown>;
  openGroups: TaskGroup[];
  completed: Task[];
  loading: boolean;
  search: string;
  empty: { title: string; description: string; icon: ReactNode };
  /** Offered when a search finds nothing in a single list. */
  onSearchAll: (() => void) | null;
  showCompleted: boolean;
  onShowCompleted: (show: boolean) => void;
  selectedId: string | null;
  listNames: Map<string, string> | null;
  now: Date;
  settling: Set<string>;
  listRef: Ref<HTMLDivElement>;
  addRef: Ref<HTMLInputElement>;
}

/** Centre pane: list title, "Add a task", open tasks and the Completed section. */
export function TaskListView(props: TaskListViewProps) {
  const {
    title,
    subtitle,
    icon,
    addTarget,
    addHint,
    onAdd,
    openGroups,
    completed,
    loading,
    search,
    empty,
    onSearchAll,
    showCompleted,
    onShowCompleted,
    selectedId,
    listNames,
    now,
    settling,
    listRef,
    addRef,
    ...actions
  } = props;
  const id = useId();
  const [draft, setDraft] = useState("");
  const openCount = openGroups.reduce((n, g) => n + g.tasks.length, 0);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (!text || !addTarget) return;
    setDraft("");
    onAdd(text).catch(() => setDraft((current) => current || text));
  };

  let body: ReactNode;
  if (loading) {
    body = <PaneSpinner />;
  } else if (openCount === 0) {
    body = search ? (
      <EmptyState
        icon={<SearchX size={40} strokeWidth={1.5} />}
        title="No matching tasks"
        description={`Nothing ${completed.length > 0 ? "open " : ""}matches “${search}” in ${title}.`}
        action={
          onSearchAll && (
            <button type="button" className="btn" onClick={onSearchAll}>
              Search all tasks
            </button>
          )
        }
      />
    ) : completed.length > 0 ? (
      <EmptyState
        icon={empty.icon}
        title="All done"
        description="Everything in this list is complete. Nice work."
      />
    ) : (
      <EmptyState icon={empty.icon} title={empty.title} description={empty.description} />
    );
  } else {
    body = (
      <TaskListbox
        label={title}
        groups={openGroups}
        selectedId={selectedId}
        listNames={listNames}
        now={now}
        settling={settling}
        listRef={listRef}
        {...actions}
      />
    );
  }

  return (
    <>
      <header className="tasks-header">
        <div className="tasks-header__row">
          <span className="tasks-header__icon" aria-hidden="true">
            {icon}
          </span>
          <h2 className="tasks-header__title">{title}</h2>
        </div>
        {subtitle && <p className="tasks-header__subtitle">{subtitle}</p>}
      </header>

      <form className="tasks-add" onSubmit={submit}>
        <span className="tasks-add__icon" aria-hidden="true">
          <Plus size={20} />
        </span>
        <input
          ref={addRef}
          className="tasks-add__input"
          placeholder={addTarget ? "Add a task" : "No list to add tasks to"}
          aria-label="Add a task"
          aria-describedby={addHint ? `${id}-hint` : undefined}
          disabled={!addTarget}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape" && draft) {
              e.preventDefault();
              setDraft("");
            }
          }}
        />
        {addHint && (
          <span id={`${id}-hint`} className="tasks-add__hint">
            {addHint}
          </span>
        )}
        {draft.trim() && (
          <button type="submit" className="btn btn--sm">
            Add
          </button>
        )}
      </form>

      <div className="tasks-scroll">
        {search && openCount > 0 && (
          <p className="tasks-search-note" role="status">
            Tasks matching “{search}”
          </p>
        )}
        {body}
        {!loading && completed.length > 0 && (
          <div className="tasks-completed">
            <button
              type="button"
              className="tasks-completed__toggle"
              aria-expanded={showCompleted}
              aria-controls={showCompleted ? `${id}-completed` : undefined}
              onClick={() => onShowCompleted(!showCompleted)}
            >
              <ChevronRight size={16} className="tasks-completed__chevron" aria-hidden="true" />
              Completed <span className="tasks-completed__count">{completed.length}</span>
            </button>
            {showCompleted && (
              <TaskListbox
                id={`${id}-completed`}
                label="Completed tasks"
                groups={[{ key: "completed", label: null, tasks: completed }]}
                selectedId={selectedId}
                listNames={listNames}
                now={now}
                {...actions}
              />
            )}
          </div>
        )}
      </div>
    </>
  );
}
