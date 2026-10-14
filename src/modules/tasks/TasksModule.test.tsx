import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import type { Backend } from "../../api/backend";
import type { Task } from "../../api/types";
import { useAppStore } from "../../app/store";
import { Toaster } from "../../components/Toaster";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { useTasksStore } from "./store";
import { TasksModule } from "./TasksModule";

// Wednesday, October 14, 2026, 10 AM local time. Only Date is faked so that
// React Query, user-event and the debounce timers keep running normally.
const NOW = new Date(2026, 9, 14, 10, 0);

function renderTasks(backend = new MockBackend()) {
  return renderWithBackend(
    <>
      <TasksModule />
      <Toaster />
    </>,
    { backend },
  );
}

const sidebar = () => screen.getByRole("listbox", { name: "Lists" });
const openList = (name: string) => screen.getByRole("listbox", { name });
const taskTitles = (list: HTMLElement) =>
  within(list)
    .queryAllByRole("option")
    .map((o) => o.querySelector(".tasks-row__title")?.textContent);
const details = () => screen.getByRole("complementary", { name: "Task details" });

async function allTasks(backend: Backend): Promise<Task[]> {
  return backend.listTasks(null, true);
}

async function findTask(backend: Backend, title: string): Promise<Task | undefined> {
  return (await allTasks(backend)).find((t) => t.title === title);
}

async function pickList(user: ReturnType<typeof userEvent.setup>, name: RegExp) {
  await user.click(within(sidebar()).getByRole("option", { name }));
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  useAppStore.setState({ module: "tasks", search: "" });
  useTasksStore.setState({ view: null, selectedTaskId: null, showCompleted: false });
  useToasts.setState({ toasts: [] });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("Task lists", () => {
  it("shows smart lists and each account's lists with open-task counts", async () => {
    renderTasks();
    const nav = await screen.findByRole("listbox", { name: "Lists" });
    const names = within(nav)
      .getAllByRole("option")
      .map((o) => o.getAttribute("aria-label"));
    expect(names).toEqual([
      "Important, 4 open",
      "Planned, 11 open",
      "All tasks, 17 open",
      "Tasks, 6 open",
      "Atlas launch, 4 open",
      "Tasks, 4 open",
      "Groceries, 3 open",
    ]);
    const work = within(nav).getByRole("group", { name: /Alex Morgan/ });
    expect(within(work).getAllByRole("option")).toHaveLength(2);
    expect(within(nav).getByRole("group", { name: /Alex \(Personal\)/ })).toBeInTheDocument();
  });

  it("opens the first account's default list and navigates lists with the keyboard", async () => {
    const user = userEvent.setup();
    renderTasks();
    expect(await screen.findByRole("heading", { name: "Tasks", level: 2 })).toBeInTheDocument();
    expect(screen.getByText("Alex Morgan · alex.morgan@acme.example")).toBeInTheDocument();
    expect(taskTitles(openList("Tasks"))).toEqual([
      "Send Q3 metrics to Dana",
      "Add top three bets to planning doc",
      "Benefits enrollment",
      "Expense Lisbon trip",
      // No due date: newest first.
      "Read load test report",
      "Reply to Contoso about renewal terms",
    ]);

    sidebar().focus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("heading", { name: "Atlas launch", level: 2 })).toBeInTheDocument();
    await user.keyboard("{Home}");
    expect(screen.getByRole("heading", { name: "Important", level: 2 })).toBeInTheDocument();
  });

  it("lists Important tasks across accounts with their list names", async () => {
    const user = userEvent.setup();
    renderTasks();
    await screen.findByRole("listbox", { name: "Lists" });
    await pickList(user, /^Important/);
    const list = openList("Important");
    expect(taskTitles(list)).toEqual([
      "Send Q3 metrics to Dana",
      "Review rollback plan",
      "Add top three bets to planning doc",
      "Reply to Contoso about renewal terms",
    ]);
    // Lists named "Tasks" in two accounts are told apart.
    expect(within(list).getByRole("option", { name: /^Review rollback plan/ })).toHaveTextContent(
      "Atlas launch",
    );
    expect(within(list).getByRole("option", { name: /^Send Q3 metrics/ })).toHaveTextContent(
      "Tasks · Alex Morgan",
    );
  });

  it("groups Planned tasks into Overdue, Today, Tomorrow, This week and Later", async () => {
    const user = userEvent.setup();
    renderTasks();
    await screen.findByRole("listbox", { name: "Lists" });
    await pickList(user, /^Planned/);
    const list = openList("Planned");
    const group = (name: string) =>
      taskTitles(within(list).getByRole("group", { name: new RegExp(`^${name}`) }));

    expect(
      within(list)
        .getAllByRole("group")
        .map((g) => g.firstChild?.firstChild?.textContent),
    ).toEqual(["Overdue", "Today", "Tomorrow", "This week", "Later"]);
    expect(group("Overdue")).toEqual(["Send Q3 metrics to Dana", "Update status page copy"]);
    expect(group("Today")).toEqual(["Review rollback plan"]);
    expect(group("Tomorrow")).toEqual(["Confirm launch-week on-call rotation"]);
    // Friday, October 16 and Saturday, October 17 are still this week.
    expect(group("This week")).toEqual([
      "Approve customer announcement",
      "Add top three bets to planning doc",
      "Benefits enrollment",
      "Buy flowers for Sunday",
    ]);
    expect(group("Later")).toEqual([
      "Book car service",
      "Expense Lisbon trip",
      "Renew climbing membership",
    ]);

    // Due chips: red when overdue, relative names close by, dates further out.
    const overdue = within(list).getByRole("option", { name: /^Send Q3 metrics/ });
    expect(within(overdue).getByText("Mon, Oct 12")).toHaveClass("tasks-due--overdue");
    const yesterday = within(list).getByRole("option", { name: /^Update status page copy/ });
    expect(yesterday).toHaveAccessibleName(/due Yesterday, overdue/);
    expect(
      within(within(list).getByRole("option", { name: /^Review rollback/ })).getByText("Today"),
    ).toHaveClass("tasks-due--today");
    expect(within(list).getByRole("option", { name: /^Approve customer/ })).toHaveTextContent(
      "Friday",
    );
    expect(within(list).getByRole("option", { name: /^Book car service/ })).toHaveTextContent(
      "Fri, Oct 23",
    );
  });

  it("shows an empty state for a list without tasks", async () => {
    const backend = new MockBackend();
    for (const t of await backend.listTasks(null, true)) {
      if (t.importance === "high") await backend.updateTask(t.id, { importance: "normal" });
    }
    const user = userEvent.setup();
    renderTasks(backend);
    await screen.findByRole("listbox", { name: "Lists" });
    await pickList(user, /^Important/);
    expect(await screen.findByText("Nothing important yet")).toBeInTheDocument();
  });
});

