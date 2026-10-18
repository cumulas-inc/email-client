import clsx from "clsx";
import { ExternalLink } from "lucide-react";
import { useState } from "react";

import { useBackend } from "../../api/context";
import { Dialog } from "../../components/Dialog";
import { toast } from "../../components/toast";
import { useMailStore } from "./mailStore";

/**
 * Reply, Reply all or Forward on an encrypted message: Cumulas Mail can't
 * answer it without removing the encryption, so Outlook on the web does
 * (Microsoft accounts), Microsoft's encrypted message portal when the
 * message links to it, or the organization's own app (IMAP).
 */
export function ProtectedReplyDialog() {
  const backend = useBackend();
  const request = useMailStore((s) => s.protectedReply);
  const close = useMailStore((s) => s.closeProtectedReply);
  const [opening, setOpening] = useState(false);
  if (!request) return null;

  const forward = request.kind === "forward";
  const microsoft = request.accountKind === "microsoft";
  const verb = forward ? "forward" : "reply to";
  const portalUrl = microsoft ? null : request.portalUrl;

  const open = async () => {
    // Busy, but keeping the focus: a second press waits for the first.
    if (opening) return;
    setOpening(true);
    try {
      await backend.openInOutlook(request.messageId);
      close();
    } catch (e) {
      toast.error(e);
    } finally {
      setOpening(false);
    }
  };

  const openPortal = () => {
    if (!portalUrl) return;
    backend.openLink(portalUrl).catch((e: unknown) => toast.error(e));
    close();
  };

  let text: string;
  if (microsoft && request.protection === "rightsManaged") {
    // Outlook on the web keeps Microsoft 365's protection.
    text = `This message is protected. Cumulas Mail can't ${verb} it without removing the protection, so open it in Outlook on the web, which keeps it.`;
  } else if (microsoft && request.protection === "smimeEncrypted") {
    // S/MIME only where it's set up there.
    text = `This message is encrypted. Cumulas Mail can't ${verb} it without removing the encryption. Open it in Outlook on the web, which can ${verb} it encrypted if S/MIME is set up there.`;
  } else if (microsoft) {
    text = `This message is encrypted. Cumulas Mail can't ${verb} it without removing the encryption, so open it in Outlook on the web.`;
  } else if (portalUrl) {
    text = `Cumulas Mail can't ${verb} this message without removing its protection. ${forward ? "Forward" : "Reply to"} it in Microsoft's encrypted message portal, with the button below.`;
  } else {
    text = `Cumulas Mail can't ${verb} this message without removing its protection. Use Outlook, or the app your organization provides for protected mail.`;
  }

  return (
    <Dialog
      open
      onClose={close}
      title={
        microsoft
          ? forward
            ? "Forward in Outlook on the web"
            : "Reply in Outlook on the web"
          : forward
            ? "Can't forward a protected message"
            : "Can't reply to a protected message"
      }
      width={440}
      footer={
        microsoft ? (
          <>
            <button
              type="button"
              className={clsx("btn btn--primary", opening && "btn--busy")}
              aria-busy={opening || undefined}
              aria-disabled={opening || undefined}
              onClick={() => void open()}
            >
              <ExternalLink size={15} aria-hidden="true" />
              Open in Outlook on the web
            </button>
            <button type="button" className="btn" onClick={close}>
              Cancel
            </button>
          </>
        ) : portalUrl ? (
          <>
            <button type="button" className="btn btn--primary" onClick={openPortal}>
              <ExternalLink size={15} aria-hidden="true" />
              Open Microsoft&apos;s encrypted message portal
            </button>
            <button type="button" className="btn" onClick={close}>
              Close
            </button>
          </>
        ) : (
          <button type="button" className="btn btn--primary" onClick={close}>
            Close
          </button>
        )
      }
    >
      <p>{text}</p>
    </Dialog>
  );
}
