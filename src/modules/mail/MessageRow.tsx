import {
  ArrowDown,
  Check,
  CircleAlert,
  Flag,
  Mail,
  MailOpen,
  Paperclip,
  Trash2,
} from "lucide-react";
import { type DragEvent, type MouseEvent, memo } from "react";

import type { MessageSummary, SearchHit } from "../../api/types";
import { Avatar } from "../../components/Avatar";
import { displayName } from "../../lib/format";
import { type HitPatterns, leadToFirstHit } from "../../lib/search/highlight";
import { FUTURE_NOTE, isFutureDated, listDate, messageDate } from "./dates";
import { optionId } from "./dom";
import { Highlighted, Runs } from "./search/Highlighted";

export interface RowHandlers {
  onClick: (e: MouseEvent, id: string) => void;
  onDoubleClick: (message: MessageSummary) => void;
  onToggleCheck: (id: string) => void;
  onToggleFlag: (id: string, flagged: boolean) => void;
  onToggleRead: (id: string, read: boolean) => void;
  onDelete: (id: string) => void;
  onDragStart: (e: DragEvent, message: MessageSummary) => void;
  onDragEnd: () => void;
}

interface RowProps {
  message: MessageSummary;
  selected: boolean;
  active: boolean;
  /** Show checkboxes instead of avatars (several messages selected). */
  checkMode: boolean;
  /** Show recipients instead of the sender (Sent Items, Drafts). */
  showRecipients: boolean;
  /** Folder/account shown in search results and the unified inbox. */
  context: string | null;
  contextColor: string | null;
  handlers: RowHandlers;
  /** What the search the row was found by found: marked in it. */
  highlight?: HitPatterns | null;
}

/** Characters a search result's preview keeps before what it found, to show it on one line. */
const PREVIEW_LEAD = 18;

/** Who the row is about, and whether that's a stand-in ("(no sender)"). */
function who(message: MessageSummary, showRecipients: boolean): [string, boolean] {
  if (showRecipients) {
    const names = message.to.map((a) => displayName(a));
    return names.length > 0 ? [names.join("; "), false] : ["(no recipients)", true];
  }
  return [displayName(message.from), !message.from];
}

