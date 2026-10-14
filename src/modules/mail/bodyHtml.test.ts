import { describe, expect, it } from "vitest";

import { buildSrcDoc, classifyLink, contentSecurityPolicy, linkify } from "./bodyHtml";

describe("message frame document", () => {
  it("blocks everything but inline styles and embedded images by default", () => {
    const csp = contentSecurityPolicy(false);
    expect(csp).toContain("default-src 'none'");
    expect(csp).toContain("style-src 'unsafe-inline'");
    expect(csp).toContain("img-src data: cid:;");
    expect(csp).toContain("font-src data:");
    expect(csp).not.toContain("https:");
    expect(contentSecurityPolicy(true)).toContain("img-src data: cid: https: http:");
  });

  it("wraps the body with the CSP meta tag and readable base styles", () => {
    const doc = buildSrcDoc("<p>Hello</p>", { allowRemote: false, appearance: "paper" });
    expect(doc).toMatch(/^<!doctype html>/);
    expect(doc).toContain(
      `<meta http-equiv="Content-Security-Policy" content="${contentSecurityPolicy(false)}">`,
    );
    expect(doc).toContain("img { max-width: 100%; height: auto; }");
    expect(doc).toContain("overflow-wrap: break-word");
    expect(doc).toContain("<body><p>Hello</p></body>");
    // Dark mode inverts the page and turns images back.
    expect(buildSrcDoc("", { allowRemote: false, appearance: "dark" })).toContain(
      "filter: invert(1) hue-rotate(180deg)",
    );
  });
});

describe("links", () => {
  it("only follows web and mail links", () => {
    expect(classifyLink("https://example.org/a?b=1")).toEqual({
      kind: "open",
      url: "https://example.org/a?b=1",
    });
    expect(classifyLink("http://example.org")).toEqual({
      kind: "open",
      url: "http://example.org/",
    });
    expect(classifyLink("mailto:ann@example.org?subject=Hi")).toEqual({
      kind: "mailto",
      url: "mailto:ann@example.org?subject=Hi",
    });
    expect(classifyLink("javascript:alert(1)")).toEqual({ kind: "ignore" });
    expect(classifyLink("file:///etc/passwd")).toEqual({ kind: "ignore" });
    expect(classifyLink("data:text/html,hi")).toEqual({ kind: "ignore" });
    expect(classifyLink("not a url")).toEqual({ kind: "ignore" });
    expect(classifyLink(null)).toEqual({ kind: "ignore" });
  });

  it("linkifies plain text without trailing punctuation", () => {
    expect(
      linkify("See https://example.org/x. Or mail ann@example.org, or www.example.org!"),
    ).toEqual([
      { text: "See " },
      { text: "https://example.org/x", href: "https://example.org/x" },
      { text: ". Or mail " },
      { text: "ann@example.org", href: "mailto:ann@example.org" },
      { text: ", or " },
      { text: "www.example.org", href: "https://www.example.org" },
      { text: "!" },
    ]);
    expect(linkify("<b>not html</b>")).toEqual([{ text: "<b>not html</b>" }]);
  });
});
