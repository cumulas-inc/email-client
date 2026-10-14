import { clsx } from "clsx";
import { List } from "lucide-react";
import { useEffect, useId, type KeyboardEvent, type ReactNode } from "react";

import type { Account, TaskList } from "../../api/types";
import { listboxTarget, scrollIdIntoView } from "../../components/listboxKeys";
import { smartListIcon } from "./smartIcons";
import { SMART_LISTS, type TaskView } from "./taskUtils";

interface TaskSidebarProps {
  accounts: Account[];
  /** Task lists in display order. */
  lists: TaskList[];
  view: TaskView;
  onSelect: (view: TaskView) => void;
  /** Open tasks per smart list id and per list id. */
  counts: Map<string, number>;
}

function viewKey(view: TaskView): string {
  return `${view.kind}:${view.id}`;
}

/** Smart lists, then each account's lists, as one listbox. */
export function TaskSidebar({ accounts, lists, view, onSelect, counts }: TaskSidebarProps) {
  const baseId = useId();
  const groups = accounts
    .map((account) => ({ account, lists: lists.filter((l) => l.accountId === account.id) }))
    .filter((g) => g.lists.length > 0);
  const options: TaskView[] = [
    ...SMART_LISTS.map((s) => ({ kind: "smart" as const, id: s.id })),
    ...groups.flatMap((g) => g.lists.map((l) => ({ kind: "list" as const, id: l.id }))),
  ];
  const optionId = (v: TaskView) => `${baseId}-${viewKey(v)}`;
  const index = options.findIndex((o) => viewKey(o) === viewKey(view));
  const activeId = index >= 0 ? optionId(view) : undefined;

  useEffect(() => {
    scrollIdIntoView(activeId);
  }, [activeId]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const target = listboxTarget(e.key, index, options.length);
    if (target !== null) {
      e.preventDefault();
      onSelect(options[target]!);
    }
  };

  const option = (v: TaskView, icon: ReactNode, label: string) => {
    const selected = viewKey(v) === viewKey(view);
    const count = counts.get(v.id) ?? 0;
    return (
      <div
        key={viewKey(v)}
        id={optionId(v)}
        role="option"
        aria-selected={selected}
        aria-label={count > 0 ? `${label}, ${count} open` : label}
        className={clsx("tasks-nav__item", v.kind === "smart" && `tasks-nav__item--${v.id}`)}
        onClick={() => onSelect(v)}
      >
        <span className="tasks-nav__icon" aria-hidden="true">
          {icon}
        </span>
        <span className="tasks-nav__label">{label}</span>
        {count > 0 && <span className="tasks-nav__count">{count}</span>}
      </div>
    );
  };

  return (
    <nav className="tasks-sidebar" aria-label="Task lists">
      <h1 className="tasks-sidebar__title">Tasks</h1>
      <div
        role="listbox"
        aria-label="Lists"
        aria-activedescendant={activeId}
        tabIndex={0}
        className="tasks-nav"
        onKeyDown={onKeyDown}
      >
        {SMART_LISTS.map((s) => option({ kind: "smart", id: s.id }, smartListIcon(s.id), s.label))}
        {groups.map(({ account, lists: accountLists }) => {
          const headerId = `${baseId}-account-${account.id}`;
          return (
            <div
              key={account.id}
              role="group"
              aria-labelledby={headerId}
              className="tasks-nav__group"
            >
              <div id={headerId} className="tasks-nav__account" title={account.email}>
                <span
                  className="tasks-nav__account-dot"
                  style={{ background: account.color }}
                  aria-hidden="true"
                />
                <span className="tasks-nav__account-name">{account.displayName}</span>
                <span className="tasks-nav__account-email">{account.email}</span>
              </div>
              {accountLists.map((l) =>
                option({ kind: "list", id: l.id }, <List size={18} />, l.name),
              )}
            </div>
          );
        })}
      </div>
    </nav>
  );
}
