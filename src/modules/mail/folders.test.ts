import { describe, expect, it } from "vitest";

import type { Folder } from "../../api/types";
import { compactPath, moveTargets, visibleDepth } from "./folders";

function folder(id: string, name: string, parentId: string | null = null, accountId = "a"): Folder {
  return {
    id,
    accountId,
    name,
    parentId,
    role: "other",
    unreadCount: 0,
    totalCount: 0,
  } as Folder;
}

const DEEP = ["Clients", "Northwind", "2026", "Q3", "Contracts", "Amendments", "Signed", "Scans"];
const deep = DEEP.map((name, i) => folder(`d${i}`, name, i === 0 ? null : `d${i - 1}`));

describe("compactPath", () => {
  it("keeps short paths and folds the middle of long ones", () => {
    expect(compactPath(["Projects", "Atlas"])).toBe("Projects / Atlas");
    expect(compactPath(["A", "B", "C"])).toBe("A / B / C");
    expect(compactPath(DEEP)).toBe("Clients / … / Signed / Scans");
  });
});

describe("moveTargets", () => {
  it("labels deep folders with a compact path and keeps the full one", () => {
    const targets = moveTargets([folder("inbox", "Inbox"), ...deep], "a");
    const scans = targets.find((t) => t.folder.id === "d7")!;
    expect(scans.label).toBe("Clients / … / Signed / Scans");
    expect(scans.path).toBe(DEEP.join(" / "));
  });

  it("falls back to the full path when compact labels would collide", () => {
    const folders = [
      folder("x", "X"),
      folder("x1", "One", "x"),
      folder("x2", "Two", "x1"),
      folder("x3", "Same", "x2"),
      folder("x4", "Leaf", "x3"),
      folder("y1", "Other", "x"),
      folder("y2", "Same", "y1"),
      folder("y3", "Leaf", "y2"),
    ];
    // "X / One / Two / Same / Leaf" and "X / Other / Same / Leaf" both fold
    // to "X / … / Same / Leaf".
    const labels = moveTargets(folders, "a")
      .filter((t) => t.folder.name === "Leaf")
      .map((t) => t.label);
    expect(labels).toEqual(["X / One / Two / Same / Leaf", "X / Other / Same / Leaf"]);
  });
});

describe("visibleDepth", () => {
  it("counts only levels that are on screen", () => {
    expect(visibleDepth([folder("i", "Inbox")], {})).toBe(0);
    expect(visibleDepth(deep, {})).toBe(7);
    expect(visibleDepth(deep, { "folder:d3": true })).toBe(3);
    expect(visibleDepth(deep, { "account:a": true })).toBe(0);
  });

  it("puts orphans at the top level", () => {
    expect(visibleDepth([folder("o", "Orphan", "missing")], {})).toBe(0);
  });
});
