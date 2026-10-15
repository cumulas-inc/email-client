import { format, isToday, isTomorrow } from "date-fns";
import { BellRing, ShieldCheck, TriangleAlert } from "lucide-react";
import { useState } from "react";

import { useBackend } from "../../../api/context";
import { errorMessage } from "../../../api/errors";
import {
  useAppInfo,
  useBackgroundState,
  useSetRunInBackground,
  useSetStartAtLogin,
  useSettings,
} from "../../../api/queries";
import type { BackgroundState, Settings } from "../../../api/types";
import { PaneSpinner } from "../../../components/Spinner";
import { Switch } from "../../../components/Switch";
import { toast } from "../../../components/toast";
import { PRIVACY_URL } from "../constants";
import { trackSave, useSaveSettings } from "../saveStatus";
import { SettingRow, SettingsGroup } from "../SettingsLayout";
import { AppearanceGroup } from "./AppearanceGroup";

const SYNC_INTERVALS = [1, 5, 15, 30, 60];

function intervalLabel(minutes: number): string {
  if (minutes === 1) return "Every minute";
  if (minutes === 60) return "Every hour";
  return `Every ${minutes} minutes`;
}

/** "3:45 PM", "tomorrow at 8:00 AM" or "Oct 18 at 8:00 AM". */
function untilLabel(iso: string): string {
  const date = new Date(iso);
  const time = format(date, "h:mm a");
  if (isToday(date)) return time;
  if (isTomorrow(date)) return `tomorrow at ${time}`;
  return `${format(date, "MMM d")} at ${time}`;
}

/** When paused notifications resume, if they're paused. */
function pausedUntil(settings: Settings): string | null {
  const until = settings.notificationsPausedUntil;
  return until !== null && new Date(until).getTime() > Date.now() ? until : null;
}

type Save = (patch: Partial<Settings>) => Promise<boolean>;

export function GeneralSection() {
  const { data: settings } = useSettings();
  const { data: background } = useBackgroundState();
  const save = useSaveSettings();
  if (!settings) return <PaneSpinner />;

  return (
    <>
      <AppearanceGroup settings={settings} save={save} />
      <NotificationsGroup settings={settings} background={background} save={save} />
      {background && <BackgroundGroup settings={settings} background={background} save={save} />}
      <SyncGroup settings={settings} save={save} />
    </>
  );
}

interface GroupProps {
  settings: Settings;
  save: Save;
}

function NotificationsGroup({
  settings,
  background,
  save,
}: GroupProps & { background: BackgroundState | undefined }) {
  const backend = useBackend();
  const [sending, setSending] = useState(false);
  const on = settings.notificationsEnabled;
  const paused = pausedUntil(settings);
  const unavailable = background !== undefined && background.notifications === null;

  const sendTest = async () => {
    setSending(true);
    try {
      await backend.sendTestNotification();
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      setSending(false);
    }
  };

  return (
    <SettingsGroup title="Notifications">
      <div className="setting-row">
        <Switch
          label="New message notifications"
          description="When mail arrives in your Inbox. They don't appear while you're looking at Cumulas Mail."
          checked={on}
          onChange={(notificationsEnabled) => void save({ notificationsEnabled })}
        />
      </div>
      <div className="setting-row">
        <Switch
          label="Show sender and subject"
          description="Turn off to show only “New message” and the account."
          checked={settings.notifyShowPreview}
          disabled={!on}
          onChange={(notifyShowPreview) => void save({ notifyShowPreview })}
        />
      </div>
      <div className="setting-row">
        <Switch
          label="Play a sound"
          description="Your desktop's sound for new mail."
          checked={settings.notifySound}
          disabled={!on}
          onChange={(notifySound) => void save({ notifySound })}
        />
      </div>
      {paused && (
        <SettingRow
          label="Notifications are paused"
          labelFor={false}
          description={`Until ${untilLabel(paused)}.`}
        >
          {() => (
            <button
              type="button"
              className="btn btn--sm"
              onClick={() => void save({ notificationsPausedUntil: null })}
            >
              Resume
            </button>
          )}
        </SettingRow>
      )}
      <SettingRow
        label="Test notification"
        labelFor={false}
        description={
          unavailable ? (
            <span className="setting-note setting-note--warning">
              <TriangleAlert size={14} aria-hidden="true" />
              No notification service is running, so notifications can&apos;t be shown.
            </span>
          ) : (
            "Do Not Disturb keeps notifications quiet. If nothing appears, check that notifications are on for Cumulas Mail in your system's settings."
          )
        }
      >
        {({ descriptionId }) => (
          <button
            type="button"
            className="btn btn--sm"
            aria-label="Send a test notification"
            aria-describedby={descriptionId}
            disabled={sending || unavailable}
            onClick={() => void sendTest()}
          >
            <BellRing size={14} aria-hidden="true" />
            Send
          </button>
        )}
      </SettingRow>
    </SettingsGroup>
  );
}

