// Folder paths. A folder is a path of names joined by "/": "Work/Clients" is the folder "Clients" inside "Work", the same way
// Bitwarden writes nested folders, so an import keeps its structure. Items carry their folder path; the vault also keeps the
// list of folders that were created on purpose (so an empty folder can exist). Everything here is plain text work on
// summaries: no secret is involved.
import type { ItemSummary } from "../backend";

export const FOLDER_SEP = "/";
/** Limits the core enforces too (model.rs): a name part, a whole path, and how deep it nests. */
export const MAX_FOLDER_PART = 64;
export const MAX_FOLDER_PATH = 128;
export const MAX_FOLDER_DEPTH = 6;

/** Splits, trims and drops empty parts: " Work // Clients/ " is "Work/Clients". `null` when nothing is left. */
export function cleanFolder(raw: string | null | undefined): string | null {
  const parts = (raw ?? "")
    .split(FOLDER_SEP)
    .map((p) => p.trim())
    .filter(Boolean);
  return parts.length ? parts.join(FOLDER_SEP) : null;
}

/** Why a folder path cannot be used, in words; `null` when it can. */
export function folderProblem(raw: string): string | null {
  const path = cleanFolder(raw);
  if (!path) return "Give the folder a name.";
  const parts = path.split(FOLDER_SEP);
  if (parts.length > MAX_FOLDER_DEPTH) return `Folders can be nested ${MAX_FOLDER_DEPTH} levels deep at most.`;
  if (parts.some((p) => [...p].length > MAX_FOLDER_PART)) return `A folder name can have ${MAX_FOLDER_PART} characters at most.`;
  if ([...path].length > MAX_FOLDER_PATH) return `A folder path can have ${MAX_FOLDER_PATH} characters at most.`;
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(path)) return "A folder name cannot contain control characters.";
  return null;
}

export const folderName = (path: string): string => path.split(FOLDER_SEP).pop() ?? path;

export function parentFolder(path: string): string | null {
  const at = path.lastIndexOf(FOLDER_SEP);
  return at < 0 ? null : path.slice(0, at);
}

export const joinFolder = (parent: string | null, name: string): string => (parent ? `${parent}${FOLDER_SEP}${name.trim()}` : name.trim());

/** Is `path` the folder `ancestor` or something inside it? */
export const isInside = (path: string, ancestor: string): boolean => path === ancestor || path.startsWith(ancestor + FOLDER_SEP);

/** Every folder that exists: the ones made on purpose, the ones items sit in, and every parent of those. Sorted by name. */
export function allFolders(explicit: readonly string[], items: readonly Pick<ItemSummary, "folder">[]): string[] {
  const set = new Set<string>();
  const add = (raw: string | null) => {
    const path = cleanFolder(raw);
    if (!path) return;
    const parts = path.split(FOLDER_SEP);
    for (let i = 1; i <= parts.length; i++) set.add(parts.slice(0, i).join(FOLDER_SEP));
  };
  explicit.forEach(add);
  items.forEach((i) => add(i.folder));
  return [...set].sort(byName);
}

const byName = (a: string, b: string): number => a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });

export interface FolderNode {
  path: string;
  name: string;
  /** Items directly in this folder. */
  items: number;
  /** Items in this folder and in every folder inside it. */
  total: number;
  /** Folders directly inside it. */
  folders: number;
}

/** The folders directly inside `parent` (`null` is the top level), with their counts, sorted by name. */
export function childFolders(all: readonly string[], items: readonly Pick<ItemSummary, "folder">[], parent: string | null): FolderNode[] {
  const prefix = parent ? parent + FOLDER_SEP : "";
  const direct = all.filter((p) => p.startsWith(prefix) && !p.slice(prefix.length).includes(FOLDER_SEP) && p.length > prefix.length);
  return direct.map((path) => {
    const inside = items.filter((i) => i.folder && isInside(i.folder, path));
    return {
      path,
      name: folderName(path),
      items: items.filter((i) => i.folder === path).length,
      total: inside.length,
      folders: all.filter((p) => parentFolder(p) === path).length,
    };
  });
}

/** The items sitting directly in `folder` (`null` is the top level: items with no folder). */
export const itemsIn = <T extends Pick<ItemSummary, "folder">>(items: readonly T[], folder: string | null): T[] =>
  items.filter((i) => (cleanFolder(i.folder) ?? null) === folder);

/** The trail of folders from the top to `path`, for a breadcrumb. */
export function trail(path: string | null): { path: string; name: string }[] {
  if (!path) return [];
  const parts = path.split(FOLDER_SEP);
  return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join(FOLDER_SEP) }));
}
