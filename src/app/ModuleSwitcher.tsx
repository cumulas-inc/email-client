import { CalendarDays, CircleCheckBig, Mail, Users } from "lucide-react";
import type { ReactNode } from "react";

import { type Module, useAppStore } from "./store";

const ITEMS: { module: Module; label: string; icon: ReactNode; shortcut: string }[] = [
  { module: "mail", label: "Mail", icon: <Mail size={18} />, shortcut: "Ctrl+1" },
  { module: "calendar", label: "Calendar", icon: <CalendarDays size={18} />, shortcut: "Ctrl+2" },
  { module: "people", label: "Contacts", icon: <Users size={18} />, shortcut: "Ctrl+3" },
  { module: "tasks", label: "Tasks", icon: <CircleCheckBig size={18} />, shortcut: "Ctrl+4" },
];

/** Mail / Calendar / Contacts / Tasks, at the foot of every sidebar. */
export function ModuleSwitcher() {
  const module = useAppStore((s) => s.module);
  const setModule = useAppStore((s) => s.setModule);
  return (
    <nav className="module-switcher" aria-label="Apps">
      {ITEMS.map((item) => (
        <button
          key={item.module}
          type="button"
          className="module-switcher__item"
          aria-current={module === item.module ? "page" : undefined}
          title={`${item.label} (${item.shortcut})`}
          onClick={() => setModule(item.module)}
        >
          {item.icon}
          <span>{item.label}</span>
        </button>
      ))}
    </nav>
  );
}
