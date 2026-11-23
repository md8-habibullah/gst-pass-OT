import { describe, expect, it } from "vitest";
import { fieldName, formatBytes, groupDigits, hueFor, monogram, plural, recencyGroup, relativeTime } from "./format";
import { ringFraction, ringTone, secondsLeft } from "./totp";

const NOW = new Date("2026-06-15T12:00:00").getTime();
const MIN = 60_000, HOUR = 60 * MIN, DAY = 24 * HOUR;

describe("relativeTime", () => {
  it("uses plain phrases", () => {
    expect(relativeTime(NOW - 10_000, NOW)).toBe("just now");
    expect(relativeTime(NOW - 5 * MIN, NOW)).toBe("5 minutes ago");
    expect(relativeTime(NOW - 3 * HOUR, NOW)).toBe("3 hours ago");
    expect(relativeTime(NOW - DAY, NOW)).toBe("yesterday");
    expect(relativeTime(NOW - 4 * DAY, NOW)).toBe("4 days ago");
  });
  it("switches to dates after 30 days and adds the year across years", () => {
    expect(relativeTime(new Date("2026-03-03T09:00:00").getTime(), NOW)).toBe("Mar 3");
    expect(relativeTime(new Date("2025-03-03T09:00:00").getTime(), NOW)).toBe("Mar 3, 2025");
  });
});

describe("recencyGroup", () => {
  it("buckets by age", () => {
    expect(recencyGroup(NOW - HOUR, NOW)).toBe("Today");
    expect(recencyGroup(NOW - 3 * DAY, NOW)).toBe("Last 7 days");
    expect(recencyGroup(NOW - 20 * DAY, NOW)).toBe("Last 30 days");
    expect(recencyGroup(NOW - 90 * DAY, NOW)).toBe("Older");
  });
});

describe("small helpers", () => {
  it("monogram handles one word, two words and empty", () => {
    expect(monogram("GitHub")).toBe("G");
    expect(monogram("Northline Bank")).toBe("NB");
    expect(monogram("  ")).toBe("?");
    expect(monogram("éclair rouge")).toBe("ÉR");
  });
  it("hueFor is stable and in range", () => {
    expect(hueFor("GitHub")).toBe(hueFor(" github "));
    for (const t of ["a", "Fastmail", "日本語"]) expect(hueFor(t)).toBeGreaterThanOrEqual(0), expect(hueFor(t)).toBeLessThan(360);
  });
  it("groups digits", () => {
    expect(groupDigits("482913")).toBe("482 913");
    expect(groupDigits("48291307")).toBe("4829 1307");
    expect(groupDigits("1234567")).toBe("1234567");
  });
  it("formats bytes and plurals", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(plural(1, "version")).toBe("1 version");
    expect(plural(3, "version")).toBe("3 versions");
  });
  it("names changed fields for the history timeline", () => {
    expect(fieldName("password")).toBe("Password");
    expect(fieldName("totp")).toBe("One-time code");
    expect(fieldName("title")).toBe("Title");
    expect(fieldName("mystery")).toBe("mystery");
  });
});

describe("totp ring maths", () => {
  it("computes fraction and seconds from the validity window", () => {
    expect(ringFraction(NOW + 15_000, 30, NOW)).toBeCloseTo(0.5);
    expect(ringFraction(NOW - 1, 30, NOW)).toBe(0);
    expect(ringFraction(NOW + 90_000, 30, NOW)).toBe(1);
    expect(secondsLeft(NOW + 14_100, NOW)).toBe(15);
    expect(secondsLeft(NOW - 5, NOW)).toBe(0);
  });
  it("shifts tone as time runs out", () => {
    expect(ringTone(29)).toBe("fresh");
    expect(ringTone(10)).toBe("low");
    expect(ringTone(5)).toBe("critical");
    expect(ringTone(0)).toBe("critical");
  });
});
