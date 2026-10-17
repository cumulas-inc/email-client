import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { lazy, Suspense, useEffect, useState } from "react";

import type { Backend } from "../api/backend";
import { BackendProvider } from "../api/BackendProvider";
import { useAccounts } from "../api/queries";
import { PaneSpinner } from "../components/Spinner";
import { Toaster } from "../components/Toaster";
import { MailModule } from "../modules/mail/MailModule";
import { Welcome } from "../modules/onboarding/Welcome";
import { AddAccountDialog } from "../modules/settings/AddAccountDialog";
import { useFeatureChangeToasts } from "../modules/settings/featureToasts";
import { SettingsDialog } from "../modules/settings/SettingsDialog";
import { AppHeader } from "./AppHeader";
import { QuitConfirmDialog } from "./QuitConfirmDialog";
import { StartAtLoginOffer } from "./StartAtLoginOffer";
import { type Module, useAppStore } from "./store";
import { useAppearance } from "./useAppearance";
import { useBackendEvents } from "./useBackendEvents";
import { useQuitRequests } from "./useQuit";
import { useTheme } from "./useTheme";

// Mail is the start screen; the other modules load on first use.
const CalendarModule = lazy(() =>
  import("../modules/calendar/CalendarModule").then((m) => ({ default: m.CalendarModule })),
);
const PeopleModule = lazy(() =>
  import("../modules/people/PeopleModule").then((m) => ({ default: m.PeopleModule })),
);
const TasksModule = lazy(() =>
  import("../modules/tasks/TasksModule").then((m) => ({ default: m.TasksModule })),
);

const MODULE_KEYS: Record<string, Module> = {
  "1": "mail",
  "2": "calendar",
  "3": "people",
  "4": "tasks",
};

// Ctrl+Q is Mail's Mark as read, as in Outlook; quitting is in the main
// menu and the tray.
function useGlobalShortcuts() {
  const setModule = useAppStore((s) => s.setModule);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.altKey || e.metaKey) return;
      const module = MODULE_KEYS[e.key];
      if (e.key === "," && !e.shiftKey) {
        e.preventDefault();
        useAppStore.getState().openSettings();
      } else if (module) {
        e.preventDefault();
        setModule(module);
      } else if (e.key === "k" || e.key === "e" || (e.shiftKey && e.key.toLowerCase() === "f")) {
        // Ctrl+K (or Ctrl+E) jumps to the list's search box. Calendar has
        // none, so it searches mail.
        e.preventDefault();
        const search = document.getElementById("global-search");
        if (search) search.focus();
        else {
          setModule("mail");
          requestAnimationFrame(() => document.getElementById("global-search")?.focus());
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setModule]);
}

function Shell() {
  useBackendEvents();
  useTheme();
  useAppearance();
  useGlobalShortcuts();
  useQuitRequests();
  useFeatureChangeToasts();
  const module = useAppStore((s) => s.module);
  const { data: accounts, isLoading } = useAccounts();

  let content;
  if (isLoading) content = <PaneSpinner />;
  else if (!accounts || accounts.length === 0) content = <Welcome />;
  else
    content = (
      <Suspense fallback={<PaneSpinner />}>
        {module === "mail" && <MailModule />}
        {module === "calendar" && <CalendarModule />}
        {module === "people" && <PeopleModule />}
        {module === "tasks" && <TasksModule />}
      </Suspense>
    );

  return (
    <div className="app">
      <AppHeader />
      <main className="app__main">{content}</main>
      <SettingsDialog />
      <AddAccountDialog />
      <QuitConfirmDialog />
      {/* Present before a card arrives, so that screen readers announce it. */}
      <div className="offer-anchor" aria-live="polite">
        <StartAtLoginOffer />
      </div>
      <Toaster />
    </div>
  );
}

export function App({ backend, queryClient }: { backend: Backend; queryClient?: QueryClient }) {
  const [client] = useState(
    () =>
      queryClient ??
      new QueryClient({
        defaultOptions: {
          queries: { staleTime: 30_000, refetchOnWindowFocus: false, retry: 1 },
          mutations: { retry: 0 },
        },
      }),
  );
  return (
    <BackendProvider backend={backend}>
      <QueryClientProvider client={client}>
        <Shell />
      </QueryClientProvider>
    </BackendProvider>
  );
}
