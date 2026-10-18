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
import {
  type ChangeEvent,
  type KeyboardEvent,
  lazy,
  Suspense,
  useEffect,
  useRef,
  useState,
} from "react";

import { useBackend } from "../../api/context";
import { errorMessage, isBackendError } from "../../api/errors";
import { useAccounts, useProtectionSetup } from "../../api/queries";
import type { EmailAddress, Importance, ProtectionSetup, ReplyKind } from "../../api/types";
import { Dialog } from "../../components/Dialog";
import { Menu } from "../../components/Menu";
import { toast } from "../../components/toast";
import { formatBytes } from "../../lib/format";
import { FileBadge, FileName } from "./Attachments";
import { bodyAppearance } from "./bodyHtml";
import { ComposeProtection, type ComposeProtectionHandle } from "./ComposeProtection";
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
import {
  ACTION_NAMES,
  canChooseLabel,
  checkDate,
  droppedLabel,
  initialLabelId,
  labelTitle,
  offeredActions,
  offeredLabels,
  sendProblem,
} from "./protection";
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

const LABEL_REQUIRED = "Choose a sensitivity label. Your organization requires one.";

/** "reply" or "forward", as the label copy names a draft. */
function draftNoun(kind: ReplyKind | null): string {
  return kind === "forward" ? "forward" : "reply";
}

/** The original's label, by name when it has one. */
function originalLabeled(name: string | null): string {
  return name ? `The original is labeled ${name}.` : "The original has a sensitivity label.";
}

/** What the composer says under the fields about encryption and labels. */
function protectionHints(
  draft: ComposeDraft,
  setup: ProtectionSetup | undefined,
  email: string | undefined,
): string[] {
  if (!setup?.supported) return [];
  const hints: string[] = [];
  const labels = offeredLabels(setup);
  const label = labels.find((l) => l.id === draft.labelId) ?? null;
  const protect = draft.protect && !label?.hasProtection ? draft.protect : null;
  if (protect && offeredActions(setup).includes(protect)) {
    const at = protect === "encrypt" ? setup.encrypt.at : setup.doNotForward.at;
    const asks =
      protect === "encrypt"
        ? "Cumulas Mail asks your organization's mail server to encrypt this message."
        : "Cumulas Mail asks your organization's mail server to apply Do Not Forward to this message.";
    const date = at ? ` A test message on ${checkDate(at)} arrived encrypted.` : "";
    hints.push(`${asks}${date} Your copy in Sent Items won't show it.`);
  }
  if (label) {
    const date = label.check.at
      ? ` A test message on ${checkDate(label.check.at)} arrived with it.`
      : "";
    hints.push(
      `Cumulas Mail asks your organization's mail server to apply the label ${labelTitle(label)}.${date}` +
        (label.hasProtection ? " The label encrypts the message." : ""),
    );
  }
  if (setup.policy.required && labels.length === 0) {
    hints.push(
      `Your organization requires a sensitivity label on email. Cumulas Mail can't apply labels for ${email ?? "this account"} until your IT team sets them up.`,
    );
  }
  const dropped = droppedLabel(draft, setup);
  if (dropped) {
    hints.push(
      `${originalLabeled(dropped.name)} Cumulas Mail can't apply that label, so this ${draftNoun(draft.replyKind)} might not keep it.`,
    );
  }
  const blocked =
    labels.some((l) => !canChooseLabel(setup, draft.originalLabel, l)) ||
    (!setup.policy.required && !canChooseLabel(setup, draft.originalLabel, null));
  if (labels.length > 0 && blocked) {
    hints.push(
      "Your organization asks for a reason to lower a label, which Cumulas Mail can't give. To lower it, use Outlook on the web.",
    );
  }
  return hints;
}

