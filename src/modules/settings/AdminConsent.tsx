import { Check, Copy, ExternalLink, Info, Mail, ShieldCheck } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";

import { useBackend } from "../../api/context";
import type { AccountFeature } from "../../api/types";
import { toast } from "../../components/toast";
import {
  type ApprovalPurpose,
  FEATURE_COPY,
  adminConsentMailto,
  linkIncludesPush,
  organizationName,
} from "./microsoft";
import { PermissionList } from "./PermissionList";
import { ResultPanel } from "./StepLayout";

interface AdminConsentProps {
  adminConsentUrl: string;
  tenant?: string;
  /** The address the user tried to sign in with. */
  email?: string;
  /** Microsoft's own "Request approval" button is available to the user. */
  canRequestApproval?: boolean;
  /** The page for IT administrators; null for an organization's own app registration. */
  adminPageUrl?: string | null;
  /** The approval is for this optional feature; the account's mail works. */
  feature?: AccountFeature;
  /** What the button that signs in again says. */
  retryLabel?: "Try again" | "Turn on";
  /** Offers to sign in for mail only, which some organizations allow. */
  onMailOnly?: () => void;
  /** Signing in for mail only needed an administrator's approval too. */
  mailOnlyFailed?: boolean;
  /**
   * The link came from "Microsoft said I need admin approval" or a typed
   * code, so whether Microsoft's page offers Request approval is unknown.
   */
  manual?: boolean;
  /** The title's id, for callers that move focus to it. */
  titleId?: string;
}

/**
 * Shown when the organization only allows apps an administrator approved.
 * Explains that nothing is wrong and makes forwarding the approval link easy.
 */
export function AdminConsent({
  adminConsentUrl,
  tenant,
  email,
  canRequestApproval,
  adminPageUrl,
  feature,
  retryLabel = "Try again",
  onMailOnly,
  mailOnlyFailed,
  manual,
  titleId,
}: AdminConsentProps) {
  const ownTitleId = useId();
  const org = organizationName(tenant);
  const orgName = org ? <strong>{org}</strong> : null;
  const object = feature ? FEATURE_COPY[feature].object : null;

  let steps;
  if (canRequestApproval) {
    steps = (
      <>
        <li>
          <span>
            Select <strong>Request approval</strong>. Microsoft&apos;s sign-in page opens with a
            form to ask your IT team.
          </span>
        </li>
        <li>
          <span>
            Add a short reason, such as “I use Linux and need my work mail”, and send the request.
          </span>
        </li>
        <li>
          <span>
            Microsoft usually emails you when your IT team responds. Once they approve it, come back
            and select <strong>{retryLabel}</strong>.
          </span>
        </li>
      </>
    );
  } else if (feature) {
    steps = (
      <>
        <li>
          <span>Send the approval link below to your IT team.</span>
        </li>
        <li>
          <span>
            Once they&apos;ve approved it, Cumulas Mail picks up the approval by itself within a
            couple of hours. To use it right away, select <strong>Turn on</strong>.
          </span>
        </li>
      </>
    );
  } else {
    steps = (
      <>
        <li>
          <span>Send the approval link below to your IT team.</span>
        </li>
        <li>
          <span>
            Once they&apos;ve approved it, come back and select <strong>{retryLabel}</strong>.
          </span>
        </li>
      </>
    );
  }

  return (
    <ResultPanel
      icon={<ShieldCheck size={28} />}
      tone="brand"
      align="start"
      titleId={titleId ?? ownTitleId}
      title={
        object
          ? `Your organization needs to approve access to ${object}`
          : "Your organization needs to approve Cumulas Mail"
      }
    >
      {object ? (
        <p>
          Your mail keeps working. {orgName ?? "Your organization"} only lets apps use {object}{" "}
          after its IT team approves them.
        </p>
      ) : (
        <p>
          This isn&apos;t something you did wrong, and nothing is broken.{" "}
          {orgName ? <>{orgName} only allows</> : "Your organization only allows"} apps that its IT
          team has approved. An administrator can approve Cumulas Mail for everyone in a couple of
          minutes.
        </p>
      )}
      {mailOnlyFailed && (
        <p>
          In your organization, mail needs an administrator&apos;s approval too, so mail only
          didn&apos;t help.
        </p>
      )}
      <ol className="consent-steps">{steps}</ol>
      {manual && !canRequestApproval && (
        <p className="consent-alt">
          <Info size={16} aria-hidden="true" />
          <span>
            If Microsoft&apos;s page offered <strong>Request approval</strong>, you can use it
            instead: select <strong>{retryLabel}</strong> and send the request from there.
          </span>
        </p>
      )}
      {onMailOnly && (
        <div className="consent-mail-only">
          <p className="consent-alt">
            <Info size={16} aria-hidden="true" />
            <span>
              Your organization may let you use mail before an administrator approves the rest.
              Select <strong>Try with mail only</strong> to sign in without calendar, contacts and
              tasks; you can turn them on later.
            </span>
          </p>
          <button type="button" className="btn btn--sm" onClick={onMailOnly}>
            Try with mail only
          </button>
        </div>
      )}
      {canRequestApproval && <p>Or send the approval link to IT yourself:</p>}

      <ApprovalLink
        adminConsentUrl={adminConsentUrl}
        tenant={tenant}
        email={email}
        adminPageUrl={adminPageUrl}
        purpose={feature ? "features" : "signIn"}
        features={feature ? [feature] : undefined}
        emphasis={canRequestApproval ? "secondary" : "primary"}
      />

      <details className="permissions">
        <summary>What Cumulas Mail asks for</summary>
        <p className="permissions__intro">
          Only permissions that act as you. Your data stays on this computer and goes only to
          Microsoft.
        </p>
        <PermissionList showScopes push={linkIncludesPush(adminConsentUrl)} onRequest />
      </details>
    </ResultPanel>
  );
}

