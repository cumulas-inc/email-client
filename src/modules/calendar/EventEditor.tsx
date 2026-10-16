import {
  AlignLeft,
  Bell,
  CalendarDays,
  CircleAlert,
  Clock,
  MapPin,
  Users,
  Video,
} from "lucide-react";
import { type FormEvent, type ReactNode, useId, useRef, useState } from "react";

import { errorMessage } from "../../api/errors";
import type {
  Account,
  Calendar,
  CalendarEvent,
  EmailAddress,
  EventDraft,
  ShowAs,
} from "../../api/types";
import { Dialog } from "../../components/Dialog";
import { toast } from "../../components/toast";
import { AttendeeInput } from "./AttendeeInput";
import { tintStyle } from "./colors";
import {
  type EventTimes,
  htmlToText,
  moveStart,
  setAllDay,
  textToHtml,
  timesFromEvent,
  timesToIso,
  validateTimes,
} from "./dates";
import { SHOW_AS_LABELS } from "./eventText";
import { groupCalendars } from "./model";
import { useSaveEvent } from "./queries";

export type EditorRequest =
  | { mode: "create"; times: EventTimes; calendarId?: string | null }
  | { mode: "edit"; event: CalendarEvent };

interface EventEditorProps {
  request: EditorRequest;
  accounts: Account[];
  calendars: Calendar[];
  defaultCalendarId: string | null;
  onClose: () => void;
  onSaved: (event: CalendarEvent) => void;
}

const REMINDERS: (number | null)[] = [null, 0, 5, 15, 30, 60, 1440];

function reminderLabel(minutes: number | null): string {
  if (minutes === null) return "Don't remind me";
  if (minutes === 0) return "At the time of the event";
  if (minutes % 1440 === 0)
    return minutes === 1440 ? "1 day before" : `${minutes / 1440} days before`;
  if (minutes % 60 === 0) return minutes === 60 ? "1 hour before" : `${minutes / 60} hours before`;
  return `${minutes} minutes before`;
}

const SHOW_AS_ORDER: ShowAs[] = ["free", "workingElsewhere", "tentative", "busy", "outOfOffice"];

function FormRow({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <div className="cal-form__row">
      <span className="cal-form__icon" aria-hidden="true">
        {icon}
      </span>
      <div className="cal-form__fields">{children}</div>
    </div>
  );
}

