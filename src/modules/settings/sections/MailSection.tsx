import clsx from "clsx";
import { PanelBottom, PanelRight, ShieldCheck, Square } from "lucide-react";
import { useId } from "react";

import { useSettings } from "../../../api/queries";
import type { ReadingPane, RemoteContentPolicy } from "../../../api/types";
import { SegmentedControl, type SegmentedOption } from "../../../components/SegmentedControl";
import { PaneSpinner } from "../../../components/Spinner";
import { useSaveSettings } from "../saveStatus";
import { SettingRow, SettingsGroup } from "../SettingsLayout";

const READING_PANES: SegmentedOption<ReadingPane>[] = [
  { value: "right", label: "Right", icon: <PanelRight size={16} /> },
  { value: "bottom", label: "Bottom", icon: <PanelBottom size={16} /> },
  { value: "off", label: "Off", icon: <Square size={16} /> },
];

const REMOTE_CONTENT: { value: RemoteContentPolicy; label: string; description: string }[] = [
  {
    value: "block",
    label: "Block",
    description: "Recommended. Load pictures only when you choose to, one message at a time.",
  },
  {
    value: "allowContacts",
    label: "Allow from my contacts",
    description: "Load automatically for people in your contacts; block everyone else.",
  },
  {
    value: "allow",
    label: "Always allow",
    description: "Load automatically for every message. Senders can tell when you open their mail.",
  },
];

type MarkRead = "0" | "3" | "never" | `${number}`;

function markReadValue(seconds: number | null): MarkRead {
  return seconds === null ? "never" : (String(seconds) as MarkRead);
}

export function MailSection() {
  const { data: settings } = useSettings();
  const save = useSaveSettings();
  const remoteName = useId();
  if (!settings) return <PaneSpinner />;

  const markRead = markReadValue(settings.markReadAfterSeconds);
  const markReadOptions: { value: MarkRead; label: string }[] = [
    { value: "0", label: "Immediately" },
    { value: "3", label: "After 3 seconds" },
    { value: "never", label: "Never" },
  ];
  if (!markReadOptions.some((o) => o.value === markRead)) {
    const unit = markRead === "1" ? "second" : "seconds";
    markReadOptions.splice(2, 0, { value: markRead, label: `After ${markRead} ${unit}` });
  }

  return (
    <>
      <SettingsGroup title="Reading">
        <SettingRow
          label="Reading pane"
          labelFor={false}
          description="Where to show the message you select."
        >
          {({ labelId, descriptionId }) => (
            <SegmentedControl
              labelledBy={labelId}
              describedBy={descriptionId}
              value={settings.readingPane}
              options={READING_PANES}
              onChange={(readingPane) => void save({ readingPane })}
            />
          )}
        </SettingRow>
        <SettingRow
          label="Mark messages as read"
          description="When you open a message in the reading pane."
        >
          {({ controlId, descriptionId }) => (
            <select
              id={controlId}
              className="select setting-select"
              aria-describedby={descriptionId}
              value={markRead}
              onChange={(e) => {
                const v = e.target.value;
                void save({ markReadAfterSeconds: v === "never" ? null : Number(v) });
              }}
            >
              {markReadOptions.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          )}
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Privacy">
        <SettingRow
          label="Pictures and other remote content"
          labelFor={false}
          stacked
          description={
            <div className="privacy-note">
              <ShieldCheck size={16} aria-hidden="true" />
              <p>
                Images loaded from the internet can tell senders when and where you opened their
                message, often through invisible &ldquo;tracking pixels&rdquo;. Blocking them keeps
                that private; you can still show them for a single message.
              </p>
            </div>
          }
        >
          {({ labelId, descriptionId }) => (
            <div
              role="radiogroup"
              aria-labelledby={labelId}
              aria-describedby={descriptionId}
              className="radio-cards"
            >
              {REMOTE_CONTENT.map((option) => {
                const selected = settings.remoteContent === option.value;
                const descriptionId = `${remoteName}-${option.value}`;
                return (
                  <label
                    key={option.value}
                    className={clsx("radio-card", selected && "radio-card--selected")}
                  >
                    <input
                      type="radio"
                      name={remoteName}
                      value={option.value}
                      checked={selected}
                      aria-labelledby={`${remoteName}-${option.value}-label`}
                      aria-describedby={descriptionId}
                      onChange={() => void save({ remoteContent: option.value })}
                    />
                    <span className="radio-card__text">
                      <span
                        id={`${remoteName}-${option.value}-label`}
                        className="radio-card__label"
                      >
                        {option.label}
                      </span>
                      <span id={descriptionId} className="radio-card__description">
                        {option.description}
                      </span>
                    </span>
                  </label>
                );
              })}
            </div>
          )}
        </SettingRow>
      </SettingsGroup>
    </>
  );
}
