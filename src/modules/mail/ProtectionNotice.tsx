import clsx from "clsx";
import { ExternalLink, Link, LoaderCircle, Lock, Signature } from "lucide-react";
import { type ReactNode, useId, useState } from "react";

import { useBackend } from "../../api/context";
import type { AttachmentMeta, EmailAddress, Message, Protection } from "../../api/types";
import { toast } from "../../components/toast";
import { hasBlankBody, hasProtectionNotice, noticeName } from "./protection";

/**
 * Who a protection notice says the message is from: the address the app
 * saw, after the display name only when that can't pass for an address.
 * Each part is isolated, so neither can reorder the sentence around it.
 */
export function SenderIdentity({ from }: { from: EmailAddress }) {
  const name = noticeName(from.name, from.address);
  if (!name) return <bdi className="mail-notice__address">{from.address}</bdi>;
  return (
    <>
      <bdi>{name}</bdi> (<bdi className="mail-notice__address">{from.address}</bdi>)
    </>
  );
}

function isRpmsg(a: AttachmentMeta): boolean {
  return /\.rpmsg$/i.test(a.name.trim()) || /rpmsg/i.test(a.contentType);
}

function isSignatureFile(a: AttachmentMeta): boolean {
  return /\.p7s$/i.test(a.name.trim()) || /pkcs7-signature/i.test(a.contentType);
}

/** Why a container isn't listed with the attachments. */
function containerLine(a: AttachmentMeta, protection: Protection): ReactNode {
  const file = <bdi className="mail-notice__file">{a.name}</bdi>;
  if (isRpmsg(a) || protection.kind === "rightsManaged") {
    return (
      <>
        {file} holds the protected content. It isn&apos;t a file you can open on this computer, so
        it isn&apos;t listed with the attachments.
      </>
    );
  }
  if (isSignatureFile(a)) {
    return <>{file} holds the signature, so it isn&apos;t listed with the attachments.</>;
  }
  if (protection.kind === "smimeSigned") {
    return (
      <>
        {file} holds the signed content and its attachments, so it isn&apos;t listed with the
        attachments.
      </>
    );
  }
  return <>{file} holds the encrypted content, so it isn&apos;t listed with the attachments.</>;
}

interface ProtectionNoticeProps {
  message: Message;
  /** The message is in a Microsoft account, which Outlook on the web can open it from. */
  microsoft: boolean;
}

/**
 * What the reading pane says about a message Cumulas Mail can't fully
 * open: what it observed (never who protected or signed it, which any
 * sender can claim), why some files aren't listed, and where to open it.
 */
export function ProtectionNotice({ message, microsoft }: ProtectionNoticeProps) {
  const backend = useBackend();
  const titleId = useId();
  const [opening, setOpening] = useState(false);
  const protection = message.protection;
  if (!protection || !hasProtectionNotice(message)) return null;

  const { kind, portalUrl } = protection;
  const from = message.from ? (
    <>
      {" "}
      from <SenderIdentity from={message.from} />
    </>
  ) : null;

  let title: string;
  let text: ReactNode;
  let Icon = Lock;
  switch (kind) {
    case "rightsManaged":
      title = "Protected message";
      text = (
        <>
          This message{from} is protected with Microsoft 365, so Cumulas Mail can&apos;t open it.{" "}
          {microsoft
            ? "Open it in Outlook on the web to read it, reply to it or forward it."
            : portalUrl
              ? "Open it in Microsoft's encrypted message portal with the button below, not with links in the message."
              : "Open it in Outlook, or in Outlook on the web if your mailbox is in Microsoft 365."}
        </>
      );
      break;
    case "smimeEncrypted":
      title = "Encrypted message (S/MIME)";
      text = (
        <>
          This message{from} is encrypted with S/MIME. Cumulas Mail can&apos;t open S/MIME-encrypted
          messages yet.
          {microsoft && " Open it in Outlook on the web if S/MIME is set up there."}
        </>
      );
      break;
    case "smimeSigned":
      title = "Signed message (S/MIME)";
      Icon = Signature;
      text =
        protection.contentHidden && !hasBlankBody(message) ? (
          <>
            This message{from} has an S/MIME signature, which Cumulas Mail doesn&apos;t check. Any
            attachments it has are inside the signature file, which Cumulas Mail can&apos;t open
            yet.
            {microsoft && " Open it in Outlook on the web to see them."}
          </>
        ) : (
          <>
            This message{from} has an S/MIME signature, which Cumulas Mail doesn&apos;t check. Its
            content is inside the signature file, which Cumulas Mail can&apos;t open yet.
            {microsoft && " Open it in Outlook on the web to read it."}
          </>
        );
      break;
    case "portalLink":
      title = "Link to an encrypted message";
      Icon = Link;
      text = (
        <>
          This message{from} links to Microsoft&apos;s encrypted message portal, where you sign in
          to read it. Only open it if you expected a protected message from this sender, and use the
          button below, not links in the message: it goes only to Microsoft&apos;s portal.
        </>
      );
      break;
  }

  const containers = message.attachments.filter((a) => protection.containerIds.includes(a.id));
  const outlook = microsoft && kind !== "portalLink";

  const openInOutlook = async () => {
    // Busy, but keeping the focus: a second press waits for the first.
    if (opening) return;
    setOpening(true);
    try {
      await backend.openInOutlook(message.id);
    } catch (e) {
      toast.error(e);
    } finally {
      setOpening(false);
    }
  };

  const openPortal = () => {
    if (portalUrl) backend.openLink(portalUrl).catch((e: unknown) => toast.error(e));
  };

  return (
    <section
      className={`mail-notice${kind === "portalLink" ? " mail-notice--caution" : ""}`}
      role="note"
      aria-labelledby={titleId}
    >
      <Icon size={18} strokeWidth={1.75} className="mail-notice__icon" aria-hidden="true" />
      <div className="mail-notice__content">
        <h2 id={titleId} className="mail-notice__title">
          {title}
        </h2>
        <p className="mail-notice__text">{text}</p>
        {containers.length > 0 && (
          <ul className="mail-notice__files">
            {containers.map((a) => (
              <li key={a.id}>{containerLine(a, protection)}</li>
            ))}
          </ul>
        )}
        {(outlook || portalUrl) && (
          <div className="mail-notice__actions">
            {outlook && (
              <button
                type="button"
                className={clsx("btn btn--primary btn--sm", opening && "btn--busy")}
                aria-busy={opening || undefined}
                aria-disabled={opening || undefined}
                onClick={() => void openInOutlook()}
              >
                {opening ? (
                  <LoaderCircle size={14} className="spin" aria-hidden="true" />
                ) : (
                  <ExternalLink size={14} aria-hidden="true" />
                )}
                Open in Outlook on the web
              </button>
            )}
            {portalUrl && (
              <button
                type="button"
                className={`btn btn--sm${outlook ? "" : " btn--primary"}`}
                onClick={openPortal}
              >
                <ExternalLink size={14} aria-hidden="true" />
                Open Microsoft&apos;s encrypted message portal
              </button>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
