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
import { exactCount } from "./counts";
import { fileKind, type FileKind } from "./files";
import { splitFileName } from "./text";

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

/** "DOCX" for "plan-v7.docx"; nothing when there is no short extension. */
function extensionOf(name: string): string | null {
  const match = /\.([a-z0-9]{1,4})$/i.exec(name);
  return match ? match[1]!.toUpperCase() : null;
}

/** A small page with the file's kind on it, tinted by type. */
export function FileBadge({ name, contentType }: { name: string; contentType: string }) {
  const kind = fileKind(name, contentType);
  const ext = extensionOf(name);
  return (
    <span className={`mail-file-badge mail-file-badge--${kind}`} aria-hidden="true">
      <FileTypeIcon kind={kind} size={ext ? 14 : 18} />
      {ext && <span className="mail-file-badge__ext">{ext}</span>}
    </span>
  );
}

/**
 * A file name cut in the middle when it doesn't fit: the end of the name and
 * its extension stay, so files that share a long prefix stay apart.
 */
export function FileName({ name, className }: { name: string; className?: string }) {
  const { head, tail } = splitFileName(name);
  return (
    <span className={`mail-filename${className ? ` ${className}` : ""}`} dir="auto" title={name}>
      {tail ? (
        <>
          <span className="mail-filename__head">{head}</span>
          <span className="mail-filename__tail">{tail}</span>
        </>
      ) : (
        <span className="mail-filename__head">{name}</span>
      )}
    </span>
  );
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
      {files.length > 1 && (
        <div className="mail-attachments__summary">
          <span>
            {exactCount(files.length)} attachments · {formatBytes(total)}
          </span>
          <button type="button" className="btn btn--subtle btn--sm" onClick={() => void saveAll()}>
            <Download size={14} aria-hidden="true" />
            Save all
          </button>
        </div>
      )}
      <ul className={`mail-attachments__list${files.length > 1 ? " is-grid" : ""}`}>
        {files.map((a) => (
          <li key={a.id} className="mail-attachment">
            <button
              type="button"
              className="mail-attachment__open"
              title={`Open ${a.name}`}
              aria-label={`Open ${a.name}`}
              disabled={busy === a.id}
              onClick={() => void open(a)}
            >
              <FileBadge name={a.name} contentType={a.contentType} />
              <span className="mail-attachment__text">
                <FileName name={a.name} className="mail-attachment__name" />
                <span className="mail-attachment__size">{formatBytes(a.size)}</span>
              </span>
            </button>
            <button
              type="button"
              className="icon-btn mail-attachment__save"
              aria-label={`Save ${a.name}`}
              title="Save"
              onClick={() => void save(a)}
            >
              <Download size={16} strokeWidth={1.75} />
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
