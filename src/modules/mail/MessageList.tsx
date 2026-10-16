import { useQuery } from "@tanstack/react-query";
import {
  Archive,
  Flag,
  FolderInput,
  Inbox,
  Mail,
  MailOpen,
  SearchX,
  Trash2,
  X,
} from "lucide-react";
import {
  type DragEvent,
  type KeyboardEvent,
  type MouseEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { useAppStore } from "../../app/store";
import { useBackend } from "../../api/context";
import { errorMessage } from "../../api/errors";
import { qk, useAccounts } from "../../api/queries";
import type { Account, Folder, Message, MessageSummary, ReadingPane } from "../../api/types";
import { EmptyState } from "../../components/EmptyState";
import { Menu } from "../../components/Menu";
import { SearchField } from "../../components/SearchField";
import { PaneSpinner, Spinner } from "../../components/Spinner";
import { formatCount } from "../../lib/format";
import type { MailActions } from "./actions";
import { exactCount } from "./counts";
import { listGroup } from "./dates";
import { endMailDrag, MAIL_DRAG_TYPE, startMailDrag } from "./dnd";
import { FolderIcon } from "./FolderIcon";
import { folderPath, moveTargets } from "./folders";
import { useDebounced } from "./hooks";
import { ALL_INBOXES, type MailFilter, useMailStore } from "./mailStore";
import { optionId } from "./dom";
import { MessageRow, type RowHandlers } from "./MessageRow";
import { useFolders, useMessageList, useSearch } from "./queries";

const FILTERS: { value: MailFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "unread", label: "Unread" },
  { value: "flagged", label: "Flagged" },
];

function matchesFilter(filter: MailFilter) {
  return (m: MessageSummary) => filter === "all" || (filter === "unread" ? !m.isRead : m.isFlagged);
}

function insertByDate(items: MessageSummary[], item: MessageSummary): MessageSummary[] {
  const index = items.findIndex((m) => m.receivedAt < item.receivedAt);
  if (index < 0) return [...items, item];
  return [...items.slice(0, index), item, ...items.slice(index)];
}

interface SelectionBarProps {
  ids: string[];
  actions: MailActions;
  folders: Folder[];
  messages: MessageSummary[];
}

function SelectionBar({ ids, actions, folders, messages }: SelectionBarProps) {
  const clearSelection = useMailStore((s) => s.clearSelection);
  const chosen = messages.filter((m) => ids.includes(m.id));
  const accountIds = new Set(chosen.map((m) => m.accountId));
  const accountId = accountIds.size === 1 ? [...accountIds][0]! : null;
  const sourceFolders = new Set(chosen.map((m) => m.folderId));
  const exclude = sourceFolders.size === 1 ? [...sourceFolders][0] : undefined;
  const allFlagged = chosen.length > 0 && chosen.every((m) => m.isFlagged);
  const targets = accountId ? moveTargets(folders, accountId, exclude) : [];
  return (
    <div className="mail-selection" role="toolbar" aria-label="Selected messages">
      <button
        type="button"
        className="icon-btn icon-btn--sm"
        aria-label="Clear selection"
        title="Clear selection (Esc)"
        onClick={clearSelection}
      >
        <X size={15} />
      </button>
      <span
        className="mail-selection__count"
        title={
          formatCount(ids.length) === exactCount(ids.length) ? undefined : exactCount(ids.length)
        }
      >
        {formatCount(ids.length)} selected
      </span>
      <span className="toolbar__spacer" />
      <button
        type="button"
        className="icon-btn"
        aria-label="Mark as read"
        title="Mark as read (Ctrl+Q)"
        onClick={() => actions.markRead(ids, true)}
      >
        <MailOpen size={16} />
      </button>
      <button
        type="button"
        className="icon-btn"
        aria-label="Mark as unread"
        title="Mark as unread (Ctrl+U)"
        onClick={() => actions.markRead(ids, false)}
      >
        <Mail size={16} />
      </button>
      <button
        type="button"
        className="icon-btn"
        aria-label={allFlagged ? "Unflag" : "Flag"}
        title={allFlagged ? "Unflag (Insert)" : "Flag (Insert)"}
        onClick={() => actions.toggleFlag(ids, !allFlagged)}
      >
        <Flag size={16} />
      </button>
      <span className="mail-menu-scroll">
        <Menu
          label="Move to"
          align="end"
          items={targets.map((t) => ({
            label: t.label,
            icon: <FolderIcon role={t.folder.role} />,
            onSelect: () => actions.move(ids, t.folder.id),
          }))}
          trigger={(props) => (
            <button
              type="button"
              className="icon-btn"
              aria-label="Move to"
              title={accountId ? "Move to" : "Select messages from one account to move them"}
              disabled={!accountId || targets.length === 0}
              {...props}
            >
              <FolderInput size={16} />
            </button>
          )}
        />
      </span>
      <button
        type="button"
        className="icon-btn"
        aria-label="Archive"
        title="Archive"
        onClick={() => actions.archive(ids)}
      >
        <Archive size={16} />
      </button>
      <button
        type="button"
        className="icon-btn mail-selection__delete"
        aria-label="Delete"
        title="Delete (Delete)"
        onClick={() => actions.remove(ids)}
      >
        <Trash2 size={16} />
      </button>
    </div>
  );
}

