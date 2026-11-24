import { FIELD_SCHEMA, type ItemType } from "../backend/types";
import { offsetMinutes } from "./zones";

const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto" });

// Every stored time is UTC. These formatters only decide how an instant is written for the person: in the zone chosen
// on the first screen (Settings > Date and time), or in the device's own zone until one is chosen.
let zone: string | undefined;
function build(z: string | undefined) {
  return {
    exact: new Intl.DateTimeFormat("en", { dateStyle: "medium", timeStyle: "short", timeZone: z }),
    day: new Intl.DateTimeFormat("en", { month: "short", day: "numeric", timeZone: z }),
    dayYear: new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric", timeZone: z }),
    year: new Intl.DateTimeFormat("en", { year: "numeric", timeZone: z }),
    clock: new Intl.DateTimeFormat("en", { hour: "2-digit", minute: "2-digit", hourCycle: "h23", timeZone: z }),
  };
}
let fmt = build(undefined);

/** Set the zone every time is shown in (`null`: the device's zone). */
export function setDisplayZone(z: string | null): void {
  try {
    const next = z ?? undefined;
    fmt = build(next);
    zone = next;
  } catch {
    // an unknown name: keep what worked before
  }
}
export const displayZone = (): string | undefined => zone;

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
  return (fmt.year.format(then) === fmt.year.format(new Date(now)) ? fmt.day : fmt.dayYear).format(then);
}

export function exactTime(ms: number): string {
  return fmt.exact.format(new Date(ms));
}

/** "14:35" in the display zone. */
export function clockTime(ms: number): string {
  return fmt.clock.format(new Date(ms));
}

/** When a wait ends, for "Try again at 14:35": the time alone on the same day, the date and time after that. */
export function untilLabel(untilMs: number, now = Date.now()): string {
  return startOfDay(untilMs) === startOfDay(now) ? clockTime(untilMs) : `${fmt.day.format(new Date(untilMs))}, ${clockTime(untilMs)}`;
}

/** UTC milliseconds of the start of the calendar day (in the display zone) that contains `at`. */
export function startOfDay(at: number): number {
  const z = zone ?? Intl.DateTimeFormat().resolvedOptions().timeZone;
  const off = offsetMinutes(z, at);
  const localMidnight = Math.floor((at + off * MIN) / DAY) * DAY;
  // The offset can change between midnight and now (daylight saving): ask again at the midnight itself.
  return localMidnight - offsetMinutes(z, localMidnight - off * MIN) * MIN;
}

export type RecencyGroup = "Today" | "Last 7 days" | "Last 30 days" | "Older";

export function recencyGroup(ms: number, now = Date.now()): RecencyGroup {
  if (ms >= startOfDay(now)) return "Today";
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
  title: "Title", favorite: "Favorite", reprompt: "Ask for master password", folder: "Folder", tags: "Tags", customFields: "Custom fields",
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
  archived: "Moved to the archive",
  unarchived: "Taken out of the archive",
} as const;

export const TYPE_NOUN: Record<ItemType, string> = {
  login: "login",
  card: "card",
  identity: "identity",
  sshKey: "SSH key",
  totp: "code",
  secureNote: "note",
  media: "media item",
};

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/**
 * A countdown for the wait after wrong passcodes: "1:25", "44:59", "7:59:59", "2 days 23:59:59". Rounds up, so it never
 * shows 0:00 while the wait is still running, and is empty once it is over.
 */
export function formatCountdown(ms: number): string {
  if (!(ms > 0)) return "";
  const total = Math.ceil(ms / 1000);
  const days = Math.floor(total / 86_400);
  const hours = Math.floor((total % 86_400) / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  if (days > 0) return `${plural(days, "day")} ${hours}:${pad2(minutes)}:${pad2(seconds)}`;
  if (hours > 0) return `${hours}:${pad2(minutes)}:${pad2(seconds)}`;
  return `${minutes}:${pad2(seconds)}`;
}

/** Labels for the "lock after leaving the app" choices. */
export function leaveLabel(seconds: number): string {
  if (seconds === 0) return "Right away";
  if (seconds < 60) return `${seconds} seconds`;
  if (seconds < 3600) return plural(seconds / 60, "minute");
  return plural(seconds / 3600, "hour");
}

/** Labels for the inactivity choices (minutes). */
export function idleLabel(minutes: number): string {
  return minutes >= 60 ? plural(minutes / 60, "hour") : plural(minutes, "minute");
}
