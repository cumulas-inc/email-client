import { Settings } from "lucide-react";

import { useAccounts } from "../api/queries";
import { Avatar } from "../components/Avatar";
import { LogoMark } from "./LogoMark";
import { useAppStore } from "./store";

/** The quiet strip above the cards: the name on the left, settings and the
    signed-in person on the right. */
export function AppHeader() {
  const openSettings = useAppStore((s) => s.openSettings);
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
      {me && (
        <>
          <button
            type="button"
            className="icon-btn"
            title="Settings"
            aria-label="Settings"
            onClick={() => openSettings()}
          >
            <Settings size={18} />
          </button>
          <button
            type="button"
            className="app-header__me"
            title={`Manage accounts (${me.displayName || me.email})`}
            onClick={() => openSettings("accounts")}
          >
            <Avatar name={me.displayName || me.email} colorKey={me.email} size={26} />
            <span className="app-header__me-name" dir="auto">
              {me.displayName || me.email}
            </span>
          </button>
        </>
      )}
    </header>
  );
}
