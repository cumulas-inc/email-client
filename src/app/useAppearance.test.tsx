import { act, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MockBackend } from "../api/mock";
import { qk } from "../api/queries";
import type { Settings } from "../api/types";
import { SANS_STACK, SERIF_STACK } from "../lib/fonts";
import { renderWithBackend } from "../test/render";
import { useAppearance } from "./useAppearance";

function Probe() {
  useAppearance();
  return null;
}

const root = document.documentElement;

async function setup(patch: Partial<Settings> = {}) {
  const backend = new MockBackend();
  await backend.updateSettings({ ...(await backend.getSettings()), ...patch });
  const setZoom = vi.spyOn(backend, "setZoom");
  const utils = renderWithBackend(<Probe />, { backend });
  await waitFor(() => expect(setZoom).toHaveBeenCalled());
  return { ...utils, setZoom };
}

afterEach(() => {
  root.removeAttribute("style");
  delete root.dataset.interfaceFont;
});

describe("useAppearance", () => {
  it("leaves the default fonts to tokens.css and zooms to 100%", async () => {
    const { setZoom } = await setup();
    expect(setZoom).toHaveBeenLastCalledWith(1);
    expect(root.style.getPropertyValue("--font-sans")).toBe("");
    expect(root.style.getPropertyValue("--font-serif")).toBe("");
    expect(root.dataset.interfaceFont).toBeUndefined();
  });

  it("puts the chosen families in front of the stacks and zooms the window", async () => {
    const { setZoom } = await setup({
      interfaceFont: { kind: "family", family: "Fira Sans" },
      readingFont: { kind: "family", family: "Literata" },
      textSize: "largest",
    });
    expect(setZoom).toHaveBeenLastCalledWith(1.2);
    expect(root.style.getPropertyValue("--font-sans")).toBe(`"Fira Sans", ${SANS_STACK}`);
    expect(root.style.getPropertyValue("--font-serif")).toBe(`"Literata", ${SERIF_STACK}`);
    // Inter's alternates are switched off for other families.
    expect(root.dataset.interfaceFont).toBe("custom");
  });

  it("sets the reading font to the interface font when asked", async () => {
    await setup({ interfaceFont: { kind: "system" }, readingFont: { kind: "interface" } });
    expect(root.style.getPropertyValue("--font-sans")).toBe(`system-ui, ${SANS_STACK}`);
    expect(root.style.getPropertyValue("--font-serif")).toBe(`system-ui, ${SANS_STACK}`);
  });

  it("follows changes and goes back to the defaults", async () => {
    const { client, setZoom } = await setup({
      readingFont: { kind: "family", family: "Literata" },
      textSize: "smaller",
    });
    expect(setZoom).toHaveBeenLastCalledWith(0.92);
    const settings = client.getQueryData<Settings>(qk.settings())!;

    act(() => client.setQueryData(qk.settings(), { ...settings, textSize: "larger" }));
    await waitFor(() => expect(setZoom).toHaveBeenLastCalledWith(1.1));

    act(() =>
      client.setQueryData(qk.settings(), {
        ...settings,
        readingFont: { kind: "default" },
        textSize: "default",
      }),
    );
    await waitFor(() => expect(setZoom).toHaveBeenLastCalledWith(1));
    expect(root.style.getPropertyValue("--font-serif")).toBe("");
  });

  it("keeps a hostile family name inside one quoted CSS string", async () => {
    // As if a font file (or an edited settings file) named itself this way.
    const hostile = 'x"; } :root { background: red } a { b: "\n</style>';
    const client = (await setup()).client;
    const settings = client.getQueryData<Settings>(qk.settings())!;
    act(() =>
      client.setQueryData(qk.settings(), {
        ...settings,
        interfaceFont: { kind: "family", family: hostile },
      }),
    );
    await waitFor(() =>
      expect(root.style.getPropertyValue("--font-sans")).toBe(
        `"x\\"; } :root { background: red } a { b: \\"\\3c /style\\3e ", ${SANS_STACK}`,
      ),
    );
    expect(root.style.getPropertyValue("background")).toBe("");
  });

  it("doesn't touch the zoom before the settings load", async () => {
    const backend = new MockBackend({ latency: 50 });
    const setZoom = vi.spyOn(backend, "setZoom");
    renderWithBackend(<Probe />, { backend });
    expect(setZoom).not.toHaveBeenCalled();
    await waitFor(() => expect(setZoom).toHaveBeenCalledWith(1));
  });
});
