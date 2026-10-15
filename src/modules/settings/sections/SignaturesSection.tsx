import { Building2, LoaderCircle, Signature } from "lucide-react";
import {
  type CSSProperties,
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";

import {
  useAccounts,
  useAccountSignature,
  useCheckOrganizationSignature,
  useUpdateOwnSignature,
} from "../../../api/queries";
import type { Account, AccountSignature, OrganizationSignature } from "../../../api/types";
import { EmptyState } from "../../../components/EmptyState";
import { PaneSpinner } from "../../../components/Spinner";
import { signatureAppearance } from "../../mail/bodyHtml";
import { useIsDark } from "../../mail/hooks";
import { SignatureEditor, type SignatureEditorHandle } from "../../mail/SignatureEditor";
import { trackSave } from "../saveStatus";

/** Opening the section asks organizations again after this long (or after a failed check). */
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
  const { mutate: checkNow, isPending: checking } = useCheckOrganizationSignature();

  // Ask the organization again when its last answer is getting old, or the
  // last check failed. Not one that said it doesn't sign the account's mail:
  // the app asks that once a day, and at sign-in.
  const asked = useRef(false);
  useEffect(() => {
    if (!signature || asked.current) return;
    asked.current = true;
    const { state, checkedAt, stale } = signature.organization;
    if (state === "unsupported" || state === "notUsed") return;
    const age = checkedAt ? Date.now() - new Date(checkedAt).getTime() : Infinity;
    if (stale || age > RECHECK_MS) checkNow(account.id);
  }, [signature, checkNow, account.id]);

  let body;
  if (isError) {
    body = <p className="signature-caption">Couldn&apos;t load this account&apos;s signature.</p>;
  } else if (signature) {
    body = <SignatureSettings signature={signature} checking={checking} />;
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

/** What the account's organization said; `checking` while Settings asks it again. */
function SignatureSettings({
  signature,
  checking,
}: {
  signature: AccountSignature;
  checking: boolean;
}) {
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
            Your organization adds your signature to new messages
            {organization.onReplies ? ", replies and forwards" : ""} when you send, so Cumulas Mail
            doesn&apos;t add one.
          </span>
        </p>
        {organization.stale && (
          <StaleNote>
            Couldn&apos;t check for changes; this is what your organization last said.
          </StaleNote>
        )}
      </>
    );
  }
  return (
    <>
      {/* Microsoft or the service refused the app, which may be down to
          how the service is set up rather than to the organization. */}
      {organization.state === "notAllowed" && (
        <p className="signature-caption">
          <Building2 size={16} strokeWidth={1.75} aria-hidden="true" />
          <span>
            Cumulas Mail can&apos;t get a company signature from Cumulas Signatures for this account
            yet. Until it can, your own signature below is used.
          </span>
        </p>
      )}
      {/* Never asked yet, or asked again here after a failed first check. */}
      {organization.state === "unchecked" && (!organization.stale || checking) && (
        <p className="signature-checking" role="status">
          <LoaderCircle size={14} className="spin" aria-hidden="true" />
          Checking for your organization&apos;s signature…
        </p>
      )}
      {organization.state === "unchecked" && organization.stale && !checking && (
        <p className="signature-caption">
          <Building2 size={16} strokeWidth={1.75} aria-hidden="true" />
          <span>
            Couldn&apos;t check for your organization&apos;s signature yet. Until Cumulas Mail can,
            your own signature below is used.
          </span>
        </p>
      )}
      <OwnSignature signature={signature} />
    </>
  );
}

/** The last check failed: what's shown is what the organization last said. */
function StaleNote({ children }: { children: ReactNode }) {
  return <p className="signature-caption signature-caption--quiet">{children}</p>;
}

function OrganizationPreview({
  organization,
  html,
}: {
  organization: OrganizationSignature;
  html: string;
}) {
  const isDark = useIsDark();
  const appearance = signatureAppearance(html, isDark);
  return (
    <>
      {/* In the frame the composer edits it in, so it looks as it does
          there, and as it's sent. */}
      <div className={`signature-preview signature-preview--${appearance}`}>
        <SignatureEditor html={html} readOnly title="Your organization's signature" />
      </div>
      <p className="signature-caption">
        <Signature size={16} strokeWidth={1.75} aria-hidden="true" />
        <span>
          Set by your organization with Cumulas Signatures. Added to new messages
          {organization.onReplies ? ", replies and forwards" : ""}. You can change it in any message
          before you send.
        </span>
      </p>
      {organization.stale && (
        <StaleNote>Couldn&apos;t check for updates; showing the last version.</StaleNote>
      )}
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
