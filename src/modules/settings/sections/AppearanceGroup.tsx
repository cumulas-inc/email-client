import { Monitor, Moon, RotateCcw, Sun } from "lucide-react";
import { type CSSProperties, useCallback, useId, useMemo, useState } from "react";

import { useFonts } from "../../../api/queries";
import type { InterfaceFont, ReadingFont, Settings, TextSize, Theme } from "../../../api/types";
import { SegmentedControl, type SegmentedOption } from "../../../components/SegmentedControl";
import {
  DEFAULT_TYPE,
  fontKey,
  interfaceStack,
  isDefaultType,
  readingStack,
  SANS_STACK,
  SERIF_STACK,
} from "../../../lib/fonts";
import { type FontOption, FontPicker } from "../FontPicker";
import { SettingRow, SettingsGroup } from "../SettingsLayout";

const THEMES: SegmentedOption<Theme>[] = [
  { value: "system", label: "System", icon: <Monitor size={20} /> },
  { value: "light", label: "Light", icon: <Sun size={20} /> },
  { value: "dark", label: "Dark", icon: <Moon size={20} /> },
];

const TEXT_SIZES: SegmentedOption<TextSize>[] = [
  { value: "smaller", label: "Smaller" },
  { value: "default", label: "Default" },
  { value: "larger", label: "Larger" },
  { value: "largest", label: "Largest" },
];

const INTERFACE_PRESETS: FontOption<InterfaceFont>[] = [
  { key: "default", label: "Inter (default)", value: { kind: "default" }, fontFamily: SANS_STACK },
  {
    key: "system",
    label: "System font",
    value: { kind: "system" },
    fontFamily: interfaceStack({ kind: "system" }),
  },
];

const interfaceFamily = (family: string): InterfaceFont => ({ kind: "family", family });
const readingFamily = (family: string): ReadingFont => ({ kind: "family", family });

function interfaceLabel(font: InterfaceFont): string {
  if (font.kind === "family") return font.family;
  return font.kind === "system" ? "System font" : "Inter (default)";
}

function readingLabel(font: ReadingFont): string {
  if (font.kind === "family") return font.family;
  return font.kind === "interface" ? "Same as interface font" : "Charis SIL (default)";
}

/** "Fira Sans isn't installed…", when a saved family is missing from this computer. */
function missingNote(family: string | null, families: string[] | undefined, fallback: string) {
  if (!family || !families || families.includes(family)) return null;
  return ` ${family} isn't installed on this computer, so ${fallback} is used.`;
}

/** The list with a saved family that isn't installed, so it can still show as chosen. */
function withSaved(families: string[] | undefined, family: string | null): string[] | undefined {
  if (!families || !family || families.includes(family)) return families;
  return [family, ...families];
}

type Preview = { interfaceFont?: InterfaceFont; readingFont?: ReadingFont };

interface AppearanceGroupProps {
  settings: Settings;
  save: (patch: Partial<Settings>) => Promise<boolean>;
}

/**
 * Theme, fonts and text size, with a sample letter that shows the fonts as
 * they are, or as they'd be while a font list is open.
 */
