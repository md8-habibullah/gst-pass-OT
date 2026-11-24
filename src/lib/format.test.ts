import { describe, expect, it } from "vitest";
import { clockTime, exactTime, fieldName, formatBytes, formatCountdown, groupDigits, hueFor, idleLabel, leaveLabel, monogram, plural, recencyGroup, relativeTime, setDisplayZone, startOfDay, untilLabel } from "./format";
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

describe("display zone (stored times are UTC; the zone only changes how they are written)", () => {
  const utc = Date.UTC(2026, 10, 24, 20, 30); // 2026-11-24 20:30 UTC
  it("writes the same instant in the chosen zone", () => {
    setDisplayZone("Asia/Dhaka"); // UTC+6, no daylight saving
    expect(clockTime(utc)).toBe("02:30");
    expect(exactTime(utc)).toContain("Nov 25, 2026");
    setDisplayZone("America/New_York"); // UTC-5 in November
    expect(clockTime(utc)).toBe("15:30");
    expect(exactTime(utc)).toContain("Nov 24, 2026");
    setDisplayZone("UTC");
    expect(clockTime(utc)).toBe("20:30");
    setDisplayZone(null);
  });
  it("starts a day at midnight in that zone, also across a daylight-saving change", () => {
    setDisplayZone("Asia/Dhaka");
    expect(startOfDay(utc)).toBe(Date.UTC(2026, 10, 24, 18, 0)); // Nov 25 00:00 in Dhaka
    setDisplayZone("America/New_York");
    expect(startOfDay(Date.UTC(2026, 2, 8, 20, 0))).toBe(Date.UTC(2026, 2, 8, 5, 0)); // the day the clocks go forward: midnight is still UTC-5
    setDisplayZone(null);
  });
  it("buckets Today by the zone's calendar day", () => {
    setDisplayZone("Asia/Dhaka");
    // 19:00 UTC is already the next day in Dhaka (01:00), so 17:00 UTC the same day is "yesterday-ish" by the calendar
    const now = Date.UTC(2026, 10, 24, 19, 0);
    expect(recencyGroup(Date.UTC(2026, 10, 24, 17, 0), now)).toBe("Last 7 days");
    expect(recencyGroup(Date.UTC(2026, 10, 24, 18, 30), now)).toBe("Today");
    setDisplayZone(null);
  });
  it("says when a wait ends: the time today, the date and time later", () => {
    setDisplayZone("UTC");
    const now = Date.UTC(2026, 10, 24, 10, 0);
    expect(untilLabel(Date.UTC(2026, 10, 24, 14, 35), now)).toBe("14:35");
    expect(untilLabel(Date.UTC(2026, 10, 27, 14, 35), now)).toBe("Nov 27, 14:35");
    setDisplayZone(null);
  });
  it("keeps the old zone when given a name it cannot use", () => {
    setDisplayZone("UTC");
    setDisplayZone("Not/AZone");
    expect(clockTime(utc)).toBe("20:30");
    setDisplayZone(null);
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

describe("lock countdown and labels", () => {
  it("counts down in the shortest readable form and rounds up", () => {
    expect(formatCountdown(90_000)).toBe("1:30");
    expect(formatCountdown(89_001)).toBe("1:30");
    expect(formatCountdown(1)).toBe("0:01");
    expect(formatCountdown(59_000)).toBe("0:59");
    expect(formatCountdown(45 * 60_000)).toBe("45:00");
    expect(formatCountdown(8 * 3_600_000)).toBe("8:00:00");
    expect(formatCountdown(3 * 86_400_000)).toBe("3 days 0:00:00");
    expect(formatCountdown(86_400_000 + 3_661_000)).toBe("1 day 1:01:01");
  });
  it("is empty when there is nothing to wait for", () => {
    expect(formatCountdown(0)).toBe("");
    expect(formatCountdown(-5)).toBe("");
    expect(formatCountdown(Number.NaN)).toBe("");
  });
  it("labels the lock choices in plain words", () => {
    expect(leaveLabel(0)).toBe("Right away");
    expect(leaveLabel(15)).toBe("15 seconds");
    expect(leaveLabel(60)).toBe("1 minute");
    expect(leaveLabel(900)).toBe("15 minutes");
    expect(leaveLabel(3600)).toBe("1 hour");
    expect(leaveLabel(14_400)).toBe("4 hours");
    expect(idleLabel(1)).toBe("1 minute");
    expect(idleLabel(30)).toBe("30 minutes");
    expect(idleLabel(60)).toBe("1 hour");
    expect(idleLabel(240)).toBe("4 hours");
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
