import { useSettings } from "../../../api/queries";
import { PaneSpinner } from "../../../components/Spinner";
import { Switch } from "../../../components/Switch";
import { useSaveSettings } from "../saveStatus";
import { SettingRow, SettingsGroup } from "../SettingsLayout";
import { AppearanceGroup } from "./AppearanceGroup";

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
      <AppearanceGroup settings={settings} save={save} />

      <SettingsGroup title="Notifications">
        <div className="setting-row">
          <Switch
            label="New message notifications"
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
