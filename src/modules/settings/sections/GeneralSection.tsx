import { Monitor, Moon, Sun } from "lucide-react";

import { useSettings } from "../../../api/queries";
import type { Theme } from "../../../api/types";
import { SegmentedControl, type SegmentedOption } from "../../../components/SegmentedControl";
import { PaneSpinner } from "../../../components/Spinner";
import { Switch } from "../../../components/Switch";
import { useSaveSettings } from "../saveStatus";
import { SettingRow, SettingsGroup } from "../SettingsLayout";

const THEMES: SegmentedOption<Theme>[] = [
  { value: "system", label: "System", icon: <Monitor size={20} /> },
  { value: "light", label: "Light", icon: <Sun size={20} /> },
  { value: "dark", label: "Dark", icon: <Moon size={20} /> },
];

const SYNC_INTERVALS = [1, 5, 15, 30, 60];

function intervalLabel(minutes: number): string {
  if (minutes === 1) return "Every minute";
  if (minutes === 60) return "Every hour";
  return `Every ${minutes} minutes`;
}

export function GeneralSection() {
  const { data: settings } = useSettings();
  const save = useSaveSettings();
  if (!settings) return <PaneSpinner />;

  const intervals = SYNC_INTERVALS.includes(settings.syncIntervalMinutes)
    ? SYNC_INTERVALS
    : [...SYNC_INTERVALS, settings.syncIntervalMinutes].sort((a, b) => a - b);

  return (
    <>
      <SettingsGroup title="Appearance">
        <SettingRow
          label="Theme"
          labelFor={false}
          description="System follows your desktop's light or dark style."
        >
          {({ labelId, descriptionId }) => (
            <SegmentedControl
              variant="tiles"
              labelledBy={labelId}
              describedBy={descriptionId}
              value={settings.theme}
              options={THEMES}
              onChange={(theme) => void save({ theme })}
            />
          )}
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Notifications">
        <div className="setting-row">
          <Switch
            label="New mail notifications"
            description="Show a desktop notification when new messages arrive."
            checked={settings.notificationsEnabled}
            onChange={(notificationsEnabled) => void save({ notificationsEnabled })}
          />
        </div>
      </SettingsGroup>

      <SettingsGroup title="Sync">
        <SettingRow
          label="Check for new mail"
          description="How often to look for new mail and other changes."
        >
          {({ controlId, descriptionId }) => (
            <select
              id={controlId}
              className="select setting-select"
              aria-describedby={descriptionId}
              value={settings.syncIntervalMinutes}
              onChange={(e) => void save({ syncIntervalMinutes: Number(e.target.value) })}
            >
              {intervals.map((m) => (
                <option key={m} value={m}>
                  {intervalLabel(m)}
                </option>
              ))}
            </select>
          )}
        </SettingRow>
      </SettingsGroup>
    </>
  );
}
