import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { MockBackend } from "../../api/mock";
import { useAppStore } from "../../app/store";
import { useToasts } from "../../components/toast";
import { Toaster } from "../../components/Toaster";
import { renderWithBackend } from "../../test/render";
import { CalendarModule } from "./CalendarModule";
import type { CalendarView } from "./layout";
import { toAllDayIso } from "./dates";
import { HIDDEN_KEY, VIEW_KEY } from "./prefs";
import { currentTimeZone, setTimeZone } from "./testing";

// Run in a zone west of UTC so mixing up UTC and local dates shows up.
const previousTz = currentTimeZone();
beforeAll(() => setTimeZone("America/Los_Angeles"));
afterAll(() => setTimeZone(previousTz));

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 9, 14, 12, 0)); // Wednesday, October 14, 2026, at noon
  localStorage.clear();
  useToasts.setState({ toasts: [] });
});

afterEach(() => {
  vi.useRealTimers();
});

async function setup({ view, backend }: { view?: CalendarView; backend?: MockBackend } = {}) {
  if (view) localStorage.setItem(VIEW_KEY, view);
  const user = userEvent.setup();
  const utils = renderWithBackend(
    <>
      <CalendarModule />
      <Toaster />
    </>,
    { backend: backend ?? new MockBackend() },
  );
  return { ...utils, backend: utils.backend as MockBackend, user };
}

const title = () => screen.getByRole("heading", { level: 1 });
const eventButton = (name: string) =>
  screen.findByRole("button", { name: new RegExp(`^${name},`) });
const queryEvent = (name: string) =>
  screen.queryByRole("button", { name: new RegExp(`^${name},`) });