export const MessageRow = memo(function MessageRow({
  message: m,
  selected,
  active,
  checkMode,
  showRecipients,
  context,
  contextColor,
  handlers,
  highlight = null,
}: RowProps) {
  const [sender, noSender] = who(m, showRecipients);
  // The body's words around a match, from the search, in place of the preview.
  const snippet = (m as Partial<SearchHit>).snippet ?? null;
  const date = listDate(m.receivedAt);
  const sent = messageDate(m.receivedAt);
  const future = !!sent && isFutureDated(sent);
  const noSubject = !m.subject.trim();
  const subject = m.subject.trim() || "(no subject)";
  const label = [
    m.isRead ? null : "Unread",
    m.isDraft ? "Draft" : null,
    sender,
    subject,
    date || "No date",
    m.isFlagged ? "Flagged" : null,
    m.hasAttachments ? "Has attachments" : null,
    m.importance === "high" ? "Important" : null,
    context,
  ]
    .filter(Boolean)
    .join(", ");

  const stop = (e: MouseEvent) => e.stopPropagation();

  return (
    <div
      id={optionId(m.id)}
      role="option"
      aria-selected={selected}
      aria-label={label}
      tabIndex={active ? 0 : -1}
      className={[
        "mail-row",
        !m.isRead && "is-unread",
        selected && "is-selected",
        active && "is-active",
        m.isFlagged && "is-flagged",
        checkMode && "is-check-mode",
      ]
        .filter(Boolean)
        .join(" ")}
      draggable
      onClick={(e) => handlers.onClick(e, m.id)}
      onDoubleClick={() => handlers.onDoubleClick(m)}
      onMouseDown={(e) => {
        // Shift+click selects a range, not text.
        if (e.shiftKey) e.preventDefault();
      }}
      onDragStart={(e) => handlers.onDragStart(e, m)}
      onDragEnd={handlers.onDragEnd}
    >
      <div
        className="mail-row__lead"
        onClick={(e) => {
          e.stopPropagation();
          handlers.onToggleCheck(m.id);
        }}
        title={selected ? "Unselect" : "Select"}
      >
        <span className="mail-row__avatar">
          <Avatar
            name={displayName(showRecipients ? m.to[0] : m.from)}
            colorKey={(showRecipients ? m.to[0] : m.from)?.address}
            size={34}
          />
        </span>
        <span className={`mail-row__check${selected ? " is-checked" : ""}`} aria-hidden="true">
          {selected && <Check size={13} strokeWidth={2.75} />}
        </span>
      </div>
      <div className="mail-row__body">
        <div className="mail-row__line">
          <span
            className={`mail-row__sender${noSender ? " is-empty" : ""}`}
            dir="auto"
            title={sender}
          >
            {showRecipients || noSender ? (
              sender
            ) : (
              <Highlighted text={sender} patterns={highlight?.sender} />
            )}
          </span>
          <span className="mail-row__status">
            {m.importance === "high" && (
              <CircleAlert size={14} className="mail-row__importance" aria-label="Important" />
            )}
            {m.importance === "low" && (
              <ArrowDown size={14} className="mail-row__low" aria-label="Low importance" />
            )}
            {m.hasAttachments && <Paperclip size={14} aria-label="Has attachments" />}
            {m.isFlagged && <Flag size={14} className="mail-row__flag" aria-label="Flagged" />}
          </span>
          <span className="mail-row__date" title={future ? FUTURE_NOTE : undefined}>
            {date}
          </span>
        </div>
        <div className="mail-row__line">
          <span className="mail-row__subject-cell">
            {m.isDraft && <span className="mail-row__draft">Draft ·</span>}
            <span
              className={`mail-row__subject${noSubject ? " is-empty" : ""}`}
              dir={noSubject ? undefined : "auto"}
              title={subject}
            >
              {noSubject ? subject : <Highlighted text={subject} patterns={highlight?.subject} />}
            </span>
          </span>
          <span className="mail-row__actions" onClick={stop} onDoubleClick={stop}>
            <button
              type="button"
              className="icon-btn icon-btn--sm"
              tabIndex={-1}
              aria-label="Delete"
              title="Delete"
              onClick={() => handlers.onDelete(m.id)}
            >
              <Trash2 size={15} />
            </button>
            <button
              type="button"
              className="icon-btn icon-btn--sm"
              tabIndex={-1}
              aria-label={m.isRead ? "Mark as unread" : "Mark as read"}
              title={m.isRead ? "Mark as unread" : "Mark as read"}
              onClick={() => handlers.onToggleRead(m.id, !m.isRead)}
            >
              {m.isRead ? <Mail size={15} /> : <MailOpen size={15} />}
            </button>
            <button
              type="button"
              className="icon-btn icon-btn--sm mail-row__flag-btn"
              tabIndex={-1}
              aria-label="Flag"
              aria-pressed={m.isFlagged}
              title={m.isFlagged ? "Unflag" : "Flag"}
              onClick={() => handlers.onToggleFlag(m.id, !m.isFlagged)}
            >
              <Flag size={15} />
            </button>
          </span>
        </div>
        <div className="mail-row__preview">
          {context && (
            <span className="mail-row__context" title={context}>
              {contextColor && (
                <span className="mail-row__dot" style={{ background: contextColor }} />
              )}
              <span className="mail-row__context-text">{context}</span>
            </span>
          )}
          <span className="mail-row__preview-text" dir={snippet || m.preview ? "auto" : undefined}>
            {snippet ? (
              <Runs runs={leadToFirstHit(snippet, PREVIEW_LEAD)} />
            ) : m.preview ? (
              <Highlighted text={m.preview} patterns={highlight?.body} lead={PREVIEW_LEAD} />
            ) : (
              "This message has no content."
            )}
          </span>
        </div>
      </div>
    </div>
  );
});
