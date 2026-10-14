import { CalendarClock, CircleCheckBig, ListChecks, Plus, Star } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { errorMessage } from "../../api/errors";
import { useAccounts } from "../../api/queries";
import type { Account, Task, TaskList, TaskPatch } from "../../api/types";
import { ModuleSidebar } from "../../app/ModuleSidebar";
import { useAppStore } from "../../app/store";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { EmptyState } from "../../components/EmptyState";
import { PaneSpinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { useCreateTask, useDeleteTask, useTaskLists, useTasks, useUpdateTask } from "./queries";
import { useTasksStore } from "./store";
import { TaskDetails, TaskDetailsPane } from "./TaskDetails";
import type { ListTag, TaskGroup } from "./TaskListbox";
import { type ListSubtitle, TaskListView } from "./TaskListView";
import { TaskSidebar } from "./TaskSidebar";
import {
  DUE_GROUPS,
  SETTLE_MS,
  SMART_LISTS,
  type TaskView,
  defaultTaskList,
  dueGroup,
  inView,
  isCompleted,
  matchesSearch,
  sortTaskLists,
  toDateString,
} from "./taskUtils";
import "./tasks.css";

const EMPTY_TEXT = {
  starred: {
    title: "No starred tasks",
    description: "Star a task in any list to keep it here.",
    icon: <Star size={40} strokeWidth={1.5} />,
  },
  scheduled: {
    title: "Nothing scheduled",
    description: "Give a task a due date and it shows up here, grouped by when it's due.",
    icon: <CalendarClock size={40} strokeWidth={1.5} />,
  },
  all: {
    title: "No tasks yet",
    description: "Type a task above and press Enter to add it.",
    icon: <CircleCheckBig size={40} strokeWidth={1.5} />,
  },
  list: {
    title: "No tasks yet",
    description: "Type a task above and press Enter to add it.",
    icon: <CircleCheckBig size={40} strokeWidth={1.5} />,
  },
};

/** List tags for rows in smart lists; names that repeat get their account's name. */
function listTagsFor(lists: TaskList[], accountOf: (id: string) => Account | undefined) {
  const repeated = new Set(
    lists.map((l) => l.name).filter((name, i, all) => all.indexOf(name) !== i),
  );
  return new Map<string, ListTag>(
    lists.map((l) => {
      const account = accountOf(l.accountId);
      return [
        l.id,
        {
          name: repeated.has(l.name) ? `${l.name} · ${account?.displayName ?? ""}` : l.name,
          color: account?.color ?? "var(--fg-tertiary)",
        },
      ];
    }),
  );
}

export function TasksModule() {
  const { data: accounts = [] } = useAccounts();
  const listsQuery = useTaskLists();
  const lists = useMemo(
    () => sortTaskLists(listsQuery.data ?? [], accounts),
    [listsQuery.data, accounts],
  );
  const {
    view: storedView,
    selectedTaskId,
    showCompleted,
    setView,
    selectTask,
    setShowCompleted,
  } = useTasksStore();
  const search = useAppStore((s) => (s.module === "tasks" ? s.search.trim() : ""));
  const now = new Date();

  // The details card slides in when a task is picked, but not when Tasks
  // opens (switching modules) with a task already showing.
  const [detailsOpenAtStart, setDetailsOpenAtStart] = useState(selectedTaskId !== null);
  if (detailsOpenAtStart && selectedTaskId === null) setDetailsOpenAtStart(false);

  const defaultList = defaultTaskList(lists, accounts);
  // The remembered list may have gone (account removed): fall back to the default list.
  const view: TaskView =
    storedView && (storedView.kind === "smart" || lists.some((l) => l.id === storedView.id))
      ? storedView
      : defaultList
        ? { kind: "list", id: defaultList.id }
        : { kind: "smart", id: "all" };
  const currentList = view.kind === "list" ? lists.find((l) => l.id === view.id) : undefined;

  const allQuery = useTasks(null, true);
  const viewQuery = useTasks(view.kind === "list" ? view.id : null, true);

  /** Tasks just checked off, by the place they had in the open list. */
  const [settling, setSettling] = useState<ReadonlyMap<string, number>>(() => new Map());
  const [toDelete, setToDelete] = useState<Task | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const addRef = useRef<HTMLInputElement>(null);
  const titleRef = useRef<HTMLTextAreaElement>(null);
  const returnFocus = useRef<HTMLElement | null>(null);

  const create = useCreateTask();
  const update = useUpdateTask();
  const remove = useDeleteTask();

  const accountOf = (id: string) => accounts.find((a) => a.id === id);

  // --- derived lists ---------------------------------------------------------------

  const counts = new Map<string, number>();
  const bump = (key: string) => counts.set(key, (counts.get(key) ?? 0) + 1);
  for (const t of allQuery.data ?? []) {
    if (isCompleted(t)) continue;
    bump(t.listId);
    bump("all");
    if (t.importance === "high") bump("starred");
    if (t.dueDate) bump("scheduled");
  }

  const matching = (viewQuery.data ?? [])
    .filter((t) => inView(t, view))
    .filter((t) => matchesSearch(t, search));
  // A task being checked off keeps its place until it leaves, even when a
  // refresh has already sorted it after the open tasks.
  const open = matching.filter((t) => !isCompleted(t) && !settling.has(t.id));
  for (const [id, at] of [...settling].sort((a, b) => a[1] - b[1])) {
    const task = matching.find((t) => t.id === id);
    if (task) open.splice(Math.min(at, open.length), 0, task);
  }
  const completed = matching
    .filter((t) => isCompleted(t) && !settling.has(t.id))
    .sort((a, b) => (b.completedAt ?? "").localeCompare(a.completedAt ?? ""));

  const openGroups: TaskGroup[] =
    view.kind === "smart" && view.id === "scheduled"
      ? DUE_GROUPS.map((g) => ({
          key: g.id,
          label: g.label,
          tasks: open.filter((t) => t.dueDate && dueGroup(t.dueDate, now) === g.id),
        })).filter((g) => g.tasks.length > 0)
      : [{ key: "open", label: null, tasks: open }];

  const listTags = view.kind === "smart" ? listTagsFor(lists, accountOf) : null;

  const selectedTask =
    (viewQuery.data ?? []).find((t) => t.id === selectedTaskId) ??
    (allQuery.data ?? []).find((t) => t.id === selectedTaskId) ??
    null;

  // --- header & add box ----------------------------------------------------------------

  const smart = view.kind === "smart" ? SMART_LISTS.find((s) => s.id === view.id) : undefined;
  const title = smart?.label ?? currentList?.name ?? "Tasks";
  const listAccount = currentList ? accountOf(currentList.accountId) : undefined;
  const subtitle: ListSubtitle | null = smart
    ? accounts.length > 1
      ? { name: "Across all your accounts" }
      : null
    : listAccount
      ? { name: listAccount.displayName, email: listAccount.email, color: listAccount.color }
      : null;
  const addTarget = view.kind === "list" ? (currentList ?? null) : defaultList;
  const addHint =
    view.kind === "smart" && addTarget
      ? {
          list: addTarget.name,
          email: accounts.length > 1 ? (accountOf(addTarget.accountId)?.email ?? null) : null,
        }
      : null;

  // --- actions -----------------------------------------------------------------------

  const patch = (task: Task, p: TaskPatch) => update.mutate({ id: task.id, patch: p });

  const stopSettling = (id: string) =>
    setSettling((s) => {
      if (!s.has(id)) return s;
      const next = new Map(s);
      next.delete(id);
      return next;
    });

  const toggleComplete = (task: Task) => {
    const completing = !isCompleted(task);
    if (completing) {
      // Let the check mark animate in place before the task moves to Completed.
      const at = open.findIndex((t) => t.id === task.id);
      setSettling((s) => new Map(s).set(task.id, at >= 0 ? at : open.length));
      setTimeout(() => stopSettling(task.id), SETTLE_MS);
    } else {
      // Unchecked again before it left: it stays, at full strength.
      stopSettling(task.id);
    }
    patch(task, { status: completing ? "completed" : "notStarted" });
  };

  const toggleStar = (task: Task) =>
    patch(task, { importance: task.importance === "high" ? "normal" : "high" });

  const addTask = (text: string) => {
    if (!addTarget) return Promise.reject(new Error("No task list"));
    return create
      .mutateAsync({
        listId: addTarget.id,
        title: text,
        importance: view.kind === "smart" && view.id === "starred" ? "high" : null,
        dueDate: view.kind === "smart" && view.id === "scheduled" ? toDateString(now) : null,
      })
      .catch((e: unknown) => {
        toast.error(e);
        throw e;
      });
  };

  const askDelete = (task: Task) => {
    returnFocus.current = document.activeElement as HTMLElement | null;
    setToDelete(task);
  };

  const refocus = (el: HTMLElement | null | undefined) => {
    if (el) setTimeout(() => el.isConnected && el.focus(), 0);
  };

  /** The task that takes `task`'s place in its section once it's gone. */
  const neighborOf = (task: Task): Task | null => {
    const section = [open.filter((t) => !isCompleted(t)), completed].find((tasks) =>
      tasks.some((t) => t.id === task.id),
    );
    if (!section) return null;
    const i = section.findIndex((t) => t.id === task.id);
    return section[i + 1] ?? section[i - 1] ?? null;
  };

  const confirmDelete = () => {
    const task = toDelete;
    if (!task) return;
    setToDelete(null);
    // Like deleting a message, the next task opens, so the keyboard keeps its place.
    if (selectedTaskId === task.id) selectTask(neighborOf(task)?.id ?? null);
    // Isolate the title so a right-to-left one doesn't reorder the sentence.
    remove.mutate(task.id, {
      onSuccess: () => toast.success(`“\u2068${task.title}\u2069” was deleted`),
    });
    refocus(listRef.current ?? addRef.current);
  };

  const closeDetails = () => {
    selectTask(null);
    refocus(listRef.current ?? addRef.current);
  };

  const focusAdd = () => addRef.current?.focus();

  // Ctrl+N starts a new task, as it starts a message in Mail.
  const canAdd = addTarget !== null;
  useEffect(() => {
    if (!canAdd) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || document.querySelector("dialog[open]")) return;
      if (e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        addRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [canAdd]);

  // --- render ------------------------------------------------------------------------

  if (listsQuery.isLoading) return <PaneSpinner />;
  if (listsQuery.isError) {
    return (
      <EmptyState
        title="Couldn't load your task lists"
        description={errorMessage(listsQuery.error)}
        action={
          <button type="button" className="btn" onClick={() => void listsQuery.refetch()}>
            Try again
          </button>
        }
      />
    );
  }

  const emptyText =
    view.kind === "smart" ? EMPTY_TEXT[view.id] : lists.length === 0 ? null : EMPTY_TEXT.list;

  return (
    <div className="tasks module-layout">
      <ModuleSidebar
        label="Task lists"
        className="tasks__sidebar"
        action={
          <button
            type="button"
            className="btn btn--primary"
            onClick={focusAdd}
            disabled={!canAdd}
            aria-keyshortcuts="Control+N"
          >
            <Plus size={18} strokeWidth={1.75} aria-hidden="true" />
            New task
            <span className="kbd" aria-hidden="true">
              Ctrl N
            </span>
          </button>
        }
      >
        <TaskSidebar
          accounts={accounts}
          lists={lists}
          view={view}
          counts={counts}
          onSelect={(v) => setView(v)}
        />
      </ModuleSidebar>

      <section className="tasks__main card" aria-label={title}>
        <TaskListView
          title={title}
          subtitle={subtitle}
          addTarget={addTarget}
          addHint={addHint}
          onAdd={addTask}
          openGroups={openGroups}
          completed={completed}
          loading={viewQuery.isLoading}
          search={search}
          empty={
            emptyText ?? {
              title: "No task lists yet",
              description: "Task lists from your accounts show up here once they've synced.",
              icon: <ListChecks size={40} strokeWidth={1.5} />,
            }
          }
          onSearchAll={
            view.kind === "smart" && view.id === "all"
              ? null
              : () => setView({ kind: "smart", id: "all" })
          }
          showCompleted={showCompleted}
          onShowCompleted={setShowCompleted}
          selectedId={selectedTask?.id ?? null}
          listTags={listTags}
          now={now}
          settling={new Set(settling.keys())}
          listRef={listRef}
          addRef={addRef}
          onSelect={(t) => selectTask(t.id)}
          onToggleComplete={toggleComplete}
          onToggleStar={toggleStar}
          onDelete={askDelete}
          onOpen={() => titleRef.current?.focus()}
        />
      </section>

      {selectedTask && (
        <TaskDetailsPane enter={!detailsOpenAtStart} onClose={closeDetails}>
          <TaskDetails
            key={selectedTask.id}
            task={selectedTask}
            list={lists.find((l) => l.id === selectedTask.listId)}
            account={accountOf(selectedTask.accountId)}
            now={now}
            titleRef={titleRef}
            onUpdate={(p) => patch(selectedTask, p)}
            onToggleComplete={() => toggleComplete(selectedTask)}
            onDelete={() => askDelete(selectedTask)}
            onClose={closeDetails}
          />
        </TaskDetailsPane>
      )}

      <ConfirmDialog
        open={toDelete !== null}
        title="Delete task?"
        confirmLabel="Delete task"
        message={
          toDelete ? (
            <>
              <strong>
                “<bdi>{toDelete.title}</bdi>”
              </strong>{" "}
              will be permanently deleted.
            </>
          ) : null
        }
        onConfirm={confirmDelete}
        onCancel={() => {
          setToDelete(null);
          refocus(returnFocus.current);
        }}
      />
    </div>
  );
}
