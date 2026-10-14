import { describe, expect, it } from "vitest";

import {
  hueFor,
  displayName,
  formatBytes,
  formatListDate,
  formatTimeRange,
  formatCount,
  initials,
} from "./format";

describe("format helpers", () => {
  it("derives initials from names and addresses", () => {
    expect(initials("Priya Raman")).toBe("PR");
    expect(initials("Lucía Fernández")).toBe("LF");
    expect(initials("servicedesk@acme.example")).toBe("SE");
    expect(initials("john.smith@acme.example")).toBe("JS");
    expect(initials("")).toBe("?");
    expect(initials("(no sender)")).toBe("?");
    expect(initials("+1 (206) 555-0145")).toBe("?");
    expect(initials("🦄✨🌈")).toBe("?");
    expect(initials("Maximilian Montgomery III")).toBe("MM");
    expect(initials("@home Wi-Fi support")).toBe("HS");
    expect(initials("נועה כהן (Noa Cohen)")).toBe("נכ");
    expect(initials("Jane Doe <jane@acme.example>")).toBe("JD");
    expect(initials("山田 太郎")).toBe("山太");
  });

  it("falls back to the address for display", () => {
    expect(displayName({ name: null, address: "a@b.example" })).toBe("a@b.example");
    expect(displayName({ name: "Ann", address: "a@b.example" })).toBe("Ann");
    expect(displayName(null)).toBe("(no sender)");
  });

  it("gives stable colors", () => {
    expect(hueFor("A@b.example")).toBe(hueFor("a@B.example"));
  });

  it("formats list dates relative to now", () => {
    const now = new Date(2026, 9, 14, 15, 0);
    expect(formatListDate(new Date(2026, 9, 14, 9, 5).toISOString(), now)).toBe("9:05 AM");
    expect(formatListDate(new Date(2026, 9, 12, 9, 5).toISOString(), now)).toBe("Mon 9:05 AM");
    expect(formatListDate(new Date(2026, 6, 1, 9, 5).toISOString(), now)).toBe("Jul 1");
    expect(formatListDate(new Date(2024, 6, 1, 9, 5).toISOString(), now)).toBe("Jul 1, 2024");
  });

  it("reads all-day ranges from their stored UTC dates", () => {
    // Stored as midnight UTC; must not shift a day in time zones west of UTC.
    expect(formatTimeRange("2026-10-20T00:00:00.000Z", "2026-10-21T00:00:00.000Z", true)).toBe(
      "All day",
    );
    expect(formatTimeRange("2026-10-20T00:00:00.000Z", "2026-10-24T00:00:00.000Z", true)).toBe(
      "Oct 20 – 23",
    );
  });

  it("formats sizes", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(184_320)).toBe("180 KB");
    expect(formatBytes(8_412_220)).toBe("8.0 MB");
  });
});

describe("formatCount", () => {
  it("groups thousands and turns large counts compact", () => {
    expect(formatCount(7)).toBe("7");
    expect(formatCount(1204)).toBe("1,204");
    expect(formatCount(12_345)).toBe("12K");
    expect(formatCount(251_877)).toBe("252K");
  });
});
