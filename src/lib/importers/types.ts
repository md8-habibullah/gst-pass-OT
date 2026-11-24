import type { ItemDraft } from "../../backend/types";

export interface ImportSkip {
  name: string;
  reason: string;
}

export interface ImportResult {
  format: "Bitwarden JSON" | "CSV";
  drafts: ItemDraft[];
  skipped: ImportSkip[];
  /** Things worth telling the person that did not stop an item from importing. */
  warnings: string[];
}

export class ImportError extends Error {}

export const MAX_TITLE = 256;
export const MAX_TEXT = 65_536;

/** Sets a field only when it has content, trimming nothing the person typed (passwords keep their spaces). */
export function put(fields: Record<string, string>, key: string, value: unknown) {
  if (typeof value !== "string") return;
  if (value.trim() === "") return;
  fields[key] = value.length > MAX_TEXT ? value.slice(0, MAX_TEXT) : value;
}

export const blankDraft = (type: ItemDraft["type"], title: string): ItemDraft => ({
  type,
  title: (title.trim() || "(no name)").slice(0, MAX_TITLE),
  favorite: false,
  tags: [],
  folder: null,
  fields: {},
  customFields: [],
});
