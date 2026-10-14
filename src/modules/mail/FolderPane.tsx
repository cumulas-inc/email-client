import { ChevronDown, ChevronRight } from "lucide-react";
import {
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useMemo,
  useRef,
  useState,
} from "react";

import { useAppStore } from "../../app/store";
import { useAccounts } from "../../api/queries";
import type { Account, Folder } from "../../api/types";
import { AccountBadge } from "../../components/AccountBadge";
import type { MailActions } from "./actions";
import { currentMailDrag, endMailDrag } from "./dnd";
import { FolderIcon } from "./FolderIcon";
import { ALL_INBOXES, useMailStore } from "./mailStore";
import { useFolders } from "./queries";

/** Escapes a value for use inside a quoted attribute selector. */
function quoteAttr(value: string): string {
  return value.replace(/["\\]/g, "\\$&");
}

/** The number shown next to a folder: drafts count all items, others unread. */
function folderCount(folder: Folder): number {
  return folder.role === "drafts" ? folder.totalCount : folder.unreadCount;
}

function countLabel(folder: Folder): string {
  const n = folderCount(folder);
  if (n === 0) return "";
  if (folder.role === "drafts") return n === 1 ? ", 1 item" : `, ${n} items`;
  return `, ${n} unread`;
}

interface ItemProps {
  nodeKey: string;
  parentKey: string | null;
  level: number;
  label: string;
  /** Shown before the label, quieter: "Work ·" in "Work · Inbox". */
  prefix?: string;
  ariaLabel: string;
  icon: ReactNode;
  count?: number;
  countMuted?: boolean;
  secondary?: string;
  title?: string;
  selected?: boolean;
  expandable?: boolean;
  expanded?: boolean;
  focused: boolean;
  dropFolder?: Folder;
  /** "section": a small label (Favorites); "account": badge, name and address. */
  heading?: "section" | "account";
  onActivate: () => void;
  onToggle?: () => void;
  onFocusKey: (key: string) => void;
  onDropMessages?: (folder: Folder) => void;
  children?: ReactNode;
}

function TreeItem(props: ItemProps) {
  const {
    nodeKey,
    parentKey,
    level,
    label,
    prefix,
    ariaLabel,
    icon,
    count,
    countMuted,
    secondary,
    title,
    selected,
    expandable,
    expanded,
    focused,
    dropFolder,
    heading,
    onActivate,
    onToggle,
    onFocusKey,
    onDropMessages,
    children,
  } = props;
  const [dropping, setDropping] = useState(false);

  const canDrop = () => {
    const drag = currentMailDrag();
    return (
      !!dropFolder &&
      !!drag &&
      drag.accountId === dropFolder.accountId &&
      drag.ids.length > 0 &&
      drag.folderId !== dropFolder.id
    );
  };

  const dropHandlers = dropFolder
    ? {
        onDragEnter: (e: DragEvent) => {
          if (canDrop()) {
            e.preventDefault();
            setDropping(true);
          }
        },
        onDragOver: (e: DragEvent) => {
          if (canDrop()) {
            e.preventDefault();
            e.dataTransfer.dropEffect = "move";
          }
        },
        onDragLeave: (e: DragEvent) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropping(false);
        },
        onDrop: (e: DragEvent) => {
          setDropping(false);
          if (!canDrop()) return;
          e.preventDefault();
          onDropMessages?.(dropFolder);
          endMailDrag();
        },
      }
    : {};

  return (
    <li
      role="treeitem"
      aria-label={ariaLabel}
      aria-level={level}
      aria-selected={heading ? undefined : !!selected}
      aria-expanded={expandable ? !!expanded : undefined}
      tabIndex={focused ? 0 : -1}
      data-key={nodeKey}
      data-parent={parentKey ?? ""}
      data-expandable={expandable ? "true" : undefined}
      data-expanded={expanded ? "true" : undefined}
      className="mail-tree__item"
      onFocus={(e) => {
        if (e.target === e.currentTarget) onFocusKey(nodeKey);
      }}
    >
      <div
        className={[
          "mail-tree__row",
          heading && `mail-tree__row--${heading}`,
          selected && "is-selected",
          dropping && "is-drop-target",
        ]
          .filter(Boolean)
          .join(" ")}
        style={heading ? undefined : ({ "--depth": level - 2 } as CSSProperties)}
        title={title}
        onClick={(e) => {
          e.stopPropagation();
          onFocusKey(nodeKey);
          onActivate();
        }}
        {...dropHandlers}
      >
        {icon && <span className="mail-tree__icon">{icon}</span>}
        <span className="mail-tree__label">
          {prefix && <span className="mail-tree__prefix">{prefix} · </span>}
          {label}
        </span>
        {secondary && <span className="mail-tree__secondary">{secondary}</span>}
        {!!count && (
          <span className={`mail-tree__count${countMuted ? " mail-tree__count--muted" : ""}`}>
            {count}
          </span>
        )}
        {expandable && (
          <span
            className={`mail-tree__chevron${expanded ? "" : " is-collapsed"}`}
            onClick={(e) => {
              e.stopPropagation();
              onToggle?.();
            }}
            aria-hidden="true"
          >
            {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </span>
        )}
      </div>
      {expandable && expanded && children && <ul role="group">{children}</ul>}
    </li>
  );
}

interface FolderPaneProps {
  actions: MailActions;
}

export function FolderPane({ actions }: FolderPaneProps) {
  const { data: folders = [] } = useFolders();
  const { data: accounts = [] } = useAccounts();
  const folderId = useMailStore((s) => s.folderId);
  const selectFolder = useMailStore((s) => s.selectFolder);
  const collapsed = useMailStore((s) => s.collapsed);
  const setCollapsed = useMailStore((s) => s.toggleCollapsed);
  const setSearch = useAppStore((s) => s.setSearch);
  const treeRef = useRef<HTMLUListElement>(null);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  // An inbox appears twice (Favorites and its account): highlight the one clicked.
  const [clickedKey, setClickedKey] = useState<string | null>(null);

  const childrenOf = useMemo(() => {
    const map = new Map<string, Folder[]>();
    for (const f of folders) {
      // Orphans (parent not synced) are shown at the top level.
      const parent = f.parentId && folders.some((p) => p.id === f.parentId) ? f.parentId : null;
      const key = `${f.accountId}:${parent ?? ""}`;
      map.set(key, [...(map.get(key) ?? []), f]);
    }
    return map;
  }, [folders]);

  const inboxes = useMemo(() => folders.filter((f) => f.role === "inbox"), [folders]);
  const accountName = useCallback(
    (id: string) => accounts.find((a) => a.id === id)?.displayName ?? "",
    [accounts],
  );

  const open = (id: string, key: string) => {
    setClickedKey(key);
    setSearch("");
    selectFolder(id);
  };

  const onDropMessages = (folder: Folder) => {
    const drag = currentMailDrag();
    if (drag) actions.move(drag.ids, folder.id);
  };

  const defaultKey = folderId
    ? folderId === ALL_INBOXES || inboxes.some((f) => f.id === folderId)
      ? `fav:${folderId}`
      : `folder:${folderId}`
    : null;
  const selectedKey =
    clickedKey && folderId && clickedKey.endsWith(`:${folderId}`) ? clickedKey : defaultKey;
  const activeKey = focusKey ?? selectedKey ?? "favorites";

  const common = { onFocusKey: setFocusKey, onDropMessages };

  const renderFolders = (account: Account, parentId: string | null, level: number): ReactNode =>
    (childrenOf.get(`${account.id}:${parentId ?? ""}`) ?? []).map((f) => {
      const key = `folder:${f.id}`;
      const kids = childrenOf.get(`${account.id}:${f.id}`) ?? [];
      const expanded = !collapsed[key];
      return (
        <TreeItem
          key={f.id}
          {...common}
          nodeKey={key}
          parentKey={parentId ? `folder:${parentId}` : `account:${account.id}`}
          level={level}
          label={f.name}
          ariaLabel={`${f.name}${countLabel(f)}`}
          icon={<FolderIcon role={f.role} size={17} open={kids.length > 0 && expanded} />}
          count={folderCount(f)}
          countMuted={f.role === "drafts"}
          selected={selectedKey === key}
          expandable={kids.length > 0}
          expanded={expanded}
          focused={activeKey === key}
          dropFolder={f}
          onActivate={() => open(f.id, key)}
          onToggle={() => toggleCollapsed(key)}
        >
          {renderFolders(account, f.id, level + 1)}
        </TreeItem>
      );
    });

  /** Collapses or expands a node, keeping keyboard focus out of hidden children. */
  const toggleCollapsed = (key: string, value?: boolean) => {
    const node = treeRef.current?.querySelector<HTMLElement>(`[data-key="${quoteAttr(key)}"]`);
    const focused = focusKey
      ? treeRef.current?.querySelector(`[data-key="${quoteAttr(focusKey)}"]`)
      : null;
    if (node && focused && focused !== node && node.contains(focused)) setFocusKey(key);
    setCollapsed(key, value);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLUListElement>) => {
    const tree = treeRef.current;
    const current = (e.target as HTMLElement).closest<HTMLElement>("[role=treeitem]");
    if (!tree || !current) return;
    const items = Array.from(tree.querySelectorAll<HTMLElement>("[role=treeitem]"));
    const index = items.indexOf(current);
    const focus = (el: HTMLElement | undefined) => {
      if (!el) return;
      setFocusKey(el.dataset.key ?? null);
      el.focus();
    };
    const key = current.dataset.key ?? "";
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        focus(items[index + 1]);
        break;
      case "ArrowUp":
        e.preventDefault();
        focus(items[index - 1]);
        break;
      case "Home":
        e.preventDefault();
        focus(items[0]);
        break;
      case "End":
        e.preventDefault();
        focus(items[items.length - 1]);
        break;
      case "ArrowRight":
        e.preventDefault();
        if (current.dataset.expandable && !current.dataset.expanded) toggleCollapsed(key, false);
        else if (current.dataset.expanded) focus(items[index + 1]);
        break;
      case "ArrowLeft": {
        e.preventDefault();
        if (current.dataset.expanded) toggleCollapsed(key, true);
        else focus(items.find((el) => el.dataset.key === current.dataset.parent));
        break;
      }
      case "Enter":
      case " ":
        e.preventDefault();
        (current.querySelector(".mail-tree__row") as HTMLElement | null)?.click();
        break;
    }
  };

  const totalUnifiedUnread = inboxes.reduce((sum, f) => sum + f.unreadCount, 0);
  const favExpanded = !collapsed.favorites;

  return (
    <div className="mail-folders">
      <ul
        ref={treeRef}
        role="tree"
        aria-label="Folders"
        className="mail-tree"
        onKeyDown={onKeyDown}
      >
        <TreeItem
          {...common}
          nodeKey="favorites"
          parentKey={null}
          level={1}
          label="Favorites"
          ariaLabel="Favorites"
          icon={null}
          heading="section"
          expandable
          expanded={favExpanded}
          focused={activeKey === "favorites"}
          onActivate={() => toggleCollapsed("favorites")}
          onToggle={() => toggleCollapsed("favorites")}
        >
          {accounts.length > 1 && (
            <TreeItem
              {...common}
              nodeKey={`fav:${ALL_INBOXES}`}
              parentKey="favorites"
              level={2}
              label="All inboxes"
              ariaLabel={`All inboxes${totalUnifiedUnread ? `, ${totalUnifiedUnread} unread` : ""}`}
              icon={<FolderIcon role="unified" size={17} />}
              count={totalUnifiedUnread}
              selected={selectedKey === `fav:${ALL_INBOXES}`}
              focused={activeKey === `fav:${ALL_INBOXES}`}
              onActivate={() => open(ALL_INBOXES, `fav:${ALL_INBOXES}`)}
            />
          )}
          {inboxes.map((f) => {
            const key = `fav:${f.id}`;
            const name = accountName(f.accountId);
            return (
              <TreeItem
                key={f.id}
                {...common}
                nodeKey={key}
                parentKey="favorites"
                level={2}
                label={f.name}
                prefix={accounts.length > 1 ? name : undefined}
                ariaLabel={`${f.name}${accounts.length > 1 ? ` (${name})` : ""}${countLabel(f)}`}
                icon={<FolderIcon role="inbox" size={17} />}
                count={folderCount(f)}
                selected={selectedKey === key}
                focused={activeKey === key}
                dropFolder={f}
                onActivate={() => open(f.id, key)}
              />
            );
          })}
        </TreeItem>
        {accounts.map((account) => {
          const key = `account:${account.id}`;
          const expanded = !collapsed[key];
          return (
            <TreeItem
              key={account.id}
              {...common}
              nodeKey={key}
              parentKey={null}
              level={1}
              label={account.displayName}
              secondary={account.email}
              title={`${account.displayName} (${account.email})`}
              ariaLabel={`${account.displayName} (${account.email})`}
              icon={<AccountBadge name={account.displayName} color={account.color} />}
              heading="account"
              expandable
              expanded={expanded}
              focused={activeKey === key}
              onActivate={() => toggleCollapsed(key)}
              onToggle={() => toggleCollapsed(key)}
            >
              {renderFolders(account, null, 2)}
            </TreeItem>
          );
        })}
      </ul>
    </div>
  );
}
