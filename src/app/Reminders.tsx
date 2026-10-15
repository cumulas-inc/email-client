import "./reminders.css";

import { clsx } from "clsx";
import {
  AlarmClock,
  Bell,
  BellRing,
  CalendarClock,
  ChevronDown,
  CircleCheckBig,
  Video,
  X,
} from "lucide-react";
import {
  useEffect,
  useId,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type RefObject,
} from "react";

import { useBackend } from "../api/context";
import { errorMessage } from "../api/errors";
import { useActiveReminders, useDismissReminders, useSnoozeReminder } from "../api/queries";
import type { ActiveReminder, SnoozeFor } from "../api/types";
import { Menu } from "../components/Menu";
import { toast } from "../components/toast";
import { useTasksStore } from "../modules/tasks/store";
import {
  isDue,
  joinUrl,
  reminderDetail,
  reminderTime,
  snoozeChoices,
  useReminderAnnouncer,
} from "./reminders";
import { useAppStore } from "./store";

/** How often the panel's "in 5 min" and "12 min overdue" catch up with the clock. */
const TICK_MS = 30_000;

/** The time, kept current while the component is shown. */
function useNow(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), TICK_MS);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

/**
 * The header's bell: how many reminders are due, and the Reminders panel
 * that lists them with Join, Snooze and Dismiss. Reminders that come due
 * are also read out (a live region), as the desktop shows them as
 * notifications.
 */
export function RemindersButton() {
  const { data: reminders = [] } = useActiveReminders();
  const open = useAppStore((s) => s.remindersOpen);
  const setOpen = useAppStore((s) => s.setRemindersOpen);
  const announcement = useReminderAnnouncer();
  const bellRef = useRef<HTMLButtonElement>(null);
  const panelId = useId();
  const due = reminders.filter(isDue).length;
  const label = due === 0 ? "Reminders" : `Reminders, ${due} due`;

  const close = (focusBell: boolean) => {
    setOpen(false);
    if (focusBell) bellRef.current?.focus();
  };

  return (
    <div className="reminders">
      <button
        ref={bellRef}
        type="button"
        className={clsx("icon-btn reminders__bell", due > 0 && "reminders__bell--due")}
        aria-label={label}
        title={label}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        onClick={() => setOpen(!open)}
      >
        {due > 0 ? (
          <BellRing size={18} aria-hidden="true" />
        ) : (
          <Bell size={18} aria-hidden="true" />
        )}
        {due > 0 && (
          <span className="reminders__badge" aria-hidden="true">
            {due > 99 ? "99+" : due}
          </span>
        )}
      </button>
      {open && (
        <RemindersPanel id={panelId} reminders={reminders} bellRef={bellRef} onClose={close} />
      )}
      <div className="visually-hidden" role="status">
        {announcement.text && <p key={announcement.seq}>{announcement.text}</p>}
      </div>
    </div>
  );
}

function RemindersPanel({
  id,
  reminders,
  bellRef,
  onClose,
}: {
  id: string;
  reminders: ActiveReminder[];
  bellRef: RefObject<HTMLButtonElement | null>;
  onClose: (focusBell: boolean) => void;
}) {
  const titleId = useId();
  const panelRef = useRef<HTMLElement>(null);
  const now = useNow();
  const dismiss = useDismissReminders();
  const due = reminders.filter(isDue);
  const snoozed = reminders.filter((r) => !isDue(r));

  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  // A click outside closes the panel (the bell toggles it itself).
  useEffect(() => {
    const onPointer = (e: MouseEvent) => {
      const target = e.target as Node;
      if (panelRef.current?.contains(target) || bellRef.current?.contains(target)) return;
      onClose(false);
    };
    document.addEventListener("mousedown", onPointer);
    return () => document.removeEventListener("mousedown", onPointer);
  }, [bellRef, onClose]);

  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === "Escape" && !e.defaultPrevented) {
      e.preventDefault();
      onClose(true);
    }
  };

  // Tabbing out of the panel closes it.
  const onBlur = (e: FocusEvent<HTMLElement>) => {
    const next = e.relatedTarget as Node | null;
    if (next && !panelRef.current?.contains(next) && !bellRef.current?.contains(next)) {
      onClose(false);
    }
  };

  /** A row went: the panel keeps the focus, so the keyboard isn't lost. */
  const keepFocus = () => panelRef.current?.focus();

  const dismissAll = () => {
    keepFocus();
    dismiss.mutate(
      reminders.map((r) => r.key),
      { onError: (e) => toast.error(errorMessage(e)) },
    );
  };

  return (
    <section
      id={id}
      ref={panelRef}
      className="reminders-panel"
      role="dialog"
      aria-labelledby={titleId}
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onBlur={onBlur}
    >
      <header className="reminders-panel__head">
        <h2 id={titleId} className="reminders-panel__title">
          Reminders
        </h2>
        {due.length > 0 && <span className="reminders-panel__count">{due.length} due</span>}
      </header>
      {reminders.length === 0 ? (
        <div className="reminders-panel__empty">
          <Bell size={28} strokeWidth={1.5} aria-hidden="true" />
          <p className="reminders-panel__empty-title">No reminders</p>
          <p className="reminders-panel__empty-text">
            Reminders for your events and tasks show up here when they&apos;re due.
          </p>
        </div>
      ) : (
        <div className="reminders-panel__scroll">
          {due.length > 0 && (
            <ul className="reminders-panel__list" aria-label="Due">
              {due.map((r) => (
                <ReminderRow
                  key={r.key}
                  reminder={r}
                  now={now}
                  onClose={onClose}
                  onGone={keepFocus}
                />
              ))}
            </ul>
          )}
          {snoozed.length > 0 && (
            <>
              <h3 className="reminders-panel__section">Snoozed</h3>
              <ul className="reminders-panel__list" aria-label="Snoozed">
                {snoozed.map((r) => (
                  <ReminderRow
                    key={r.key}
                    reminder={r}
                    now={now}
                    onClose={onClose}
                    onGone={keepFocus}
                  />
                ))}
              </ul>
            </>
          )}
        </div>
      )}
      {reminders.length > 0 && (
        <footer className="reminders-panel__foot">
          <button type="button" className="btn btn--subtle btn--sm" onClick={dismissAll}>
            Dismiss all
          </button>
        </footer>
      )}
    </section>
  );
}

