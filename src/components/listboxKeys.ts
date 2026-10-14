/**
 * Index a listbox should move to for a navigation key (arrows, Home/End,
 * PageUp/PageDown), or null when the key does not navigate. `index` is -1
 * when nothing is selected yet.
 */
export function listboxTarget(key: string, index: number, count: number, page = 10): number | null {
  if (count === 0) return null;
  const last = count - 1;
  switch (key) {
    case "ArrowDown":
      return index < 0 ? 0 : Math.min(index + 1, last);
    case "ArrowUp":
      return index < 0 ? last : Math.max(index - 1, 0);
    case "Home":
      return 0;
    case "End":
      return last;
    case "PageDown":
      return Math.min(Math.max(index, 0) + page, last);
    case "PageUp":
      return Math.max(index - page, 0);
    default:
      return null;
  }
}

/** Scrolls the element with the given id into view if it exists (no-op in jsdom). */
export function scrollIdIntoView(id: string | null | undefined): void {
  if (!id) return;
  const el = document.getElementById(id);
  if (el && typeof el.scrollIntoView === "function") el.scrollIntoView({ block: "nearest" });
}
