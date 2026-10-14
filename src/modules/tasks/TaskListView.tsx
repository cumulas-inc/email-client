import { ChevronRight, Plus, SearchX } from "lucide-react";
import { useId, useState, type FormEvent, type ReactNode, type Ref } from "react";

import type { Task } from "../../api/types";
import { EmptyState } from "../../components/EmptyState";
import { SearchField } from "../../components/SearchField";
import { PaneSpinner } from "../../components/Spinner";
import { Count } from "./Count";
import { titleWhenCut } from "./fullText";
import { type ListTag, type TaskActions, type TaskGroup, TaskListbox } from "./TaskListbox";

/** Under the title: the list's account ("Alex Morgan · alex@…"), or "Across all your accounts". */
export interface ListSubtitle {
  name: string;
  email?: string;
  color?: string;
}

interface TaskListViewProps extends TaskActions {
  title: string;
  subtitle: ListSubtitle | null;
  /** Where "New task" saves to; null disables adding. */
  addTarget: { name: string } | null;
  /** Shown in the add box in smart lists: "Adds to Tasks · alex@…". */
  addHint: { list: string; email: string | null } | null;
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
  listTags: Map<string, ListTag> | null;
  now: Date;
  settling: Set<string>;
  listRef: Ref<HTMLDivElement>;
  addRef: Ref<HTMLInputElement>;
}

/** Center card: search, list title, "New task", open tasks and the Completed section. */
export function TaskListView(props: TaskListViewProps) {
  const {
    title,
    subtitle,
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
    listTags,
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
        description={
          <>
            Nothing {completed.length > 0 ? "open " : ""}matches “<bdi>{search}</bdi>” in{" "}
            <bdi>{title}</bdi>.
          </>
        }
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
        description="Everything here is complete. Type a task above to add another."
      />
    ) : (
      <EmptyState icon={empty.icon} title={empty.title} description={empty.description} />
    );
  } else {
    body = (
      <TaskListbox
        label={title}
        kind="open"
        groups={openGroups}
        selectedId={selectedId}
        listTags={listTags}
        now={now}
        settling={settling}
        listRef={listRef}
        {...actions}
      />
    );
  }

  return (
    <>
      {/* Search first, at the top of the card, where Ctrl+K lands in Mail and Contacts too. */}
      <header className="tasks-header">
        <SearchField placeholder="Search tasks" />
        <div className="tasks-header__titles">
          {/* Long list names wrap to a second line, then clamp; the tooltip has the rest. */}
          <h2 className="pane-title tasks-header__title" dir="auto" onMouseEnter={titleWhenCut}>
            {title}
          </h2>
          {subtitle && (
            <p className="tasks-header__subtitle">
              {subtitle.color && (
                <span
                  className="tasks-header__dot"
                  style={{ background: subtitle.color }}
                  aria-hidden="true"
                />
              )}
              <span className="tasks-header__subtitle-name" dir="auto" onMouseEnter={titleWhenCut}>
                {subtitle.name}
              </span>
              {subtitle.email && (
                <>
                  {/* Spaces keep the words apart for screen readers; flex ignores them. */}{" "}
                  <span className="tasks-header__subtitle-sep" aria-hidden="true">
                    ·
                  </span>{" "}
                  <span className="tasks-header__subtitle-email" onMouseEnter={titleWhenCut}>
                    {subtitle.email}
                  </span>
                </>
              )}
            </p>
          )}
        </div>
      </header>

      <form className="tasks-add" onSubmit={submit}>
        {/* The whole field is a label, so a click anywhere in it starts typing. */}
        <label className="tasks-add__field">
          <span className="tasks-add__icon" aria-hidden="true">
            <Plus size={18} strokeWidth={2} />
          </span>
          <input
            ref={addRef}
            className="tasks-add__input"
            dir="auto"
            placeholder={addTarget ? "New task" : "No list to add tasks to"}
            aria-label="New task"
            aria-describedby={addHint ? `${id}-hint` : undefined}
            aria-keyshortcuts="Control+N"
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
            <span id={`${id}-hint`} className="tasks-add__hint" onMouseEnter={titleWhenCut}>
              Adds to <bdi>{addHint.list}</bdi>
              {addHint.email && ` · ${addHint.email}`}
            </span>
          )}
          <span className="kbd tasks-add__enter" aria-hidden="true">
            Enter
          </span>
        </label>
      </form>

      <div className="tasks-scroll">
        {search && openCount > 0 && (
          <p className="tasks-search-note" role="status">
            Tasks matching “<bdi>{search}</bdi>”
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
              <ChevronRight
                size={14}
                strokeWidth={2}
                className="tasks-completed__chevron"
                aria-hidden="true"
              />
              Completed <Count n={completed.length} className="tasks-completed__count" />
            </button>
            {showCompleted && (
              <TaskListbox
                id={`${id}-completed`}
                label="Completed tasks"
                kind="completed"
                groups={[{ key: "completed", label: null, tasks: completed }]}
                selectedId={selectedId}
                listTags={listTags}
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