describe("Adding tasks", () => {
  it("adds to the current list with Enter", async () => {
    const user = userEvent.setup();
    const { backend } = renderTasks();
    await screen.findByRole("heading", { name: "Tasks", level: 2 });
    await pickList(user, /^Atlas launch/);

    const input = screen.getByRole("textbox", { name: "Add a task" });
    await user.type(input, "Write release notes{Enter}");
    expect(input).toHaveValue("");
    expect(input).toHaveFocus();
    expect(
      await within(openList("Atlas launch")).findByRole("option", { name: /^Write release notes/ }),
    ).toBeInTheDocument();
    expect(await findTask(backend, "Write release notes")).toMatchObject({
      listId: "tl-atlas",
      importance: "normal",
      dueDate: null,
    });
  });

  it("adds to the first account's default list from smart lists", async () => {
    const user = userEvent.setup();
    const { backend } = renderTasks();
    await screen.findByRole("listbox", { name: "Lists" });

    await pickList(user, /^Important/);
    expect(screen.getByText("Adds to Tasks · alex.morgan@acme.example")).toBeInTheDocument();
    await user.type(
      screen.getByRole("textbox", { name: "Add a task" }),
      "Call the auditors{Enter}",
    );
    expect(
      await within(openList("Important")).findByRole("option", { name: /^Call the auditors/ }),
    ).toBeInTheDocument();
    expect(await findTask(backend, "Call the auditors")).toMatchObject({
      listId: "tl-work",
      importance: "high",
    });

    await pickList(user, /^Planned/);
    await user.type(screen.getByRole("textbox", { name: "Add a task" }), "Water plants{Enter}");
    await waitFor(async () =>
      expect(await findTask(backend, "Water plants")).toMatchObject({
        listId: "tl-work",
        dueDate: "2026-10-14",
      }),
    );
    expect(
      taskTitles(within(openList("Planned")).getByRole("group", { name: /^Today/ })),
    ).toContain("Water plants");
  });

  it("keeps the text and shows an error when adding fails", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    vi.spyOn(backend, "createTask").mockRejectedValueOnce({
      code: "network",
      message: "Couldn't reach the server.",
    });
    renderTasks(backend);
    const input = await screen.findByRole("textbox", { name: "Add a task" });
    await user.type(input, "Flaky{Enter}");
    expect(await screen.findByText("Couldn't reach the server.")).toBeInTheDocument();
    expect(input).toHaveValue("Flaky");
  });
});

