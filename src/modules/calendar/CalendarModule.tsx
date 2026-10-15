import "./calendar.css";

import { addMonths, isSameDay, startOfDay, startOfMonth } from "date-fns";
import { CalendarDays, CalendarPlus, CircleAlert, EyeOff } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useAccounts } from "../../api/queries";
import type { Account, Calendar, CalendarEvent } from "../../api/types";
import { errorMessage } from "../../api/errors";
import { ModuleSidebar } from "../../app/ModuleSidebar";
import { type CalendarRequest, useAppStore } from "../../app/store";
import { EmptyState } from "../../components/EmptyState";
import { Spinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { Agenda } from "./Agenda";
import { CalendarList } from "./CalendarList";
import { defaultNewEventTimes, eventRange, timesFromRange } from "./dates";
import { EventDetails } from "./EventDetails";
import { type EditorRequest, EventEditor } from "./EventEditor";
import { drawnEvent, isTypingTarget, useNow } from "./hooks";
import { type CalendarView, VIEWS, rangeTitle, shiftAnchor, viewRange, weekLabel } from "./layout";
import { MiniMonth } from "./MiniMonth";
import { defaultCalendar } from "./model";
import { MonthView } from "./MonthView";
import { loadHiddenCalendars, loadView, saveHiddenCalendars, saveView } from "./prefs";
import { useCalendars, useEventsInRange } from "./queries";
import { TimeGrid } from "./TimeGrid";
import { Toolbar } from "./Toolbar";

const NO_ACCOUNTS: Account[] = [];
const NO_CALENDARS: Calendar[] = [];
const NO_EVENTS: CalendarEvent[] = [];
/** Range titles longer than this (in characters) get a short form for narrow windows. */
const SHORT_TITLE_AFTER = 24;

/** The day a request shows: an all-day event's (UTC) date, else the local day it starts on. */
function requestedDay(request: CalendarRequest): Date {
  const start = new Date(request.start);
  return request.isAllDay
    ? new Date(start.getUTCFullYear(), start.getUTCMonth(), start.getUTCDate())
    : startOfDay(start);
}

interface Selection {
  event: CalendarEvent;
  /** The event as shown, which the details point at; null when it isn't shown. */
  anchor: HTMLElement | null;
}

export function CalendarModule() {
  const now = useNow();
  const today = useMemo(() => startOfDay(now), [now]);
  const openSettings = useAppStore((s) => s.openSettings);

  const [view, setView] = useState<CalendarView>(loadView);
  const [anchor, setAnchor] = useState<Date>(() => startOfDay(new Date()));
  const [hidden, setHidden] = useState<ReadonlySet<string>>(() => new Set(loadHiddenCalendars()));
  const [selection, setSelection] = useState<Selection | null>(null);
  const [editor, setEditor] = useState<EditorRequest | null>(null);
  const [editorKey, setEditorKey] = useState(0);
  const mainRef = useRef<HTMLElement>(null);

  const accounts = useAccounts().data ?? NO_ACCOUNTS;
  const calendarsQuery = useCalendars();
  const calendars = calendarsQuery.data ?? NO_CALENDARS;
  const calendarsById = useMemo(() => new Map(calendars.map((c) => [c.id, c])), [calendars]);
  const accountsById = useMemo(() => new Map(accounts.map((a) => [a.id, a])), [accounts]);

  const range = useMemo(() => viewRange(view, anchor), [view, anchor]);
  const eventsQuery = useEventsInRange(range.start, range.end);
  const allEvents = eventsQuery.data ?? NO_EVENTS;
  const events = useMemo(
    () => allEvents.filter((e) => !hidden.has(e.calendarId)),
    [allEvents, hidden],
  );

  // Asked to show an event (a reminder was clicked): its day, then its
  // details once the day's events are in.
  const calendarRequest = useAppStore((s) => s.calendarRequest);
  const [seenRequest, setSeenRequest] = useState<CalendarRequest | null>(null);
  const [pendingEventId, setPendingEventId] = useState<string | null>(null);
  const [missingEvents, setMissingEvents] = useState(0);
  if (calendarRequest && calendarRequest !== seenRequest) {
    const day = requestedDay(calendarRequest);
    setSeenRequest(calendarRequest);
    setAnchor(day);
    // A weekend day isn't in the work week: the whole week then, this time.
    if (!viewRange(view, day).days.some((d) => isSameDay(d, day))) setView("week");
    setSelection(null);
    setPendingEventId(calendarRequest.eventId);
  }
  useEffect(() => {
    if (calendarRequest) useAppStore.getState().clearCalendarRequest();
  }, [calendarRequest]);
  useEffect(() => {
    if (missingEvents > 0) toast.info("That event isn't in the calendar anymore.");
  }, [missingEvents]);

  const changeView = useCallback((next: CalendarView) => {
    setView(next);
    saveView(next);
    setSelection(null);
  }, []);

  const step = useCallback(
    (direction: 1 | -1) => {
      setAnchor((a) => shiftAnchor(view, a, direction));
      setSelection(null);
    },
    [view],
  );

  const goToday = useCallback(() => {
    setAnchor(startOfDay(new Date()));
    setSelection(null);
  }, []);

  const openDay = useCallback(
    (day: Date) => {
      setAnchor(startOfDay(day));
      changeView("day");
    },
    [changeView],
  );

  const openEditor = useCallback((request: EditorRequest) => {
    setSelection(null);
    setEditor(request);
    setEditorKey((k) => k + 1);
  }, []);

  const newEvent = useCallback(() => {
    const day = view === "month" && anchor.getMonth() === today.getMonth() ? today : anchor;
    openEditor({ mode: "create", times: defaultNewEventTimes(day, new Date()) });
  }, [anchor, today, view, openEditor]);

  const createAt = useCallback(
    (start: Date, end: Date, allDay: boolean) =>
      openEditor({ mode: "create", times: timesFromRange(start, end, allDay) }),
    [openEditor],
  );

  const toggleCalendar = (calendarId: string, visible: boolean) => {
    const next = new Set(hidden);
    if (visible) next.delete(calendarId);
    else next.add(calendarId);
    setHidden(next);
    saveHiddenCalendars(next);
    if (!visible && selection?.event.calendarId === calendarId) setSelection(null);
  };

  const showAll = () => {
    setHidden(new Set());
    saveHiddenCalendars([]);
  };

  const selectEvent = useCallback((event: CalendarEvent, element: HTMLElement) => {
    setSelection((current) => (current?.event.id === event.id ? null : { event, anchor: element }));
  }, []);

  const closeDetails = useCallback(
    (restoreFocus: boolean) => {
      // Opened from a reminder, the details have no anchor: the event as drawn,
      // or the calendar's title when it isn't (its calendar is hidden).
      const element = selection?.anchor?.isConnected
        ? selection.anchor
        : selection &&
          (drawnEvent(mainRef.current, selection.event.id) ??
            mainRef.current?.querySelector<HTMLElement>(".cal-toolbar__title"));
      setSelection(null);
      if (restoreFocus) element?.focus();
    },
    [selection],
  );

  const onSaved = (event: CalendarEvent) => {
    setEditor(null);
    const { start } = eventRange(event);
    if (start < range.start || start >= range.end) setAnchor(startOfDay(start));
  };

  // Keyboard shortcuts: Ctrl+Alt+1–4 views, Alt+arrows and Page Up/Down move,
  // T goes to today, Ctrl+N creates.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || document.querySelector("dialog[open]")) return;
      // Dialogs (the details, the Reminders panel) and menus handle their own keys.
      if (e.target instanceof Element && e.target.closest("dialog, [role=dialog], [role=menu]")) {
        return;
      }
      if (e.ctrlKey && e.altKey && !e.metaKey && !e.shiftKey) {
        const digit = /^(?:Digit|Numpad)([1-4])$/.exec(e.code)?.[1] ?? e.key;
        const next = VIEWS[Number(digit) - 1];
        if (next) {
          e.preventDefault();
          changeView(next);
        }
        return;
      }
      if (e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
        if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
          e.preventDefault();
          step(e.key === "ArrowLeft" ? -1 : 1);
        }
        return;
      }
      if (e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        newEvent();
        return;
      }
      if (e.ctrlKey || e.altKey || e.metaKey || isTypingTarget(e.target)) return;
      const inGrid = e.target instanceof Element && e.target.closest(".cal-tg__scroll");
      if ((e.key === "PageUp" || e.key === "PageDown") && !inGrid) {
        e.preventDefault();
        step(e.key === "PageUp" ? -1 : 1);
      } else if (e.key === "t" || e.key === "T") {
        e.preventDefault();
        goToday();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [changeView, step, newEvent, goToday]);

  const eventsSettled = eventsQuery.isSuccess && !eventsQuery.isPlaceholderData;
  if (pendingEventId && eventsSettled) {
    const event = allEvents.find((e) => e.id === pendingEventId);
    if (event) {
      setPendingEventId(null);
      // The details find the event on the day drawn (see EventDetails).
      setSelection({ event, anchor: null });
    } else if (!eventsQuery.isFetching) {
      // Not in events that may be out of date (cached, and refetching since
      // the event came): only once they're fresh is it really gone.
      setPendingEventId(null);
      setMissingEvents((n) => n + 1);
    }
  }

  const selectedEvent = selection
    ? (allEvents.find((e) => e.id === selection.event.id) ?? selection.event)
    : null;
  const selectedCalendar = selectedEvent ? calendarsById.get(selectedEvent.calendarId) : undefined;
  const allHidden = calendars.length > 0 && calendars.every((c) => hidden.has(c.id));
  const noCalendars = calendarsQuery.isSuccess && calendars.length === 0;
  const nothingScheduled = eventsSettled && !allHidden && events.length === 0;
  const title = rangeTitle(view, anchor);
  // Narrow windows shorten titles that wouldn't fit ("Wed, Oct 14, 2026");
  // shorter ones only in the narrowest (a small window at a larger text size).
  const shortTitle = rangeTitle(view, anchor, true);
  const shortenEarly = title.length > SHORT_TITLE_AFTER;
  // In month view the mini month highlights the month itself, not the 6-week grid.
  const miniRange =
    view === "month"
      ? { start: startOfMonth(anchor), end: addMonths(startOfMonth(anchor), 1) }
      : range;

  return (
    <div className="calendar module-layout">
      <ModuleSidebar
        label="Calendar navigation"
        className="cal-sidebar"
        action={
          <button
            type="button"
            className="btn btn--primary cal-sidebar__new"
            onClick={newEvent}
            aria-keyshortcuts="Control+N"
            disabled={noCalendars}
          >
            <CalendarPlus size={18} strokeWidth={1.75} aria-hidden="true" />
            New event
            <span className="kbd" aria-hidden="true">
              Ctrl N
            </span>
          </button>
        }
      >
        <MiniMonth
          anchor={anchor}
          rangeStart={miniRange.start}
          rangeEnd={miniRange.end}
          today={today}
          onSelect={(day) => {
            setAnchor(day);
            setSelection(null);
          }}
        />
        {calendarsQuery.isLoading ? (
          <div className="cal-sidebar__loading">
            <Spinner label="Loading calendars" />
          </div>
        ) : (
          <CalendarList
            accounts={accounts}
            calendars={calendars}
            hidden={hidden}
            onToggle={toggleCalendar}
          />
        )}
        {calendars.length > 0 && (
          <Agenda
            today={today}
            hidden={hidden}
            calendars={calendarsById}
            selectedId={selectedEvent?.id ?? null}
            onSelect={selectEvent}
            onOpenDay={openDay}
          />
        )}
      </ModuleSidebar>

      <section className="cal-main card" aria-label="Calendar" ref={mainRef}>
        <Toolbar
          view={view}
          title={title}
          shortTitle={shortTitle}
          shortenEarly={shortenEarly}
          weekLabel={weekLabel(view, anchor)}
          busy={eventsQuery.isFetching && !eventsQuery.isLoading}
          onToday={goToday}
          onStep={step}
          onView={changeView}
        />
        {allHidden && (
          <div className="cal-banner" role="status">
            <EyeOff size={16} aria-hidden="true" />
            <span>All calendars are hidden.</span>
            <button type="button" className="btn btn--subtle btn--sm" onClick={showAll}>
              Show all
            </button>
          </div>
        )}
        {eventsQuery.isError && (
          <div className="cal-banner cal-banner--error" role="alert">
            <CircleAlert size={16} aria-hidden="true" />
            <span>Couldn't load events: {errorMessage(eventsQuery.error)}</span>
            <button
              type="button"
              className="btn btn--subtle btn--sm"
              onClick={() => void eventsQuery.refetch()}
            >
              Retry
            </button>
          </div>
        )}
        <div className="cal-view">
          {noCalendars ? (
            <EmptyState
              icon={<CalendarDays size={40} />}
              title="No calendars yet"
              description="Add an account with a calendar to see and plan your events."
              action={
                <button type="button" className="btn btn--primary" onClick={() => openSettings()}>
                  Manage accounts
                </button>
              }
            />
          ) : view === "month" ? (
            <MonthView
              anchor={anchor}
              days={range.days}
              events={events}
              calendars={calendarsById}
              today={today}
              selectedId={selectedEvent?.id ?? null}
              onSelect={selectEvent}
              onCreate={createAt}
              onOpenDay={openDay}
            />
          ) : (
            <TimeGrid
              days={range.days}
              events={events}
              calendars={calendarsById}
              now={now}
              selectedId={selectedEvent?.id ?? null}
              onSelect={selectEvent}
              onCreate={createAt}
              onOpenDay={view === "day" ? undefined : openDay}
            />
          )}
          {eventsQuery.isLoading && !noCalendars && (
            <div className="cal-view__loading">
              <Spinner size={24} label="Loading events" />
            </div>
          )}
          {nothingScheduled && (
            <p className="cal-view__hint" role="status">
              Nothing scheduled
            </p>
          )}
        </div>
      </section>

      {selection && selectedEvent && (
        <EventDetails
          key={selectedEvent.id}
          event={selectedEvent}
          calendar={selectedCalendar}
          account={accountsById.get(selectedEvent.accountId)}
          anchor={selection.anchor}
          boundary={mainRef}
          onClose={closeDetails}
          onEdit={(event) => openEditor({ mode: "edit", event })}
        />
      )}

      {editor && (
        <EventEditor
          key={editorKey}
          request={editor}
          accounts={accounts}
          calendars={calendars}
          defaultCalendarId={defaultCalendar(accounts, calendars, hidden)?.id ?? null}
          onClose={() => setEditor(null)}
          onSaved={onSaved}
        />
      )}
    </div>
  );
}