function ReminderRow({
  reminder,
  now,
  onClose,
  onGone,
}: {
  reminder: ActiveReminder;
  now: Date;
  onClose: (focusBell: boolean) => void;
  onGone: () => void;
}) {
  const backend = useBackend();
  const snooze = useSnoozeReminder();
  const dismiss = useDismissReminders();
  const title = reminder.title.trim() || "(no title)";
  const time = reminderTime(reminder, now);
  const detail = reminderDetail(reminder, now);
  const join = isDue(reminder) ? joinUrl(reminder) : null;
  const failed = (e: unknown) => toast.error(errorMessage(e));

  const open = () => {
    onClose(false);
    if (reminder.kind === "event") {
      useAppStore.getState().requestCalendarEvent({
        eventId: reminder.itemId,
        // Local midnight for an all-day event: its local date either way.
        start: reminder.startsAt ?? reminder.fireAt,
        isAllDay: false,
      });
    } else {
      useAppStore.getState().setModule("tasks");
      const tasks = useTasksStore.getState();
      tasks.setView({ kind: "smart", id: "all" });
      tasks.selectTask(reminder.itemId);
    }
  };

  const snoozeFor = (choice: SnoozeFor) => {
    onGone();
    snooze.mutate({ key: reminder.key, snooze: choice }, { onError: failed });
  };

  return (
    <li className="reminder-row">
      <span className={clsx("reminder-row__icon", `reminder-row__icon--${reminder.kind}`)}>
        {reminder.kind === "event" ? (
          <CalendarClock size={16} aria-hidden="true" />
        ) : (
          <CircleCheckBig size={16} aria-hidden="true" />
        )}
      </span>
      <div className="reminder-row__text">
        <button
          type="button"
          className="reminder-row__title"
          dir="auto"
          title={reminder.kind === "event" ? "Open in Calendar" : "Open in Tasks"}
          onClick={open}
        >
          {title}
        </button>
        <p className="reminder-row__when">
          <span className={clsx("reminder-row__time", `reminder-row__time--${time.tone}`)}>
            {time.text}
          </span>
          {detail && (
            <>
              {" "}
              <span aria-hidden="true">·</span>{" "}
              <span className="reminder-row__detail" dir="auto">
                {detail}
              </span>
            </>
          )}
        </p>
        <div className="reminder-row__actions">
          {join && (
            <button
              type="button"
              className="btn btn--primary btn--sm"
              aria-label={`Join ${title}`}
              onClick={() => void backend.openLink(join).catch(failed)}
            >
              <Video size={14} aria-hidden="true" />
              Join
            </button>
          )}
          {isDue(reminder) && (
            <Menu
              label={`Snooze ${title}`}
              fixed
              items={snoozeChoices(reminder, now).map((c) => ({
                label: c.label,
                disabled: c.disabled,
                onSelect: () => snoozeFor(c.snooze),
              }))}
              trigger={(props) => (
                <button
                  {...props}
                  type="button"
                  className="btn btn--sm"
                  aria-label={`Snooze ${title}`}
                >
                  <AlarmClock size={14} aria-hidden="true" />
                  Snooze
                  <ChevronDown size={14} aria-hidden="true" />
                </button>
              )}
            />
          )}
          <button
            type="button"
            className="btn btn--subtle btn--sm reminder-row__dismiss"
            aria-label={`Dismiss ${title}`}
            onClick={() => {
              onGone();
              dismiss.mutate([reminder.key], { onError: failed });
            }}
          >
            <X size={14} aria-hidden="true" />
            Dismiss
          </button>
        </div>
      </div>
    </li>
  );
}
