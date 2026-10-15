import { afterEach, describe, expect, it } from "vitest";

import { MockBackend } from ".";

function addStyle(css: string): CSSRule[] {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  return Array.from(style.sheet!.cssRules);
}

afterEach(async () => {
  await new MockBackend().setZoom(1);
  document.head.querySelectorAll("style").forEach((s) => s.remove());
  document.documentElement.removeAttribute("style");
});

describe("the demo's text size", () => {
  it("scales media queries and viewport units to the zoomed window", async () => {
    const [media, dialog] = addStyle(
      "@media (max-width: 900px) { a { color: red } }" +
        ".dialog { width: 400px; max-height: calc(100vh - 48px) }",
    ) as [CSSMediaRule, CSSStyleRule];
    const backend = new MockBackend();

    await backend.setZoom(1.2);
    expect(media.media.mediaText).toBe("(max-width: 1080px)");
    expect(dialog.style.maxHeight).toBe("calc(83.333vh - 48px)");
    expect(dialog.style.width).toBe("400px");

    await backend.setZoom(1);
    expect(media.media.mediaText).toBe("(max-width: 900px)");
    expect(dialog.style.maxHeight).toBe("calc(100vh - 48px)");
  });

  it("scales style sheets that load later", async () => {
    await new MockBackend().setZoom(1.1);
    const [media] = addStyle("@media (max-width: 900px) { a { color: red } }") as [CSSMediaRule];
    await new Promise((resolve) => setTimeout(resolve));
    expect(media.media.mediaText).toBe("(max-width: 990px)");
  });
});