describe("Completing and starring", () => {
  it("completes a task optimistically and moves it to Completed", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    // Hold the first update until the test releases it.
    let release = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    const updateTask = backend.updateTask.bind(backend);
    vi.spyOn(backend, "updateTask").mockImplementation(async (id, patch) => {
      await gate;
      return updateTask(id, patch);
    });
    renderTasks(backend);
    const list = await screen.findByRole("listbox", { name: "Tasks" });

    const checkbox = within(list).getByRole("checkbox", {
      name: "Completed: Read load test report",
    });
    await user.click(checkbox);
    // Checked right away, before the backend answers.
    expect(checkbox).toHaveAttribute("aria-checked", "true");
    expect((await findTask(backend, "Read load test report"))?.status).toBe("notStarted");
    release();

    // After the check mark animation it leaves the open tasks.
    await waitFor(() => expect(taskTitles(list)).not.toContain("Read load test report"));
    const toggle = screen.getByRole("button", { name: /^Completed \d/ });
    expect(toggle).toHaveTextContent("Completed 2");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    const done = screen.getByRole("listbox", { name: "Completed tasks" });
    expect(taskTitles(done)).toEqual(["Read load test report", "Submit timesheet"]);
    await waitFor(async () =>
      expect(await findTask(backend, "Read load test report")).toMatchObject({
        status: "completed",
        completedAt: NOW.toISOString(),
      }),
    );

    // Un-completing puts it back.
    await user.click(
      within(done).getByRole("checkbox", { name: "Completed: Read load test report" }),
    );
    await waitFor(() => expect(taskTitles(openList("Tasks"))).toContain("Read load test report"));
    await waitFor(async () =>
      expect((await findTask(backend, "Read load test report"))?.status).toBe("notStarted"),
    );
  });

  it("stars a task and it appears in Important", async () => {
    const user = userEvent.setup();
    const { backend } = renderTasks();
    const list = await screen.findByRole("listbox", { name: "Tasks" });
    const star = within(list).getByRole("button", { name: "Important: Benefits enrollment" });
    expect(star).toHaveAttribute("aria-pressed", "false");
    await user.click(star);
    expect(star).toHaveAttribute("aria-pressed", "true");
    await waitFor(async () =>
      expect((await findTask(backend, "Benefits enrollment"))?.importance).toBe("high"),
    );
    // Starring does not open the details pane.
    expect(screen.queryByRole("complementary", { name: "Task details" })).not.toBeInTheDocument();

    await pickList(user, /^Important/);
    expect(taskTitles(openList("Important"))).toContain("Benefits enrollment");
  });

  it("rolls back and shows an error when the backend rejects an update", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    vi.spyOn(backend, "updateTask").mockRejectedValueOnce({
      code: "network",
      message: "You're offline.",
    });
    renderTasks(backend);
    const list = await screen.findByRole("listbox", { name: "Tasks" });
    await user.click(within(list).getByRole("button", { name: "Important: Benefits enrollment" }));
    expect(await screen.findByText("You're offline.")).toBeInTheDocument();
    expect(
      within(list).getByRole("button", { name: "Important: Benefits enrollment" }),
    ).toHaveAttribute("aria-pressed", "false");
    expect((await findTask(backend, "Benefits enrollment"))?.importance).toBe("normal");
  });

  it("rolls back only the failed task when updates overlap", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    let fail = () => {};
    const failing = new Promise<never>((_, reject) => {
      fail = () => reject({ code: "network", message: "Timed out." });
    });
    vi.spyOn(backend, "updateTask").mockReturnValueOnce(failing);
    renderTasks(backend);
    const list = await screen.findByRole("listbox", { name: "Tasks" });
    const star = (title: string) =>
      within(list).getByRole("button", { name: `Important: ${title}` });

    await user.click(star("Benefits enrollment"));
    await user.click(star("Expense Lisbon trip"));
    await waitFor(async () =>
      expect((await findTask(backend, "Expense Lisbon trip"))?.importance).toBe("high"),
    );
    fail();
    expect(await screen.findByText("Timed out.")).toBeInTheDocument();
    expect(star("Benefits enrollment")).toHaveAttribute("aria-pressed", "false");
    expect(star("Expense Lisbon trip")).toHaveAttribute("aria-pressed", "true");
  });

  it("supports the keyboard: arrows select, Space completes, Enter edits", async () => {
    const user = userEvent.setup();
    const { backend } = renderTasks();
    const list = await screen.findByRole("listbox", { name: "Tasks" });
    list.focus();
    await user.keyboard("{ArrowDown}");
    expect(within(details()).getByRole("textbox", { name: "Task title" })).toHaveValue(
      "Send Q3 metrics to Dana",
    );
    await user.keyboard("{ArrowDown}");
    expect(within(details()).getByRole("textbox", { name: "Task title" })).toHaveValue(
      "Add top three bets to planning doc",
    );
    await user.keyboard(" ");
    await waitFor(async () =>
      expect((await findTask(backend, "Add top three bets to planning doc"))?.status).toBe(
        "completed",
      ),
    );
    await user.keyboard("{Enter}");
    expect(within(details()).getByRole("textbox", { name: "Task title" })).toHaveFocus();
  });
});

