// React Query hooks for task lists and tasks. Keys come from `qk` and start
// with the "tasks" DataKind, so backend data-changed events refresh them.
// Updates are applied optimistically to every cached task list and rolled
// back if the backend rejects them.

import {
  type Query,
  type QueryClient,
  type QueryKey,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import { useBackend } from "../../api/context";
import { qk } from "../../api/queries";
import type { Task, TaskDraft, TaskPatch } from "../../api/types";
import { toast } from "../../components/toast";
import { applyTaskPatch } from "./taskUtils";

export function useTaskLists() {
  const backend = useBackend();
  return useQuery({ queryKey: qk.taskLists(), queryFn: () => backend.listTaskLists() });
}

/** Tasks of one list, or of every list when `listId` is null. */
export function useTasks(listId: string | null, includeCompleted: boolean) {
  const backend = useBackend();
  const client = useQueryClient();
  const allKey = qk.tasks(null, true);
  return useQuery({
    queryKey: qk.tasks(listId, includeCompleted),
    queryFn: () => backend.listTasks(listId, includeCompleted),
    // A list shows its tasks from the all-tasks cache (same order) while it loads.
    placeholderData: () => {
      if (listId === null) return undefined;
      return client
        .getQueryData<Task[]>(allKey)
        ?.filter((t) => t.listId === listId && (includeCompleted || t.status !== "completed"));
    },
  });
}

/** Every cached task array (all `qk.tasks(...)` queries). */
const taskItems = {
  queryKey: qk.root("tasks"),
  predicate: (q: Query) => q.queryKey[1] === qk.tasks(null, false)[1],
};

/** Whether a cached task query (by key) would contain tasks of `listId`. */
function keyCoversList(key: readonly unknown[], listId: string): boolean {
  const scope = key[2];
  return scope === "all" || scope === listId;
}

/** Mutations that write tasks; caches are refreshed once the last one settles. */
const WRITE_KEY = [...qk.root("tasks"), "write"];

/** The cached copies of one task, so a failed update can put them back. */
type SavedTask = { key: QueryKey; task: Task; index: number }[];

function saveTask(client: QueryClient, id: string): SavedTask {
  const saved: SavedTask = [];
  for (const [key, data] of client.getQueriesData<Task[]>(taskItems)) {
    const index = data?.findIndex((t) => t.id === id) ?? -1;
    if (data && index >= 0) saved.push({ key, task: data[index]!, index });
  }
  return saved;
}

/** Restores only the saved task, leaving other (concurrent) changes alone. */
function restoreTask(client: QueryClient, saved: SavedTask) {
  for (const { key, task, index } of saved) {
    client.setQueryData<Task[]>(key, (old) => {
      if (!old) return old;
      const next = [...old];
      const at = next.findIndex((t) => t.id === task.id);
      if (at >= 0) next[at] = task;
      else next.splice(Math.min(index, next.length), 0, task);
      return next;
    });
  }
}

/**
 * Refreshes task caches after a write, unless other writes are still in
 * flight: refetching then would briefly undo their optimistic changes.
 */
function refreshAfterWrite(client: QueryClient) {
  if (client.isMutating({ mutationKey: WRITE_KEY }) > 1) return;
  return client.invalidateQueries({ queryKey: qk.root("tasks") });
}

export function useCreateTask() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationKey: WRITE_KEY,
    mutationFn: (draft: TaskDraft) => backend.createTask(draft),
    onSuccess: (task) => {
      // Show the new task straight away, at the top of its list.
      for (const query of client.getQueryCache().findAll(taskItems)) {
        if (!keyCoversList(query.queryKey, task.listId)) continue;
        client.setQueryData<Task[]>(query.queryKey, (old) =>
          old && !old.some((t) => t.id === task.id) ? [task, ...old] : old,
        );
      }
    },
    onSettled: () => refreshAfterWrite(client),
  });
}

export interface TaskUpdate {
  id: string;
  patch: TaskPatch;
}

export function useUpdateTask() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationKey: WRITE_KEY,
    mutationFn: ({ id, patch }: TaskUpdate) => backend.updateTask(id, patch),
    onMutate: async ({ id, patch }: TaskUpdate) => {
      await client.cancelQueries(taskItems);
      const saved = saveTask(client, id);
      const now = new Date();
      client.setQueriesData<Task[]>(taskItems, (old) =>
        old?.map((t) => (t.id === id ? applyTaskPatch(t, patch, now) : t)),
      );
      return { saved };
    },
    onError: (error, _vars, context) => {
      if (context) restoreTask(client, context.saved);
      toast.error(error);
    },
    onSettled: () => refreshAfterWrite(client),
  });
}

export function useDeleteTask() {
  const backend = useBackend();
  const client = useQueryClient();
  return useMutation({
    mutationKey: WRITE_KEY,
    mutationFn: (id: string) => backend.deleteTask(id),
    onMutate: async (id: string) => {
      await client.cancelQueries(taskItems);
      const saved = saveTask(client, id);
      client.setQueriesData<Task[]>(taskItems, (old) => old?.filter((t) => t.id !== id));
      return { saved };
    },
    onError: (error, _id, context) => {
      if (context) restoreTask(client, context.saved);
      toast.error(error);
    },
    onSettled: () => refreshAfterWrite(client),
  });
}
