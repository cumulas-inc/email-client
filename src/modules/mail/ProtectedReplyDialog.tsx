import { ExternalLink } from "lucide-react";
import { useState } from "react";

import { useBackend } from "../../api/context";
import { Dialog } from "../../components/Dialog";
import { toast } from "../../components/toast";
import { useMailStore } from "./mailStore";

/**
 * Reply, Reply all or Forward on an encrypted message: Cumulas Mail can't
 * answer it without removing the encryption, so Outlook on the web does
 * (Microsoft accounts), or the organization's own app (IMAP).
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

  const open = async () => {
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
              className="btn btn--primary"
              aria-busy={opening || undefined}
              disabled={opening}
              onClick={() => void open()}
            >
              <ExternalLink size={15} aria-hidden="true" />
              Open in Outlook on the web
            </button>
            <button type="button" className="btn" onClick={close}>
              Cancel
            </button>
          </>
        ) : (
          <button type="button" className="btn btn--primary" onClick={close}>
            Close
          </button>
        )
      }
    >
      <p>
        {microsoft
          ? `This message is protected. Cumulas Mail can't ${verb} it without removing the protection, so open it in Outlook on the web, which keeps it.`
          : `Cumulas Mail can't ${verb} this message without removing its protection. Use Outlook, or the app your organization provides for protected mail.`}
      </p>
    </Dialog>
  );
}
