import { describe, expect, it } from "vitest";
import { checkDisplayName, MAX_NAME_CHARS } from "./profileName";

const have = [{ id: "a", displayName: "Zoë Work" }];

describe("checkDisplayName", () => {
  it("normalizes, trims and collapses whitespace", () => {
    expect(checkDisplayName("  My   vault \t")).toMatchObject({ ok: true, name: "My vault" });
  });
  it("treats differently-composed or full-width forms as the same name", () => {
    expect(checkDisplayName("Zoë Work", have).ok).toBe(false); // decomposed ë
    expect(checkDisplayName("ＺＯË　ＷＯＲＫ", have).ok).toBe(false); // full-width, upper case
    expect(checkDisplayName("zoë work", have).ok).toBe(false);
  });
  it("lets an account keep its own name when renaming", () => {
    expect(checkDisplayName("zoë work", have, "a").ok).toBe(true);
  });
  it("rejects empty, too long and control characters", () => {
    expect(checkDisplayName("   ").ok).toBe(false);
    expect(checkDisplayName("x".repeat(MAX_NAME_CHARS + 1)).ok).toBe(false);
    expect(checkDisplayName("x".repeat(MAX_NAME_CHARS)).ok).toBe(true);
    expect(checkDisplayName("bad\u0007name").ok).toBe(false);
    expect(checkDisplayName("zero​width").ok).toBe(false);
  });
  it("counts characters, not UTF-16 units", () => {
    expect(checkDisplayName("🙂".repeat(MAX_NAME_CHARS)).ok).toBe(true);
  });
});
