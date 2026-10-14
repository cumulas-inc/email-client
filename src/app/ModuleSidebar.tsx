import type { ReactNode } from "react";

import { ModuleSwitcher } from "./ModuleSwitcher";
import { SyncStatus } from "./SyncStatus";

interface ModuleSidebarProps {
  /** Accessible name, e.g. "Mail folders". */
  label: string;
  /** The module's main action, pinned above the scrolling content. */
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}

/** The left card every module shares: its main action, its own navigation,
    and the sync status and app switcher at the foot. */
export function ModuleSidebar({ label, action, children, className }: ModuleSidebarProps) {
  return (
    <aside className={`module-sidebar card${className ? ` ${className}` : ""}`} aria-label={label}>
      {action && <div className="module-sidebar__action">{action}</div>}
      <div className="module-sidebar__body">{children}</div>
      <div className="module-sidebar__foot">
        <SyncStatus />
        <ModuleSwitcher />
      </div>
    </aside>
  );
}
