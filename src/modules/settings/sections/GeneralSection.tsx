import { format, isToday, isTomorrow } from "date-fns";
import { BellRing, Check, ShieldCheck, TriangleAlert } from "lucide-react";
import { useRef, useState } from "react";

import { useBackend } from "../../../api/context";
import { errorMessage } from "../../../api/errors";
import {
  useAppInfo,
  useBackgroundState,
  useSetRunInBackground,
  useSetStartAtLogin,
  useSettings,
} from "../../../api/queries";
import type { BackgroundState, NotifyScope, Settings } from "../../../api/types";
import { PaneSpinner } from "../../../components/Spinner";
import { Switch } from "../../../components/Switch";
import { toast } from "../../../components/toast";
import { PRIVACY_URL } from "../constants";
import { NOTIFY_SCOPES, notifyWhere } from "../notifyScope";
import { trackSave, useSaveSettings } from "../saveStatus";
import { SettingRow, SettingsGroup } from "../SettingsLayout";
import { AppearanceGroup } from "./AppearanceGroup";

const SYNC_INTERVALS = [1, 5, 15, 30, 60];
const SNOOZE_MINUTES = [5, 10, 15, 30, 60];

function snoozeLabel(minutes: number): string {
  if (minutes === 60) return "1 hour";
  if (minutes % 60 === 0) return `${minutes / 60} hours`;
  return minutes === 1 ? "1 minute" : `${minutes} minutes`;
}

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

/** After a test notification: whether new mail will show up like it. */
function testSentLabel(on: boolean, paused: string | null): string {
  if (!on) return "Sent. New message notifications are off, so new mail won't show up like this.";
  if (paused) return `Sent. New message notifications are paused until ${untilLabel(paused)}.`;
  return "Sent. If nothing appeared, check that notifications are on for Cumulas Mail in your system's settings.";
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
  // A press while one is on its way does nothing. (Disabling the button
  // would take the keyboard focus away.)
  const sending = useRef(false);
  /** The settings when the last test was sent; what it said holds while they do. */
  const [sent, setSent] = useState<{ on: boolean; paused: string | null } | null>(null);
  const on = settings.notificationsEnabled;
  const paused = pausedUntil(settings);
  const unavailable = background !== undefined && background.notifications === null;
  const sentLabel =
    sent && sent.on === on && sent.paused === paused ? testSentLabel(on, paused) : null;

  const sendTest = async () => {
    if (sending.current) return;
    sending.current = true;
    setSent(null);
    try {
      await backend.sendTestNotification();
      setSent({ on, paused });
    } catch (e) {
      toast.error(errorMessage(e));
    } finally {
      sending.current = false;
    }
  };

  return (
    <SettingsGroup title="Notifications">
      <div className="setting-row">
        <Switch
          label="New message notifications"
          description={`When mail arrives in ${notifyWhere(settings.notifyScope, "your")}. They don't appear while you're looking at Cumulas Mail. You can turn them off for one account in Accounts.`}
          checked={on}
          onChange={(notificationsEnabled) => void save({ notificationsEnabled })}
        />
      </div>
      <SettingRow
        label="Notify for"
        description="All folders includes mail that rules move out of your Inbox. Instant updates cover only the Inbox, so notifications for other folders can come later."
      >
        {({ controlId, descriptionId }) => (
          <select
            id={controlId}
            className="select setting-select"
            aria-describedby={descriptionId}
            value={settings.notifyScope}
            disabled={!on}
            onChange={(e) => void save({ notifyScope: e.target.value as NotifyScope })}
          >
            {NOTIFY_SCOPES.map((s) => (
              <option key={s.value} value={s.value}>
                {s.label}
              </option>
            ))}
          </select>
        )}
      </SettingRow>
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
          label="Event and task reminders"
          description="Before events start and when a task's reminder is due, even while you're looking at Cumulas Mail. The bell at the top lists them."
          checked={settings.remindersEnabled}
          onChange={(remindersEnabled) => void save({ remindersEnabled })}
        />
      </div>
      <SettingRow label="Snooze for" description="How long a reminder's Snooze button puts it off.">
        {({ controlId, descriptionId }) => (
          <select
            id={controlId}
            className="select setting-select"
            aria-describedby={descriptionId}
            value={settings.defaultSnoozeMinutes}
            disabled={!settings.remindersEnabled}
            onChange={(e) => void save({ defaultSnoozeMinutes: Number(e.target.value) })}
          >
            {[
              ...new Set([...SNOOZE_MINUTES, settings.defaultSnoozeMinutes].sort((a, b) => a - b)),
            ].map((m) => (
              <option key={m} value={m}>
                {snoozeLabel(m)}
              </option>
            ))}
          </select>
        )}
      </SettingRow>
      <div className="setting-row">
        <Switch
          label="Play a sound"
          description="Your desktop's sound for new mail and reminders."
          checked={settings.notifySound}
          disabled={!on && !settings.remindersEnabled}
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
          <>
            {unavailable ? (
              <span className="setting-note setting-note--warning">
                <TriangleAlert size={14} aria-hidden="true" />
                No notification service is running, so notifications can&apos;t be shown.
              </span>
            ) : (
              "Do Not Disturb keeps notifications quiet."
            )}
            {/* There before it has something to say, so that it's announced. */}
            <div role="status">
              {sentLabel && (
                <span className="setting-note">
                  <Check size={14} aria-hidden="true" />
                  {sentLabel}
                </span>
              )}
            </div>
          </>
        }
      >
        {({ descriptionId }) => (
          <button
            type="button"
            className="btn btn--sm"
            aria-label="Send a test notification"
            aria-describedby={descriptionId}
            disabled={unavailable}
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
  const tray = background.trayAvailable && settings.showTrayIcon;
  // While the system answers (in a Flatpak, it may ask the user first), the
  // switches show the choice and ignore more presses. Disabling them would
  // take the keyboard focus away.
  const keepRunning = runInBackground.isPending
    ? runInBackground.variables
    : settings.runInBackground;
  const login = startAtLogin.isPending
    ? startAtLogin.variables
    : { enabled: settings.startAtLogin, hidden: settings.startHidden };

  const setKeepRunning = (enabled: boolean) => {
    if (runInBackground.isPending) return;
    void trackSave(runInBackground.mutateAsync(enabled)).catch(failed);
  };

  const setLogin = (enabled: boolean, hidden: boolean) => {
    if (startAtLogin.isPending) return;
    void trackSave(startAtLogin.mutateAsync({ enabled, hidden })).catch(failed);
  };

  return (
    <SettingsGroup title="Background">
      <div className="setting-row">
        <Switch
          label="Keep running when closed"
          description={
            <>
              Closing the window keeps Cumulas Mail running, so new mail still shows up. To quit,
              choose Quit in the main menu{tray && " or the tray icon's menu"}.
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
          checked={keepRunning}
          onChange={setKeepRunning}
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
          checked={login.enabled}
          disabled={loginUnsupported}
          onChange={(enabled) => setLogin(enabled, login.hidden)}
        />
      </div>
      {login.enabled && (
        <div className="setting-row setting-row--nested">
          <Switch
            label="Without a window"
            description={
              tray
                ? "Start in the background. Open it from the app launcher or the tray icon."
                : "Start in the background. Open it from the app launcher."
            }
            checked={login.hidden}
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
                your Inbox or calendar changes, never what&apos;s in it. Turned off, the Inbox is
                checked every 30 seconds.{" "}
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
