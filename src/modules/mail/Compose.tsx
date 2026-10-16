import {
  ChevronDown,
  CircleAlert,
  Ellipsis,
  Minimize2,
  Paperclip,
  Send,
  Trash2,
  X,
} from "lucide-react";
import clsx from "clsx";
import { type ChangeEvent, type KeyboardEvent, lazy, Suspense, useRef, useState } from "react";

import { useAccounts } from "../../api/queries";
import type { EmailAddress, Importance } from "../../api/types";
import { Dialog } from "../../components/Dialog";
import { Menu } from "../../components/Menu";
import { toast } from "../../components/toast";
import { formatBytes } from "../../lib/format";
import { FileBadge, FileName } from "./Attachments";
import { bodyAppearance } from "./bodyHtml";
import { ComposeSignatureBlock } from "./ComposeSignature";
import {
  ATTACHMENT_WARN_BYTES,
  composeBodyHtml,
  readFileBase64,
  type SentSignature,
} from "./draft";
import type { EditorHandle } from "./Editor";
import { useIsDark } from "./hooks";
import { type ComposeDraft, type DraftAttachment, useMailStore } from "./mailStore";
import { MessageBody } from "./MessageBody";
import { RecipientField, type RecipientFieldHandle } from "./RecipientField";
import { useSendMessage } from "./queries";
import { useSentSignature, useSignatureView } from "./signature";

// The editor (TipTap/ProseMirror) is the largest dependency; load it on first use.
const RichTextEditor = lazy(() => import("./Editor").then((m) => ({ default: m.RichTextEditor })));

let attachmentSeq = 0;

/** Confirmation before throwing away a draft with changes. */
export function DiscardDialog({
  open,
  onCancel,
  onDiscard,
}: {
  open: boolean;
  onCancel: () => void;
  onDiscard: () => void;
}) {
  return (
    <Dialog
      open={open}
      onClose={onCancel}
      title="Discard this draft?"
      width={400}
      footer={
        <>
          <button type="button" className="btn btn--primary" onClick={onDiscard}>
            Discard
          </button>
          <button type="button" className="btn" onClick={onCancel}>
            Cancel
          </button>
        </>
      }
    >
      <p>The message hasn't been sent. Your changes will be lost.</p>
    </Dialog>
  );
}

