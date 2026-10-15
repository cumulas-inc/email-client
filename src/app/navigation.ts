// Where the desktop sends the page: a notification or the tray was clicked
// (the `navigate` event, or takePendingNavigation when the window was
// created for it).

import type { QueryClient } from "@tanstack/react-query";

import type { Backend } from "../api/backend";
import { qk } from "../api/queries";
import type { Folder, Navigate } from "../api/types";
import { toast } from "../components/toast";
import { ALL_INBOXES, useMailStore } from "../modules/mail/mailStore";
import { useAppStore } from "./store";

async function folders(backend: Backend, client: QueryClient): Promise<Folder[]> {
  try {
    return await client.fetchQuery({
      queryKey: qk.folders(),
      queryFn: () => backend.listFolders(),
    });
  } catch {
    return [];
  }
}

export async function followNavigation(
  to: Navigate,
  backend: Backend,
  client: QueryClient,
): Promise<void> {
  const app = useAppStore.getState();
  // What was asked for comes first: Settings would cover it.
  app.closeSettings();
  if (to.kind === "compose") {
    app.requestCompose({});
    return;
  }
  app.setModule("mail");
  app.setSearch("");
  const mail = useMailStore.getState();
  if (to.kind === "message") {
    const folder = (await folders(backend, client)).find((f) => f.id === to.folderId);
    mail.openMessage(to.messageId, to.folderId, folder?.role === "inbox");
    return;
  }
  if (to.notice) toast.info(to.notice);
  const inbox =
    to.accountId === null
      ? undefined
      : (await folders(backend, client)).find(
          (f) => f.accountId === to.accountId && f.role === "inbox",
        );
  mail.selectFolder(inbox?.id ?? ALL_INBOXES);
}
