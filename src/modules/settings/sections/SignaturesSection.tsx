import { Building2, LoaderCircle, Signature } from "lucide-react";
import { type CSSProperties, useCallback, useEffect, useId, useRef, useState } from "react";

import { useBackend } from "../../../api/context";
import {
  useAccounts,
  useAccountSignature,
  useCheckOrganizationSignature,
  useUpdateOwnSignature,
} from "../../../api/queries";
import type { Account, AccountSignature, OrganizationSignature } from "../../../api/types";
import { EmptyState } from "../../../components/EmptyState";
import { PaneSpinner } from "../../../components/Spinner";
import { classifyLink, signatureAppearance } from "../../mail/bodyHtml";
import { useIsDark } from "../../mail/hooks";
import { MessageBody } from "../../mail/MessageBody";
import { SignatureEditor, type SignatureEditorHandle } from "../../mail/SignatureEditor";
import { Callout } from "../Fields";
import { trackSave } from "../saveStatus";

/** Opening the section asks organizations again after this long. */
const RECHECK_MS = 10 * 60_000;
/** Typing saves after this pause (and on leaving the editor). */
const SAVE_DELAY_MS = 600;

export function SignaturesSection() {
  const { data: accounts, isLoading } = useAccounts();
  if (isLoading || !accounts) return <PaneSpinner />;
  if (accounts.length === 0) {
    return (
      <EmptyState
        title="No accounts yet"
        description="Add an account, then set up its signature here."
      />
    );
  }
  return (
    <>
      <p className="settings-intro">
        Each account&apos;s signature goes below what you write, above any message you reply to.
      </p>
      {accounts.map((account) => (
        <AccountSignatures key={account.id} account={account} />
      ))}
    </>
  );
}

function AccountSignatures({ account }: { account: Account }) {
  const titleId = useId();
  const { data: signature, isError } = useAccountSignature(account.id);
  const { mutate: checkNow } = useCheckOrganizationSignature();

  // Ask the organization again when its last answer is getting old.
  const asked = useRef(false);
  useEffect(() => {
    if (!signature || asked.current) return;
    asked.current = true;
    const { state, checkedAt } = signature.organization;
    if (state === "unsupported") return;
    const age = checkedAt ? Date.now() - new Date(checkedAt).getTime() : Infinity;
    if (age > RECHECK_MS) checkNow(account.id);
  }, [signature, checkNow, account.id]);

  let body;
  if (isError) {
    body = <p className="signature-caption">Couldn&apos;t load this account&apos;s signature.</p>;
  } else if (signature) {
    body = <SignatureSettings signature={signature} />;
  }

  return (
    <section className="settings-group" aria-labelledby={titleId}>
      <h4 id={titleId} className="settings-group__title signature-account">
        <span
          className="signature-account__dot"
          style={{ "--account-color": account.color } as CSSProperties}
          aria-hidden="true"
        />
        <span className="signature-account__name" dir="auto">
          {account.displayName}
        </span>{" "}
        <span className="signature-account__email">{account.email}</span>
      </h4>
      <div className="settings-card signature-card">{body}</div>
    </section>
  );
}

function SignatureSettings({ signature }: { signature: AccountSignature }) {
  const organization = signature.organization;
  if (organization.state === "ready" && organization.html) {
    return <OrganizationPreview organization={organization} html={organization.html} />;
  }
  if (organization.state === "addedOnSend") {
    return (
      <>
        <p className="signature-caption">
          <Building2 size={16} strokeWidth={1.75} aria-hidden="true" />
          <span>
            Your organization adds your signature when you send, so Cumulas Mail doesn&apos;t add
            one.
          </span>
        </p>
        {organization.stale && <StaleNote />}
      </>
    );
  }
  return (
    <>
      {organization.state === "notAllowed" && (
        <Callout>
          <p>
            Your organization uses Cumulas Signatures but hasn&apos;t allowed Cumulas Mail yet. Ask
            your IT admin to allow it, and your signature will appear here.
          </p>
        </Callout>
      )}
      {organization.state === "unchecked" && (
        <p className="signature-checking" role="status">
          <LoaderCircle size={14} className="spin" aria-hidden="true" />
          Checking for your organization&apos;s signature…
        </p>
      )}
      <OwnSignature signature={signature} />
    </>
  );
}

function StaleNote() {
  return (
    <p className="signature-caption signature-caption--quiet">
      Couldn&apos;t check for updates; showing the last version.
    </p>
  );
}

function OrganizationPreview({
  organization,
  html,
}: {
  organization: OrganizationSignature;
  html: string;
}) {
  const backend = useBackend();
  const isDark = useIsDark();
  const appearance = signatureAppearance(html, isDark);
  return (
    <>
      <div className={`signature-preview signature-preview--${appearance}`}>
        <MessageBody
          html={html}
          allowRemote
          appearance={appearance}
          title="Your organization's signature"
          onLink={(href) => {
            const link = classifyLink(href);
            if (link.kind === "open") void backend.openLink(link.url);
          }}
        />
      </div>
      <p className="signature-caption">
        <Signature size={16} strokeWidth={1.75} aria-hidden="true" />
        <span>
          Set by your organization with Cumulas Signatures. Added to new messages
          {organization.onReplies ? " and replies" : ""}. You can change it in any message before
          you send.
        </span>
      </p>
      {organization.stale && <StaleNote />}
    </>
  );
}

/** The account's own signature; edits save as you type. */
function OwnSignature({ signature }: { signature: AccountSignature }) {
  const { accountId } = signature;
  const { mutateAsync } = useUpdateOwnSignature();
  const editorRef = useRef<SignatureEditorHandle>(null);
  // The editor starts from the saved signature and keeps what's typed;
  // saves don't put their (sanitized) answer back into it mid-edit.
  const [initial] = useState(signature.ownHtml);
  const [onReplies, setOnReplies] = useState(signature.ownOnReplies);
  const latest = useRef({ html: signature.ownHtml, onReplies: signature.ownOnReplies });
  const pending = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const flush = useCallback(() => {
    clearTimeout(timer.current);
    if (!pending.current) return;
    pending.current = false;
    // A failure shows next to the title; the editor keeps the text.
    trackSave(mutateAsync({ accountId, ...latest.current })).catch(() => undefined);
  }, [accountId, mutateAsync]);

  // Closing Settings mid-pause still saves.
  useEffect(() => () => flush(), [flush]);

  const change = (patch: Partial<typeof latest.current>, delay: number) => {
    latest.current = { ...latest.current, ...patch };
    pending.current = true;
    clearTimeout(timer.current);
    if (delay > 0) timer.current = setTimeout(flush, delay);
    else flush();
  };

  return (
    <>
      <div
        className="signature-field"
        // The frame is as tall as the signature; the rest of the field
        // focuses it too.
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) {
            e.preventDefault();
            editorRef.current?.focus();
          }
        }}
      >
        <SignatureEditor
          ref={editorRef}
          html={initial}
          letter
          minLines={3}
          placeholder="Add a signature"
          onChange={(html) => change({ html }, SAVE_DELAY_MS)}
          onBlur={flush}
        />
      </div>
      <label className="checkbox signature-replies">
        <input
          type="checkbox"
          checked={onReplies}
          onChange={(e) => {
            setOnReplies(e.target.checked);
            change({ onReplies: e.target.checked }, 0);
          }}
        />
        Add it to replies and forwards
      </label>
    </>
  );
}
