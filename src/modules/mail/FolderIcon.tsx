import {
  Archive,
  FilePenLine,
  Folder,
  FolderOpen,
  Inbox,
  Mails,
  Send,
  ShieldAlert,
  Trash2,
  Upload,
} from "lucide-react";

import type { FolderRole } from "../../api/types";

export function FolderIcon({
  role,
  size = 16,
  open = false,
}: {
  role: FolderRole | "unified";
  size?: number;
  open?: boolean;
}) {
  const props = { size, strokeWidth: 1.75, "aria-hidden": true as const };
  switch (role) {
    case "unified":
      return <Mails {...props} />;
    case "inbox":
      return <Inbox {...props} />;
    case "drafts":
      return <FilePenLine {...props} />;
    case "sent":
      return <Send {...props} />;
    case "archive":
      return <Archive {...props} />;
    case "junk":
      return <ShieldAlert {...props} />;
    case "trash":
      return <Trash2 {...props} />;
    case "outbox":
      return <Upload {...props} />;
    default:
      return open ? <FolderOpen {...props} /> : <Folder {...props} />;
  }
}