/** Dialog to create or edit an event. */
export function EventEditor({
  request,
  accounts,
  calendars,
  defaultCalendarId,
  onClose,
  onSaved,
}: EventEditorProps) {
  const editing = request.mode === "edit" ? request.event : null;
  const formId = useId();
  const ids = {
    title: useId(),
    calendar: useId(),
    attendees: useId(),
    location: useId(),
    reminder: useId(),
    showAs: useId(),
    description: useId(),
    error: useId(),
  };
  const titleRef = useRef<HTMLInputElement | null>(null);
  const save = useSaveEvent();

  const writable = calendars.filter((c) => c.canEdit || c.id === editing?.calendarId);
  const initialCalendar =
    editing?.calendarId ??
    (request.mode === "create" ? request.calendarId : null) ??
    defaultCalendarId ??
    writable[0]?.id ??
    "";

  const [subject, setSubject] = useState(editing?.subject ?? "");
  const [calendarId, setCalendarId] = useState(initialCalendar);
  const [times, setTimes] = useState<EventTimes>(() =>
    request.mode === "edit" ? timesFromEvent(request.event) : request.times,
  );
  const [location, setLocation] = useState(editing?.location ?? "");
  const [attendees, setAttendees] = useState<EmailAddress[]>(
    () => editing?.attendees.map((a) => a.email) ?? [],
  );
  const [online, setOnline] = useState(editing?.isOnlineMeeting ?? false);
  const [reminder, setReminder] = useState<number | null>(editing ? editing.reminderMinutes : 15);
  const [showAs, setShowAs] = useState<ShowAs>(editing?.showAs ?? "busy");
  const [description, setDescription] = useState(() =>
    editing?.bodyHtml ? htmlToText(editing.bodyHtml) : (editing?.bodyPreview ?? ""),
  );
  const [descriptionDirty, setDescriptionDirty] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);

  const calendar = calendars.find((c) => c.id === calendarId);
  const account = accounts.find((a) => a.id === calendar?.accountId);
  const isMicrosoft = account?.kind === "microsoft";
  const titleError = subject.trim() ? null : "Add a title for the event.";
  const timeError = validateTimes(times);
  const reminderOptions =
    reminder !== null && !REMINDERS.includes(reminder)
      ? [...REMINDERS, reminder].sort((a, b) => (a ?? -1) - (b ?? -1))
      : REMINDERS;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    setSubmitted(true);
    setServerError(null);
    if (titleError) {
      titleRef.current?.focus();
      return;
    }
    if (timeError || !calendarId) return;
    const { start, end } = timesToIso(times);
    const draft: EventDraft = {
      calendarId,
      subject: subject.trim(),
      location: location.trim() || null,
      bodyHtml: descriptionDirty
        ? description.trim()
          ? textToHtml(description)
          : null
        : (editing?.bodyHtml ?? (description.trim() ? textToHtml(description) : null)),
      start,
      end,
      isAllDay: times.allDay,
      attendees,
      reminderMinutes: reminder,
      showAs,
      isOnlineMeeting: isMicrosoft && online,
    };
    save.mutate(
      { eventId: editing?.id ?? null, draft },
      {
        onSuccess: (event) => {
          toast.success(editing ? "Event updated" : "Event created");
          onSaved(event);
        },
        onError: (err) => setServerError(errorMessage(err)),
      },
    );
  };

  const groups = groupCalendars(accounts, writable);

  return (
    <Dialog
      open
      onClose={onClose}
      title={editing ? "Edit event" : "New event"}
      width={640}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="submit"
            form={formId}
            className="btn btn--primary"
            disabled={save.isPending}
          >
            {save.isPending ? "Saving…" : editing ? "Save changes" : "Save"}
          </button>
        </>
      }
    >
      <form id={formId} className="cal-form" noValidate onSubmit={submit}>
        {serverError && (
          <div className="cal-form__alert" role="alert" id={ids.error}>
            <CircleAlert size={16} aria-hidden="true" />
            <span>{serverError}</span>
          </div>
        )}

        <div className="cal-form__title-row">
          <span
            className="cal-form__swatch cal-tint"
            style={tintStyle(calendar?.color)}
            aria-hidden="true"
          />
          <div className="field cal-form__title-field">
            <label htmlFor={ids.title} className="visually-hidden">
              Title
            </label>
            <input
              id={ids.title}
              ref={(el) => {
                titleRef.current = el;
                // Native <dialog> focuses [autofocus] when it opens.
                el?.setAttribute("autofocus", "");
              }}
              className="input cal-form__title"
              dir="auto"
              placeholder="Add a title"
              value={subject}
              maxLength={255}
              aria-invalid={submitted && titleError ? true : undefined}
              aria-describedby={submitted && titleError ? `${ids.title}-error` : undefined}
              onChange={(e) => setSubject(e.target.value)}
            />
            {submitted && titleError && (
              <span id={`${ids.title}-error`} className="field__error">
                {titleError}
              </span>
            )}
          </div>
        </div>

        <FormRow icon={<CalendarDays size={16} />}>
          <label htmlFor={ids.calendar} className="visually-hidden">
            Calendar
          </label>
          <div className="cal-form__inline">
            <select
              id={ids.calendar}
              className="select"
              aria-describedby={account ? `${ids.calendar}-account` : undefined}
              value={calendarId}
              disabled={!!editing}
              title={editing ? "Events can't be moved to another calendar" : undefined}
              onChange={(e) => setCalendarId(e.target.value)}
            >
              {groups.map((g) => (
                <optgroup
                  key={g.accountId}
                  label={g.account ? `${g.account.displayName} (${g.account.email})` : "Other"}
                >
                  {g.calendars.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </optgroup>
              ))}
            </select>
            {account && (
              <span
                id={`${ids.calendar}-account`}
                className="cal-form__account"
                title={account.email}
              >
                {account.email}
              </span>
            )}
          </div>
        </FormRow>

        <FormRow icon={<Clock size={16} />}>
          <div className="cal-form__times" role="group" aria-label="Date and time">
            <div className="cal-form__when">
              <input
                type="date"
                className="input cal-form__date"
                aria-label="Start date"
                value={times.startDate}
                required
                onChange={(e) =>
                  e.target.value && setTimes((t) => moveStart(t, e.target.value, t.startTime))
                }
              />
              {!times.allDay && (
                <input
                  type="time"
                  className="input cal-form__time"
                  aria-label="Start time"
                  step={300}
                  value={times.startTime}
                  required
                  onChange={(e) =>
                    e.target.value && setTimes((t) => moveStart(t, t.startDate, e.target.value))
                  }
                />
              )}
              <span className="cal-form__to" aria-hidden="true">
                to
              </span>
              <input
                type="date"
                className="input cal-form__date"
                aria-label={times.allDay ? "End date (last day)" : "End date"}
                value={times.endDate}
                required
                aria-invalid={submitted && timeError ? true : undefined}
                onChange={(e) =>
                  e.target.value && setTimes((t) => ({ ...t, endDate: e.target.value }))
                }
              />
              {!times.allDay && (
                <input
                  type="time"
                  className="input cal-form__time"
                  aria-label="End time"
                  step={300}
                  value={times.endTime}
                  required
                  aria-invalid={submitted && timeError ? true : undefined}
                  onChange={(e) =>
                    e.target.value && setTimes((t) => ({ ...t, endTime: e.target.value }))
                  }
                />
              )}
            </div>
            <label className="cal-switch">
              <input
                type="checkbox"
                role="switch"
                checked={times.allDay}
                onChange={(e) => setTimes((t) => setAllDay(t, e.target.checked))}
              />
              <span className="cal-switch__track" aria-hidden="true" />
              All day
            </label>
          </div>
          {timeError && (submitted || times.endDate < times.startDate) && (
            <span className="field__error" role="alert">
              {timeError}
            </span>
          )}
        </FormRow>

        <FormRow icon={<Users size={16} />}>
          <label htmlFor={ids.attendees} className="visually-hidden">
            Invite attendees
          </label>
          <AttendeeInput
            id={ids.attendees}
            value={attendees}
            onChange={setAttendees}
            placeholder="Invite attendees"
          />
        </FormRow>

        <FormRow icon={<MapPin size={16} />}>
          <div className="cal-form__inline">
            <label htmlFor={ids.location} className="visually-hidden">
              Location
            </label>
            <input
              id={ids.location}
              className="input"
              dir="auto"
              placeholder="Add a location"
              value={location}
              onChange={(e) => setLocation(e.target.value)}
            />
            {isMicrosoft && (
              <label className="cal-switch" title="Add an online meeting link">
                <input
                  type="checkbox"
                  role="switch"
                  checked={online}
                  onChange={(e) => setOnline(e.target.checked)}
                />
                <span className="cal-switch__track" aria-hidden="true" />
                <Video size={14} aria-hidden="true" />
                Teams meeting
              </label>
            )}
          </div>
        </FormRow>

        <FormRow icon={<Bell size={16} />}>
          <div className="cal-form__inline">
            <div className="field">
              <label htmlFor={ids.reminder} className="field__label">
                Reminder
              </label>
              <select
                id={ids.reminder}
                className="select"
                value={reminder === null ? "none" : String(reminder)}
                onChange={(e) =>
                  setReminder(e.target.value === "none" ? null : Number(e.target.value))
                }
              >
                {reminderOptions.map((m) => (
                  <option key={m ?? "none"} value={m === null ? "none" : String(m)}>
                    {reminderLabel(m)}
                  </option>
                ))}
              </select>
            </div>
            <div className="field">
              <label htmlFor={ids.showAs} className="field__label">
                Availability
              </label>
              <select
                id={ids.showAs}
                className="select"
                value={showAs}
                onChange={(e) => setShowAs(e.target.value as ShowAs)}
              >
                {SHOW_AS_ORDER.map((s) => (
                  <option key={s} value={s}>
                    {SHOW_AS_LABELS[s]}
                  </option>
                ))}
              </select>
            </div>
          </div>
        </FormRow>

        <FormRow icon={<AlignLeft size={16} />}>
          <label htmlFor={ids.description} className="visually-hidden">
            Description
          </label>
          <textarea
            id={ids.description}
            className="textarea cal-form__description"
            dir="auto"
            placeholder="Add a description"
            rows={4}
            value={description}
            onChange={(e) => {
              setDescription(e.target.value);
              setDescriptionDirty(true);
            }}
          />
        </FormRow>
      </form>
    </Dialog>
  );
}