function BackgroundGroup({
  settings,
  background,
  save,
}: GroupProps & { background: BackgroundState }) {
  const runInBackground = useSetRunInBackground();
  const startAtLogin = useSetStartAtLogin();
  const failed = (e: unknown) => toast.error(errorMessage(e));
  const portal = background.autostart === "portal";
  const refused = background.backgroundAllowed === false;
  const loginUnsupported = background.autostart === "unsupported";

  const setLogin = (enabled: boolean, hidden: boolean) =>
    void trackSave(startAtLogin.mutateAsync({ enabled, hidden })).catch(failed);

  return (
    <SettingsGroup title="Background">
      <div className="setting-row">
        <Switch
          label="Keep running when closed"
          description={
            <>
              Closing the window keeps Cumulas Mail running, so new mail still shows up. Quit with
              Ctrl+Q.
              {portal && refused && (
                <span className="setting-note setting-note--warning">
                  <TriangleAlert size={14} aria-hidden="true" />
                  Your system doesn&apos;t allow Cumulas Mail to run in the background, so closing
                  the window quits it. You can allow it in your system&apos;s settings for apps.
                </span>
              )}
              {portal && background.backgroundAllowed === true && (
                <span className="setting-note">Your system allows it.</span>
              )}
              {portal && background.backgroundAllowed === null && settings.runInBackground && (
                <span className="setting-note">
                  Your system may ask the first time you close the window.
                </span>
              )}
            </>
          }
          checked={settings.runInBackground}
          disabled={runInBackground.isPending}
          onChange={(enabled) => void trackSave(runInBackground.mutateAsync(enabled)).catch(failed)}
        />
      </div>
      <div className="setting-row">
        <Switch
          label="Start at login"
          description={
            loginUnsupported
              ? "Not available on this system."
              : "Start Cumulas Mail when you log in, so new mail shows up from the start."
          }
          checked={settings.startAtLogin}
          disabled={loginUnsupported || startAtLogin.isPending}
          onChange={(enabled) => setLogin(enabled, settings.startHidden)}
        />
      </div>
      {settings.startAtLogin && (
        <div className="setting-row setting-row--nested">
          <Switch
            label="Without a window"
            description={
              background.trayAvailable && settings.showTrayIcon
                ? "Start in the background. Open it from the app launcher or the tray icon."
                : "Start in the background. Open it from the app launcher."
            }
            checked={settings.startHidden}
            disabled={startAtLogin.isPending}
            onChange={(hidden) => setLogin(true, hidden)}
          />
        </div>
      )}
      <div className="setting-row">
        <Switch
          label="Show a tray icon"
          description={
            background.trayAvailable
              ? "Shows how much mail is unread, with a menu to check mail, pause notifications or quit."
              : "Your desktop doesn't show tray icons. On GNOME, the AppIndicator extension adds them."
          }
          checked={settings.showTrayIcon && background.trayAvailable}
          disabled={!background.trayAvailable}
          onChange={(showTrayIcon) => void save({ showTrayIcon })}
        />
      </div>
    </SettingsGroup>
  );
}

function SyncGroup({ settings, save }: GroupProps) {
  const backend = useBackend();
  const { data: info } = useAppInfo();
  const server = info?.syncServer ?? "sync.cumulas.co";
  const intervals = SYNC_INTERVALS.includes(settings.syncIntervalMinutes)
    ? SYNC_INTERVALS
    : [...SYNC_INTERVALS, settings.syncIntervalMinutes].sort((a, b) => a - b);
  const instant = settings.instantPush;

  return (
    <SettingsGroup title="Sync">
      <div className="setting-row">
        <Switch
          label="Instant updates"
          description="Keep a connection open, so new mail in your Inbox arrives within seconds."
          checked={instant}
          onChange={(instantPush) => void save({ instantPush })}
        />
      </div>
      <div className="setting-row">
        <Switch
          label="Microsoft push service"
          description={
            <span className="privacy-note privacy-note--inline">
              <ShieldCheck size={14} aria-hidden="true" />
              <span>
                For Microsoft accounts, Microsoft tells Cumulas&apos;s push service ({server}) when
                your Inbox changes, never what&apos;s in it. Turned off, the Inbox is checked every
                30 seconds.{" "}
                <button
                  type="button"
                  className="link-button"
                  onClick={() =>
                    void backend.openLink(PRIVACY_URL).catch((e: unknown) => toast.error(e))
                  }
                >
                  Privacy policy
                </button>
              </span>
            </span>
          }
          checked={settings.usePushRelay}
          disabled={!instant}
          onChange={(usePushRelay) => void save({ usePushRelay })}
        />
      </div>
      <SettingRow
        label={instant ? "Also check everything else" : "Check for new mail"}
        description={
          instant
            ? "Other folders, the calendar, contacts and tasks."
            : "How often to look for new mail and other changes."
        }
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
  );
}
