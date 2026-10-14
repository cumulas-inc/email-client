import { AppWindow, Info, Mail, SlidersHorizontal, UserRound } from "lucide-react";
import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { type SettingsSection, useAppStore } from "../../app/store";
import type { Account } from "../../api/types";
import { Dialog } from "../../components/Dialog";
import { focusLater, useFocusOnChange, useRestoreFocus } from "./focus";
import { RemoveAccountDialog } from "./RemoveAccountDialog";
import { resetSaveStatus } from "./saveStatus";
import { AboutSection } from "./sections/AboutSection";
import { ADD_ACCOUNT_BUTTON_ID } from "./constants";
import { AccountsSection } from "./sections/AccountsSection";
import { GeneralSection } from "./sections/GeneralSection";
import { MailSection } from "./sections/MailSection";
import { MicrosoftSection } from "./sections/MicrosoftSection";
import { SaveIndicator } from "./SettingsLayout";

import "./settings.css";

const ICON = { size: 18, strokeWidth: 1.75 };

const SECTIONS: { id: SettingsSection; label: string; icon: ReactNode }[] = [
  { id: "accounts", label: "Accounts", icon: <UserRound {...ICON} /> },
  { id: "general", label: "General", icon: <SlidersHorizontal {...ICON} /> },
  { id: "mail", label: "Mail", icon: <Mail {...ICON} /> },
  { id: "microsoft", label: "Microsoft app registration", icon: <AppWindow {...ICON} /> },
  { id: "about", label: "About", icon: <Info {...ICON} /> },
];

/** Settings, shown while `settingsOpen` names a section. Changes save as you make them. */
export function SettingsDialog() {
  const section = useAppStore((s) => s.settingsOpen);
  const closeSettings = useAppStore((s) => s.closeSettings);
  const [removing, setRemoving] = useState<Account | null>(null);

  return (
    <>
      <Dialog open={section !== null} onClose={closeSettings} title="Settings" width={880}>
        {section && <SettingsBody section={section} onRemoveAccount={setRemoving} />}
      </Dialog>
      {/* A sibling, not a child, so Escape in it doesn't also close Settings. */}
      <RemoveAccountDialog
        account={removing}
        onClose={() => setRemoving(null)}
        onRemoved={() => focusLater(ADD_ACCOUNT_BUTTON_ID)}
      />
    </>
  );
}

function SettingsBody({
  section,
  onRemoveAccount,
}: {
  section: SettingsSection;
  onRemoveAccount: (account: Account) => void;
}) {
  const openSettings = useAppStore((s) => s.openSettings);
  const id = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const tabId = (s: SettingsSection) => `${id}-tab-${s}`;
  const panelId = `${id}-panel`;
  const current = SECTIONS.find((s) => s.id === section) ?? SECTIONS[0]!;

  useEffect(() => resetSaveStatus(), []);
  useRestoreFocus();
  // Start on the selected section in the navigation.
  useFocusOnChange(rootRef, "open");

  const onNavKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const index = SECTIONS.findIndex((s) => s.id === section);
    let next: number | null = null;
    if (e.key === "ArrowDown") next = (index + 1) % SECTIONS.length;
    else if (e.key === "ArrowUp") next = (index - 1 + SECTIONS.length) % SECTIONS.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = SECTIONS.length - 1;
    if (next === null) return;
    e.preventDefault();
    const target = SECTIONS[next]!.id;
    openSettings(target);
    focusLater(tabId(target));
  };

  return (
    <div className="settings" ref={rootRef}>
      <div
        role="tablist"
        aria-orientation="vertical"
        aria-label="Settings sections"
        className="settings__nav"
        onKeyDown={onNavKey}
      >
        {SECTIONS.map((s) => {
          const selected = s.id === section;
          return (
            <button
              key={s.id}
              id={tabId(s.id)}
              type="button"
              role="tab"
              className="settings__tab"
              aria-selected={selected}
              aria-controls={panelId}
              tabIndex={selected ? 0 : -1}
              data-autofocus={selected ? true : undefined}
              onClick={() => openSettings(s.id)}
            >
              <span className="settings__tab-icon" aria-hidden="true">
                {s.icon}
              </span>
              <span className="settings__tab-label">{s.label}</span>
            </button>
          );
        })}
      </div>
      <div
        id={panelId}
        role="tabpanel"
        aria-labelledby={tabId(current.id)}
        className="settings__panel"
      >
        <div className="settings__header">
          <h3 className="settings__title pane-title">{current.label}</h3>
          <SaveIndicator />
        </div>
        <div className="settings__content">
          {section === "accounts" && <AccountsSection onRemove={onRemoveAccount} />}
          {section === "general" && <GeneralSection />}
          {section === "mail" && <MailSection />}
          {section === "microsoft" && <MicrosoftSection />}
          {section === "about" && <AboutSection />}
        </div>
      </div>
    </div>
  );
}
