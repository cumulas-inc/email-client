import { describe, expect, it } from "vitest";

import { looksPreformatted, plainTextBlocks, splitFileName, visualLength } from "./text";

describe("splitFileName", () => {
  it("keeps short names whole", () => {
    expect(splitFileName("invoice.pdf")).toEqual({ head: "invoice.pdf", tail: "" });
    expect(splitFileName("winmail.dat")).toEqual({ head: "winmail.dat", tail: "" });
  });

  it("keeps the end of the name and the extension apart from the head", () => {
    const { head, tail } = splitFileName(
      "Northwind_Industries_Holdings_MSA_v14_consolidated_redlines_FINAL_final_v2.docx",
    );
    expect(tail).toBe("L_final_v2.docx");
    expect(head + tail).toBe(
      "Northwind_Industries_Holdings_MSA_v14_consolidated_redlines_FINAL_final_v2.docx",
    );
  });

  it("tells apart names that differ only at the end", () => {
    const a = splitFileName("Quarterly-report-for-the-steering-committee-v1.pdf");
    const b = splitFileName("Quarterly-report-for-the-steering-committee-v2.pdf");
    expect(a.tail).not.toBe(b.tail);
  });

  it("treats a name without an extension as all stem", () => {
    expect(splitFileName("Scan_2026-10-13_14-22-07_IMG_0001_compressed")).toEqual({
      head: "Scan_2026-10-13_14-22-07_IMG_0001_",
      tail: "compressed",
    });
  });

  it("never splits an emoji or a combined character", () => {
    const name = "Family photos 👨‍👩‍👧‍👦 from the summer trip to the coast.jpeg";
    const { head, tail } = splitFileName(name, 3);
    expect(head + tail).toBe(name);
    const { tail: t2 } = splitFileName("Zdjęcia z wakacji nad morzem 👨‍👩‍👧‍👦ab.png", 3);
    expect(t2).toBe("👨‍👩‍👧‍👦ab.png");
  });

  it("keeps right-to-left names intact", () => {
    const name = "تقرير_الموردين_الربع_الثالث_٢٠٢٦.pdf";
    const { head, tail } = splitFileName(name);
    expect(head + tail).toBe(name);
    expect(tail.endsWith(".pdf")).toBe(true);
  });
});

describe("visualLength", () => {
  it("counts wide characters double", () => {
    expect(visualLength("abc")).toBe(3);
    expect(visualLength("受信トレイ")).toBe(10);
    expect(visualLength("🎉")).toBe(1);
  });
});

describe("looksPreformatted", () => {
  it("spots ASCII tables and box drawing", () => {
    expect(looksPreformatted(["+----+-----+", "| a  | b   |", "+----+-----+"])).toBe(true);
    expect(looksPreformatted(["┌──┬──┐", "│ab│cd│"])).toBe(true);
  });

  it("spots aligned columns", () => {
    expect(
      looksPreformatted([
        "Name        Qty    Price",
        "Widget        4    12.00",
        "Gadget       10     3.50",
      ]),
    ).toBe(true);
  });

  it("leaves prose and quotes alone", () => {
    expect(looksPreformatted(["Thanks. Price +2.5% feels light.", "Can we model +4%?"])).toBe(
      false,
    );
    expect(looksPreformatted(["> Fine by me.", ">", "> > Morning only."])).toBe(false);
    expect(looksPreformatted([])).toBe(false);
  });
});

describe("plainTextBlocks", () => {
  it("splits paragraphs, marks quotes and sets the signature apart", () => {
    const blocks = plainTextBlocks(
      [
        "Great, booked.",
        "",
        "On Wed, Jo wrote:",
        "> Fine by me.",
        "",
        "+----+----+",
        "| a  | b  |",
        "+----+----+",
        "",
        "-- ",
        "Ops Reporting Bot",
      ].join("\r\n"),
    );
    expect(blocks).toEqual([
      { kind: "text", lines: [{ text: "Great, booked.", quoted: false }] },
      {
        kind: "text",
        lines: [
          { text: "On Wed, Jo wrote:", quoted: false },
          { text: "> Fine by me.", quoted: true },
        ],
      },
      { kind: "pre", text: "+----+----+\n| a  | b  |\n+----+----+" },
      { kind: "signature", text: "Ops Reporting Bot" },
    ]);
  });

  it("returns nothing for an empty body", () => {
    expect(plainTextBlocks("")).toEqual([]);
    expect(plainTextBlocks("\n\n")).toEqual([]);
  });
});
