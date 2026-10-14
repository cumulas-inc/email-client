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

import type { MessageSummary } from "../../api/types";
import { Avatar } from "../../components/Avatar";
import { displayName, formatListDate } from "../../lib/format";
import { optionId } from "./dom";

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
}

function who(message: MessageSummary, showRecipients: boolean): string {
  if (showRecipients) {
    const names = message.to.map((a) => displayName(a));
    return names.length > 0 ? names.join("; ") : "(no recipients)";
  }
  return displayName(message.from);
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
}: RowProps) {
  const sender = who(m, showRecipients);
  const date = formatListDate(m.receivedAt);
  const subject = m.subject.trim() || "(no subject)";
  const label = [
    m.isRead ? null : "Unread",
    m.isDraft ? "Draft" : null,
    sender,
    subject,
    date,
    m.isFlagged ? "Flagged" : null,
    m.hasAttachments ? "Has attachments" : null,
    m.importance === "high" ? "High importance" : null,
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
          <span className="mail-row__sender" title={sender}>
            {sender}
          </span>
          <span className="mail-row__status">
            {m.importance === "high" && (
              <CircleAlert
                size={14}
                className="mail-row__importance"
                aria-label="High importance"
              />
            )}
            {m.importance === "low" && (
              <ArrowDown size={14} className="mail-row__low" aria-label="Low importance" />
            )}
            {m.hasAttachments && <Paperclip size={14} aria-label="Has attachments" />}
            {m.isFlagged && <Flag size={14} className="mail-row__flag" aria-label="Flagged" />}
          </span>
          <span className="mail-row__date">{date}</span>
        </div>
        <div className="mail-row__line">
          <span className="mail-row__subject" title={subject}>
            {m.isDraft && <span className="mail-row__draft">Draft · </span>}
            {subject}
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
            <span className="mail-row__context">
              {contextColor && (
                <span className="mail-row__dot" style={{ background: contextColor }} />
              )}
              {context}
            </span>
          )}
          {m.preview || "This message has no content."}
        </div>
      </div>
    </div>
  );
});