export function Compose({ onMinimize }: { onMinimize: () => void }) {
  const backend = useBackend();
  const draft = useMailStore((s) => s.compose);
  const update = useMailStore((s) => s.updateCompose);
  const closeCompose = useMailStore((s) => s.closeCompose);
  const askProtectedReply = useMailStore((s) => s.askProtectedReply);
  const { data: accounts = [] } = useAccounts();
  const account = accounts.find((a) => a.id === draft?.accountId);
  // Only Microsoft 365 can encrypt or label what's sent.
  const microsoft = account?.kind === "microsoft";
  const { data: protectionSetup } = useProtectionSetup(draft?.accountId ?? "", microsoft);
  const setup =
    microsoft && protectionSetup?.accountId === draft?.accountId ? protectionSetup : undefined;
  const send = useSendMessage();
  const isDark = useIsDark();
  const signature = useSignatureView(draft);
  const signatureFor = useSentSignature();

  const toRef = useRef<RecipientFieldHandle>(null);
  const ccRef = useRef<RecipientFieldHandle>(null);
  const bccRef = useRef<RecipientFieldHandle>(null);
  const editorRef = useRef<EditorHandle>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const protectionRef = useRef<ComposeProtectionHandle>(null);

  const [recipientError, setRecipientError] = useState<string | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [confirmNoSubject, setConfirmNoSubject] = useState(false);
  /** The original's label that a send would drop, while asking whether to. */
  const [confirmLabelDrop, setConfirmLabelDrop] = useState<{ name: string | null } | null>(null);
  /** Why Send stopped over the label or encryption: said under the fields. */
  const [protectionError, setProtectionError] = useState<string | null>(null);
  const [showQuote, setShowQuote] = useState(true);
  // Waiting for the account's signature before sending.
  const [preparing, setPreparing] = useState(false);
  const sending = send.isPending || preparing;

  // The label a message starts with, once the account's labels are known:
  // the original's, or the organization's default. Filling it in isn't an
  // edit of the draft.
  const labelChosen = draft?.labelChosen ?? true;
  const labelId = draft?.labelId ?? null;
  const originalLabel = draft?.originalLabel ?? null;
  useEffect(() => {
    if (!setup || labelChosen || labelId !== null) return;
    const initial = initialLabelId(setup, originalLabel);
    const dirty = useMailStore.getState().compose?.dirty ?? false;
    if (initial) update({ labelId: initial, dirty });
  }, [setup, labelChosen, labelId, originalLabel, update]);

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

  const doSend = async ({
    allowEmptySubject = false,
    allowLabelDrop = false,
  }: { allowEmptySubject?: boolean; allowLabelDrop?: boolean } = {}) => {
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
    const problem = sendProblem(draft, setup);
    if (problem?.kind === "labelRequired") {
      setProtectionError(LABEL_REQUIRED);
      protectionRef.current?.focusLabels();
      return;
    }
    if (problem?.kind === "chooseLabel") {
      // The original's label can't be kept, and one is required: choosing
      // it is the person's, so the choice opens.
      setProtectionError(
        `${originalLabeled(problem.name)} Cumulas Mail can't apply that label, so choose one for this ${draftNoun(draft.replyKind)}. Your organization requires one.`,
      );
      protectionRef.current?.openLabels();
      return;
    }
    if (!draft.subject.trim() && !allowEmptySubject) {
      setConfirmNoSubject(true);
      return;
    }
    if (problem?.kind === "originalLabelDropped" && !allowLabelDrop) {
      setConfirmLabelDrop({ name: problem.name });
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
    // The label as chosen; Encrypt or Do Not Forward unless the label
    // encrypts. Microsoft 365 refuses what isn't set up (any more).
    const label = setup?.labels.find((l) => l.id === current.labelId);
    const protect = label?.hasProtection ? null : current.protect;
    setProtectionError(null);
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
        protect,
        labelId: current.labelId,
      });
      toast.success("Message sent");
      closeCompose();
    } catch (e) {
      const reason = isBackendError(e) ? e.details?.reason : undefined;
      if (reason === "protectedOriginal" && current.replyToMessageId) {
        // The original turned out to be encrypted: Outlook on the web answers it.
        askProtectedReply({
          messageId: current.replyToMessageId,
          kind: current.replyKind ?? "reply",
          accountKind: account?.kind ?? "microsoft",
          protection: null,
        });
      } else if (
        reason === "labelRequired" ||
        reason === "labelCopied" ||
        reason === "headersNotKept"
      ) {
        // Nothing was sent; the draft stays for another label, or Outlook on the web.
        setProtectionError(errorMessage(e));
        if (reason === "labelRequired") protectionRef.current?.focusLabels();
      } else {
        toast.error(e);
      }
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

  const openOriginal = () => {
    setConfirmLabelDrop(null);
    if (!draft.replyToMessageId) return;
    backend.openInOutlook(draft.replyToMessageId).catch((e: unknown) => toast.error(e));
  };

  const hints = protectionHints(draft, setup, account?.email);
  const tagLabel = offeredLabels(setup).find((l) => l.id === draft.labelId) ?? null;
  const tagAction =
    draft.protect && !tagLabel?.hasProtection && offeredActions(setup).includes(draft.protect)
      ? draft.protect
      : null;

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
        <ComposeProtection
          ref={protectionRef}
          setup={setup}
          protect={draft.protect}
          labelId={draft.labelId}
          originalLabel={draft.originalLabel}
          disabled={sending}
          onProtect={(protect) => {
            setProtectionError(null);
            update({ protect });
          }}
          onLabel={(id) => {
            setProtectionError(null);
            update({ labelId: id, labelChosen: true });
          }}
        />
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
                  onChange={(e) => {
                    setProtectionError(null);
                    // The new account's signature replaces the old one, edited
                    // or not, as in Outlook; its own checks and labels apply.
                    update({
                      accountId: e.target.value,
                      signature: draft.signature === "off" ? "off" : "auto",
                      protect: null,
                      labelId: null,
                      labelChosen: false,
                    });
                  }}
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
              {tagLabel && (
                <span className="mail-tag mail-tag--label">
                  {tagLabel.color && (
                    <span
                      className="mail-tag__dot"
                      style={{ background: tagLabel.color }}
                      aria-hidden="true"
                    />
                  )}
                  <bdi className="mail-tag__name">{labelTitle(tagLabel)}</bdi>
                </span>
              )}
              {tagAction && <span className="mail-tag">{ACTION_NAMES[tagAction]}</span>}
            </div>
          </div>

          {(hints.length > 0 || protectionError) && (
            <div className="mail-compose__protection">
              {protectionError && (
                <p className="mail-compose__protection-error" role="alert">
                  <CircleAlert size={14} aria-hidden="true" />
                  {protectionError}
                </p>
              )}
              {hints.length > 0 && (
                <div role="status">
                  {hints.map((hint) => (
                    <p key={hint} className="mail-compose__hint">
                      {hint}
                    </p>
                  ))}
                </div>
              )}
            </div>
          )}

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
                void doSend({ allowEmptySubject: true });
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
      <Dialog
        open={confirmLabelDrop !== null}
        onClose={() => setConfirmLabelDrop(null)}
        title="Send without the label?"
        width={440}
        footer={
          <>
            <button
              type="button"
              className="btn btn--primary"
              onClick={() => {
                setConfirmLabelDrop(null);
                void doSend({ allowEmptySubject: true, allowLabelDrop: true });
              }}
            >
              Send anyway
            </button>
            {microsoft && draft.replyToMessageId && (
              <button type="button" className="btn" onClick={openOriginal}>
                Open original in Outlook on the web
              </button>
            )}
            <button type="button" className="btn" onClick={() => setConfirmLabelDrop(null)}>
              Cancel
            </button>
          </>
        }
      >
        <p>
          {originalLabeled(confirmLabelDrop?.name ?? null)} Cumulas Mail can&apos;t apply that
          label, so this {draftNoun(draft.replyKind)} might not keep it.
        </p>
      </Dialog>
    </section>
  );
}