export function AppearanceGroup({ settings, save }: AppearanceGroupProps) {
  const fonts = useFonts();
  const [preview, setPreview] = useState<Preview>({});
  const previewInterface = useCallback(
    (font: InterfaceFont | null) => setPreview((p) => ({ ...p, interfaceFont: font ?? undefined })),
    [],
  );
  const previewReading = useCallback(
    (font: ReadingFont | null) => setPreview((p) => ({ ...p, readingFont: font ?? undefined })),
    [],
  );

  const interfaceFont = settings.interfaceFont;
  const readingFont = settings.readingFont;
  const shown = {
    interfaceFont: preview.interfaceFont ?? interfaceFont,
    readingFont: preview.readingFont ?? readingFont,
  };

  const readingPresets = useMemo<FontOption<ReadingFont>[]>(
    () => [
      {
        key: "default",
        label: "Charis SIL (default)",
        value: { kind: "default" },
        fontFamily: SERIF_STACK,
      },
      {
        key: "interface",
        label: "Same as interface font",
        value: { kind: "interface" },
        fontFamily: interfaceStack(interfaceFont),
      },
    ],
    [interfaceFont],
  );

  const interfaceSaved = interfaceFont.kind === "family" ? interfaceFont.family : null;
  const readingSaved = readingFont.kind === "family" ? readingFont.family : null;

  return (
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

      <TypePreview
        customSans={shown.interfaceFont.kind !== "default"}
        sans={interfaceStack(shown.interfaceFont)}
        serif={readingStack(shown)}
        canReset={!isDefaultType(settings)}
        onReset={() => void save(DEFAULT_TYPE)}
      />

      <SettingRow
        label="Interface font"
        description={
          <>
            Menus, lists, buttons and labels.
            {missingNote(interfaceSaved, fonts.data, "Inter")}
          </>
        }
      >
        {({ controlId, labelId, descriptionId }) => (
          <FontPicker
            id={controlId}
            labelledBy={labelId}
            describedBy={descriptionId}
            presets={INTERFACE_PRESETS}
            families={withSaved(fonts.data, interfaceSaved)}
            familiesFailed={fonts.isError}
            familyValue={interfaceFamily}
            selectedKey={fontKey(interfaceFont)}
            selectedLabel={interfaceLabel(interfaceFont)}
            onChange={(font) => void save({ interfaceFont: font })}
            onPreview={previewInterface}
          />
        )}
      </SettingRow>

      <SettingRow
        label="Reading font"
        description={
          <>
            Subjects, titles, messages and what you write. Mail that chooses its own fonts keeps
            them.
            {missingNote(readingSaved, fonts.data, "Charis SIL")}
          </>
        }
      >
        {({ controlId, labelId, descriptionId }) => (
          <FontPicker
            id={controlId}
            labelledBy={labelId}
            describedBy={descriptionId}
            presets={readingPresets}
            families={withSaved(fonts.data, readingSaved)}
            familiesFailed={fonts.isError}
            familyValue={readingFamily}
            selectedKey={fontKey(readingFont)}
            selectedLabel={readingLabel(readingFont)}
            onChange={(font) => void save({ readingFont: font })}
            onPreview={previewReading}
          />
        )}
      </SettingRow>

      <SettingRow
        label="Text size"
        labelFor={false}
        description="Makes everything in the window smaller or larger."
      >
        {({ labelId, descriptionId }) => (
          <SegmentedControl
            labelledBy={labelId}
            describedBy={descriptionId}
            value={settings.textSize}
            options={TEXT_SIZES}
            onChange={(textSize) => void save({ textSize })}
          />
        )}
      </SettingRow>
    </SettingsGroup>
  );
}

interface TypePreviewProps {
  /** The interface font isn't Inter (whose alternates the app turns on). */
  customSans: boolean;
  sans: string;
  serif: string;
  canReset: boolean;
  onReset: () => void;
}

/** A few lines of a message, set in the interface and reading fonts. */
function TypePreview({ customSans, sans, serif, canReset, onReset }: TypePreviewProps) {
  const captionId = useId();
  const style = { "--preview-sans": sans, "--preview-serif": serif } as CSSProperties;
  return (
    <figure
      className="type-preview"
      aria-labelledby={captionId}
      data-custom-sans={customSans || undefined}
      style={style}
    >
      <div className="type-preview__head">
        <figcaption id={captionId} className="type-preview__caption">
          Preview
        </figcaption>
        <button
          type="button"
          className="btn btn--subtle btn--sm type-preview__reset"
          disabled={!canReset}
          onClick={onReset}
        >
          <RotateCcw size={14} aria-hidden="true" />
          Reset to defaults
        </button>
      </div>
      <div className="type-preview__letter">
        <p className="type-preview__from">
          <span className="type-preview__sender">Priya Raman</span>
          <span className="type-preview__time tnum">9:42 AM</span>
        </p>
        <p className="type-preview__subject">Atlas launch checklist — final review today</p>
        <p className="type-preview__body">
          I&rsquo;ve folded everyone&rsquo;s comments into the launch checklist. Could you take a
          last look at the rollback plan before our 2pm review?
        </p>
      </div>
    </figure>
  );
}
