import "./calendar.css";

import { addMonths, startOfDay, startOfMonth } from "date-fns";
import { CalendarDays, CalendarPlus, CircleAlert, EyeOff } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useAccounts } from "../../api/queries";
import type { Account, Calendar, CalendarEvent } from "../../api/types";
import { errorMessage } from "../../api/errors";
import { useAppStore } from "../../app/store";
import { EmptyState } from "../../components/EmptyState";
import { Spinner } from "../../components/Spinner";
import { CalendarList } from "./CalendarList";
import { defaultNewEventTimes, eventRange, timesFromRange } from "./dates";
import { EventDetails } from "./EventDetails";
import { type EditorRequest, EventEditor } from "./EventEditor";
import { isTypingTarget, useNow } from "./hooks";
import { type CalendarView, VIEWS, rangeTitle, shiftAnchor, viewRange } from "./layout";
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

interface Selection {
  event: CalendarEvent;
  anchor: HTMLElement;
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
      const element = selection?.anchor;
      setSelection(null);
      if (restoreFocus && element?.isConnected) element.focus();
    },
    [selection],
  );

  const onSaved = (event: CalendarEvent) => {
    setEditor(null);
    const { start } = eventRange(event);
    if (start < range.start || start >= range.end) setAnchor(startOfDay(start));
  };

  // Keyboard shortcuts (Outlook style).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || document.querySelector("dialog[open]")) return;
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

  const selectedEvent = selection
    ? (allEvents.find((e) => e.id === selection.event.id) ?? selection.event)
    : null;
  const selectedCalendar = selectedEvent ? calendarsById.get(selectedEvent.calendarId) : undefined;
  const allHidden = calendars.length > 0 && calendars.every((c) => hidden.has(c.id));
  const noCalendars = calendarsQuery.isSuccess && calendars.length === 0;
  const settled = eventsQuery.isSuccess && !eventsQuery.isPlaceholderData;
  const nothingScheduled = settled && !allHidden && events.length === 0;
  // In month view the mini month highlights the month itself, not the 6-week grid.
  const miniRange =
    view === "month"
      ? { start: startOfMonth(anchor), end: addMonths(startOfMonth(anchor), 1) }
      : range;

  return (
    <div className="calendar">
      <aside className="cal-sidebar" aria-label="Calendar navigation">
        <button
          type="button"
          className="btn btn--primary cal-sidebar__new"
          onClick={newEvent}
          title="New event (Ctrl+N)"
          disabled={noCalendars}
        >
          <CalendarPlus size={16} aria-hidden="true" />
          New event
        </button>
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
      </aside>

      <section className="cal-main" aria-label="Calendar">
        <Toolbar
          view={view}
          title={rangeTitle(view, anchor)}
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