describe("Task details", () => {
  it("edits the title, due date and notes", async () => {
    const user = userEvent.setup();
    const { backend } = renderTasks();
    const list = await screen.findByRole("listbox", { name: "Tasks" });
    await user.click(within(list).getByRole("option", { name: /^Read load test report/ }));
    const pane = details();
    expect(within(pane).getByText("Created Wed, Oct 14")).toBeInTheDocument();
    expect(within(pane).getByRole("heading", { name: "Add due date" })).toBeInTheDocument();

    // Title: saved on blur.
    const title = within(pane).getByRole("textbox", { name: "Task title" });
    await user.clear(title);
    await user.type(title, "Read the load test report");
    await user.tab();
    await waitFor(async () =>
      expect(await findTask(backend, "Read the load test report")).toBeDefined(),
    );
    expect(within(list).getByRole("option", { name: /^Read the load test report/ })).toBeVisible();

    // Due date: quick picks, the date input and removal.
    await user.click(within(pane).getByRole("button", { name: "Tomorrow" }));
    await waitFor(async () =>
      expect((await findTask(backend, "Read the load test report"))?.dueDate).toBe("2026-10-15"),
    );
    expect(within(pane).getByRole("button", { name: "Tomorrow" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(within(pane).getByRole("heading", { name: "Due Thursday, October 15" })).toBeVisible();
    await user.click(within(pane).getByRole("button", { name: "Next week" }));
    await waitFor(async () =>
      expect((await findTask(backend, "Read the load test report"))?.dueDate).toBe("2026-10-19"),
    );
    fireEvent.change(within(pane).getByLabelText("Due date"), { target: { value: "2026-11-02" } });
    await waitFor(async () =>
      expect((await findTask(backend, "Read the load test report"))?.dueDate).toBe("2026-11-02"),
    );
    await user.click(within(pane).getByRole("button", { name: "Remove due date" }));
    await waitFor(async () =>
      expect((await findTask(backend, "Read the load test report"))?.dueDate).toBeNull(),
    );

    // Notes: saved on blur, cleared when emptied.
    const notes = within(pane).getByRole("textbox", { name: "Notes" });
    await user.type(notes, "Focus on the export endpoint.");
    expect((await findTask(backend, "Read the load test report"))?.notes).toBeNull();
    await user.tab();
    await waitFor(async () =>
      expect((await findTask(backend, "Read the load test report"))?.notes).toBe(
        "Focus on the export endpoint.",
      ),
    );
    await user.clear(notes);
    await user.tab();
    await waitFor(async () =>
      expect((await findTask(backend, "Read the load test report"))?.notes).toBeNull(),
    );
  });

  it("reverts the title on Escape and never saves an empty one", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const spy = vi.spyOn(backend, "updateTask");
    renderTasks(backend);
    const list = await screen.findByRole("listbox", { name: "Tasks" });
    await user.click(within(list).getByRole("option", { name: /^Benefits enrollment/ }));
    const title = within(details()).getByRole("textbox", { name: "Task title" });
    await user.clear(title);
    await user.type(title, "Something else{Escape}");
    expect(title).toHaveValue("Benefits enrollment");
    await user.clear(title);
    await user.tab();
    expect(title).toHaveValue("Benefits enrollment");
    expect(spy).not.toHaveBeenCalled();
    // The pane is still open (Escape in the title only reverts).
    expect(details()).toBeInTheDocument();
  });

  it("completes and stars from the details pane", async () => {
    const user = userEvent.setup();
    const { backend } = renderTasks();
    const list = await screen.findByRole("listbox", { name: "Tasks" });
    await user.click(within(list).getByRole("option", { name: /^Expense Lisbon trip/ }));
    const pane = details();
    await user.click(within(pane).getByRole("button", { name: "Important: Expense Lisbon trip" }));
    await user.click(
      within(pane).getByRole("checkbox", { name: "Completed: Expense Lisbon trip" }),
    );
    expect(await within(pane).findByText("Completed Wed, Oct 14")).toBeInTheDocument();
    await waitFor(async () =>
      expect(await findTask(backend, "Expense Lisbon trip")).toMatchObject({
        status: "completed",
        importance: "high",
      }),
    );
  });

  it("closes with Escape and deletes after confirmation", async () => {
    const user = userEvent.setup();
    const { backend } = renderTasks();
    const list = await screen.findByRole("listbox", { name: "Tasks" });
    await user.click(within(list).getByRole("option", { name: /^Benefits enrollment/ }));
    await user.click(within(details()).getByRole("textbox", { name: "Notes" }));
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("complementary", { name: "Task details" })).not.toBeInTheDocument();

    await user.click(within(list).getByRole("option", { name: /^Benefits enrollment/ }));
    await user.click(within(details()).getByRole("button", { name: "Delete task" }));
    const dialog = screen.getByRole("dialog", { name: "Delete task?" });
    expect(dialog).toHaveTextContent("“Benefits enrollment” will be permanently deleted.");
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));

    expect(screen.queryByRole("complementary", { name: "Task details" })).not.toBeInTheDocument();
    expect(taskTitles(openList("Tasks"))).not.toContain("Benefits enrollment");
    await waitFor(async () =>
      expect(await findTask(backend, "Benefits enrollment")).toBeUndefined(),
    );
    expect(await screen.findByText("“Benefits enrollment” was deleted")).toBeInTheDocument();
  });
});