describe("CalendarModule", () => {
  it("navigates between ranges and views with buttons and shortcuts", async () => {
    const { user } = await setup();
    await eventButton("Atlas launch review");
    expect(title()).toHaveTextContent("October 12 – 16, 2026");

    await user.click(screen.getByRole("button", { name: "Next week" }));
    expect(title()).toHaveTextContent("October 19 – 23, 2026");
    await user.click(screen.getByRole("button", { name: "Today" }));
    expect(title()).toHaveTextContent("October 12 – 16, 2026");

    await user.click(screen.getByRole("button", { name: "Month" }));
    expect(title()).toHaveTextContent("October 2026");
    expect(localStorage.getItem(VIEW_KEY)).toBe("month");
    await user.click(screen.getByRole("button", { name: "Next month" }));
    expect(title()).toHaveTextContent("November 2026");

    await user.click(screen.getByRole("button", { name: "Day" }));
    expect(title()).toHaveTextContent("Saturday, November 14, 2026");

    await user.keyboard("{Control>}{Alt>}2{/Alt}{/Control}");
    expect(title()).toHaveTextContent("November 9 – 13, 2026");
    expect(screen.getByRole("button", { name: "Work week" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await user.keyboard("{Alt>}{ArrowLeft}{/Alt}");
    expect(title()).toHaveTextContent("November 2 – 6, 2026");
    await user.keyboard("{Alt>}{ArrowLeft}{/Alt}");
    expect(title()).toHaveTextContent("October 26 – 30, 2026");
    await user.keyboard("t");
    expect(title()).toHaveTextContent("October 12 – 16, 2026");
    await user.keyboard("{Control>}{Alt>}3{/Alt}{/Control}");
    expect(title()).toHaveTextContent("October 11 – 17, 2026");
  });

  it("restores the last view", async () => {
    await setup({ view: "month" });
    expect(title()).toHaveTextContent("October 2026");
    expect(screen.getByRole("grid", { name: "October 2026" })).toBeInTheDocument();
  });

  it("lists and refreshes events for the visible range", async () => {
    const backend = new MockBackend();
    const list = vi.spyOn(backend, "listEvents");
    const refresh = vi.spyOn(backend, "refreshEvents");
    const { user } = await setup({ backend });
    await eventButton("Atlas launch review");
    // Local midnights in Los Angeles (UTC-7) of Monday, October 12 and Saturday, October 17.
    expect(list).toHaveBeenCalledWith("2026-10-12T07:00:00.000Z", "2026-10-17T07:00:00.000Z");
    expect(refresh).toHaveBeenCalledWith("2026-10-12T07:00:00.000Z", "2026-10-17T07:00:00.000Z");

    await user.click(screen.getByRole("button", { name: "Next week" }));
    expect(refresh).toHaveBeenCalledWith("2026-10-19T07:00:00.000Z", "2026-10-24T07:00:00.000Z");
  });

  it("shows events in local time, including ones that cross midnight", async () => {
    const { user } = await setup({ view: "week" });
    const review = await eventButton("Atlas launch review");
    expect(review).toHaveAccessibleName(/Wednesday, October 14, 2:00 – 3:00 PM/);
    expect(review).toHaveTextContent("2:00 – 3:00 PM");

    // Friday 10 PM to Saturday 1 AM is drawn on both days.
    const release = screen.getAllByRole("button", { name: /^Release window,/ });
    expect(release).toHaveLength(2);
    expect(release[0]!.closest('[role="group"]')).toHaveAccessibleName("Friday, October 16");
    expect(release[1]!.closest('[role="group"]')).toHaveAccessibleName("Saturday, October 17");

    // All-day events are in the all-day lane on the right local days.
    const wfh = screen.getByRole("button", { name: /^Working from home,/ });
    expect(wfh).toHaveAccessibleName(/Friday, October 16, all day/);
    expect(wfh.closest('[role="group"]')).toHaveAccessibleName(
      "All-day events, Friday, October 16",
    );

    await user.click(screen.getByRole("button", { name: "Next week" }));
    const trip = await eventButton("Lisbon trip");
    expect(trip).toHaveAccessibleName(/Thursday, October 22 to Sunday, October 25, all day/);
  });

  it("creates an event by clicking a time slot", async () => {
    const { user, backend, container } = await setup();
    await eventButton("Atlas launch review");
    const create = vi.spyOn(backend, "createEvent");

    await user.click(container.querySelector('[data-time="2026-10-15T10:00"]')!);
    const dialog = await screen.findByRole("dialog", { name: "New event" });
    expect(within(dialog).getByLabelText("Start date")).toHaveValue("2026-10-15");
    expect(within(dialog).getByLabelText("Start time")).toHaveValue("10:00");
    expect(within(dialog).getByLabelText("End time")).toHaveValue("10:30");
    expect(within(dialog).getByLabelText("Availability")).toHaveValue("busy");

    await user.type(within(dialog).getByLabelText("Title"), "Roadmap review");
    await user.type(within(dialog).getByLabelText("Location"), "Room 4.12");
    await user.type(within(dialog).getByRole("combobox", { name: "Invite attendees" }), "pri");
    await user.click(await within(dialog).findByRole("option", { name: /Priya Raman/ }));
    expect(within(dialog).getByRole("button", { name: "Remove Priya Raman" })).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(create).toHaveBeenCalledTimes(1));
    expect(create.mock.calls[0]![0]).toMatchObject({
      calendarId: "cal-work",
      subject: "Roadmap review",
      location: "Room 4.12",
      // 10:00 in Los Angeles is 17:00 UTC.
      start: "2026-10-15T17:00:00.000Z",
      end: "2026-10-15T17:30:00.000Z",
      isAllDay: false,
      attendees: [{ name: "Priya Raman", address: "priya.raman@acme.example" }],
      reminderMinutes: 15,
      showAs: "busy",
    });
    expect(await screen.findByText("Event created")).toBeInTheDocument();
    expect(await eventButton("Roadmap review")).toHaveAccessibleName(
      /Thursday, October 15, 10:00 – 10:30 AM/,
    );
    expect(screen.queryByRole("dialog", { name: "New event" })).not.toBeInTheDocument();
  });

  it("creates all-day events as midnight UTC of the local days", async () => {
    const { user, backend, container } = await setup();
    await eventButton("Atlas launch review");
    const create = vi.spyOn(backend, "createEvent");

    await user.click(container.querySelector('[data-time="2026-10-15T10:00"]')!);
    const dialog = await screen.findByRole("dialog", { name: "New event" });
    await user.type(within(dialog).getByLabelText("Title"), "Offsite");
    await user.click(within(dialog).getByRole("switch", { name: "All day" }));
    expect(within(dialog).queryByLabelText("Start time")).not.toBeInTheDocument();
    fireEvent.change(within(dialog).getByLabelText("End date (last day)"), {
      target: { value: "2026-10-16" },
    });
    await user.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(create).toHaveBeenCalledTimes(1));
    expect(create.mock.calls[0]![0]).toMatchObject({
      start: "2026-10-15T00:00:00.000Z",
      end: "2026-10-17T00:00:00.000Z",
      isAllDay: true,
    });
    expect(await eventButton("Offsite")).toHaveAccessibleName(
      /Thursday, October 15 to Friday, October 16, all day/,
    );
  });

  it("validates the title and the times", async () => {
    const { user, backend } = await setup();
    await eventButton("Atlas launch review");
    const create = vi.spyOn(backend, "createEvent");

    await user.click(screen.getByRole("button", { name: /New event/ }));
    const dialog = await screen.findByRole("dialog", { name: "New event" });
    // 12:00 now: the next half hour.
    expect(within(dialog).getByLabelText("Start time")).toHaveValue("12:00");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText("Add a title for the event.")).toBeInTheDocument();

    await user.type(within(dialog).getByLabelText("Title"), "Planning");
    fireEvent.change(within(dialog).getByLabelText("End time"), { target: { value: "11:00" } });
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText("The end must be after the start.")).toBeInTheDocument();
    expect(create).not.toHaveBeenCalled();

    // Moving the start keeps the duration.
    fireEvent.change(within(dialog).getByLabelText("End time"), { target: { value: "12:45" } });
    fireEvent.change(within(dialog).getByLabelText("Start time"), { target: { value: "15:00" } });
    expect(within(dialog).getByLabelText("End time")).toHaveValue("15:45");
  });

  it("offers a Teams meeting only for Microsoft calendars", async () => {
    const { user } = await setup();
    await eventButton("Atlas launch review");
    await user.click(screen.getByRole("button", { name: /New event/ }));
    const dialog = await screen.findByRole("dialog", { name: "New event" });
    const calendar = within(dialog).getByLabelText("Calendar");
    // Read-only calendars are not offered.
    expect(within(calendar).queryByRole("option", { name: "US holidays" })).toBeNull();
    expect(within(dialog).getByRole("switch", { name: /Teams meeting/ })).toBeInTheDocument();
    await user.selectOptions(calendar, "cal-home");
    expect(within(dialog).queryByRole("switch", { name: /Teams meeting/ })).toBeNull();
  });

  it("edits an event from its details", async () => {
    const { user, backend } = await setup();
    const update = vi.spyOn(backend, "updateEvent");
    await user.click(await eventButton("Atlas launch review"));

    const details = screen.getByRole("dialog", { name: "Atlas launch review" });
    expect(within(details).getByText("2:00 – 3:00 PM")).toBeInTheDocument();
    // How the time shows to others, beside it as on the grid.
    expect(within(details).getByText("· Busy")).toBeInTheDocument();
    expect(within(details).getByText("Room 4.12 / Online")).toBeInTheDocument();
    await user.click(within(details).getByRole("button", { name: "Edit event" }));

    const dialog = await screen.findByRole("dialog", { name: "Edit event" });
    const input = within(dialog).getByLabelText("Title");
    expect(input).toHaveValue("Atlas launch review");
    await user.clear(input);
    await user.type(input, "Atlas go/no-go");
    // Edits save changes; new events just save.
    expect(within(dialog).queryByRole("button", { name: "Save" })).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
    const [id, draft] = update.mock.calls[0]!;
    expect(id).toMatch(/^evt-/);
    expect(draft).toMatchObject({
      subject: "Atlas go/no-go",
      start: "2026-10-14T21:00:00.000Z",
      end: "2026-10-14T22:00:00.000Z",
      isOnlineMeeting: true,
    });
    expect(await screen.findByText("Event updated")).toBeInTheDocument();
    expect(await eventButton("Atlas go/no-go")).toBeInTheDocument();
    expect(queryEvent("Atlas launch review")).toBeNull();
  });

  it("deletes an event after confirmation", async () => {
    const { user, backend } = await setup();
    const remove = vi.spyOn(backend, "deleteEvent");
    await user.click(await eventButton("Focus time"));
    const details = screen.getByRole("dialog", { name: "Focus time" });
    await user.click(within(details).getByRole("button", { name: "Delete event" }));

    const confirm = await screen.findByRole("dialog", { name: "Delete event?" });
    await user.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(remove).not.toHaveBeenCalled();
    expect(queryEvent("Focus time")).toBeInTheDocument();

    await user.click(within(details).getByRole("button", { name: "Delete event" }));
    await user.click(
      within(await screen.findByRole("dialog", { name: "Delete event?" })).getByRole("button", {
        name: "Delete",
      }),
    );
    await waitFor(() => expect(remove).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(queryEvent("Focus time")).toBeNull());
    expect(screen.queryByRole("dialog", { name: "Focus time" })).toBeNull();
    expect(await screen.findByText("Event deleted")).toBeInTheDocument();
  });

  it("shows RSVP buttons only for invitations", async () => {
    const { user, backend } = await setup();
    const respond = vi.spyOn(backend, "respondToEvent");

    await user.click(await eventButton("Coffee with Mei"));
    let details = screen.getByRole("dialog", { name: "Coffee with Mei" });
    expect(within(details).queryByRole("button", { name: /Accept/ })).toBeNull();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Coffee with Mei" })).toBeNull();

    await user.click(await eventButton("Customer escalation: Contoso"));
    details = screen.getByRole("dialog", { name: "Customer escalation: Contoso" });
    expect(within(details).getByText("You haven't responded yet.")).toBeInTheDocument();
    expect(within(details).getByRole("button", { name: /Accept/ })).toBeInTheDocument();
    expect(within(details).getByRole("button", { name: /Decline/ })).toBeInTheDocument();

    await user.click(within(details).getByRole("button", { name: "Add a message" }));
    await user.type(within(details).getByLabelText("Message to the organizer"), "Joining late");
    await user.click(within(details).getByRole("checkbox", { name: "Send a response" }));
    await user.click(within(details).getByRole("button", { name: /Tentative/ }));

    await waitFor(() => expect(respond).toHaveBeenCalledTimes(1));
    expect(respond.mock.calls[0]!.slice(1)).toEqual(["tentative", "Joining late", false]);
    expect(await screen.findByText("Tentatively accepted")).toBeInTheDocument();
    await waitFor(() =>
      expect(within(details).getByRole("button", { name: /Tentative/ })).toHaveAttribute(
        "aria-pressed",
        "true",
      ),
    );
  });

  it("renders descriptions sanitized and opens links through the backend", async () => {
    const backend = new MockBackend();
    await backend.createEvent({
      calendarId: "cal-work",
      subject: "Shady invite",
      bodyHtml:
        '<p style="position:fixed">Hello</p><img src="https://track.example/p.gif" onerror="alert(1)">' +
        '<script>alert(2)</script><a href="javascript:alert(3)">bad</a> <a href="https://ok.example/">good</a>',
      start: new Date(2026, 9, 14, 17).toISOString(),
      end: new Date(2026, 9, 14, 17, 30).toISOString(),
      isAllDay: false,
      attendees: [],
      showAs: "busy",
      isOnlineMeeting: false,
    });
    const open = vi.spyOn(backend, "openLink").mockResolvedValue();
    const { user } = await setup({ backend });
    await user.click(await eventButton("Shady invite"));
    const details = screen.getByRole("dialog", { name: "Shady invite" });
    const description = details.querySelector(".cal-details__description")!;
    expect(description).toHaveTextContent("Hello");
    expect(description.innerHTML).not.toMatch(/script|onerror|<img|javascript:|style=/i);

    await user.click(within(details).getByRole("link", { name: "good" }));
    expect(open).toHaveBeenCalledWith("https://ok.example/");
  });

  it("hides and shows calendars and remembers the choice", async () => {
    const { user, unmount } = await setup();
    expect(await eventButton("Climbing")).toBeInTheDocument();
    const personal = screen.getByRole("checkbox", { name: "Calendar (Alex (Personal))" });
    expect(personal).toBeChecked();

    await user.click(personal);
    expect(personal).not.toBeChecked();
    expect(queryEvent("Climbing")).toBeNull();
    expect(queryEvent("Atlas launch review")).toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem(HIDDEN_KEY)!)).toEqual(["cal-home"]);

    unmount();
    await setup();
    await eventButton("Atlas launch review");
    expect(queryEvent("Climbing")).toBeNull();
    const again = screen.getByRole("checkbox", { name: "Calendar (Alex (Personal))" });
    expect(again).not.toBeChecked();
    await user.click(again);
    expect(await eventButton("Climbing")).toBeInTheDocument();
  });

  it("tells when every calendar is hidden", async () => {
    localStorage.setItem(
      HIDDEN_KEY,
      JSON.stringify(["cal-work", "cal-team", "cal-holidays", "cal-home"]),
    );
    const { user } = await setup();
    expect(await screen.findByText("All calendars are hidden.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show all" }));
    expect(await eventButton("Atlas launch review")).toBeInTheDocument();
  });

  it("shows +k more in month view and opens the day", async () => {
    const { user } = await setup({ view: "month" });
    // "+N more" sits beside the date, so the day keeps all three lines for events.
    const more = await screen.findByRole("button", {
      name: "4 more events on Wednesday, October 14",
    });
    expect(more).toHaveTextContent("+4 more");
    expect(more.previousElementSibling).toHaveAccessibleName(/^Wednesday, October 14, 2026/);
    // Bars span days: the launch week is drawn once for Monday to Friday.
    expect(screen.getAllByRole("button", { name: /^Atlas launch week,/ })).toHaveLength(1);

    await user.click(more);
    expect(title()).toHaveTextContent("Wednesday, October 14, 2026");
    expect(screen.getByRole("button", { name: "Day" })).toHaveAttribute("aria-pressed", "true");
    expect(await eventButton("Customer escalation: Contoso")).toBeInTheDocument();
  });

  it("puts overlaps that don't fit behind +N, which opens the day", async () => {
    // A 700px grid: a week column has room for one of two overlapping events.
    class FixedSizeObserver {
      constructor(private callback: ResizeObserverCallback) {}
      observe() {
        const entry = { contentRect: { width: 700, height: 600 } } as ResizeObserverEntry;
        this.callback([entry], this as unknown as ResizeObserver);
      }
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal("ResizeObserver", FixedSizeObserver);
    try {
      const { user } = await setup({ view: "week" });
      await eventButton("Focus time");
      expect(queryEvent("Security review")).not.toBeInTheDocument();
      const chip = screen.getByRole("button", {
        name: "1 more event at 11:00 AM on Wednesday, October 14",
      });
      expect(chip).toHaveTextContent("+1");

      await user.click(chip);
      expect(title()).toHaveTextContent("Wednesday, October 14, 2026");
      // Day view has room for both.
      expect(await eventButton("Security review")).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /^1 more event at/ })).not.toBeInTheDocument();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("points a reminder's event behind +N at the +N, and gives it the keyboard back", async () => {
    // A 700px grid: Security review (11 AM) waits behind "+1".
    class FixedSizeObserver {
      constructor(private callback: ResizeObserverCallback) {}
      observe() {
        const entry = { contentRect: { width: 700, height: 600 } } as ResizeObserverEntry;
        this.callback([entry], this as unknown as ResizeObserver);
      }
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal("ResizeObserver", FixedSizeObserver);
    try {
      const backend = new MockBackend();
      const event = (
        await backend.listEvents(
          new Date(2026, 9, 14).toISOString(),
          new Date(2026, 9, 15).toISOString(),
        )
      ).find((e) => e.subject === "Security review")!;
      useAppStore
        .getState()
        .requestCalendarEvent({ eventId: event.id, start: event.start, isAllDay: false });
      const { user } = await setup({ view: "week", backend });
      expect(await screen.findByRole("dialog", { name: "Security review" })).toBeInTheDocument();
      expect(queryEvent("Security review")).not.toBeInTheDocument();
      await user.keyboard("{Escape}");
      expect(
        screen.getByRole("button", { name: "1 more event at 11:00 AM on Wednesday, October 14" }),
      ).toHaveFocus();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("does the same in Month view, behind a day's +N more", async () => {
    // A 450px month: two lines of events a day.
    class FixedSizeObserver {
      constructor(private callback: ResizeObserverCallback) {}
      observe() {
        const entry = { contentRect: { width: 700, height: 450 } } as ResizeObserverEntry;
        this.callback([entry], this as unknown as ResizeObserver);
      }
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal("ResizeObserver", FixedSizeObserver);
    try {
      const backend = new MockBackend();
      const today = await backend.listEvents(
        new Date(2026, 9, 14).toISOString(),
        new Date(2026, 9, 15).toISOString(),
      );
      // The day's last event, below its first two.
      const event = today
        .filter((e) => !e.isAllDay)
        .sort((a, b) => Date.parse(b.start) - Date.parse(a.start))[0]!;
      expect(today.length).toBeGreaterThan(2);
      useAppStore
        .getState()
        .requestCalendarEvent({ eventId: event.id, start: event.start, isAllDay: false });
      const { user } = await setup({ view: "month", backend });
      expect(await screen.findByRole("dialog", { name: event.subject })).toHaveFocus();
      const month = screen.getByRole("grid", { name: "October 2026" });
      expect(
        within(month).queryByRole("button", { name: new RegExp(`^${event.subject},`) }),
      ).toBeNull();
      await user.keyboard("{Escape}");
      expect(
        within(month).getByRole("button", { name: /^\d+ more events on Wednesday, October 14$/ }),
      ).toHaveFocus();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("gives the keyboard to the calendar's title when the event isn't drawn", async () => {
    const backend = new MockBackend();
    const event = (
      await backend.listEvents(
        new Date(2026, 9, 14).toISOString(),
        new Date(2026, 9, 15).toISOString(),
      )
    ).find((e) => e.subject === "Security review")!;
    // Its calendar is hidden.
    localStorage.setItem(HIDDEN_KEY, JSON.stringify([event.calendarId]));
    useAppStore
      .getState()
      .requestCalendarEvent({ eventId: event.id, start: event.start, isAllDay: false });
    const { user } = await setup({ backend });
    const details = await screen.findByRole("dialog", { name: "Security review" });
    expect(details).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(details).not.toBeInTheDocument();
    expect(title()).toHaveFocus();
  });

  it("collapses a crowded all-day lane until it is opened", async () => {
    const backend = new MockBackend();
    const calendar = (await backend.listCalendars()).find((c) => c.canEdit)!;
    for (const subject of ["Offsite", "Hackathon", "Board visit", "Fire drill"]) {
      await backend.createEvent({
        calendarId: calendar.id,
        subject,
        start: toAllDayIso(new Date(2026, 9, 14)),
        end: toAllDayIso(new Date(2026, 9, 15)),
        isAllDay: true,
        attendees: [],
        showAs: "free",
        isOnlineMeeting: false,
      });
    }
    const { user } = await setup({ backend });
    // Wednesday has five all-day rows: three show, the rest wait behind "+2 more".
    const more = await screen.findByRole("button", {
      name: "2 more all-day events on Wednesday, October 14",
    });
    expect(queryEvent("Board visit")).not.toBeInTheDocument();

    await user.click(more);
    expect(await eventButton("Board visit")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /more all-day events/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show fewer all-day events" }));
    expect(queryEvent("Board visit")).not.toBeInTheDocument();
  });

  it("gives the keyboard back to the all-day +N a reminder's event is behind", async () => {
    const backend = new MockBackend();
    const calendar = (await backend.listCalendars()).find((c) => c.canEdit)!;
    const created = [];
    for (const subject of ["Offsite", "Hackathon", "Board visit", "Fire drill"]) {
      created.push(
        await backend.createEvent({
          calendarId: calendar.id,
          subject,
          start: toAllDayIso(new Date(2026, 9, 14)),
          end: toAllDayIso(new Date(2026, 9, 15)),
          isAllDay: true,
          attendees: [],
          showAs: "free",
          isOnlineMeeting: false,
        }),
      );
    }
    const event = created.find((e) => e.subject === "Board visit")!;
    useAppStore
      .getState()
      .requestCalendarEvent({ eventId: event.id, start: event.start, isAllDay: true });
    const { user } = await setup({ backend });
    expect(await screen.findByRole("dialog", { name: "Board visit" })).toHaveFocus();
    expect(queryEvent("Board visit")).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(
      screen.getByRole("button", { name: "2 more all-day events on Wednesday, October 14" }),
    ).toHaveFocus();
  });

  it("lists what tomorrow holds in the sidebar", async () => {
    const { user } = await setup();
    const agenda = await screen.findByRole("region", { name: "Tomorrow · Thu, Oct 15" });
    // The declined meeting and the launch week (already under way) stay out.
    const items = within(agenda).getAllByRole("listitem");
    expect(items.map((item) => item.textContent)).toEqual([
      expect.stringMatching(/^Atlas stand-up/),
      expect.stringMatching(/^1:1 Dana \/ Alex/),
      expect.stringMatching(/^School pickup/),
    ]);

    await user.click(within(agenda).getByRole("button", { name: /^School pickup/ }));
    expect(await screen.findByRole("dialog", { name: "School pickup" })).toBeInTheDocument();

    await user.click(within(agenda).getByRole("button", { name: "1 more event tomorrow" }));
    expect(title()).toHaveTextContent("Thursday, October 15, 2026");
  });

  it("jumps to a date from the mini month", async () => {
    const { user } = await setup();
    await eventButton("Atlas launch review");
    await user.click(screen.getByRole("button", { name: "Go to next month" }));
    await user.click(screen.getByRole("button", { name: "Wednesday, November 18, 2026" }));
    expect(title()).toHaveTextContent("November 16 – 20, 2026");
  });

  it("scrolls the grid to an event a reminder opens, and keeps its details open", async () => {
    const backend = new MockBackend();
    const today = await backend.listEvents(
      new Date(2026, 9, 14).toISOString(),
      new Date(2026, 9, 15).toISOString(),
    );
    const event = today
      .filter((e) => !e.isAllDay)
      .sort((a, b) => Date.parse(b.start) - Date.parse(a.start))[0]!;
    // The grid shows 300px from y=200; the event is drawn 900px down it,
    // below what shows.
    const box = (top: number, height: number) =>
      ({ top, bottom: top + height, height, left: 0, right: 100, width: 100 }) as DOMRect;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      const grid = this.closest<HTMLElement>(".cal-tg__scroll");
      if (this === grid) return box(200, 300);
      if (grid && this.dataset.eventId === event.id) return box(200 + 900 - grid.scrollTop, 40);
      return box(0, 0);
    });
    // Frames come when the test says.
    const frames: FrameRequestCallback[] = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((frame) => frames.push(frame));
    try {
      useAppStore
        .getState()
        .requestCalendarEvent({ eventId: event.id, start: event.start, isAllDay: false });
      await setup({ backend });
      const details = await screen.findByRole("dialog", { name: event.subject });
      const grid = document.querySelector<HTMLElement>(".cal-tg__scroll")!;
      // In the middle of the grid's view.
      expect(200 + 900 - grid.scrollTop).toBe(200 + (300 - 40) / 2);

      // The grid's scroll to the event arrives before the next frame: the
      // details stay. The user's scrolling after it closes them.
      fireEvent.scroll(grid);
      expect(details).toBeInTheDocument();
      act(() => frames.splice(0).forEach((frame) => frame(0)));
      fireEvent.scroll(grid);
      await waitFor(() => expect(details).not.toBeInTheDocument());
    } finally {
      vi.restoreAllMocks();
    }
  });
});
