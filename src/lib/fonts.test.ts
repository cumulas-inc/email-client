/// <reference types="node" />
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import modelRs from "../../crates/core/src/model.rs?raw";
import {
  cleanFamily,
  cssString,
  fontKey,
  interfaceStack,
  isDefaultType,
  readingStack,
  SANS_STACK,
  SERIF_STACK,
  TEXT_SCALE,
  withFamily,
} from "./fonts";

const LINE_SEPARATOR = String.fromCodePoint(0x2028);
const RIGHT_TO_LEFT_OVERRIDE = String.fromCodePoint(0x202e);

/** Splits a font-family value into its comma-separated items, honoring CSS strings. */
function families(value: string): string[] {
  const out: string[] = [];
  let item = "";
  let quoted = false;
  for (let i = 0; i < value.length; i++) {
    const ch = value[i]!;
    if (quoted && ch === "\\") {
      item += ch + value[++i];
    } else if (ch === '"') {
      quoted = !quoted;
      item += ch;
    } else if (ch === "," && !quoted) {
      out.push(item.trim());
      item = "";
    } else {
      item += ch;
    }
  }
  expect(quoted).toBe(false);
  out.push(item.trim());
  return out;
}

describe("CSS strings", () => {
  it("quotes plain names", () => {
    expect(cssString("Fira Sans")).toBe('"Fira Sans"');
    expect(cssString("文泉驿正黑")).toBe('"文泉驿正黑"');
    expect(cssString("O'Reilly")).toBe(`"O'Reilly"`);
  });

  it("escapes quotes and backslashes, so the string can't end early", () => {
    expect(cssString('Evil", serif; color: red; x: "')).toBe(
      '"Evil\\", serif; color: red; x: \\""',
    );
    expect(cssString("Back\\")).toBe('"Back\\\\"');
    // An escaped backslash before a quote still leaves the quote escaped.
    expect(cssString('a\\"b')).toBe('"a\\\\\\"b"');
  });

  it("writes < and > as hex escapes, so it can't close a <style> element", () => {
    const out = cssString("</style><script>alert(1)</script>");
    expect(out).not.toMatch(/[<>]/);
    expect(out).toBe('"\\3c /style\\3e \\3c script\\3e alert(1)\\3c /script\\3e "');
  });

  it("drops controls, line breaks and bidirectional overrides", () => {
    expect(cssString("A\nB\rC\fD\tE\0F\u007fG")).toBe('"ABCDEFG"');
    expect(cssString(`x${LINE_SEPARATOR}y${RIGHT_TO_LEFT_OVERRIDE}z`)).toBe('"xyz"');
  });
});

describe("family names", () => {
  it("cleans names and rejects unusable ones", () => {
    expect(cleanFamily("  Noto Sans  ")).toBe("Noto Sans");
    expect(cleanFamily("Noto\nSans")).toBe("NotoSans");
    expect(cleanFamily("")).toBeNull();
    expect(cleanFamily(" \t\n")).toBeNull();
    expect(cleanFamily("x".repeat(201))).toBeNull();
    expect(cleanFamily("x".repeat(200))).toBe("x".repeat(200));
  });

  it("trims the same spaces as the Rust core, which keeps a byte order mark", () => {
    // The same names as `font_family_names_are_validated` in crates/core/src/model.rs.
    const bom = String.fromCodePoint(0xfeff);
    const noBreak = String.fromCodePoint(0xa0);
    const ideographic = String.fromCodePoint(0x3000);
    expect(cleanFamily(`${bom}Foo`)).toBe(`${bom}Foo`);
    expect(cleanFamily(`Foo${bom}`)).toBe(`Foo${bom}`);
    expect(cleanFamily(`${noBreak}Inter${ideographic}`)).toBe("Inter");
    expect(cleanFamily(" Foo Bar ")).toBe("Foo Bar");
    expect(withFamily(`${bom}Foo`, SERIF_STACK)).toBe(`"${bom}Foo", ${SERIF_STACK}`);
  });

  it("puts the chosen family in front of the stack as one quoted item", () => {
    expect(withFamily("Fira Sans", SANS_STACK)).toBe(`"Fira Sans", ${SANS_STACK}`);
    const hostile = withFamily('X", monospace; } body { display: none } /*', SERIF_STACK);
    expect(families(hostile)).toEqual([
      '"X\\", monospace; } body { display: none } /*"',
      ...families(SERIF_STACK),
    ]);
    expect(withFamily("\n\t", SERIF_STACK)).toBe(SERIF_STACK);
  });

  it("names a generic family literally, not as the generic", () => {
    expect(withFamily("serif", SANS_STACK).startsWith('"serif", ')).toBe(true);
  });
});

describe("stacks", () => {
  it("match the tokens in tokens.css", () => {
    // Read from disk: the test run doesn't load style sheets.
    const tokensCss = readFileSync(`${process.cwd()}/src/styles/tokens.css`, "utf8");
    expect(tokensCss).toContain(`--font-sans: ${SANS_STACK};`);
    expect(tokensCss).toContain(`--font-serif: ${SERIF_STACK};`);
  });

  it("build the interface stack", () => {
    expect(interfaceStack({ kind: "default" })).toBe(SANS_STACK);
    expect(interfaceStack({ kind: "system" })).toBe(`system-ui, ${SANS_STACK}`);
    expect(interfaceStack({ kind: "family", family: "Cantarell" })).toBe(
      `"Cantarell", ${SANS_STACK}`,
    );
  });

  it("build the reading stack, which can follow the interface font", () => {
    const system = { kind: "system" } as const;
    expect(readingStack({ interfaceFont: system, readingFont: { kind: "default" } })).toBe(
      SERIF_STACK,
    );
    expect(
      readingStack({
        interfaceFont: system,
        readingFont: { kind: "family", family: "Literata" },
      }),
    ).toBe(`"Literata", ${SERIF_STACK}`);
    expect(readingStack({ interfaceFont: system, readingFont: { kind: "interface" } })).toBe(
      `system-ui, ${SANS_STACK}`,
    );
    expect(
      readingStack({ interfaceFont: { kind: "default" }, readingFont: { kind: "interface" } }),
    ).toBe(SANS_STACK);
  });

  it("key choices and recognize the defaults", () => {
    expect(fontKey({ kind: "default" })).toBe("default");
    expect(fontKey({ kind: "interface" })).toBe("interface");
    expect(fontKey({ kind: "family", family: "Lato" })).toBe("family:Lato");
    const defaults = {
      interfaceFont: { kind: "default" },
      readingFont: { kind: "default" },
      textSize: "default",
    } as const;
    expect(isDefaultType(defaults)).toBe(true);
    expect(isDefaultType({ ...defaults, textSize: "larger" })).toBe(false);
    expect(isDefaultType({ ...defaults, readingFont: { kind: "interface" } })).toBe(false);
  });
});

describe("text size", () => {
  it("scales by the same factors as the Rust core", () => {
    const rust = Object.fromEntries(
      [...modelRs.matchAll(/Self::(Smaller|Default|Larger|Largest) => ([\d.]+),/g)].map((m) => [
        m[1]!.toLowerCase(),
        Number(m[2]),
      ]),
    );
    expect(rust).toEqual(TEXT_SCALE);
    expect(TEXT_SCALE.default).toBe(1);
  });
});
