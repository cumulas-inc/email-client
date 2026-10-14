import { CalendarDays, CircleCheckBig, Mail, Settings, Users } from "lucide-react";
import type { ReactNode } from "react";

import { type Module, useAppStore } from "./store";

const ITEMS: { module: Module; label: string; icon: ReactNode; shortcut: string }[] = [
  { module: "mail", label: "Mail", icon: <Mail size={20} />, shortcut: "Ctrl+1" },
  { module: "calendar", label: "Calendar", icon: <CalendarDays size={20} />, shortcut: "Ctrl+2" },
  { module: "people", label: "Contacts", icon: <Users size={20} />, shortcut: "Ctrl+3" },
  { module: "tasks", label: "Tasks", icon: <CircleCheckBig size={20} />, shortcut: "Ctrl+4" },
];

export function NavRail() {
  const { module, setModule, openSettings } = useAppStore();
  return (
    <nav className="nav-rail" aria-label="Apps">
      {ITEMS.map((item) => (
        <button
          key={item.module}
          type="button"
          className="nav-rail__item"
          aria-current={module === item.module ? "page" : undefined}
          title={`${item.label} (${item.shortcut})`}
          onClick={() => setModule(item.module)}
        >
          {item.icon}
          <span className="nav-rail__label">{item.label}</span>
        </button>
      ))}
      <div className="nav-rail__spacer" />
      <button
        type="button"
        className="nav-rail__item"
        title="Settings"
        onClick={() => openSettings()}
      >
        <Settings size={20} />
        <span className="visually-hidden">Settings</span>
      </button>
    </nav>
  );
}
