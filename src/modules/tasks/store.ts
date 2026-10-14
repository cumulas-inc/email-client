import { create } from "zustand";

import type { TaskView } from "./taskUtils";

interface TasksState {
  /** Selected list; null until the user picks one (shows the default list). */
  view: TaskView | null;
  /** Task shown in the details pane. */
  selectedTaskId: string | null;
  /** Whether the "Completed" section is expanded. */
  showCompleted: boolean;
  setView: (view: TaskView) => void;
  selectTask: (id: string | null) => void;
  setShowCompleted: (show: boolean) => void;
}

/** UI state of the Tasks module, kept across module switches. */
export const useTasksStore = create<TasksState>((set) => ({
  view: null,
  selectedTaskId: null,
  showCompleted: false,
  setView: (view) => set({ view, selectedTaskId: null }),
  selectTask: (selectedTaskId) => set({ selectedTaskId }),
  setShowCompleted: (showCompleted) => set({ showCompleted }),
}));