describe("Completed section and search", () => {
  it("collapses and expands the completed tasks of a list", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const spy = vi.spyOn(backend, "listTasks");
    renderTasks(backend);
    await screen.findByRole("listbox", { name: "Lists" });
    await pickList(user, /^Atlas launch/);
    await waitFor(() => expect(spy).toHaveBeenCalledWith("tl-atlas", true));

    const toggle = screen.getByRole("button", { name: /^Completed/ });
    expect(toggle).toHaveTextContent("Completed 1");
    expect(screen.queryByRole("listbox", { name: "Completed tasks" })).not.toBeInTheDocument();
    await user.click(toggle);
    const done = screen.getByRole("listbox", { name: "Completed tasks" });
    expect(taskTitles(done)).toEqual(["Book retro room"]);
    expect(within(done).getByRole("option")).toHaveClass("tasks-row--done");
    await user.click(toggle);
    expect(screen.queryByRole("listbox", { name: "Completed tasks" })).not.toBeInTheDocument();
  });

  it("filters the current list by the search box", async () => {
    const user = userEvent.setup();
    renderTasks();
    await screen.findByRole("listbox", { name: "Lists" });
    await pickList(user, /^All tasks/);
    expect(taskTitles(openList("All tasks"))).toHaveLength(17);

    act(() => useAppStore.setState({ search: "ROLLBACK" }));
    expect(taskTitles(openList("All tasks"))).toEqual(["Review rollback plan"]);
    expect(screen.getByRole("status")).toHaveTextContent("Tasks matching “ROLLBACK”");

    // Notes are searched too.
    act(() => useAppStore.setState({ search: "pricing" }));
    expect(taskTitles(openList("All tasks"))).toEqual(["Approve customer announcement"]);

    // Nothing in this list: offer to search everything.
    await pickList(user, /^Groceries/);
    expect(screen.getByText("No matching tasks")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Search all tasks" }));
    expect(taskTitles(openList("All tasks"))).toEqual(["Approve customer announcement"]);

    // Another module's search text is ignored.
    act(() => useAppStore.setState({ module: "mail" }));
    expect(taskTitles(openList("All tasks"))).toHaveLength(17);
  });
});
