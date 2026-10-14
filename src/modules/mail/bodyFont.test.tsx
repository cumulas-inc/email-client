import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { MockBackend } from "../../api/mock";
import type { Settings } from "../../api/types";
import { SANS_STACK, SERIF_STACK } from "../../lib/fonts";
import { renderWithBackend } from "../../test/render";
import { bodyFont } from "./bodyFont";
import { buildSrcDoc, DEFAULT_BODY_FONT } from "./bodyHtml";
import { MessageBody } from "./MessageBody";

const fonts = (interfaceFont: Settings["interfaceFont"], readingFont: Settings["readingFont"]) => ({
  interfaceFont,
  readingFont,
});

describe("the message frame's font", () => {
  it("carries Charis SIL inside the frame by default", () => {
    expect(bodyFont(undefined, null)).toBe(DEFAULT_BODY_FONT);
    expect(bodyFont(fonts({ kind: "system" }, { kind: "default" }), null)).toBe(DEFAULT_BODY_FONT);
    const doc = buildSrcDoc("<p>Hi</p>", { allowRemote: false, appearance: "plain" });
    expect(doc).toContain('@font-face { font-family: "Charis SIL"');
    expect(doc).toContain("src: url(data:");
    expect(doc).toContain(`font-family: ${SERIF_STACK};`);
  });

  it("only names an installed family, without carrying any faces", () => {
    const font = bodyFont(fonts({ kind: "default" }, { kind: "family", family: "Literata" }), null);
    expect(font).toEqual({ family: `"Literata", ${SERIF_STACK}`, faces: "" });
    const doc = buildSrcDoc("<p>Hi</p>", { allowRemote: false, appearance: "plain", font: font! });
    expect(doc).not.toContain("@font-face");
    expect(doc).toContain(`font-family: "Literata", ${SERIF_STACK};`);
  });

  it("follows the interface font, waiting for Inter's faces when it's the bundled one", () => {
    const system = bodyFont(fonts({ kind: "system" }, { kind: "interface" }), null);
    expect(system).toEqual({ family: `system-ui, ${SANS_STACK}`, faces: "" });
    const installed = bodyFont(
      fonts({ kind: "family", family: "Cantarell" }, { kind: "interface" }),
      null,
    );
    expect(installed).toEqual({ family: `"Cantarell", ${SANS_STACK}`, faces: "" });

    const inter = fonts({ kind: "default" }, { kind: "interface" });
    expect(bodyFont(inter, null)).toBeNull();
    expect(bodyFont(inter, "@font-face {}")).toEqual({
      family: SANS_STACK,
      faces: "@font-face {}",
    });
  });

  it("keeps a hostile family name from closing the frame's style sheet", () => {
    const hostile = '</style><script>alert(1)</script><style>"';
    const font = bodyFont(fonts({ kind: "default" }, { kind: "family", family: hostile }), null);
    const doc = buildSrcDoc("<p>Hi</p>", { allowRemote: false, appearance: "plain", font: font! });
    expect(doc).not.toContain("<script>");
    expect(doc.match(/<\/style>/g)).toHaveLength(1);
    expect(doc).toContain('font-family: "\\3c /style\\3e \\3c script\\3e alert(1)');
  });
});

describe("MessageBody", () => {
  async function renderBody(patch: Partial<Settings>) {
    const backend = new MockBackend();
    await backend.updateSettings({ ...(await backend.getSettings()), ...patch });
    renderWithBackend(
      <MessageBody
        html='<p>Hello</p><p style="font-family: Arial">Own font</p>'
        allowRemote={false}
        appearance="plain"
        title="Message: Hello"
        onLink={() => {}}
      />,
      { backend },
    );
  }

  const srcdoc = () => screen.getByTitle("Message: Hello").getAttribute("srcdoc") ?? "";

  it("sets text in the reading font, leaving the mail's own fonts alone", async () => {
    await renderBody({ readingFont: { kind: "family", family: "Literata" } });
    await waitFor(() => expect(srcdoc()).toContain(`font-family: "Literata", ${SERIF_STACK};`));
    expect(srcdoc()).not.toContain("@font-face");
    expect(srcdoc()).toContain('<p style="font-family: Arial">Own font</p>');
  });

  it("brings Inter into the frame when messages read in the bundled interface font", async () => {
    await renderBody({ readingFont: { kind: "interface" } });
    await waitFor(() => expect(srcdoc()).toContain('@font-face { font-family: "Inter Variable"'));
    expect(srcdoc()).toContain(`font-family: ${SANS_STACK};`);
    expect(srcdoc()).not.toContain('font-family: "Charis SIL"; font-style');
  });
});