interface MessageListProps {
  actions: MailActions;
  readingPane: ReadingPane;
  onEditDraft: (id: string) => void;
}

export function MessageList({ actions, readingPane, onEditDraft }: MessageListProps) {
  const search = useAppStore((s) => s.search);
  const searchText = useDebounced(search.trim(), 250);
  const searching = search.trim().length > 0;

  const folderId = useMailStore((s) => s.folderId);
  const filter = useMailStore((s) => s.filter);
  const setFilter = useMailStore((s) => s.setFilter);
  const selectedIds = useMailStore((s) => s.selectedIds);
  const activeId = useMailStore((s) => s.activeId);
  const setVisibleIds = useMailStore((s) => s.setVisibleIds);

  const backend = useBackend();
  const { data: folders = [] } = useFolders();
  const { data: accounts = [] } = useAccounts();
  const list = useMessageList(searching ? null : folderId, filter);
  const results = useSearch(searching ? searchText : "");

  const fetched = useMemo(() => {
    // Unread and Flagged narrow search results too; the title says when
    // they hide some, and the empty state when they hide them all.
    if (searching) return (results.data ?? []).filter(matchesFilter(filter));
    const seen = new Set<string>();
    const out: MessageSummary[] = [];
    for (const page of list.data?.pages ?? []) {
      for (const m of page.items) {
        if (!seen.has(m.id)) {
          seen.add(m.id);
          out.push(m);
        }
      }
    }
    return out;
  }, [searching, results.data, filter, list.data]);

  // In the Unread/Flagged views the open message stays listed after it is
  // read or unflagged, until another message is opened, so it doesn't vanish
  // from under the reader.
  const activeFetched = activeId ? (fetched.find((m) => m.id === activeId) ?? null) : null;
  const [sticky, setSticky] = useState<MessageSummary | null>(null);
  if (activeFetched && activeFetched !== sticky) setSticky(activeFetched);
  // Subscribes to the open message's cache entry (filled by the reading pane).
  const { data: activeMessage } = useQuery<Message>({
    queryKey: qk.message(activeId ?? "none", false),
    queryFn: () => backend.getMessage(activeId!, false),
    enabled: false,
  });
  const items = useMemo(() => {
    if (filter === "all" || !sticky || sticky.id !== activeId || activeFetched) return fetched;
    const current =
      activeMessage?.id === sticky.id
        ? {
            ...sticky,
            isRead: activeMessage.isRead,
            isFlagged: activeMessage.isFlagged,
            folderId: activeMessage.folderId,
          }
        : sticky;
    // Moved elsewhere: let it go.
    if (!searching && folderId !== ALL_INBOXES && current.folderId !== folderId) return fetched;
    return insertByDate(fetched, current);
  }, [fetched, filter, sticky, activeId, activeFetched, activeMessage, searching, folderId]);

  // How many messages the list holds, for "3 of 76" in the reading pane: the
  // server's count while more pages wait to load, else what is listed.
  const lastPage = list.data?.pages[list.data.pages.length - 1];
  const listTotal =
    !searching && list.hasNextPage && lastPage
      ? Math.max(items.length, lastPage.total)
      : items.length;
  useEffect(() => {
    setVisibleIds(
      items.map((m) => m.id),
      listTotal,
    );
  }, [items, listTotal, setVisibleIds]);

  const folder = folders.find((f) => f.id === folderId) ?? null;
  const accountById = useMemo(() => {
    const map = new Map<string, Account>();
    for (const a of accounts) map.set(a.id, a);
    return map;
  }, [accounts]);
  const folderById = useMemo(() => {
    const map = new Map<string, Folder>();
    for (const f of folders) map.set(f.id, f);
    return map;
  }, [folders]);

  const groups = useMemo(() => {
    const now = new Date();
    const out: { label: string; items: MessageSummary[] }[] = [];
    for (const m of items) {
      const label = listGroup(m.receivedAt, now);
      const last = out[out.length - 1];
      if (last?.label === label) last.items.push(m);
      else out.push({ label, items: [m] });
    }
    return out;
  }, [items]);

  const selectedSet = useMemo(() => new Set(selectedIds), [selectedIds]);
  const checkMode = selectedIds.length > 1;
  const showRecipients =
    !searching &&
    !!folder &&
    (folder.role === "sent" || folder.role === "drafts" || folder.role === "outbox");

  // --- handlers (stable, so memoized rows don't re-render) ---------------------

  const handlers = useMemo<RowHandlers>(
    () => ({
      onClick: (e: MouseEvent, id: string) => {
        const store = useMailStore.getState();
        if (e.ctrlKey || e.metaKey) store.select(id, "toggle");
        else if (e.shiftKey) store.select(id, "range");
        else {
          store.select(id, "single");
          if (readingPane === "off") store.setReaderOpen(true);
        }
      },
      onDoubleClick: (m: MessageSummary) => {
        if (m.isDraft) onEditDraft(m.id);
        else useMailStore.getState().setReaderOpen(true);
      },
      onToggleCheck: (id: string) => useMailStore.getState().select(id, "toggle"),
      onToggleFlag: (id: string, flagged: boolean) => actions.toggleFlag([id], flagged),
      onToggleRead: (id: string, read: boolean) => actions.markRead([id], read),
      onDelete: (id: string) => actions.remove([id]),
      onDragStart: (e: DragEvent, m: MessageSummary) => {
        const { selectedIds: sel } = useMailStore.getState();
        const ids = sel.includes(m.id) ? sel : [m.id];
        startMailDrag({ ids, accountId: m.accountId, folderId: m.folderId });
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData(MAIL_DRAG_TYPE, JSON.stringify(ids));
        e.dataTransfer.setData("text/plain", m.subject);
        // A compact drag image: "Move 3 items".
        const ghost = document.createElement("div");
        ghost.className = "mail-drag-ghost";
        ghost.textContent =
          ids.length === 1 ? m.subject || "(no subject)" : `${formatCount(ids.length)} items`;
        document.body.appendChild(ghost);
        e.dataTransfer.setDragImage?.(ghost, 12, 12);
        setTimeout(() => ghost.remove(), 0);
      },
      onDragEnd: () => endMailDrag(),
    }),
    [actions, readingPane, onEditDraft],
  );

  // --- keyboard ----------------------------------------------------------------

  const listRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const store = useMailStore.getState();
    const ctrl = e.ctrlKey || e.metaKey;
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp":
        if (ctrl || e.altKey) return;
        e.preventDefault();
        store.moveActive(e.key === "ArrowDown" ? 1 : -1, e.shiftKey);
        break;
      case "PageDown":
      case "PageUp":
        e.preventDefault();
        store.moveActive(e.key === "PageDown" ? 10 : -10, e.shiftKey);
        break;
      case "Home":
      case "End":
        e.preventDefault();
        store.moveActive(e.key === "Home" ? -items.length : items.length, e.shiftKey);
        break;
      case "Enter":
        if (!store.activeId) return;
        e.preventDefault();
        if (items.find((m) => m.id === store.activeId)?.isDraft) onEditDraft(store.activeId);
        else if (readingPane === "off") store.setReaderOpen(true);
        else document.getElementById("mail-reading-pane")?.focus();
        break;
      case " ":
        if (!store.activeId) return;
        e.preventDefault();
        if (ctrl) store.select(store.activeId, "toggle");
        else store.select(store.activeId, "single");
        break;
      case "a":
      case "A":
        if (!ctrl) return;
        e.preventDefault();
        store.selectAll();
        break;
      case "Escape":
        if (store.selectedIds.length > 1) {
          e.preventDefault();
          store.clearSelection();
        }
        break;
    }
  };

  // Keep the active row visible and focused while the list has focus.
  useEffect(() => {
    if (!activeId) return;
    const el = document.getElementById(optionId(activeId));
    if (!el) return;
    el.scrollIntoView?.({ block: "nearest" });
    const focused = document.activeElement;
    const inList = !!focused && (listRef.current?.contains(focused) || focused === document.body);
    if (inList && focused !== el) el.focus({ preventScroll: true });
  }, [activeId, items]);

  // --- infinite scroll ------------------------------------------------------------

  const sentinelRef = useRef<HTMLDivElement>(null);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = list;
  useEffect(() => {
    const el = sentinelRef.current;
    if (!el || searching || !hasNextPage || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting) && !isFetchingNextPage) {
          void fetchNextPage();
        }
      },
      { root: scrollRef.current, rootMargin: "300px" },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [searching, hasNextPage, isFetchingNextPage, fetchNextPage]);

  // --- header ---------------------------------------------------------------------

  const accountName = (id: string) => accountById.get(id)?.displayName ?? "";
  let title = "Mail";
  let titleHint = title;
  let titleAccount: Account | null = null;
  if (searching) {
    title = "Search results";
    titleHint = title;
  } else if (folderId === ALL_INBOXES) {
    title = "All inboxes";
    titleHint = title;
  } else if (folder) {
    title = folder.name;
    titleHint = folderPath(folders, folder);
    titleAccount = accounts.length > 1 ? (accountById.get(folder.accountId) ?? null) : null;
  }
  // Every result, and how many of them the filter leaves listed.
  const resultCount = results.data?.length ?? 0;
  const filtered = filter !== "all" && items.length < resultCount;
  const filterWord = filter === "unread" ? "unread" : "flagged";
  const resultsWord = resultCount === 1 ? "result" : "results";
  const subtitle =
    searching && results.data && items.length > 0 ? (
      <span
        className="mail-list__subtitle"
        title={
          filtered
            ? `${exactCount(items.length)} ${filterWord} of ${exactCount(resultCount)} ${resultsWord}`
            : `${exactCount(items.length)} results`
        }
      >
        {filtered
          ? `${formatCount(items.length)} ${filterWord} of ${formatCount(resultCount)} ${resultsWord}`
          : `${formatCount(items.length)} ${items.length === 1 ? "result" : "results"}`}{" "}
        for “<bdi>{search.trim()}</bdi>”
      </span>
    ) : null;

  const contextFor = (m: MessageSummary): string | null => {
    if (searching) {
      const f = folderById.get(m.folderId)?.name ?? "";
      return accounts.length > 1 ? `${f} · ${accountName(m.accountId)}` : f;
    }
    if (folderId === ALL_INBOXES) return accountName(m.accountId);
    return null;
  };

  // --- body -------------------------------------------------------------------------

  const loading = searching
    ? results.isPending && searchText.length > 0
    : folderId !== null && list.isPending;
  const error = searching ? results.error : list.error;

  let body;
  if (!searching && folderId === null) {
    body = (
      <EmptyState
        icon={<Inbox size={40} strokeWidth={1.5} />}
        title="Select a folder"
        description="Choose a folder on the left to see its messages."
      />
    );
  } else if (loading || (searching && searchText !== search.trim() && !results.data)) {
    body = <PaneSpinner />;
  } else if (error) {
    body = (
      <EmptyState
        title="Couldn't load messages"
        description={errorMessage(error)}
        action={
          <button
            type="button"
            className="btn"
            onClick={() => void (searching ? results.refetch() : list.refetch())}
          >
            Try again
          </button>
        }
      />
    );
  } else if (items.length === 0) {
    body = searching ? (
      resultCount > 0 && filter !== "all" ? (
        // Results the filter hides: say so, rather than "No results".
        <EmptyState
          icon={<SearchX size={40} strokeWidth={1.5} />}
          title={filter === "unread" ? "No unread results" : "No flagged results"}
          description={
            resultCount === 1
              ? `The only result for “${search.trim()}” isn't ${filterWord}.`
              : `None of the ${formatCount(resultCount)} results for “${search.trim()}” are ${filterWord}.`
          }
          action={
            <button type="button" className="btn btn--subtle" onClick={() => setFilter("all")}>
              Show all results
            </button>
          }
        />
      ) : (
        <EmptyState
          icon={<SearchX size={40} strokeWidth={1.5} />}
          title="No results"
          description={`We didn't find anything for “${search.trim()}”. Try different keywords.`}
        />
      )
    ) : filter !== "all" ? (
      <EmptyState
        icon={<Inbox size={40} strokeWidth={1.5} />}
        title={filter === "unread" ? "No unread messages" : "No flagged messages"}
        description={
          <button type="button" className="btn btn--subtle" onClick={() => setFilter("all")}>
            Show all messages
          </button>
        }
      />
    ) : (
      <EmptyState
        icon={<Inbox size={40} strokeWidth={1.5} />}
        title={
          folder?.role === "inbox" || folderId === ALL_INBOXES
            ? "All caught up"
            : "This folder is empty"
        }
        description={
          folder?.role === "inbox" || folderId === ALL_INBOXES
            ? "You have no messages in your inbox."
            : "Messages you move here will show up in this list."
        }
      />
    );
  } else {
    body = (
      <div
        ref={listRef}
        role="listbox"
        aria-label={searching ? "Search results" : `Messages in ${title}`}
        aria-multiselectable="true"
        className="mail-list__items"
        onKeyDown={onKeyDown}
        onFocus={(e) => {
          // Tabbing into the list lands on the active row (or the first).
          if (e.target === e.currentTarget) {
            const store = useMailStore.getState();
            if (!store.activeId) store.moveActive(1);
            else document.getElementById(optionId(store.activeId))?.focus();
          }
        }}
        tabIndex={activeId && items.some((m) => m.id === activeId) ? -1 : 0}
      >
        {groups.map((group) => (
          <div
            key={group.label}
            role="group"
            aria-labelledby={`mail-group-${group.label.replace(/\W+/g, "-")}`}
          >
            <div
              id={`mail-group-${group.label.replace(/\W+/g, "-")}`}
              className="mail-group__header"
              role="presentation"
            >
              {group.label}
            </div>
            {group.items.map((m) => (
              <MessageRow
                key={m.id}
                message={m}
                selected={selectedSet.has(m.id)}
                active={m.id === activeId}
                checkMode={checkMode}
                showRecipients={showRecipients}
                context={contextFor(m)}
                contextColor={
                  searching || folderId === ALL_INBOXES
                    ? (accountById.get(m.accountId)?.color ?? null)
                    : null
                }
                handlers={handlers}
              />
            ))}
          </div>
        ))}
        {!searching && (hasNextPage || isFetchingNextPage) && (
          <div ref={sentinelRef} className="mail-list__more">
            {isFetchingNextPage ? (
              <Spinner size={18} label="Loading more messages" />
            ) : (
              <button
                type="button"
                className="btn btn--subtle btn--sm"
                onClick={() => void fetchNextPage()}
              >
                Load more messages
              </button>
            )}
          </div>
        )}
      </div>
    );
  }

  return (
    <section className="mail-list" aria-label="Message list">
      <header className="mail-list__header">
        <SearchField placeholder="Search mail" />
        {/* The selection's actions take the title row's place, so the list
            doesn't jump under the pointer while messages are picked. */}
        <div className={`mail-list__bar${checkMode ? " is-selecting" : ""}`}>
          <div className="mail-list__title-row">
            <div className="mail-list__titles">
              <h2 className="pane-title mail-list__title" dir="auto" title={titleHint}>
                {title}
              </h2>
              {subtitle}
            </div>
            {titleAccount && (
              <span
                className="mail-chip-account"
                title={`${titleAccount.displayName} (${titleAccount.email})`}
              >
                <span
                  className="mail-chip-account__dot"
                  style={{ background: titleAccount.color }}
                  aria-hidden="true"
                />
                <span className="mail-chip-account__name" dir="auto">
                  {titleAccount.displayName}
                </span>
              </span>
            )}
            <div className="segmented mail-filter" role="group" aria-label="Filter messages">
              {FILTERS.map((f) => (
                <button
                  key={f.value}
                  type="button"
                  className={`segmented__option mail-filter__option${
                    filter === f.value ? " segmented__option--selected" : ""
                  }`}
                  aria-pressed={filter === f.value}
                  onClick={() => setFilter(f.value)}
                >
                  {f.label}
                </button>
              ))}
            </div>
          </div>
          {checkMode && (
            <SelectionBar ids={selectedIds} actions={actions} folders={folders} messages={items} />
          )}
        </div>
      </header>
      <div ref={scrollRef} className="mail-list__scroll">
        {body}
      </div>
    </section>
  );
}
