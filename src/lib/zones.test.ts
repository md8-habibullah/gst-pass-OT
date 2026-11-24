import { describe, expect, it } from "vitest";
import { allZoneIds, deviceZone, filterZones, isValidZone, offsetLabel, offsetMinutes, zoneChoices, zoneLabel } from "./zones";

describe("zones", () => {
  const at = Date.UTC(2026, 10, 24, 12, 0);
  it("finds offsets for a moment, so daylight saving is right for that date", () => {
    expect(offsetMinutes("Asia/Dhaka", at)).toBe(360);
    expect(offsetMinutes("UTC", at)).toBe(0);
    expect(offsetMinutes("America/New_York", at)).toBe(-300);
    expect(offsetMinutes("America/New_York", Date.UTC(2026, 6, 1))).toBe(-240);
    expect(offsetMinutes("Asia/Kolkata", at)).toBe(330);
    expect(offsetMinutes("Asia/Kathmandu", at)).toBe(345);
  });
  it("writes offsets the way people read them", () => {
    expect(offsetLabel(0)).toBe("UTC");
    expect(offsetLabel(360)).toBe("UTC+06:00");
    expect(offsetLabel(-300)).toBe("UTC-05:00");
    expect(offsetLabel(345)).toBe("UTC+05:45");
    expect(zoneLabel("America/Argentina/Buenos_Aires")).toBe("America / Argentina / Buenos Aires");
  });
  it("accepts real zones only", () => {
    expect(isValidZone("Asia/Dhaka")).toBe(true);
    expect(isValidZone("UTC")).toBe(true);
    expect(isValidZone("Mars/Olympus")).toBe(false);
    expect(isValidZone("")).toBe(false);
    expect(isValidZone(deviceZone())).toBe(true);
  });
  it("lists zones west to east and always offers UTC", () => {
    expect(allZoneIds()).toContain("UTC");
    const all = zoneChoices(at);
    expect(all.length).toBeGreaterThan(30);
    for (let i = 1; i < all.length; i++) expect(all[i]!.minutes).toBeGreaterThanOrEqual(all[i - 1]!.minutes);
    expect(all.find((z) => z.id === "Asia/Dhaka")?.offset).toBe("UTC+06:00");
  });
  it("filters by any part of the name or the offset", () => {
    const all = zoneChoices(at);
    expect(filterZones(all, "dhaka").map((z) => z.id)).toContain("Asia/Dhaka");
    expect(filterZones(all, "new york").map((z) => z.id)).toContain("America/New_York");
    expect(filterZones(all, "utc+06:00").map((z) => z.id)).toContain("Asia/Dhaka");
    expect(filterZones(all, "  ")).toHaveLength(all.length);
    expect(filterZones(all, "zzzz")).toHaveLength(0);
  });
});
