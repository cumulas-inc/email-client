import { create } from "zustand";

import { errorMessage } from "../api/errors";

export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  message: string;
  action?: { label: string; onClick: () => void };
}

interface ToastState {
  toasts: Toast[];
  push: (toast: Omit<Toast, "id">) => void;
  dismiss: (id: number) => void;
}

let nextId = 1;

export const useToasts = create<ToastState>((set, get) => ({
  toasts: [],
  push: (toast) => {
    const id = nextId++;
    set({ toasts: [...get().toasts.slice(-3), { ...toast, id }] });
    setTimeout(() => get().dismiss(id), toast.kind === "error" ? 8000 : 4000);
  },
  dismiss: (id) => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
}));

/** Shortcuts usable outside React components. */
export const toast = {
  info: (message: string) => useToasts.getState().push({ kind: "info", message }),
  success: (message: string) => useToasts.getState().push({ kind: "success", message }),
  error: (e: unknown) => useToasts.getState().push({ kind: "error", message: errorMessage(e) }),
};
