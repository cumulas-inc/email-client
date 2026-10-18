import {
  Archive,
  ArrowDown,
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  CircleAlert,
  Copy,
  Ellipsis,
  Flag,
  FolderInput,
  Forward,
  ImageOff,
  Layers,
  Mail,
  MailOpen,
  Pencil,
  Reply,
  ReplyAll,
  Trash2,
  X,
} from "lucide-react";
import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";

import { useBackend } from "../../api/context";
import { errorMessage } from "../../api/errors";
import { useAccounts, useSettings } from "../../api/queries";
import type { EmailAddress, Message } from "../../api/types";
import { Avatar } from "../../components/Avatar";
import { EmptyState } from "../../components/EmptyState";
import { Menu, type MenuItem } from "../../components/Menu";
import { PaneSpinner } from "../../components/Spinner";
import { toast } from "../../components/toast";
import { displayName, formatCount, fullAddress } from "../../lib/format";
import type { MailActions } from "./actions";
import { Attachments } from "./Attachments";
import { exactCount } from "./counts";
import { FUTURE_NOTE, fullDate, isFutureDated, messageDate } from "./dates";
import { FolderIcon } from "./FolderIcon";
import { folderPath, moveTargets } from "./folders";
import { useIsDark, usePersistentString } from "./hooks";
import { useMailStore } from "./mailStore";
import { bodyAppearance, classifyLink, paintsOwnBackground } from "./bodyHtml";
import { MessageBody, PlainTextBody } from "./MessageBody";
import { useFolders, useMessage } from "./queries";
import { FindBar } from "./search/FindBar";
import { Highlighted } from "./search/Highlighted";
import { useSearchHighlight } from "./search/useSearchHighlight";
import { visualLength } from "./text";

const ICON = { size: 17, strokeWidth: 1.75, "aria-hidden": true as const };

/** How a dark theme shows an HTML letter: on paper, or dark itself. */
const DARK_BODIES = ["paper", "dark"] as const;

interface ToolButtonProps {
  icon: ReactNode;
  label: string;
  title?: string;
  onClick: () => void;
  pressed?: boolean;
  /** Its label is the first to go when the pane gets narrow. */
  secondary?: boolean;
  disabled?: boolean;
  className?: string;
}

