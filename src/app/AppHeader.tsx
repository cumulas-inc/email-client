import { CircleArrowUp, Info, Menu as MenuIcon, Power, Settings } from "lucide-react";

import { useAccounts, useUpdateState } from "../api/queries";
import { Avatar } from "../components/Avatar";
import { Menu, type MenuItem } from "../components/Menu";
import { LogoMark } from "./LogoMark";
import { RemindersButton } from "./Reminders";
import { useQuit, useRestart } from "./useQuit";
import { useAppStore } from "./store";

/** The quiet strip above the cards: the name on the left; the reminders, the
    main menu and the signed-in person on the right. */
export function AppHeader() {
  const openSettings = useAppStore((s) => s.openSettings);
  const quit = useQuit();
  const restart = useRestart();
  const { data: accounts = [] } = useAccounts();
  const { data: update } = useUpdateState();
  const me = accounts[0];
  const installed = update?.kind === "installed" ? update : null;
  const restarting = installed?.restart === "restarting";
  // Right above Quit, the other way of leaving: Settings stays first, where
  // the menu puts the focus.
  const restartItem: MenuItem[] = installed
    ? [
        {
          label: restarting ? "Restarting…" : "Restart to update",
          icon: <CircleArrowUp size={16} />,
          separator: true,
          disabled: restarting,
          onSelect: restart,
        },
      ]
    : [];

  return (
    <header className="app-header">
      <div className="app-header__brand">
        <LogoMark />
        <span className="app-header__name">
          Cumulas<span className="app-header__product">Mail</span>
        </span>
      </div>
      <div className="app-header__spacer" />
      {me && <RemindersButton />}
      <Menu
        label="Main menu"
        align="end"
        trigger={(props) => (
          <button
            {...props}
            type="button"
            className="icon-btn app-header__menu"
            title="Main menu"
            aria-label={installed ? "Main menu, update installed" : "Main menu"}
          >
            <MenuIcon size={18} />
            {installed && <span className="app-header__menu-dot" aria-hidden="true" />}
          </button>
        )}
        items={[
          {
            label: "Settings",
            icon: <Settings size={16} />,
            shortcut: "Ctrl+,",
            onSelect: () => openSettings(),
          },
          {
            label: "About Cumulas Mail",
            icon: <Info size={16} />,
            onSelect: () => openSettings("about"),
          },
          ...restartItem,
          {
            label: "Quit",
            icon: <Power size={16} />,
            separator: !installed,
            onSelect: quit,
          },
        ]}
      />
      {me && (
        <button
          type="button"
          className="app-header__me"
          title={`Manage accounts (${me.displayName || me.email})`}
          onClick={() => openSettings("accounts")}
        >
          <Avatar name={me.displayName || me.email} color={me.color} size={26} />
          <span className="app-header__me-name" dir="auto">
            {me.displayName || me.email}
          </span>
        </button>
      )}
    </header>
  );
}
