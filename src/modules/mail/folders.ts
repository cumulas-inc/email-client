import type { Folder } from "../../api/types";

/** "Projects / Atlas" style path of a folder. */
export function folderPath(folders: Folder[], folder: Folder): string {
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
  return parts.join(" / ");
}

/** Folders of an account a message can be moved to, in tree order. */
export function moveTargets(
  folders: Folder[],
  accountId: string,
  excludeFolderId?: string,
): { folder: Folder; path: string }[] {
  const own = folders.filter((f) => f.accountId === accountId);
  const out: { folder: Folder; path: string }[] = [];
  const visit = (parentId: string | null) => {
    for (const f of own.filter((x) => (x.parentId ?? null) === parentId)) {
      if (f.id !== excludeFolderId && f.role !== "drafts" && f.role !== "outbox") {
        out.push({ folder: f, path: folderPath(folders, f) });
      }
      visit(f.id);
    }
  };
  visit(null);
  // Folders whose parent isn't synced.
  for (const f of own) {
    if (f.parentId && !own.some((p) => p.id === f.parentId) && f.id !== excludeFolderId) {
      out.push({ folder: f, path: folderPath(folders, f) });
    }
  }
  return out;
}