export function Compose({ onMinimize }: { onMinimize: () => void }) {
  const draft = useMailStore((s) => s.compose);
  const update = useMailStore((s) => s.updateCompose);
  const closeCompose = useMailStore((s) => s.closeCompose);
  const { data: accounts = [] } = useAccounts();
  const send = useSendMessage();
  const isDark = useIsDark();
  const signature = useSignatureView(draft);
  const signatureFor = useSentSignature();

  const toRef = useRef<RecipientFieldHandle>(null);
  const ccRef = useRef<RecipientFieldHandle>(null);
  const bccRef = useRef<RecipientFieldHandle>(null);
  const editorRef = useRef<EditorHandle>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  const [recipientError, setRecipientError] = useState<string | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [confirmNoSubject, setConfirmNoSubject] = useState(false);
  const [showQuote, setShowQuote] = useState(true);
  // Waiting for the account's signature before sending.
  const [preparing, setPreparing] = useState(false);
  const sending = send.isPending || preparing;

  if (!draft) return null;

  const title =
    draft.replyKind === "forward"
      ? "Forward"
      : draft.replyKind === "reply" || draft.replyKind === "replyAll"
        ? "Reply"
        : "New message";

  const totalAttachmentBytes = draft.attachments.reduce((sum, a) => sum + a.size, 0);
  const quoteAppearance = bodyAppearance(draft.quoteHtml ?? "", isDark);

  const setRecipients = (field: "to" | "cc" | "bcc", value: EmailAddress[]) => {
    update({ [field]: value });
    if (value.length > 0) setRecipientError(null);
  };

  const doSend = async (allowEmptySubject = false) => {
    if (sending) return;
    const to = toRef.current ? toRef.current.commit() : draft.to;
    const cc = draft.showCc && ccRef.current ? ccRef.current.commit() : draft.cc;
    const bcc = draft.showBcc && bccRef.current ? bccRef.current.commit() : draft.bcc;
    if (to === null || cc === null || bcc === null) return;
    if (to.length + cc.length + bcc.length === 0) {
      setRecipientError("Add at least one recipient.");
      toRef.current?.focus();
      return;
    }
    if (!draft.subject.trim() && !allowEmptySubject) {
      setConfirmNoSubject(true);
      return;
    }
    // What was typed while waiting goes too; nothing goes if the draft was
    // closed meanwhile. The signature is always the one for the account
    // and signature choice it's sent with: From and the signature's
    // buttons are off while Send waits, and should they change all the
    // same, it's worked out again.
    let current: ComposeDraft = draft;
    let sentSignature: SentSignature | null = null;
    let settled = false;
    setPreparing(true);
    try {
      while (!settled) {
        const asked = current;
        sentSignature = await signatureFor(asked);
        const now = useMailStore.getState().compose;
        if (now?.id !== draft.id) return;
        current = now;
        settled = now.accountId === asked.accountId && now.signature === asked.signature;
      }
    } finally {
      setPreparing(false);
    }
    try {
      await send.mutateAsync({
        accountId: current.accountId,
        to,
        cc,
        bcc,
        subject: current.subject.trim(),
        bodyHtml: composeBodyHtml(current.bodyHtml, sentSignature, current.quoteHtml),
        attachments: current.attachments.map((a) => ({
          name: a.name,
          contentType: a.contentType,
          dataBase64: a.dataBase64,
        })),
        importance: current.importance,
        replyToMessageId: current.replyToMessageId,
        replyKind: current.replyKind,
      });
      toast.success("Message sent");
      closeCompose();
    } catch (e) {
      toast.error(e);
    }
  };

  const discard = () => {
    if (draft.dirty) setConfirmDiscard(true);
    else closeCompose();
  };

  const onFiles = async (e: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? []);
    e.target.value = "";
    const added: DraftAttachment[] = [];
    for (const file of files) {
      try {
        attachmentSeq += 1;
        added.push({
          id: `att-${attachmentSeq}`,
          name: file.name,
          contentType: file.type || "application/octet-stream",
          size: file.size,
          dataBase64: await readFileBase64(file),
        });
      } catch (err) {
        toast.error(err);
      }
    }
    if (added.length > 0) {
      const current = useMailStore.getState().compose?.attachments ?? [];
      update({ attachments: [...current, ...added] });
    }
  };

  const setImportance = (value: Importance) =>
    update({ importance: draft.importance === value ? "normal" : value });

  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      void doSend();
    }
  };

  const ccToggles = (
    <span className="mail-compose__toggles">
      {!draft.showCc && (
        <button
          type="button"
          className="btn btn--subtle btn--sm"
          onClick={(e) => {
            e.stopPropagation();
            update({ showCc: true });
          }}
        >
          Cc
        </button>
      )}
      {!draft.showBcc && (
        <button
          type="button"
          className="btn btn--subtle btn--sm"
          onClick={(e) => {
            e.stopPropagation();
            update({ showBcc: true });
          }}
        >
          Bcc
        </button>
      )}
    </span>
  );

  return (
    <section className="mail-compose" aria-label={title} onKeyDown={onKeyDown}>
      <div className="mail-compose__toolbar" role="toolbar" aria-label="Message">
        <h2 className="mail-compose__title">{title}</h2>
        <span className="toolbar__spacer" />
        <button
          type="button"
          className="btn btn--subtle mail-tool"
          onClick={() => fileRef.current?.click()}
          title="Attach files"
        >
          <Paperclip size={17} strokeWidth={1.75} aria-hidden="true" />
          <span className="mail-tool__label">Attach</span>
        </button>
        <input
          ref={fileRef}
          type="file"
          multiple
          hidden
          aria-label="Attach files"
          data-testid="compose-file-input"
          onChange={(e) => void onFiles(e)}
        />
        {/* Sent as importance "high". The name stays when the label is
            hidden in a narrow composer. */}
        <button
          type="button"
          className="btn btn--subtle mail-tool mail-compose__important"
          aria-pressed={draft.importance === "high"}
          aria-label="Important"
          title="Mark as important"
          onClick={() => setImportance("high")}
        >
          <CircleAlert size={17} strokeWidth={1.75} aria-hidden="true" />
          <span className="mail-tool__label">Important</span>
        </button>
        <Menu
          label="More options"
          align="end"
          items={[
            {
              label: "Low importance",
              checked: draft.importance === "low",
              onSelect: () => setImportance("low"),
            },
          ]}
          trigger={(props) => (
            <button
              type="button"
              className="icon-btn"
              aria-label="More options"
              title="More options"
              {...props}
            >
              <Ellipsis size={17} strokeWidth={1.75} />
            </button>
          )}
        />
        <span className="toolbar__separator" />
        <button
          type="button"
          className="btn btn--subtle mail-tool"
          onClick={discard}
          title="Discard"
        >
          <Trash2 size={17} strokeWidth={1.75} aria-hidden="true" />
          <span className="mail-tool__label">Discard</span>
        </button>
        <button
          type="button"
          className="btn btn--primary mail-compose__send"
          onClick={() => void doSend()}
          disabled={sending}
          title="Send (Ctrl+Enter)"
        >
          <Send size={16} strokeWidth={1.9} aria-hidden="true" />
          {sending ? "Sending…" : "Send"}
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label="Minimize draft"
          title="Minimize (keep draft)"
          onClick={onMinimize}
        >
          <Minimize2 size={17} strokeWidth={1.75} />
        </button>
      </div>

      <div className="mail-compose__scroll">
        <div
          className={clsx(
            "mail-compose__page",
            draft.quoteHtml && "mail-compose__page--quoted",
            // "Add signature" and the added-on-send note sit right under
            // the text too, where the signature would be.
            signature.kind !== "none" && "mail-compose__page--signed",
          )}
        >
          <div className="mail-compose__fields">
            {accounts.length > 1 && (
              <div className="mail-compose__field">
                <label className="mail-compose__label" htmlFor="mail-compose-from">
                  From
                </label>
                <select
                  id="mail-compose-from"
                  className="mail-compose__from"
                  value={draft.accountId}
                  // The message goes from the account its signature is for.
                  disabled={sending}
                  onChange={(e) =>
                    // The new account's signature replaces the old one, edited
                    // or not, as in Outlook.
                    update({
                      accountId: e.target.value,
                      signature: draft.signature === "off" ? "off" : "auto",
                    })
                  }
                >
                  {accounts.map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.displayName} &lt;{a.email}&gt;
                    </option>
                  ))}
                </select>
                <ChevronDown size={14} className="mail-compose__from-caret" aria-hidden="true" />
              </div>
            )}
            <RecipientField
              ref={toRef}
              label="To"
              value={draft.to}
              onChange={(v) => setRecipients("to", v)}
              error={recipientError}
              autoFocus={draft.focus === "to"}
              trailing={ccToggles}
            />
            {draft.showCc && (
              <RecipientField
                ref={ccRef}
                label="Cc"
                value={draft.cc}
                onChange={(v) => setRecipients("cc", v)}
              />
            )}
            {draft.showBcc && (
              <RecipientField
                ref={bccRef}
                label="Bcc"
                value={draft.bcc}
                onChange={(v) => setRecipients("bcc", v)}
              />
            )}
            <div className="mail-compose__field">
              <label className="mail-compose__label" htmlFor="mail-compose-subject">
                Subject
              </label>
              <input
                id="mail-compose-subject"
                className="mail-compose__subject"
                dir="auto"
                aria-label="Subject"
                placeholder="Add a subject"
                value={draft.subject}
                onChange={(e) => update({ subject: e.target.value })}
              />
              {/* Says what the toolbar can't when its labels are hidden. */}
              {draft.importance === "high" && (
                <span className="mail-tag mail-tag--danger mail-compose__important-tag">
                  Important
                </span>
              )}
              {draft.importance === "low" && <span className="mail-tag">Low importance</span>}
            </div>
          </div>

          {draft.attachments.length > 0 && (
            <div className="mail-compose__attachments">
              <ul className="mail-compose__files" aria-label="Attached files">
                {draft.attachments.map((a) => (
                  <li key={a.id} className="mail-file-chip">
                    <FileBadge name={a.name} contentType={a.contentType} />
                    <FileName name={a.name} className="mail-file-chip__name" />
                    <span className="mail-file-chip__size">{formatBytes(a.size)}</span>
                    <button
                      type="button"
                      className="icon-btn icon-btn--sm"
                      aria-label={`Remove ${a.name}`}
                      onClick={() =>
                        update({ attachments: draft.attachments.filter((x) => x.id !== a.id) })
                      }
                    >
                      <X size={14} />
                    </button>
                  </li>
                ))}
              </ul>
              {totalAttachmentBytes > ATTACHMENT_WARN_BYTES && (
                <p className="mail-compose__warning" role="status">
                  <CircleAlert size={14} aria-hidden="true" />
                  Attachments total {formatBytes(totalAttachmentBytes)}. Many mail servers reject
                  messages larger than 20 MB.
                </p>
              )}
            </div>
          )}

          <Suspense fallback={<div className="mail-editor" aria-busy="true" />}>
            <RichTextEditor
              ref={editorRef}
              initialHtml={draft.bodyHtml}
              autoFocus={draft.focus === "body"}
              onChange={(html) => update({ bodyHtml: html })}
            />
          </Suspense>

          <ComposeSignatureBlock view={signature} disabled={sending} />

          {draft.quoteHtml && (
            <div className="mail-compose__quote">
              <button
                type="button"
                className="btn btn--subtle btn--sm mail-compose__quote-toggle"
                aria-expanded={showQuote}
                onClick={() => setShowQuote((v) => !v)}
              >
                {showQuote ? "Hide original message" : "Show original message"}
              </button>
              {showQuote && (
                <div className={`mail-reader__body mail-reader__body--${quoteAppearance}`}>
                  <MessageBody
                    html={draft.quoteHtml}
                    allowRemote={false}
                    appearance={quoteAppearance}
                    tone="quiet"
                    title="Original message"
                    onLink={() => undefined}
                  />
                </div>
              )}
            </div>
          )}
        </div>
      </div>

      <DiscardDialog
        open={confirmDiscard}
        onCancel={() => setConfirmDiscard(false)}
        onDiscard={() => {
          setConfirmDiscard(false);
          closeCompose();
        }}
      />
      <Dialog
        open={confirmNoSubject}
        onClose={() => setConfirmNoSubject(false)}
        title="Send without a subject?"
        width={400}
        footer={
          <>
            <button
              type="button"
              className="btn btn--primary"
              onClick={() => {
                setConfirmNoSubject(false);
                void doSend(true);
              }}
            >
              Send anyway
            </button>
            <button type="button" className="btn" onClick={() => setConfirmNoSubject(false)}>
              Cancel
            </button>
          </>
        }
      >
        <p>This message has no subject. Do you want to send it anyway?</p>
      </Dialog>
    </section>
  );
}