interface ApprovalLinkProps {
  adminConsentUrl: string;
  tenant?: string;
  /** The account's address, for the request to IT. */
  email?: string;
  /** What the approval is for: signing in (the default), instant updates or features. */
  purpose?: ApprovalPurpose;
  /** The page for IT administrators; null for an organization's own app registration. */
  adminPageUrl?: string | null;
  /** The features to approve, for `features`. */
  features?: AccountFeature[];
  /**
   * How prominent Email IT is: the screen's main action, beside another
   * main action, or small in a settings row.
   */
  emphasis?: "primary" | "secondary" | "compact";
  /** Email IT opens this instead of the approval request (a blocked sign-in's own). */
  mailto?: string;
}

/** The approval link, to copy, to send to IT in a ready-made email, or to open as an administrator. */
export function ApprovalLink({
  adminConsentUrl,
  tenant,
  email,
  purpose = "signIn",
  adminPageUrl,
  features,
  emphasis = "primary",
  mailto,
}: ApprovalLinkProps) {
  const backend = useBackend();
  const linkId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const [copied, setCopied] = useState(false);
  const compact = emphasis === "compact";

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 2500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(adminConsentUrl);
      setCopied(true);
    } catch {
      inputRef.current?.focus();
      inputRef.current?.select();
      toast.info("Couldn't copy automatically. The link is selected — press Ctrl+C to copy it.");
    }
  };

  const emailIt = () => {
    const request =
      mailto ??
      adminConsentMailto({ adminConsentUrl, email, tenant, purpose, adminPageUrl, features });
    backend.openLink(request).catch((e: unknown) => toast.error(e));
  };

  const openPage = () => {
    backend.openLink(adminConsentUrl).catch((e: unknown) => toast.error(e));
  };

  return (
    <>
      <div className="consent-link">
        <label htmlFor={linkId} className="field__label">
          Approval link for your IT administrator
        </label>
        <div className="consent-link__row">
          <input
            ref={inputRef}
            id={linkId}
            className="input consent-link__input selectable"
            readOnly
            value={adminConsentUrl}
            onFocus={(e) => e.currentTarget.select()}
          />
          <button type="button" className="btn" onClick={() => void copy()}>
            {copied ? (
              <Check size={16} aria-hidden="true" />
            ) : (
              <Copy size={16} aria-hidden="true" />
            )}
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
        <span role="status" className="visually-hidden">
          {copied ? "Link copied to the clipboard" : ""}
        </span>
      </div>

      <div className="consent-email">
        <div className="consent-email__actions">
          <button
            type="button"
            className={
              emphasis === "primary" ? "btn btn--primary" : compact ? "btn btn--sm" : "btn"
            }
            onClick={emailIt}
          >
            <Mail size={compact ? 14 : 16} aria-hidden="true" />
            Email IT
          </button>
          <button type="button" className={compact ? "btn btn--sm" : "btn"} onClick={openPage}>
            <ExternalLink size={compact ? 14 : 16} aria-hidden="true" />
            Open approval page
          </button>
        </div>
        <p className="consent-email__hint">
          Email IT opens your email app with a ready-to-send request; just add your IT team&apos;s
          address. If you&apos;re an administrator yourself, select{" "}
          <strong>Open approval page</strong> and sign in with your admin account.
          {adminPageUrl === null &&
            " After approving, your browser shows a page that can't be reached. That's expected: close it and come back to Cumulas Mail."}
        </p>
      </div>
    </>
  );
}
