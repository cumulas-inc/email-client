import { Info, Menu as MenuIcon, Power, Settings } from "lucide-react";

import { useAccounts } from "../api/queries";
import { Avatar } from "../components/Avatar";
import { Menu } from "../components/Menu";
import { LogoMark } from "./LogoMark";
import { useQuit } from "./useQuit";
import { useAppStore } from "./store";

/** The quiet strip above the cards: the name on the left, the main menu and
    the signed-in person on the right. */
export function AppHeader() {
  const openSettings = useAppStore((s) => s.openSettings);
  const quit = useQuit();
  const { data: accounts = [] } = useAccounts();
  const me = accounts[0];

  return (
    <header className="app-header">
      <div className="app-header__brand">
        <LogoMark />
        <span className="app-header__name">
          Cumulas<span className="app-header__product">Mail</span>
        </span>
      </div>
      <div className="app-header__spacer" />
      <Menu
        label="Main menu"
        align="end"
        trigger={(props) => (
          <button
            {...props}
            type="button"
            className="icon-btn"
            title="Main menu"
            aria-label="Main menu"
          >
            <MenuIcon size={18} />
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
          {
            label: "Quit",
            icon: <Power size={16} />,
            shortcut: "Ctrl+Q",
            separator: true,
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
