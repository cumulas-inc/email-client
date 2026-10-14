import {
  Download,
  File,
  FileArchive,
  FileAudio,
  FileCode,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileVideo,
  Presentation,
} from "lucide-react";
import { useState } from "react";

import { useBackend } from "../../api/context";
import type { AttachmentMeta } from "../../api/types";
import { toast } from "../../components/toast";
import { formatBytes } from "../../lib/format";
import { fileKind, type FileKind } from "./files";

export function FileTypeIcon({ kind, size = 20 }: { kind: FileKind; size?: number }) {
  const props = { size, "aria-hidden": true as const };
  switch (kind) {
    case "pdf":
    case "word":
      return <FileText {...props} />;
    case "sheet":
      return <FileSpreadsheet {...props} />;
    case "slides":
      return <Presentation {...props} />;
    case "image":
      return <FileImage {...props} />;
    case "archive":
      return <FileArchive {...props} />;
    case "audio":
      return <FileAudio {...props} />;
    case "video":
      return <FileVideo {...props} />;
    case "code":
      return <FileCode {...props} />;
    default:
      return <File {...props} />;
  }
}

export function Attachments({
  messageId,
  attachments,
}: {
  messageId: string;
  attachments: AttachmentMeta[];
}) {
  const backend = useBackend();
  const [busy, setBusy] = useState<string | null>(null);
  const files = attachments.filter((a) => !a.isInline);
  if (files.length === 0) return null;
  const total = files.reduce((sum, a) => sum + a.size, 0);

  const open = async (a: AttachmentMeta) => {
    setBusy(a.id);
    try {
      await backend.openAttachment(messageId, a.id);
    } catch (e) {
      toast.error(e);
    } finally {
      setBusy(null);
    }
  };

  const save = async (a: AttachmentMeta) => {
    try {
      const path = await backend.saveAttachment(messageId, a.id);
      if (path) toast.success(`Saved to ${path}`);
    } catch (e) {
      toast.error(e);
    }
  };

  const saveAll = async () => {
    const saved: string[] = [];
    for (const a of files) {
      try {
        const path = await backend.saveAttachment(messageId, a.id);
        if (!path) return; // Canceled.
        saved.push(path);
      } catch (e) {
        toast.error(e);
        return;
      }
    }
    const folder = saved[0]?.replace(/[/\\][^/\\]*$/, "") ?? "";
    toast.success(`Saved ${saved.length} attachments to ${folder}`);
  };

  return (
    <section className="mail-attachments" aria-label="Attachments">
      <div className="mail-attachments__summary">
        <span>
          {files.length === 1 ? "1 attachment" : `${files.length} attachments`} ·{" "}
          {formatBytes(total)}
        </span>
        {files.length > 1 && (
          <button type="button" className="btn btn--subtle btn--sm" onClick={() => void saveAll()}>
            <Download size={14} aria-hidden="true" />
            Save all
          </button>
        )}
      </div>
      <ul className="mail-attachments__list">
        {files.map((a) => {
          const kind = fileKind(a.name, a.contentType);
          return (
            <li key={a.id} className="mail-attachment">
              <button
                type="button"
                className="mail-attachment__open"
                title={`Open ${a.name}`}
                aria-label={`Open ${a.name}`}
                disabled={busy === a.id}
                onClick={() => void open(a)}
              >
                <span className={`mail-attachment__icon mail-attachment__icon--${kind}`}>
                  <FileTypeIcon kind={kind} />
                </span>
                <span className="mail-attachment__text">
                  <span className="mail-attachment__name">{a.name}</span>
                  <span className="mail-attachment__size">{formatBytes(a.size)}</span>
                </span>
              </button>
              <button
                type="button"
                className="icon-btn icon-btn--sm mail-attachment__save"
                aria-label={`Save ${a.name}`}
                title="Save"
                onClick={() => void save(a)}
              >
                <Download size={15} />
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