function ToolButton({
  icon,
  label,
  title,
  onClick,
  pressed,
  secondary,
  disabled,
  className,
}: ToolButtonProps) {
  return (
    <button
      type="button"
      className={`btn btn--subtle mail-tool${secondary ? " mail-tool--secondary" : ""}${
        className ? ` ${className}` : ""
      }`}
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
          <bdi>{displayName(a)}</bdi>
          {i < list.length - 1 ? ", " : ""}
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
        <div className="mail-reader__recipient-rows">
          {present.map(([label, list]) => (
            <div key={label} className="mail-reader__recipient-row">
              <span className="mail-reader__recipient-label">{label}:</span>{" "}
              {list.map((a, i) => (
                <span key={`${a.address}-${i}`} className="mail-reader__recipient">
                  <bdi>{fullAddress(a)}</bdi>
                  {i < list.length - 1 ? ", " : ""}
                </span>
              ))}
            </div>
          ))}
        </div>
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
          className="icon-btn icon-btn--sm mail-reader__recipients-toggle"
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
  const position = useMailStore((s) => s.visibleIds.indexOf(messageId));
  const listed = useMailStore((s) => s.visibleIds.length);
  const listTotal = useMailStore((s) => s.listTotal);
  const moveActive = useMailStore((s) => s.moveActive);
  const { data: settings } = useSettings();
  const { data: folders = [] } = useFolders();
  const { data: accounts = [] } = useAccounts();
  const query = useMessage(messageId, remoteAllowed);
  const message = query.data;
  const isDark = useIsDark();
  const [darkBody, setDarkBody] = usePersistentString<(typeof DARK_BODIES)[number]>(
    "mail.darkBody",
    "paper",
    DARK_BODIES,
  );
  const [hoverUrl, setHoverUrl] = useState<string | null>(null);
  // What the search box's words find in the message, and the match shown.
  const highlight = useSearchHighlight();
  // `step` counts the steps, so that stepping to the match shown (the only
  // one) shows it again, wherever the reader has scrolled since.
  const [found, setFound] = useState({ count: 0, current: null as number | null, step: 0 });
  const onHits = useCallback(
    (count: number) => setFound((f) => ({ count, current: null, step: f.step })),
    [],
  );
  const showMatch = useCallback(
    (current: number) => setFound((f) => ({ ...f, current, step: f.step + 1 })),
    [],
  );

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
  const account = accounts.find((a) => a.id === message.accountId);
  const targets = moveTargets(folders, message.accountId, message.folderId);
  const own = folders.filter((f) => f.accountId === message.accountId);
  const junk = own.find((f) => f.role === "junk");
  const inbox = own.find((f) => f.role === "inbox");
  const appearance = bodyAppearance(message.bodyHtml ?? "", isDark, darkBody);
  const noSubject = !message.subject.trim();
  const subject = message.subject.trim() || "(no subject)";
  // Long subjects step down a size, so the sender and the letter stay in view.
  const longSubject = visualLength(subject) > 90;
  const sender = message.from;
  const senderName = displayName(sender);
  // Mail the user sent: their avatar in the account's color, as in the header.
  const senderAccount = sender
    ? accounts.find((a) => a.email.toLowerCase() === sender.address.toLowerCase())
    : undefined;
  const date = fullDate(message.receivedAt);
  const sent = messageDate(message.receivedAt);
  const future = !!sent && isFutureDated(sent);

  const more: MenuItem[] = [];
  if (folder?.role === "junk" && inbox) {
    more.push({
      label: "Not junk",
      icon: <FolderIcon role="inbox" />,
      onSelect: () => actions.move(ids, inbox.id),
    });
  } else if (junk) {
    more.push({
      label: `Move to ${junk.name}`,
      icon: <FolderIcon role="junk" />,
      onSelect: () => actions.move(ids, junk.id),
    });
  }
  if (sender) {
    more.push({
      label: "Copy sender's address",
      icon: <Copy size={16} aria-hidden="true" />,
      onSelect: () => {
        navigator.clipboard
          ?.writeText(sender.address)
          .then(() => toast.success(`Copied ${sender.address}`))
          .catch((e: unknown) => toast.error(e));
      },
    });
  }
  // Only designed mail (its own backgrounds) offers a choice of sheet.
  if (isDark && message.bodyHtml && paintsOwnBackground(message.bodyHtml)) {
    more.push({
      label: "Dark message background",
      checked: darkBody === "dark",
      separator: more.length > 0,
      onSelect: () => setDarkBody(darkBody === "dark" ? "paper" : "dark"),
    });
  }

  // "Work · Inbox": where the message is. The account name gives way first.
  const placeAccount = folder && account && accounts.length > 1 ? account : null;
  const placeTitle = folder
    ? placeAccount
      ? `${placeAccount.displayName} · ${folderPath(folders, folder)}`
      : folderPath(folders, folder)
    : null;

  return (
    <article className="mail-reader" aria-label={subject}>
      <div
        className={`mail-reader__toolbar${onBack ? " mail-reader__toolbar--back" : ""}`}
        role="toolbar"
        aria-label="Message actions"
      >
        {onBack && (
          <>
            <button
              type="button"
              className="icon-btn"
              aria-label="Back to message list"
              title="Back (Esc)"
              onClick={onBack}
            >
              <ArrowLeft {...ICON} />
            </button>
            <span className="toolbar__separator" />
          </>
        )}
        <ToolButton
          icon={<Archive {...ICON} />}
          label="Archive"
          onClick={() => actions.archive(ids)}
          disabled={folder?.role === "archive"}
        />
        <ToolButton
          icon={<Trash2 {...ICON} />}
          label="Delete"
          title="Delete (Delete)"
          onClick={() => actions.remove(ids)}
        />
        <span className="mail-menu-scroll">
          <Menu
            label="Move to"
            items={targets.map((t) => ({
              label: t.label,
              icon: <FolderIcon role={t.folder.role} />,
              onSelect: () => actions.move(ids, t.folder.id),
            }))}
            trigger={(props) => (
              <button
                type="button"
                className="btn btn--subtle mail-tool mail-tool--secondary"
                aria-label="Move to"
                title="Move to"
                disabled={targets.length === 0}
                {...props}
              >
                <FolderInput {...ICON} />
                <span className="mail-tool__label">Move</span>
                <ChevronDown size={14} className="mail-tool__caret" aria-hidden="true" />
              </button>
            )}
          />
        </span>
        <span className="toolbar__separator" />
        <ToolButton
          icon={<Flag {...ICON} />}
          label="Flag"
          title={message.isFlagged ? "Unflag (Insert)" : "Flag (Insert)"}
          pressed={message.isFlagged}
          onClick={() => actions.toggleFlag(ids, !message.isFlagged)}
          className="mail-tool--flag"
          secondary
        />
        <ToolButton
          icon={message.isRead ? <Mail {...ICON} /> : <MailOpen {...ICON} />}
          label={message.isRead ? "Mark as unread" : "Mark as read"}
          title={message.isRead ? "Mark as unread (Ctrl+U)" : "Mark as read (Ctrl+Q)"}
          onClick={() => actions.markRead(ids, !message.isRead)}
          secondary
        />
        <span className="toolbar__spacer" />
        {position >= 0 && (
          <span
            className="mail-reader__position tnum"
            title={`${exactCount(position + 1)} of ${exactCount(listTotal)}`}
          >
            {formatCount(position + 1)} of {formatCount(listTotal)}
          </span>
        )}
        <button
          type="button"
          className="icon-btn mail-reader__step"
          aria-label="Previous message"
          title="Previous message (K)"
          disabled={position <= 0}
          onClick={() => moveActive(-1)}
        >
          <ChevronUp {...ICON} />
        </button>
        <button
          type="button"
          className="icon-btn mail-reader__step"
          aria-label="Next message"
          title="Next message (J)"
          disabled={position < 0 || position >= listed - 1}
          onClick={() => moveActive(1)}
        >
          <ChevronDown {...ICON} />
        </button>
        {more.length > 0 && (
          <Menu
            label="More actions"
            align="end"
            items={more}
            trigger={(props) => (
              <button
                type="button"
                className="icon-btn"
                aria-label="More actions"
                title="More actions"
                {...props}
              >
                <Ellipsis {...ICON} />
              </button>
            )}
          />
        )}
      </div>

      <div id="mail-reading-pane" className="mail-reader__scroll" tabIndex={-1}>
        <div className="mail-reader__page">
          <header className="mail-reader__head">
            <h1
              className={[
                "mail-reader__subject selectable",
                noSubject && "is-empty",
                longSubject && "mail-reader__subject--long",
              ]
                .filter(Boolean)
                .join(" ")}
              dir={noSubject ? undefined : "auto"}
            >
              {noSubject ? subject : <Highlighted text={subject} patterns={highlight?.subject} />}
            </h1>
            {(message.importance !== "normal" || message.isFlagged || placeTitle) && (
              <div className="mail-reader__tags">
                {message.importance === "high" && (
                  <span className="mail-tag mail-tag--danger">
                    <CircleAlert size={14} strokeWidth={2} aria-hidden="true" />
                    Important
                  </span>
                )}
                {message.importance === "low" && (
                  <span className="mail-tag">
                    <ArrowDown size={14} strokeWidth={2} aria-hidden="true" />
                    Low importance
                  </span>
                )}
                {message.isFlagged && (
                  <span className="mail-tag mail-tag--flag">
                    <Flag size={14} strokeWidth={2} aria-hidden="true" />
                    Flagged
                  </span>
                )}
                {folder && placeTitle && (
                  <span className="mail-tag mail-tag--place" title={placeTitle}>
                    {placeAccount && (
                      <>
                        <span
                          className="mail-tag__dot"
                          style={{ background: placeAccount.color }}
                          aria-hidden="true"
                        />
                        <bdi className="mail-tag__account">{placeAccount.displayName}</bdi>
                        <span className="mail-tag__sep">·</span>
                      </>
                    )}
                    <bdi className="mail-tag__folder">{folder.name}</bdi>
                  </span>
                )}
              </div>
            )}
            <div className="mail-reader__sender">
              <Avatar
                name={senderName}
                colorKey={sender?.address}
                color={senderAccount?.color}
                size={42}
              />
              <div className="mail-reader__people selectable">
                <div className="mail-reader__from">
                  <span
                    className={`mail-reader__from-name${sender ? "" : " is-empty"}`}
                    dir={sender ? "auto" : undefined}
                  >
                    {senderName}
                  </span>
                  {sender?.name && (
                    <span className="mail-reader__from-address">&lt;{sender.address}&gt;</span>
                  )}
                </div>
                <Recipients message={message} />
              </div>
              <div className="mail-reader__side">
                {date && (
                  <time
                    className="mail-reader__date tnum selectable"
                    dateTime={message.receivedAt}
                    title={future ? FUTURE_NOTE : undefined}
                  >
                    {date}
                  </time>
                )}
                <div className="mail-respond" role="group" aria-label="Respond">
                  <button
                    type="button"
                    className="mail-respond__btn mail-respond__btn--main"
                    title="Reply (Ctrl+R)"
                    onClick={() => void actions.reply(message.id, "reply")}
                  >
                    <Reply size={15} strokeWidth={1.75} aria-hidden="true" />
                    Reply
                  </button>
                  <button
                    type="button"
                    className="mail-respond__btn"
                    title="Reply all (Ctrl+Shift+R)"
                    onClick={() => void actions.reply(message.id, "replyAll")}
                  >
                    <ReplyAll size={15} strokeWidth={1.75} aria-hidden="true" />
                    Reply all
                  </button>
                  <button
                    type="button"
                    className="mail-respond__btn"
                    title="Forward (Ctrl+F)"
                    onClick={() => void actions.reply(message.id, "forward")}
                  >
                    <Forward size={15} strokeWidth={1.75} aria-hidden="true" />
                    Forward
                  </button>
                </div>
              </div>
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

          {highlight && found.count > 0 && (
            <FindBar count={found.count} current={found.current} onShow={showMatch} />
          )}

          <div className={`mail-reader__body mail-reader__body--${appearance}`}>
            {message.bodyHtml ? (
              <MessageBody
                html={message.bodyHtml}
                allowRemote={remoteAllowed}
                appearance={appearance}
                title={`Message: ${subject}`}
                onLink={onLink}
                onHoverLink={setHoverUrl}
                highlight={highlight?.body}
                current={found.current}
                revealStep={found.step}
                onHits={onHits}
              />
            ) : (
              <PlainTextBody
                text={message.bodyText ?? ""}
                onLink={onLink}
                highlight={highlight?.body}
                current={found.current}
                revealStep={found.step}
                onHits={onHits}
              />
            )}
          </div>
        </div>
      </div>
      {!message.isDraft && (
        <div className="mail-quickreply">
          <button
            type="button"
            className="mail-quickreply__btn"
            title="Reply (Ctrl+R)"
            onClick={() => void actions.reply(message.id, "reply")}
          >
            {account && (
              <Avatar name={account.displayName || account.email} color={account.color} size={26} />
            )}
            <span className="mail-quickreply__text">
              {sender ? (
                <>
                  Reply to <bdi>{senderName}</bdi>…
                </>
              ) : (
                "Reply…"
              )}
            </span>
            <span className="kbd" aria-hidden="true">
              Ctrl R
            </span>
          </button>
        </div>
      )}
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
      title={`${formatCount(ids.length)} items selected`}
      description="Choose what to do with them."
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
            <X size={14} aria-hidden="true" /> Clear selection
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
        title="No message open"
        description={
          <>
            Choose one from the list to read it here, or press <span className="kbd">Ctrl N</span>{" "}
            to write a new one.
          </>
        }
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
