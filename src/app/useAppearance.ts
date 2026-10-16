import { useEffect } from "react";

import { useBackend } from "../api/context";
import { useSettings } from "../api/queries";
import { interfaceStack, readingStack, TEXT_SCALE } from "../lib/fonts";

/**
 * Applies the font and text size settings to the whole app: the chosen
 * families go in front of the `--font-sans` and `--font-serif` stacks on
 * <html> (tokens.css keeps the defaults), and the text size zooms the window.
 * Nothing changes until the settings have loaded, so the zoom the desktop
 * app set at startup stays put.
 */
export function useAppearance(): void {
  const backend = useBackend();
  const { data: settings } = useSettings();
  const custom = settings && settings.interfaceFont.kind !== "default";
  const sans = settings && custom ? interfaceStack(settings.interfaceFont) : null;
  const serif = settings && settings.readingFont.kind !== "default" ? readingStack(settings) : null;
  const scale = settings ? TEXT_SCALE[settings.textSize] : null;

  useEffect(() => {
    const root = document.documentElement;
    setOrRemove(root, "--font-sans", sans);
    setOrRemove(root, "--font-serif", serif);
    // Inter's alternates (cv11, ss03) mean other things in other families.
    if (sans) root.dataset.interfaceFont = "custom";
    else delete root.dataset.interfaceFont;
  }, [sans, serif]);

  useEffect(() => {
    if (scale === null) return;
    backend.setZoom(scale).catch((e: unknown) => console.warn("Couldn't set the text size", e));
  }, [backend, scale]);
}

function setOrRemove(root: HTMLElement, name: string, value: string | null): void {
  if (value) root.style.setProperty(name, value);
  else root.style.removeProperty(name);
}
