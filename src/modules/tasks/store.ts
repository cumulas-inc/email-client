import { create } from "zustand";

import type { TaskView } from "./taskUtils";

interface TasksState {
  /** Selected list; null until the user picks one (shows the default list). */
  view: TaskView | null;
  /** Task shown in the details pane. */
  selectedTaskId: string | null;
  /** Whether the "Completed" section is expanded. */
  showCompleted: boolean;
  /** The selected task's details take the keyboard once shown (it was opened from elsewhere). */
  focusDetails: boolean;
  setView: (view: TaskView) => void;
  selectTask: (id: string | null, options?: { focus?: boolean }) => void;
  setShowCompleted: (show: boolean) => void;
  detailsFocused: () => void;
}

/** UI state of the Tasks module, kept across module switches. */
export const useTasksStore = create<TasksState>((set) => ({
  view: null,
  selectedTaskId: null,
  showCompleted: false,
  focusDetails: false,
  setView: (view) => set({ view, selectedTaskId: null, focusDetails: false }),
  selectTask: (selectedTaskId, options) =>
    set({ selectedTaskId, focusDetails: options?.focus ?? false }),
  setShowCompleted: (showCompleted) => set({ showCompleted }),
  detailsFocused: () => set({ focusDetails: false }),
}));
