// Search results don't move under the reader: while the text and the order
// stay the same, rows already shown keep their places, and rows that come
// later (the servers' hits, "Show more results") join below them.

/** `next` in the order `shown` had, then what `shown` lacked, in `next`'s own order. */
export function keepOrder<T extends { id: string }>(
  shown: readonly string[],
  next: readonly T[],
): T[] {
  const byId = new Map(next.map((item) => [item.id, item]));
  const kept: T[] = [];
  for (const id of shown) {
    const item = byId.get(id);
    if (item) kept.push(item);
  }
  const before = new Set(shown);
  return [...kept, ...next.filter((item) => !before.has(item.id))];
}
