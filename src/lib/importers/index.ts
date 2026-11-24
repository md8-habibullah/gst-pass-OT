import { looksLikeBitwarden, parseBitwarden } from "./bitwarden";
import { looksLikeCsv, parseCsv } from "./csv";
import { ImportError, type ImportResult } from "./types";

export type { ImportResult, ImportSkip } from "./types";
export { ImportError } from "./types";

/** Detects the format from the content (not the file name) and parses it. */
export function parseImport(text: string): ImportResult {
  if (!text.trim()) throw new ImportError("That file is empty.");
  if (looksLikeBitwarden(text)) return parseBitwarden(text);
  if (looksLikeCsv(text)) return parseCsv(text);
  throw new ImportError("Passman can import Bitwarden JSON exports and CSV files with a password column. This file is neither.");
}
