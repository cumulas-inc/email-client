import {
  Archive,
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  Flag,
  FolderInput,
  Forward,
  ImageOff,
  Layers,
  Mail,
  MailOpen,
  Moon,
  Pencil,
  Reply,
  ReplyAll,
  Sun,
  Trash2,
  X,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";

import { useBackend } from "../../api/context";
import { errorMessage } from "../../api/errors";
import { useSettings } from "../../api/queries";
import type { EmailAddress, Message } from "../../api/types";
import { Avatar } from "../../components/Avatar";
import { EmptyState } from "../../components/EmptyState";
import { Menu } from "../../components/Menu";
import { PaneSpinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { displayName, formatFullDate, fullAddress } from "../../lib/format";
import type { MailActions } from "./actions";
import { Attachments } from "./Attachments";
import { FolderIcon } from "./FolderIcon";
import { moveTargets } from "./folders";
import { useIsDark, usePersistentString } from "./hooks";
import { useMailStore } from "./mailStore";
import { type BodyAppearance, classifyLink } from "./bodyHtml";
import { MessageBody, PlainTextBody } from "./MessageBody";
import { useFolders, useMessage } from "./queries";

interface ToolButtonProps {
  icon: ReactNode;
  label: string;
  title?: string;
  onClick: () => void;
  pressed?: boolean;
  secondary?: boolean;
  disabled?: boolean;
}

function ToolButton({
  icon,
  label,
  title,
  onClick,
  pressed,
  secondary,
  disabled,
}: ToolButtonProps) {
  return (
    <button
      type="button"
      className={`btn btn--subtle btn--sm mail-tool${secondary ? " mail-tool--secondary" : ""}`}
      aria-label={label}
      aria-pressed={pressed}
      title={title ?? label}
      onClick={onClick}
      disabled={disabled}
    >
      {icon}
      <span className="mail-tool__label">{label}</span>
    </button>
  );
}

function AddressList({ list }: { list: EmailAddress[] }) {
  return (
    <>
      {list.map((a, i) => (
        <span key={`${a.address}-${i}`} className="mail-reader__recipient" title={a.address}>
          {displayName(a)}
          {i < list.length - 1 ? "; " : ""}
        </span>
      ))}
    </>
  );
}

function Recipients({ message }: { message: Message }) {
  const [expanded, setExpanded] = useState(false);
  const rows: [string, EmailAddress[]][] = [
    ["To", message.to],
    ["Cc", message.cc],
    ["Bcc", message.bcc],
  ];
  const present = rows.filter(([, list]) => list.length > 0);
  if (present.length === 0) return null;
  const count = present.reduce((n, [, list]) => n + list.length, 0);
  const collapsible = count > 3 || present.length > 1;
  return (
    <div className={`mail-reader__recipients${expanded ? " is-expanded" : ""}`}>
      {expanded ? (
        present.map(([label, list]) => (
          <div key={label} className="mail-reader__recipient-row">
            <span className="mail-reader__recipient-label">{label}:</span>{" "}
            {list.map((a, i) => (
              <span key={`${a.address}-${i}`}>
                {fullAddress(a)}
                {i < list.length - 1 ? "; " : ""}
              </span>
            ))}
          </div>
        ))
      ) : (
        <div className="mail-reader__recipient-row mail-reader__recipient-row--collapsed">
          {present.map(([label, list]) => (
            <span key={label} className="mail-reader__recipient-group">
              <span className="mail-reader__recipient-label">{label}:</span>{" "}
              <AddressList list={list} />
            </span>
          ))}
        </div>
      )}
      {collapsible && (
        <button
          type="button"
          className="icon-btn icon-btn--sm"
          aria-expanded={expanded}
          aria-label={expanded ? "Hide recipient details" : "Show all recipients"}
          title={expanded ? "Hide details" : "Show all recipients"}
          onClick={() => setExpanded((e) => !e)}
        >
          {expanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
        </button>
      )}
    </div>
  );
}

function useAutoMarkRead(
  message: Message | undefined,
  delaySeconds: number | null | undefined,
  markRead: MailActions["markRead"],
) {
  const handled = useRef<string | null>(null);
  const id = message?.id ?? null;
  const unread = message ? !message.isRead : false;
  useEffect(() => {
    if (!id || delaySeconds === undefined || delaySeconds === null) return;
    if (handled.current === id) return;
    if (!unread) {
      handled.current = id;
      return;
    }
    if (delaySeconds <= 0) {
      handled.current = id;
      markRead([id], true);
      return;
    }
    const timer = setTimeout(() => {
      handled.current = id;
      markRead([id], true);
    }, delaySeconds * 1000);
    return () => clearTimeout(timer);
  }, [id, unread, delaySeconds, markRead]);
}

interface MessageViewProps {
  messageId: string;
  actions: MailActions;
  onBack: (() => void) | null;
  onEditDraft: (id: string) => void;
}

function MessageView({ messageId, actions, onBack, onEditDraft }: MessageViewProps) {
  const backend = useBackend();
  const remoteAllowed = useMailStore((s) => !!s.remoteAllowed[messageId]);
  const allowRemote = useMailStore((s) => s.allowRemote);
  const { data: settings } = useSettings();
  const { data: folders = [] } = useFolders();
  const query = useMessage(messageId, remoteAllowed);
  const message = query.data;
  const isDark = useIsDark();
  const [darkBody, setDarkBody] = usePersistentString<"paper" | "dark">("mail.darkBody", "paper");
  const [hoverUrl, setHoverUrl] = useState<string | null>(null);

  useAutoMarkRead(message, settings?.markReadAfterSeconds, actions.markRead);

  if (query.isPending) return <PaneSpinner />;
  if (!message) {
    return (
      <EmptyState
        title="Couldn't open this message"
        description={query.error ? errorMessage(query.error) : "It may have been moved or deleted."}
      />
    );
  }

  const onLink = (href: string) => {
    const action = classifyLink(href);
    if (action.kind === "open") backend.openLink(action.url).catch((e) => toast.error(e));
    else if (action.kind === "mailto") actions.openMailto(action.url);
  };

  const ids = [message.id];
  const folder = folders.find((f) => f.id === message.folderId);
  const targets = moveTargets(folders, message.accountId, message.folderId);
  const appearance: BodyAppearance = isDark ? darkBody : "plain";
  const subject = message.subject.trim() || "(no subject)";
  const sender = message.from;

  return (
    <article className="mail-reader" aria-label={subject}>
      <div className="toolbar mail-reader__toolbar" role="toolbar" aria-label="Message actions">
        {onBack && (
          <>
            <button
              type="button"
              className="icon-btn"
              aria-label="Back to message list"
              title="Back (Esc)"
              onClick={onBack}
            >
              <ArrowLeft size={18} />
            </button>
            <span className="toolbar__separator" />
          </>
        )}
        <ToolButton
          icon={<Reply size={16} />}
          label="Reply"
          title="Reply (Ctrl+R)"
          onClick={() => void actions.reply(message.id, "reply")}
        />
        <ToolButton
          icon={<ReplyAll size={16} />}
          label="Reply all"
          title="Reply all (Ctrl+Shift+R)"
          onClick={() => void actions.reply(message.id, "replyAll")}
        />
        <ToolButton
          icon={<Forward size={16} />}
          label="Forward"
          title="Forward (Ctrl+F)"
          onClick={() => void actions.reply(message.id, "forward")}
        />
        <span className="toolbar__separator" />
        <ToolButton
          icon={<Trash2 size={16} />}
          label="Delete"
          title="Delete (Delete)"
          onClick={() => actions.remove(ids)}
          secondary
        />
        <ToolButton
          icon={<Archive size={16} />}
          label="Archive"
          onClick={() => actions.archive(ids)}
          disabled={folder?.role === "archive"}
          secondary
        />
        <span className="mail-menu-scroll">
          <Menu
            label="Move to"
            items={targets.map((t) => ({
              label: t.path,
              icon: <FolderIcon role={t.folder.role} />,
              onSelect: () => actions.move(ids, t.folder.id),
            }))}
            trigger={(props) => (
              <button
                type="button"
                className="btn btn--subtle btn--sm mail-tool mail-tool--secondary"
                aria-label="Move to"
                title="Move to"
                disabled={targets.length === 0}
                {...props}
              >
                <FolderInput size={16} />
                <span className="mail-tool__label">Move to</span>
              </button>
            )}
          />
        </span>
        <span className="toolbar__separator" />
        <ToolButton
          icon={<Flag size={16} />}
          label={message.isFlagged ? "Unflag" : "Flag"}
          title={message.isFlagged ? "Unflag (Insert)" : "Flag (Insert)"}
          pressed={message.isFlagged}
          onClick={() => actions.toggleFlag(ids, !message.isFlagged)}
          secondary
        />
        <ToolButton
          icon={message.isRead ? <Mail size={16} /> : <MailOpen size={16} />}
          label={message.isRead ? "Mark as unread" : "Mark as read"}
          title={message.isRead ? "Mark as unread (Ctrl+U)" : "Mark as read (Ctrl+Q)"}
          onClick={() => actions.markRead(ids, !message.isRead)}
          secondary
        />
        <span className="toolbar__spacer" />
        {isDark && message.bodyHtml && (
          <button
            type="button"
            className="icon-btn"
            aria-pressed={darkBody === "dark"}
            aria-label={
              darkBody === "dark"
                ? "Show message on a light background"
                : "Show message on a dark background"
            }
            title={darkBody === "dark" ? "Light background" : "Dark background"}
            onClick={() => setDarkBody(darkBody === "dark" ? "paper" : "dark")}
          >
            {darkBody === "dark" ? <Sun size={16} /> : <Moon size={16} />}
          </button>
        )}
      </div>

      <div id="mail-reading-pane" className="mail-reader__scroll" tabIndex={-1}>
        <header className="mail-reader__head selectable">
          <h1 className="mail-reader__subject">
            {subject}
            {message.importance === "high" && (
              <span className="mail-tag mail-tag--danger">High importance</span>
            )}
            {message.isFlagged && (
              <span className="mail-tag mail-tag--flag">
                <Flag size={12} aria-hidden="true" /> Flagged
              </span>
            )}
          </h1>
          <div className="mail-reader__meta">
            <Avatar name={displayName(sender)} colorKey={sender?.address} size={40} />
            <div className="mail-reader__people">
              <div className="mail-reader__from">
                <span className="mail-reader__from-name">{displayName(sender)}</span>
                {sender?.name && (
                  <span className="mail-reader__from-address">&lt;{sender.address}&gt;</span>
                )}
              </div>
              <Recipients message={message} />
            </div>
            <time className="mail-reader__date" dateTime={message.receivedAt}>
              {formatFullDate(message.receivedAt)}
            </time>
          </div>
        </header>

        {message.isDraft && (
          <div className="mail-banner" role="note">
            <Pencil size={16} aria-hidden="true" />
            <span className="mail-banner__text">This message hasn't been sent.</span>
            <button type="button" className="btn btn--sm" onClick={() => onEditDraft(message.id)}>
              Continue editing
            </button>
          </div>
        )}

        {message.hasBlockedContent && !remoteAllowed && (
          <div className="mail-banner" role="note">
            <ImageOff size={16} aria-hidden="true" />
            <span className="mail-banner__text">
              To protect your privacy, images and content from the internet were blocked in this
              message.
            </span>
            <button type="button" className="btn btn--sm" onClick={() => allowRemote(message.id)}>
              Show blocked content
            </button>
          </div>
        )}

        <Attachments messageId={message.id} attachments={message.attachments} />

        <div className={`mail-reader__body mail-reader__body--${appearance}`}>
          {message.bodyHtml ? (
            <MessageBody
              html={message.bodyHtml}
              allowRemote={remoteAllowed}
              appearance={appearance}
              title={`Message: ${subject}`}
              onLink={onLink}
              onHoverLink={setHoverUrl}
            />
          ) : (
            <PlainTextBody text={message.bodyText ?? ""} onLink={onLink} />
          )}
        </div>
      </div>
      {hoverUrl && (
        <div className="mail-reader__status" aria-hidden="true">
          {hoverUrl}
        </div>
      )}
    </article>
  );
}

function MultiSelection({ ids, actions }: { ids: string[]; actions: MailActions }) {
  const clearSelection = useMailStore((s) => s.clearSelection);
  return (
    <EmptyState
      icon={<Layers size={40} strokeWidth={1.5} />}
      title={`${ids.length} items selected`}
      description="Choose an action for the selected messages."
      action={
        <div className="mail-bulk">
          <button type="button" className="btn btn--sm" onClick={() => actions.markRead(ids, true)}>
            <MailOpen size={14} aria-hidden="true" /> Mark as read
          </button>
          <button type="button" className="btn btn--sm" onClick={() => actions.toggleFlag(ids)}>
            <Flag size={14} aria-hidden="true" /> Flag
          </button>
          <button type="button" className="btn btn--sm" onClick={() => actions.archive(ids)}>
            <Archive size={14} aria-hidden="true" /> Archive
          </button>
          <button type="button" className="btn btn--sm" onClick={() => actions.remove(ids)}>
            <Trash2 size={14} aria-hidden="true" /> Delete
          </button>
          <button type="button" className="btn btn--subtle btn--sm" onClick={clearSelection}>
            <X size={14} aria-hidden="true" /> Cancel
          </button>
        </div>
      }
    />
  );
}

interface ReadingPaneProps {
  actions: MailActions;
  /** Shown with the reading pane off: returns to the list. */
  onBack: (() => void) | null;
  onEditDraft: (id: string) => void;
}

export function ReadingPane({ actions, onBack, onEditDraft }: ReadingPaneProps) {
  const selectedIds = useMailStore((s) => s.selectedIds);
  const activeId = useMailStore((s) => s.activeId);

  if (selectedIds.length > 1) return <MultiSelection ids={selectedIds} actions={actions} />;
  if (!activeId) {
    return (
      <EmptyState
        icon={<Mail size={40} strokeWidth={1.5} />}
        title="Select an item to read"
        description="Nothing is selected."
      />
    );
  }
  return (
    <MessageView
      key={activeId}
      messageId={activeId}
      actions={actions}
      onBack={onBack}
      onEditDraft={onEditDraft}
    />
  );
}
