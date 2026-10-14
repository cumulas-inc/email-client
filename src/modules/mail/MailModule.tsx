import "./mail.css";

import { Pencil, X } from "lucide-react";
import { type CSSProperties, useCallback, useEffect, useState } from "react";

import { useAppStore } from "../../app/store";
import { useAccounts, useSettings } from "../../api/queries";
import { useMailActions } from "./actions";
import { Compose, DiscardDialog } from "./Compose";
import { FolderPane } from "./FolderPane";
import { usePersistentNumber } from "./hooks";
import { ALL_INBOXES, useMailStore } from "./mailStore";
import { MessageList } from "./MessageList";
import { optionId } from "./dom";
import { useFolders } from "./queries";
import { ReadingPane } from "./ReadingPane";
import { useMailShortcuts } from "./shortcuts";
import { Splitter } from "./Splitter";

/** Tab for a draft that is open but not on screen, like Outlook's draft tabs. */
function DraftBar() {
  const compose = useMailStore((s) => s.compose);
  const showCompose = useMailStore((s) => s.showCompose);
  const closeCompose = useMailStore((s) => s.closeCompose);
  const [confirm, setConfirm] = useState(false);
  if (!compose) return null;
  const subject = compose.subject.trim() || "(no subject)";
  return (
    <div className="mail-draftbar" role="region" aria-label="Unsent draft">
      <button
        type="button"
        className="mail-draftbar__open"
        onClick={showCompose}
        title="Continue writing"
      >
        <Pencil size={14} aria-hidden="true" />
        <span className="mail-draftbar__label">Draft</span>
        <span className="mail-draftbar__subject">{subject}</span>
      </button>
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        aria-label="Discard draft"
        title="Discard draft"
        onClick={() => (compose.dirty ? setConfirm(true) : closeCompose())}
      >
        <X size={14} />
      </button>
      <DiscardDialog
        open={confirm}
        onCancel={() => setConfirm(false)}
        onDiscard={() => {
          setConfirm(false);
          closeCompose();
        }}
      />
    </div>
  );
}

function PendingComposeDialog() {
  const pending = useMailStore((s) => s.pendingCompose);
  const confirm = useMailStore((s) => s.confirmPendingCompose);
  const cancel = useMailStore((s) => s.cancelPendingCompose);
  return <DiscardDialog open={pending !== null} onCancel={cancel} onDiscard={confirm} />;
}

export function MailModule() {
  const actions = useMailActions();
  const { data: settings } = useSettings();
  const readingPane = settings?.readingPane ?? "right";
  const { data: folders } = useFolders();
  const { data: accounts } = useAccounts();

  const folderId = useMailStore((s) => s.folderId);
  const selectFolder = useMailStore((s) => s.selectFolder);
  const readerOpen = useMailStore((s) => s.readerOpen);
  const setReaderOpen = useMailStore((s) => s.setReaderOpen);
  const compose = useMailStore((s) => s.compose);
  const composeVisible = useMailStore((s) => s.composeVisible);
  const hideCompose = useMailStore((s) => s.hideCompose);

  const [foldersWidth, setFoldersWidth] = usePersistentNumber("mail.foldersWidth", 232, 180, 420);
  const [listWidth, setListWidth] = usePersistentNumber("mail.listWidth", 400, 300, 760);
  const [listHeight, setListHeight] = usePersistentNumber("mail.listHeight", 320, 160, 720);

  useMailShortcuts(actions, readingPane);

  // Open the first inbox, and recover when the selected folder disappears.
  useEffect(() => {
    if (!folders || folders.length === 0) return;
    if (folderId === ALL_INBOXES || (folderId && folders.some((f) => f.id === folderId))) return;
    const firstAccount = accounts?.[0]?.id;
    const inbox =
      folders.find((f) => f.role === "inbox" && f.accountId === firstAccount) ??
      folders.find((f) => f.role === "inbox") ??
      folders[0]!;
    selectFolder(inbox.id);
  }, [folders, folderId, accounts, selectFolder]);

  // Other modules (e.g. People) ask for a new message through the app store.
  const composeRequest = useAppStore((s) => s.composeRequest);
  const clearComposeRequest = useAppStore((s) => s.clearComposeRequest);
  useEffect(() => {
    if (!composeRequest || !accounts) return;
    const accountId =
      composeRequest.accountId && accounts.some((a) => a.id === composeRequest.accountId)
        ? composeRequest.accountId
        : undefined;
    actions.newMessage({
      to: composeRequest.to ?? [],
      cc: composeRequest.cc ?? [],
      subject: composeRequest.subject ?? "",
      bodyHtml: composeRequest.bodyHtml ?? "",
      ...(accountId ? { accountId } : {}),
    });
    clearComposeRequest();
  }, [composeRequest, accounts, actions, clearComposeRequest]);

  const composerOnScreen = compose !== null && composeVisible;
  const off = readingPane === "off";
  const showReader = !off || readerOpen || composerOnScreen;
  const showList = !off || !(readerOpen || composerOnScreen);

  const backToList = useCallback(() => {
    setReaderOpen(false);
    hideCompose();
    // Return focus to the open message in the list.
    requestAnimationFrame(() => {
      const id = useMailStore.getState().activeId;
      if (id) document.getElementById(optionId(id))?.focus();
    });
  }, [setReaderOpen, hideCompose]);

  const minimizeCompose = useCallback(() => {
    hideCompose();
    if (off) setReaderOpen(false);
  }, [hideCompose, off, setReaderOpen]);

  const draftBar = compose && !composerOnScreen ? <DraftBar /> : null;

  const style = {
    "--mail-folders-w": `${foldersWidth}px`,
    "--mail-list-w": `${listWidth}px`,
    "--mail-list-h": `${listHeight}px`,
  } as CSSProperties;

  return (
    <div className={`mail mail--${readingPane}`} style={style}>
      <aside className="mail__folders" aria-label="Mail folders">
        <FolderPane actions={actions} />
      </aside>
      <Splitter
        orientation="vertical"
        value={foldersWidth}
        min={180}
        max={420}
        onChange={setFoldersWidth}
        label="Resize folder pane"
      />
      <div className="mail__main">
        {showList && (
          <div className="mail__panel mail__list-panel">
            <MessageList
              actions={actions}
              readingPane={readingPane}
              onEditDraft={actions.editDraft}
            />
            {!showReader && draftBar}
          </div>
        )}
        {showList && showReader && (
          <Splitter
            orientation={readingPane === "bottom" ? "horizontal" : "vertical"}
            value={readingPane === "bottom" ? listHeight : listWidth}
            min={readingPane === "bottom" ? 160 : 300}
            max={readingPane === "bottom" ? 720 : 760}
            onChange={readingPane === "bottom" ? setListHeight : setListWidth}
            label={readingPane === "bottom" ? "Resize message list height" : "Resize message list"}
          />
        )}
        {showReader && (
          <div className="mail__panel mail__reader-panel">
            {composerOnScreen && compose ? (
              <Compose key={compose.id} onMinimize={minimizeCompose} />
            ) : (
              <ReadingPane
                actions={actions}
                onBack={off ? backToList : null}
                onEditDraft={actions.editDraft}
              />
            )}
            {draftBar}
          </div>
        )}
      </div>
      <PendingComposeDialog />
    </div>
  );
}
