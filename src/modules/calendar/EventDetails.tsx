import clsx from "clsx";
import { differenceInCalendarDays } from "date-fns";
import {
  AlignLeft,
  Bell,
  CalendarDays,
  Check,
  CircleCheck,
  CircleDashed,
  CircleHelp,
  CircleX,
  Clock,
  MapPin,
  Pencil,
  Repeat,
  Trash2,
  Users,
  Video,
  X,
} from "lucide-react";
import {
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { useBackend } from "../../api/context";
import type {
  Account,
  Attendee,
  Calendar,
  CalendarEvent,
  EventResponse,
  ResponseStatus,
} from "../../api/types";
import { Avatar } from "../../components/Avatar";
import { Dialog } from "../../components/Dialog";
import { toast } from "../../components/toast";
import { displayName } from "../../lib/format";
import { tintStyle } from "./colors";
import { eventRange } from "./dates";
import {
  RESPONSE_LABELS,
  SHOW_AS_LABELS,
  eventDateLine,
  eventTimeRange,
  eventTitle,
  isInvitation,
  isStruck,
} from "./eventText";
import { useDeleteEvent, useRespondToEvent } from "./queries";
import { sanitizeDescription } from "./sanitize";

interface EventDetailsProps {
  event: CalendarEvent;
  calendar: Calendar | undefined;
  account: Account | undefined;
  /** Element the card points at (the clicked event). */
  anchor: HTMLElement | null;
  onClose: (restoreFocus: boolean) => void;
  onEdit: (event: CalendarEvent) => void;
}

const WIDTH = 380;
const GAP = 8;

function reminderText(minutes: number | null): string {
  if (minutes === null) return "No reminder";
  if (minutes === 0) return "Reminder at the time of the event";
  if (minutes % 1440 === 0) {
    const d = minutes / 1440;
    return `Reminder ${d} day${d === 1 ? "" : "s"} before`;
  }
  if (minutes % 60 === 0) {
    const h = minutes / 60;
    return `Reminder ${h} hour${h === 1 ? "" : "s"} before`;
  }
  return `Reminder ${minutes} minutes before`;
}

const RESPONSE_ICONS: Partial<Record<ResponseStatus, ReactNode>> = {
  accepted: <CircleCheck size={14} className="cal-resp cal-resp--accepted" aria-hidden="true" />,
  organizer: <CircleCheck size={14} className="cal-resp cal-resp--accepted" aria-hidden="true" />,
  tentative: <CircleHelp size={14} className="cal-resp cal-resp--tentative" aria-hidden="true" />,
  declined: <CircleX size={14} className="cal-resp cal-resp--declined" aria-hidden="true" />,
  notResponded: <CircleDashed size={14} className="cal-resp cal-resp--none" aria-hidden="true" />,
  none: <CircleDashed size={14} className="cal-resp cal-resp--none" aria-hidden="true" />,
};

function attendeeSummary(attendees: Attendee[]): string {
  const count = (r: ResponseStatus[]) => attendees.filter((a) => r.includes(a.response)).length;
  const parts: string[] = [];
  const accepted = count(["accepted", "organizer"]);
  const tentative = count(["tentative"]);
  const declined = count(["declined"]);
  const waiting = count(["none", "notResponded"]);
  if (accepted) parts.push(`${accepted} accepted`);
  if (tentative) parts.push(`${tentative} tentative`);
  if (declined) parts.push(`${declined} declined`);
  if (waiting) parts.push(`${waiting} didn't respond`);
  return parts.join(", ");
}

const MY_RESPONSE: Partial<Record<ResponseStatus, string>> = {
  accepted: "You accepted this invitation.",
  tentative: "You tentatively accepted this invitation.",
  declined: "You declined this invitation.",
  notResponded: "You haven't responded yet.",
  none: "You haven't responded yet.",
};

const RESPONSE_DONE: Record<EventResponse, string> = {
  accept: "Accepted",
  tentative: "Tentatively accepted",
  decline: "Declined",
};

/** "All day" or "All day · 3 days" for all-day events (null for timed ones). */
function allDayText(event: CalendarEvent): string | null {
  if (!event.isAllDay) return null;
  const { start, end } = eventRange(event);
  const days = differenceInCalendarDays(end, start);
  return days > 1 ? `All day · ${days} days` : "All day";
}

function Row({
  icon,
  label,
  showLabel = false,
  aside,
  children,
}: {
  icon: ReactNode;
  label: string;
  /** Print the label above the value instead of only naming the icon. */
  showLabel?: boolean;
  /** Quiet text next to a printed label. */
  aside?: string;
  children: ReactNode;
}) {
  return (
    <div className="cal-details__row">
      <span className="cal-details__icon" title={showLabel ? undefined : label}>
        {icon}
        {!showLabel && <span className="visually-hidden">{label}</span>}
      </span>
      <div className="cal-details__value">
        {showLabel && (
          <div className="cal-details__label">
            {label}
            {aside && <span className="cal-details__aside">{aside}</span>}
          </div>
        )}
        {children}
      </div>
    </div>
  );
}

/** Peek card with an event's details, opened by clicking it. */
export function EventDetails({
  event,
  calendar,
  account,
  anchor,
  onClose,
  onEdit,
}: EventDetailsProps) {
  const backend = useBackend();
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [comment, setComment] = useState("");
  const [showComment, setShowComment] = useState(false);
  const [sendResponse, setSendResponse] = useState(true);
  const remove = useDeleteEvent();
  const respond = useRespondToEvent();

  const description = useMemo(
    () => (event.bodyHtml ? sanitizeDescription(event.bodyHtml) : null),
    [event.bodyHtml],
  );

  // Position next to the anchor: right of it if there is room, else left.
  useLayoutEffect(() => {
    const card = ref.current;
    if (!card) return;
    const rect = anchor?.isConnected ? anchor.getBoundingClientRect() : null;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const height = card.offsetHeight;
    let left: number;
    let top: number;
    // The card grows out of the event it belongs to.
    let originX = WIDTH / 2;
    let originY = height / 2;
    if (!rect) {
      left = (vw - WIDTH) / 2;
      top = Math.max(GAP, (vh - height) / 2);
    } else {
      if (rect.right + GAP + WIDTH <= vw - GAP) {
        left = rect.right + GAP;
        originX = 0;
      } else if (rect.left - GAP - WIDTH >= GAP) {
        left = rect.left - GAP - WIDTH;
        originX = WIDTH;
      } else {
        left = Math.min(Math.max(GAP, rect.left), vw - WIDTH - GAP);
      }
      top = Math.min(Math.max(GAP, rect.top), Math.max(GAP, vh - height - GAP));
      originY = Math.min(Math.max(0, rect.top + Math.min(rect.height, 40) / 2 - top), height);
    }
    card.style.left = `${Math.round(left)}px`;
    card.style.top = `${Math.round(top)}px`;
    card.style.transformOrigin = `${Math.round(originX)}px ${Math.round(originY)}px`;
  });

  useEffect(() => {
    ref.current?.focus();
  }, []);

  // Clicks outside close the card (but not clicks in a dialog it opened, or
  // on the event it belongs to, which toggles it).
  useEffect(() => {
    const onPointer = (e: PointerEvent) => {
      const target = e.target as Node | null;
      if (!target || ref.current?.contains(target)) return;
      if (anchor?.contains(target)) return;
      if (target instanceof Element && target.closest("dialog")) return;
      onClose(false);
    };
    // The card belongs to a spot in the grid, so it closes when that moves.
    const onScroll = (e: Event) => {
      if (e.target instanceof Node && ref.current?.contains(e.target)) return;
      if (e.target instanceof Element && e.target.closest("dialog")) return;
      onClose(false);
    };
    const onResize = () => onClose(false);
    document.addEventListener("pointerdown", onPointer);
    document.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("pointerdown", onPointer);
      document.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
    };
  }, [anchor, onClose]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    // Keys pressed in the delete confirmation belong to that dialog.
    if ((e.target as Element).closest("dialog")) return;
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose(true);
    }
  };

  const onDescriptionClick = (e: MouseEvent<HTMLDivElement>) => {
    const link = (e.target as Element).closest("a[href]");
    if (!link) return;
    e.preventDefault();
    const href = link.getAttribute("href") ?? "";
    if (/^(https?:|mailto:)/i.test(href)) void backend.openLink(href);
  };

  const doRespond = (response: EventResponse) => {
    respond.mutate(
      {
        eventId: event.id,
        response,
        comment: comment.trim() || null,
        sendResponse,
      },
      {
        onSuccess: () => {
          toast.success(RESPONSE_DONE[response]);
          setComment("");
          setShowComment(false);
          if (response === "decline") onClose(false);
        },
        onError: (e) => toast.error(e),
      },
    );
  };

  const doDelete = () => {
    remove.mutate(event.id, {
      onSuccess: () => {
        toast.success("Event deleted");
        setConfirmDelete(false);
        onClose(false);
      },
      onError: (e) => toast.error(e),
    });
  };

  const canEdit = calendar?.canEdit ?? false;
  const invitation = isInvitation(event);
  const organizer = event.organizer;
  const attendees = event.attendees.filter(
    (a) => !organizer || a.email.address.toLowerCase() !== organizer.address.toLowerCase(),
  );

  return (
    <div
      ref={ref}
      className="cal-details cal-tint"
      style={{ ...tintStyle(calendar?.color), width: WIDTH }}
      role="dialog"
      aria-labelledby={titleId}
      tabIndex={-1}
      onKeyDown={onKeyDown}
    >
      <div className="cal-details__header">
        <h2
          id={titleId}
          className={clsx("cal-details__title serif", isStruck(event) && "cal-ev--struck")}
        >
          {eventTitle(event)}
        </h2>
        <div className="cal-details__actions">
          {canEdit && (
            <button
              type="button"
              className="icon-btn"
              aria-label="Edit event"
              title="Edit"
              onClick={() => onEdit(event)}
            >
              <Pencil size={16} />
            </button>
          )}
          {canEdit && (
            <button
              type="button"
              className="icon-btn"
              aria-label="Delete event"
              title="Delete"
              onClick={() => setConfirmDelete(true)}
            >
              <Trash2 size={16} />
            </button>
          )}
          <button
            type="button"
            className="icon-btn"
            aria-label="Close"
            title="Close"
            onClick={() => onClose(true)}
          >
            <X size={16} />
          </button>
        </div>
      </div>

      <div className="cal-details__body selectable">
        <Row icon={<Clock size={16} />} label="When">
          <div>{eventDateLine(event)}</div>
          <div className="cal-details__muted">
            {allDayText(event) ?? eventTimeRange(event)}
            {event.isRecurring && (
              <span className="cal-details__recurring">
                <Repeat size={12} aria-hidden="true" /> Recurring
              </span>
            )}
          </div>
        </Row>

        {event.location && (
          <Row icon={<MapPin size={16} />} label="Location">
            {event.location}
          </Row>
        )}

        {event.isOnlineMeeting && (
          <Row icon={<Video size={16} />} label="Online meeting">
            <div className="cal-details__join">
              <span>
                {account?.kind === "microsoft" ? "Microsoft Teams meeting" : "Online meeting"}
              </span>
              {event.onlineMeetingUrl && (
                <button
                  type="button"
                  className="btn btn--primary btn--sm"
                  onClick={() => void backend.openLink(event.onlineMeetingUrl!)}
                >
                  <Video size={14} aria-hidden="true" />
                  Join
                </button>
              )}
            </div>
          </Row>
        )}

        {(attendees.length > 0 || (organizer && invitation)) && (
          <Row
            icon={<Users size={16} />}
            label="Attendees"
            showLabel
            aside={attendees.length > 0 ? attendeeSummary(event.attendees) : undefined}
          >
            <ul className="cal-people" aria-label="Organizer and attendees">
              {organizer && (
                <li className="cal-people__item">
                  <Avatar name={displayName(organizer)} colorKey={organizer.address} size={24} />
                  <span className="cal-people__name" title={organizer.address}>
                    {displayName(organizer)}
                  </span>
                  <span className="cal-people__role">Organizer</span>
                </li>
              )}
              {attendees.map((a) => (
                <li key={a.email.address} className="cal-people__item">
                  <Avatar name={displayName(a.email)} colorKey={a.email.address} size={24} />
                  <span className="cal-people__name" title={a.email.address}>
                    {displayName(a.email)}
                  </span>
                  {a.kind === "optional" && <span className="cal-people__role">Optional</span>}
                  <span
                    className="cal-people__response"
                    title={RESPONSE_LABELS[a.response]}
                    aria-label={RESPONSE_LABELS[a.response]}
                    role="img"
                  >
                    {RESPONSE_ICONS[a.response]}
                  </span>
                </li>
              ))}
            </ul>
          </Row>
        )}

        {(description || event.bodyPreview) && (
          <Row icon={<AlignLeft size={16} />} label="Description">
            {description ? (
              <div
                className="cal-details__description"
                onClick={onDescriptionClick}
                // Sanitized above with DOMPurify (no scripts, styles, forms or images).
                dangerouslySetInnerHTML={{ __html: description }}
              />
            ) : (
              <p className="cal-details__description">{event.bodyPreview}</p>
            )}
          </Row>
        )}

        <Row icon={<Bell size={16} />} label="Reminder">
          <span className="cal-details__muted">
            {reminderText(event.reminderMinutes)} · Show as {SHOW_AS_LABELS[event.showAs]}
          </span>
        </Row>

        <Row icon={<CalendarDays size={16} />} label="Calendar">
          <span className="cal-details__calendar">
            <span className="cal-details__swatch" aria-hidden="true" />
            {calendar?.name ?? "Calendar"}
            {account && <span className="cal-details__muted"> · {account.displayName}</span>}
          </span>
        </Row>
      </div>

      {invitation && (
        <div className="cal-details__rsvp" role="group" aria-label="Respond to invitation">
          <p className="cal-details__muted">{MY_RESPONSE[event.responseStatus]}</p>
          {showComment && (
            <textarea
              className="textarea cal-details__comment"
              placeholder="Add a message to the organizer (optional)"
              aria-label="Message to the organizer"
              value={comment}
              rows={2}
              onChange={(e) => setComment(e.target.value)}
            />
          )}
          <div className="cal-rsvp">
            <button
              type="button"
              className="btn btn--sm cal-rsvp__button"
              aria-pressed={event.responseStatus === "accepted"}
              disabled={respond.isPending}
              onClick={() => doRespond("accept")}
            >
              <Check size={14} aria-hidden="true" /> Accept
            </button>
            <button
              type="button"
              className="btn btn--sm cal-rsvp__button"
              aria-pressed={event.responseStatus === "tentative"}
              disabled={respond.isPending}
              onClick={() => doRespond("tentative")}
            >
              <CircleHelp size={14} aria-hidden="true" /> Tentative
            </button>
            <button
              type="button"
              className="btn btn--sm cal-rsvp__button"
              aria-pressed={event.responseStatus === "declined"}
              disabled={respond.isPending}
              onClick={() => doRespond("decline")}
            >
              <X size={14} aria-hidden="true" /> Decline
            </button>
          </div>
          <div className="cal-rsvp__options">
            {!showComment && (
              <button type="button" className="cal-link" onClick={() => setShowComment(true)}>
                Add a message
              </button>
            )}
            <label className="checkbox cal-rsvp__send">
              <input
                type="checkbox"
                checked={sendResponse}
                onChange={(e) => setSendResponse(e.target.checked)}
              />
              Send a response
            </label>
          </div>
        </div>
      )}

      <Dialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        title="Delete event?"
        width={420}
        footer={
          <>
            <button
              type="button"
              className="btn btn--destructive"
              disabled={remove.isPending}
              onClick={doDelete}
            >
              Delete
            </button>
            <button type="button" className="btn" onClick={() => setConfirmDelete(false)}>
              Cancel
            </button>
          </>
        }
      >
        <p>
          “{eventTitle(event)}” will be deleted
          {event.attendees.length > 0 && event.responseStatus === "organizer"
            ? " and a cancellation sent to attendees"
            : ""}
          .
        </p>
      </Dialog>
    </div>
  );
}
