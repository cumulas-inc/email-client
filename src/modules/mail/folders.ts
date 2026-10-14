import type { Folder } from "../../api/types";

/** The names from the top-level folder down to `folder`. */
export function folderSegments(folders: Folder[], folder: Folder): string[] {
  const parts = [folder.name];
  let parentId = folder.parentId;
  const seen = new Set([folder.id]);
  while (parentId && !seen.has(parentId)) {
    seen.add(parentId);
    const parent = folders.find((f) => f.id === parentId);
    if (!parent) break;
    parts.unshift(parent.name);
    parentId = parent.parentId;
  }
  return parts;
}

/** "Projects / Atlas" style path of a folder. */
export function folderPath(folders: Folder[], folder: Folder): string {
  return folderSegments(folders, folder).join(" / ");
}

/**
 * A path short enough for a menu row: the top folder and the last two, with
 * the levels between folded ("Clients / … / Signed / Scanned originals").
 */
export function compactPath(segments: string[]): string {
  if (segments.length <= 3) return segments.join(" / ");
  return [segments[0], "…", ...segments.slice(-2)].join(" / ");
}

/** Folders of an account a message can be moved to, in tree order. */
export function moveTargets(
  folders: Folder[],
  accountId: string,
  excludeFolderId?: string,
): { folder: Folder; path: string; label: string }[] {
  const own = folders.filter((f) => f.accountId === accountId);
  const out: { folder: Folder; path: string; segments: string[] }[] = [];
  const add = (f: Folder) => {
    const segments = folderSegments(folders, f);
    out.push({ folder: f, path: segments.join(" / "), segments });
  };
  const visit = (parentId: string | null) => {
    for (const f of own.filter((x) => (x.parentId ?? null) === parentId)) {
      if (f.id !== excludeFolderId && f.role !== "drafts" && f.role !== "outbox") add(f);
      visit(f.id);
    }
  };
  visit(null);
  // Folders whose parent isn't synced.
  for (const f of own) {
    if (f.parentId && !own.some((p) => p.id === f.parentId) && f.id !== excludeFolderId) add(f);
  }
  // Menu rows show the compact path; two folders that would read the same
  // keep their full paths.
  const counts = new Map<string, number>();
  for (const t of out) {
    const label = compactPath(t.segments);
    counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  return out.map(({ folder, path, segments }) => {
    const label = compactPath(segments);
    return { folder, path, label: counts.get(label)! > 1 ? path : label };
  });
}

/**
 * How deep the visible folder tree goes: 0 when every folder is at the top
 * level. Folders under a collapsed folder or account don't count.
 */
export function visibleDepth(folders: Folder[], collapsed: Record<string, boolean>): number {
  const ids = new Set(folders.map((f) => f.id));
  const children = new Map<string, Folder[]>();
  for (const f of folders) {
    const parent = f.parentId && ids.has(f.parentId) ? f.parentId : null;
    const key = `${f.accountId}:${parent ?? ""}`;
    children.set(key, [...(children.get(key) ?? []), f]);
  }
  let max = 0;
  const walk = (accountId: string, parentId: string, depth: number) => {
    for (const f of children.get(`${accountId}:${parentId}`) ?? []) {
      max = Math.max(max, depth);
      if (!collapsed[`folder:${f.id}`]) walk(accountId, f.id, depth + 1);
    }
  };
  for (const accountId of new Set(folders.map((f) => f.accountId))) {
    if (!collapsed[`account:${accountId}`]) walk(accountId, "", 0);
  }
  return max;
}
