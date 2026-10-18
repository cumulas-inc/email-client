import { describe, expect, it } from "vitest";

import { keepOrder } from "./keepOrder";

const items = (...ids: string[]) => ids.map((id) => ({ id, n: id.charCodeAt(0) }));
const ids = (list: { id: string }[]) => list.map((i) => i.id);

describe("keepOrder", () => {
  it("keeps the rows shown where they were", () => {
    expect(ids(keepOrder(["b", "a", "c"], items("a", "b", "c")))).toEqual(["b", "a", "c"]);
  });

  it("adds new rows below, in their own order", () => {
    expect(ids(keepOrder(["b", "a"], items("d", "a", "c", "b")))).toEqual(["b", "a", "d", "c"]);
  });

  it("drops rows that went, and takes the new version of each", () => {
    const next = [
      { id: "a", n: 1 },
      { id: "c", n: 3 },
    ];
    const kept = keepOrder(["b", "c", "a"], next);
    expect(ids(kept)).toEqual(["c", "a"]);
    expect(kept[0]).toBe(next[1]);
    expect(keepOrder([], next)).toEqual(next);
    expect(keepOrder(["a"], [])).toEqual([]);
  });
});
