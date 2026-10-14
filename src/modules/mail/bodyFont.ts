// The reading font inside the message frame. The frame can't see the app's
// fonts (its CSP allows data: fonts only), so the bundled faces travel in
// it as data: URLs; an installed family is just named, since the webview
// finds installed fonts by name. Mail that sets its own fonts keeps them:
// this is only the frame's default.

import { useEffect, useState } from "react";

import { useSettings } from "../../api/queries";
import type { Settings } from "../../api/types";
import { readingStack } from "../../lib/fonts";
import { type BodyFont, DEFAULT_BODY_FONT, dataFace } from "./bodyHtml";

type FontSettings = Pick<Settings, "interfaceFont" | "readingFont">;

/** Whether the reading font is the bundled Inter ("Same as interface font" with the default interface font). */
function readsInBundledInter(settings: FontSettings | undefined): boolean {
  return settings?.readingFont.kind === "interface" && settings.interfaceFont.kind === "default";
}

/**
 * The frame's font for these settings. `interFaces` are Inter's @font-face
 * rules, needed only when the reading font is the bundled Inter; null until
 * they have loaded, and then this returns null too.
 */
export function bodyFont(
  settings: FontSettings | undefined,
  interFaces: string | null,
): BodyFont | null {
  if (!settings || settings.readingFont.kind === "default") return DEFAULT_BODY_FONT;
  const family = readingStack(settings);
  if (readsInBundledInter(settings))
    return interFaces === null ? null : { family, faces: interFaces };
  return { family, faces: "" };
}

let interFaces: string | null = null;
let interLoad: Promise<string> | null = null;

/**
 * Inter's Latin faces (the text cut, about 100 KB of WOFF2), loaded on first
 * use: most people never read mail in Inter, so the main bundle doesn't
 * carry them.
 */
function loadInterFaces(): Promise<string> {
  interLoad ??= Promise.all([
    import("@fontsource-variable/inter/files/inter-latin-wght-normal.woff2?inline"),
    import("@fontsource-variable/inter/files/inter-latin-wght-italic.woff2?inline"),
  ]).then(([normal, italic]) => {
    interFaces = [
      dataFace("Inter Variable", normal.default, "normal", "100 900"),
      dataFace("Inter Variable", italic.default, "italic", "100 900"),
    ].join("\n");
    return interFaces;
  });
  return interLoad;
}

/** The frame's font from the settings; null while Inter's faces load (once). */
export function useBodyFont(): BodyFont | null {
  const { data: settings } = useSettings();
  const needsInter = readsInBundledInter(settings);
  const [faces, setFaces] = useState(interFaces);
  useEffect(() => {
    if (!needsInter || faces !== null) return;
    let live = true;
    loadInterFaces().then(
      (loaded) => live && setFaces(loaded),
      // Without them the frame falls back along the stack (an installed
      // Inter, then the desktop's sans).
      () => live && setFaces(""),
    );
    return () => {
      live = false;
    };
  }, [needsInter, faces]);
  return bodyFont(settings, faces);
}
