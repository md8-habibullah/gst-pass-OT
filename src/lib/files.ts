import type { FileEntry } from "../backend/types";

/** Per-file limit, enforced in the UI and again in the core. */
export const MAX_FILE_BYTES = 52_428_800;
export const MAX_FILE_LABEL = "50 MB";

export type PreviewKind = "image" | "text" | "pdf" | "none";

const IMAGE = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "avif"]);
const TEXT = new Set(["txt", "md", "json", "csv", "log", "xml", "yml", "yaml", "ini", "conf", "toml", "env", "pem", "pub", "key", "asc", "sh", "ts", "js", "py", "rs", "html", "css", "sql"]);

const extOf = (name: string) => name.toLowerCase().split(".").pop() ?? "";

export function previewKind(f: Pick<FileEntry, "name" | "mime">): PreviewKind {
  const ext = extOf(f.name);
  if (f.mime?.startsWith("image/") || IMAGE.has(ext)) return "image";
  if (f.mime === "application/pdf" || ext === "pdf") return "pdf";
  if (f.mime?.startsWith("text/") || TEXT.has(ext)) return "text";
  return "none";
}

export function mimeFor(name: string): string | null {
  const ext = extOf(name);
  const map: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", svg: "image/svg+xml", pdf: "application/pdf", txt: "text/plain", md: "text/markdown", json: "application/json", csv: "text/csv", zip: "application/zip" };
  return map[ext] ?? null;
}
