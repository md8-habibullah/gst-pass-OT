import { FIELD_SCHEMA, type ItemType } from "../backend/types";

const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto" });
const exact = new Intl.DateTimeFormat("en", { dateStyle: "medium", timeStyle: "short" });
const dayFmt = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const dayYearFmt = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" });

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/** "just now", "5 minutes ago", "yesterday", "Mar 3" — plain phrases, no separators. */
export function relativeTime(ms: number, now = Date.now()): string {
  const d = now - ms;
  if (d < 45_000) return "just now";
  if (d < HOUR) return rtf.format(-Math.max(1, Math.round(d / MIN)), "minute");
  if (d < DAY) return rtf.format(-Math.round(d / HOUR), "hour");
  if (d < 30 * DAY) return rtf.format(-Math.round(d / DAY), "day");
  const then = new Date(ms);
  return (then.getFullYear() === new Date(now).getFullYear() ? dayFmt : dayYearFmt).format(then);
}

export function exactTime(ms: number): string {
  return exact.format(new Date(ms));
}

export type RecencyGroup = "Today" | "Last 7 days" | "Last 30 days" | "Older";

export function recencyGroup(ms: number, now = Date.now()): RecencyGroup {
  const startOfToday = new Date(now).setHours(0, 0, 0, 0);
  if (ms >= startOfToday) return "Today";
  if (now - ms < 7 * DAY) return "Last 7 days";
  if (now - ms < 30 * DAY) return "Last 30 days";
  return "Older";
}

export function monogram(title: string): string {
  const words = title.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  const first = [...(words[0] ?? "")][0] ?? "?";
  const second = words.length > 1 ? ([...(words[1] ?? "")][0] ?? "") : "";
  return (first + second).toUpperCase();
}

/** Stable hue (0-359) from a title so each item keeps its colour. */
export function hueFor(title: string): number {
  let h = 2166136261;
  for (const ch of title.trim().toLowerCase()) {
    h ^= ch.codePointAt(0) ?? 0;
    h = Math.imul(h, 16777619);
  }
  return Math.abs(h) % 360;
}

/** "482913" → "482 913"; 8 digits → "4829 1307". */
export function groupDigits(code: string): string {
  if (code.length === 8) return `${code.slice(0, 4)} ${code.slice(4)}`;
  if (code.length === 6) return `${code.slice(0, 3)} ${code.slice(3)}`;
  return code;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10240 ? 1 : 0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

const NAMES: Record<string, string> = {
  title: "Title", favorite: "Favorite", folder: "Folder", tags: "Tags", customFields: "Custom fields",
};
for (const specs of Object.values(FIELD_SCHEMA)) for (const s of specs) NAMES[s.key] = s.label;

/** Human names for changed-field keys in version history. */
export function fieldName(key: string): string {
  return NAMES[key] ?? key;
}

export const EVENT_LABEL = {
  created: "Created",
  edited: "Edited",
  trashed: "Moved to trash",
  restored: "Restored from trash",
  restoredFromVersion: "Restored from an earlier version",
} as const;

export const TYPE_NOUN: Record<ItemType, string> = {
  login: "login",
  totp: "code",
  secureNote: "note",
  media: "media item",
};

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}
