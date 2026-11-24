// Time zones for the display side only. Everything the app stores and sends is UTC; a zone decides how a UTC instant
// is written for the person (settings page, lock countdown, "last edited"). The list comes from the webview's own
// `Intl` data; a short built-in list covers a webview that cannot enumerate zones.

const FALLBACK = [
  "UTC", "Pacific/Honolulu", "America/Anchorage", "America/Los_Angeles", "America/Denver", "America/Chicago", "America/New_York",
  "America/Halifax", "America/Sao_Paulo", "America/Argentina/Buenos_Aires", "Atlantic/Azores", "Europe/London", "Europe/Dublin",
  "Europe/Lisbon", "Europe/Paris", "Europe/Berlin", "Europe/Madrid", "Europe/Rome", "Europe/Stockholm", "Europe/Athens", "Europe/Istanbul",
  "Europe/Kyiv", "Europe/Moscow", "Africa/Cairo", "Africa/Lagos", "Africa/Johannesburg", "Africa/Nairobi", "Asia/Jerusalem", "Asia/Riyadh",
  "Asia/Tehran", "Asia/Dubai", "Asia/Karachi", "Asia/Kolkata", "Asia/Kathmandu", "Asia/Dhaka", "Asia/Bangkok", "Asia/Jakarta",
  "Asia/Singapore", "Asia/Hong_Kong", "Asia/Shanghai", "Asia/Manila", "Asia/Seoul", "Asia/Tokyo", "Australia/Perth", "Australia/Adelaide",
  "Australia/Sydney", "Pacific/Auckland",
];

export interface ZoneChoice {
  /** IANA name, for example "Asia/Dhaka". */
  id: string;
  /** "UTC+06:00" at the moment asked about. */
  offset: string;
  /** Offset in minutes east of UTC, for sorting. */
  minutes: number;
  /** "Asia / Dhaka" for display, with underscores opened up. */
  label: string;
}

export function isValidZone(zone: string): boolean {
  try {
    new Intl.DateTimeFormat("en", { timeZone: zone });
    return true;
  } catch {
    return false;
  }
}

/** The zone this device reports. Only a starting suggestion: the person still has to choose. */
export function deviceZone(): string {
  try {
    const z = Intl.DateTimeFormat().resolvedOptions().timeZone;
    return z && isValidZone(z) ? z : "UTC";
  } catch {
    return "UTC";
  }
}

/** Minutes east of UTC for `zone` at the instant `at` (so daylight saving is right for that moment). */
export function offsetMinutes(zone: string, at: number): number {
  try {
    const parts = new Intl.DateTimeFormat("en-US", {
      timeZone: zone, hourCycle: "h23", year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric", second: "numeric",
    }).formatToParts(new Date(at));
    const get = (t: string) => Number(parts.find((p) => p.type === t)?.value ?? 0);
    const asUtc = Date.UTC(get("year"), get("month") - 1, get("day"), get("hour"), get("minute"), get("second"));
    return Math.round((asUtc - Math.floor(at / 1000) * 1000) / 60_000);
  } catch {
    return 0;
  }
}

export function offsetLabel(minutes: number): string {
  if (minutes === 0) return "UTC";
  const sign = minutes < 0 ? "-" : "+";
  const abs = Math.abs(minutes);
  return `UTC${sign}${String(Math.floor(abs / 60)).padStart(2, "0")}:${String(abs % 60).padStart(2, "0")}`;
}

export function zoneLabel(id: string): string {
  return id === "UTC" ? "UTC" : id.replace(/_/g, " ").replace(/\//g, " / ");
}

export function allZoneIds(): string[] {
  let ids: string[] = [];
  try {
    const supported = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf?.("timeZone");
    if (supported && supported.length > 0) ids = supported;
  } catch {
    /* an older webview: use the built-in list */
  }
  if (ids.length === 0) ids = FALLBACK;
  return ids.includes("UTC") ? ids : ["UTC", ...ids];
}

/** Every zone with its offset right now, west to east, then by name. */
export function zoneChoices(at = Date.now()): ZoneChoice[] {
  return allZoneIds()
    .filter(isValidZone)
    .map((id) => {
      const minutes = offsetMinutes(id, at);
      return { id, minutes, offset: offsetLabel(minutes), label: zoneLabel(id) };
    })
    .sort((a, b) => a.minutes - b.minutes || a.id.localeCompare(b.id));
}

export function filterZones(all: ZoneChoice[], query: string): ZoneChoice[] {
  const q = query.trim().toLowerCase().replace(/[\s_/]+/g, " ");
  if (!q) return all;
  const words = q.split(" ");
  return all.filter((z) => {
    const hay = `${z.id} ${z.label} ${z.offset}`.toLowerCase().replace(/[_/]+/g, " ");
    return words.every((w) => hay.includes(w));
  });
}
