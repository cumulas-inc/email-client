// The header's bell and Reminders panel, against the demo backend at a
// fixed time: Wednesday, October 14, 2026, 10:24 AM, when the demo's
// Focus time (10:30, reminding 15 minutes before) and Security review
// (11:00, an hour before, online) are due.

import { QueryClient } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend } from "../api/mock";
import { useToasts } from "../components/toast";
import { setLocaleForTesting } from "../lib/datetime";
import { resetMailStore, useMailStore } from "../modules/mail/mailStore";
import { useTasksStore } from "../modules/tasks/store";
import { loadLazyModules } from "../test/lazyModules";
import { App } from "./App";
import { useReminderAnnouncer } from "./reminders";
import { useAppStore } from "./store";

const NOW = new Date(2026, 9, 14, 10, 24);

function renderApp(backend = new MockBackend()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<App backend={backend} queryClient={client} />);
  return { backend, client };
}

const bell = () => screen.findByRole("button", { name: /^Reminders/ });
const panel = () => screen.getByRole("dialog", { name: "Reminders" });
const row = (title: string) =>
  within(panel()).getByRole("button", { name: title }).closest("li") as HTMLElement;

beforeAll(() => setLocaleForTesting("en-US"));
beforeAll(loadLazyModules, 60_000);

beforeEach(() => {
  // Only the clock: timers stay real for the queries and user events.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  resetMailStore();
  useAppStore.setState({
    module: "mail",
    search: "",
    settingsOpen: null,
    remindersOpen: false,
    calendarRequest: null,
    startAtLoginOffer: false,
  });
  useToasts.setState({ toasts: [] });
  useReminderAnnouncer.setState({ text: "", seq: 0 });
  window.localStorage.clear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("the bell", () => {
  it("says how many reminders are due", async () => {
    renderApp(new MockBackend({ overdueReminder: true }));
    const button = await screen.findByRole("button", { name: "Reminders, 3 due" });
    expect(button).toHaveAttribute("aria-haspopup", "dialog");
    expect(button).toHaveAttribute("aria-expanded", "false");
    expect(button).toHaveTextContent("3");
  });

  it("has no count when nothing is due", async () => {
    const backend = new MockBackend();
    await backend.dismissReminders((await backend.listActiveReminders()).map((r) => r.key));
    renderApp(backend);
    const button = await screen.findByRole("button", { name: "Reminders" });
    expect(button).not.toHaveTextContent(/\d/);
    await userEvent.setup().click(button);
    expect(within(panel()).getByText("No reminders")).toBeInTheDocument();
  });
});

describe("the Reminders panel", () => {
  it("lists what's due, soonest first, with the time and where", async () => {
    const user = userEvent.setup();
    renderApp(new MockBackend({ overdueReminder: true }));
    await user.click(await bell());
    expect(await bell()).toHaveAttribute("aria-expanded", "true");
    const items = within(panel()).getAllByRole("listitem");
    expect(items.map((i) => within(i).getAllByRole("button")[0]!.textContent)).toEqual([
      "Send Q3 metrics to Dana",
      "Focus time",
      "Security review",
    ]);
    expect(row("Send Q3 metrics to Dana")).toHaveTextContent("12 min overdue");
    expect(row("Focus time")).toHaveTextContent("in 6 min · 10:30 AM – 12:00 PM");
    expect(row("Security review")).toHaveTextContent("in 36 min · 11:00 AM – 12:00 PM · Room 3.01");
    // Join for the online meeting only.
    expect(
      within(row("Security review")).getByRole("button", { name: "Join Security review" }),
    ).toBeInTheDocument();
    expect(within(row("Focus time")).queryByRole("button", { name: /^Join/ })).toBeNull();
    expect(within(panel()).getByText("3 due")).toBeInTheDocument();
  });

  it("snoozes from the menu", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const snooze = vi.spyOn(backend, "snoozeReminder");
    renderApp(backend);
    await user.click(await bell());
    await user.click(within(row("Focus time")).getByRole("button", { name: "Snooze Focus time" }));
    const menu = screen.getByRole("menu", { name: "Snooze Focus time" });
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent),
    ).toEqual([
      "5 minutes",
      "10 minutes",
      "15 minutes",
      "30 minutes",
      "1 hour",
      "1 day",
      "5 minutes before start",
    ]);
    // Focus time starts in six minutes: five before is a minute away.
    await user.click(within(menu).getByRole("menuitem", { name: "10 minutes" }));
    await waitFor(() => expect(snooze).toHaveBeenCalledTimes(1));
    expect(snooze.mock.calls[0]![1]).toEqual({ minutes: 10 });
    expect(await bell()).toHaveAccessibleName("Reminders, 1 due");
    // Snoozed ones wait below, with when they come back.
    await waitFor(() =>
      expect(within(panel()).getByRole("list", { name: "Snoozed" })).toHaveTextContent(
        "Snoozed until 10:34 AM",
      ),
    );

    // Snoozing until shortly before the start.
    await user.click(
      within(row("Security review")).getByRole("button", { name: "Snooze Security review" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "5 minutes before start" }));
    await waitFor(() => expect(snooze).toHaveBeenCalledTimes(2));
    expect(snooze.mock.calls[1]![1]).toEqual({
      until: new Date(2026, 9, 14, 10, 55).toISOString(),
    });
  });

  it("dismisses one, then all", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ overdueReminder: true });
    const dismiss = vi.spyOn(backend, "dismissReminders");
    renderApp(backend);
    await user.click(await bell());
    const focus = within(row("Focus time"));
    await user.click(focus.getByRole("button", { name: "Dismiss Focus time" }));
    await waitFor(() => expect(within(panel()).queryByText("Focus time")).toBeNull());
    expect(dismiss).toHaveBeenCalledWith([expect.stringMatching(/^event:/)]);
    // The keyboard stays in the panel.
    expect(panel()).toHaveFocus();
    expect(await bell()).toHaveAccessibleName("Reminders, 2 due");

    await user.click(within(panel()).getByRole("button", { name: "Dismiss all" }));
    expect(await within(panel()).findByText("No reminders")).toBeInTheDocument();
    expect(dismiss.mock.calls[1]![0]).toHaveLength(2);
    expect(await bell()).toHaveAccessibleName("Reminders");
  });

  it("joins the meeting in the browser", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const open = vi.spyOn(backend, "openLink");
    renderApp(backend);
    await user.click(await bell());
    await user.click(within(panel()).getByRole("button", { name: "Join Security review" }));
    expect(open).toHaveBeenCalledWith("https://meet.example/security-review");
  });

  it("closes with Escape, giving the focus back to the bell", async () => {
    const user = userEvent.setup();
    renderApp();
    await user.click(await bell());
    expect(panel()).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Reminders" })).toBeNull();
    expect(await bell()).toHaveFocus();

    // Back on the bell, with the panel still open, too.
    await user.keyboard("{Enter}");
    expect(panel()).toHaveFocus();
    await user.tab({ shift: true });
    expect(await bell()).toHaveFocus();
    expect(panel()).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Reminders" })).toBeNull();
    expect(await bell()).toHaveFocus();
  });

  it("closes when the focus leaves the bell and the panel, not the window", async () => {
    const user = userEvent.setup();
    renderApp();
    await user.click(await bell());
    expect(panel()).toHaveFocus();
    // Another window takes the focus, and gives it back here.
    fireEvent.focusOut(panel(), { relatedTarget: null });
    await new Promise((r) => setTimeout(r, 20));
    expect(panel()).toHaveFocus();

    // Shift+Tab to the bell, and on off it, to nothing.
    await user.tab({ shift: true });
    expect(await bell()).toHaveFocus();
    expect(panel()).toBeInTheDocument();
    await user.tab({ shift: true });
    expect(document.body).toHaveFocus();
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Reminders" })).toBeNull());
    expect(await bell()).toHaveAttribute("aria-expanded", "false");
  });

  it("keeps the keys pressed in it from Mail and Calendar", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    renderApp(backend);
    await user.click(
      await screen.findByRole("option", { name: /Atlas launch checklist — final review today/ }),
    );
    const selected = useMailStore.getState().activeId;
    expect(selected).toBeTruthy();
    await waitFor(async () => expect((await backend.getMessage(selected!)).isRead).toBe(true));
    const deleted = vi.spyOn(backend, "deleteMessages");
    const moved = vi.spyOn(backend, "moveMessages");
    const read = vi.spyOn(backend, "setRead");
    await user.click(await bell());
    expect(panel()).toHaveFocus();
    // Delete (which dismisses notifications elsewhere), Q, U and J.
    await user.keyboard("{Delete}qu");
    within(row("Focus time")).getByRole("button", { name: "Dismiss Focus time" }).focus();
    await user.keyboard("{Delete}j");
    await new Promise((r) => setTimeout(r, 50));
    expect(deleted).not.toHaveBeenCalled();
    expect(moved).not.toHaveBeenCalled();
    expect(read).not.toHaveBeenCalled();
    expect(useMailStore.getState().activeId).toBe(selected);

    await user.keyboard("{Escape}");
    act(() => useAppStore.getState().setModule("calendar"));
    // Calendar's first render takes about a second here, longer on a busy
    // CI runner, so this wait gets more than the usual 4 s.
    const heading = await screen.findByRole("heading", { level: 1 }, { timeout: 10_000 });
    expect(heading).toHaveTextContent("October 12 – 16, 2026");
    await user.click(await bell());
    expect(panel()).toHaveFocus();
    await user.keyboard("{PageDown}{Alt>}{ArrowRight}{/Alt}");
    expect(heading).toHaveTextContent("October 12 – 16, 2026");
  });

  it("opens an event in Calendar and a task in Tasks, with the keyboard there", async () => {
    const user = userEvent.setup();
    renderApp(new MockBackend({ overdueReminder: true }));
    await user.click(await bell());
    await user.click(within(panel()).getByRole("button", { name: "Security review" }));
    expect(screen.queryByRole("dialog", { name: "Reminders" })).toBeNull();
    expect(useAppStore.getState().module).toBe("calendar");
    const details = await screen.findByRole("dialog", { name: "Security review" });
    expect(details).toHaveFocus();
    // Closed, the details give the keyboard to the event.
    await user.keyboard("{Escape}");
    expect(details).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Security review,/ })).toHaveFocus();

    await user.click(await bell());
    await user.click(within(panel()).getByRole("button", { name: "Send Q3 metrics to Dana" }));
    await waitFor(() => expect(useAppStore.getState().module).toBe("tasks"));
    expect(useTasksStore.getState().selectedTaskId).toBeTruthy();
    const pane = await screen.findByRole("complementary", { name: "Task details" });
    await waitFor(() =>
      expect(within(pane).getByRole("textbox", { name: "Task title" })).toHaveFocus(),
    );
    expect(within(pane).getByRole("textbox", { name: "Task title" })).toHaveValue(
      "Send Q3 metrics to Dana",
    );
  });

  it("keeps a reminder that came due while a dismissal was on its way", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const [task] = await backend.listTasks(null, false);
    const dismissReminders = backend.dismissReminders.bind(backend);
    let answered: Promise<unknown> | null = null;
    vi.spyOn(backend, "dismissReminders").mockImplementation((keys) => {
      const answer = (async () => {
        const active = await dismissReminders(keys);
        // While the server is told, a task's reminder comes due.
        await backend.updateTask(task!.id, {
          reminderAt: new Date(NOW.getTime() - 60_000).toISOString(),
        });
        await new Promise((r) => setTimeout(r, 20));
        return active;
      })();
      answered = answer;
      return answer;
    });
    renderApp(backend);
    await user.click(await bell());
    await user.click(within(row("Focus time")).getByRole("button", { name: "Dismiss Focus time" }));
    await waitFor(() => expect(answered).not.toBeNull());
    await act(async () => {
      await answered;
      await new Promise((r) => setTimeout(r, 20));
    });
    // The answer, from before it came due, doesn't take it away.
    expect(within(panel()).getByRole("button", { name: task!.title })).toBeInTheDocument();
    expect(within(panel()).queryByRole("button", { name: "Focus time" })).toBeNull();
    expect(await bell()).toHaveAccessibleName("Reminders, 2 due");
  });

  it("announces reminders as they come due", async () => {
    const backend = new MockBackend();
    renderApp(backend);
    await bell();
    const status = screen
      .getAllByRole("status")
      .find((s) => s.closest(".reminders")) as HTMLElement;
    expect(status).toBeEmptyDOMElement();

    // A task's reminder comes due.
    const [task] = await backend.listTasks(null, false);
    await act(async () => {
      await backend.updateTask(task!.id, {
        reminderAt: new Date(NOW.getTime() - 60_000).toISOString(),
      });
    });
    await waitFor(() =>
      expect(status).toHaveTextContent(`Reminder: ${task!.title}, 1 min overdue.`),
    );
    expect(await bell()).toHaveAccessibleName("Reminders, 3 due");
  });
});
