import { describe, expect, it } from "vitest";
import { canGenerate, charClass, DEFAULT_GEN, estimateStrength, GEN_LENGTH_MAX, GEN_LENGTH_MIN, generatePassword, randomBelow, savedOrDefault, type Rng } from "./password";

/** Deterministic rng that replays a fixed list of 32-bit values. */
function replay(values: number[]) {
  let i = 0;
  const rng: Rng = (a) => {
    a[0] = values[i++ % values.length]!;
    return a;
  };
  return rng;
}

describe("randomBelow", () => {
  it("rejects values in the biased tail", () => {
    // max 10: limit = floor(2^32 / 10) * 10 = 4_294_967_290; anything >= limit must be redrawn
    expect(randomBelow(10, replay([4_294_967_295, 4_294_967_290, 7]))).toBe(7);
  });
  it("is uniform within tolerance", () => {
    const counts = new Array(6).fill(0) as number[];
    for (let i = 0; i < 60_000; i++) counts[randomBelow(6)]!++;
    for (const c of counts) expect(Math.abs(c - 10_000)).toBeLessThan(500);
  });
  it("validates its argument", () => {
    expect(() => randomBelow(0)).toThrow(RangeError);
    expect(() => randomBelow(1.5)).toThrow(RangeError);
  });
});

describe("generatePassword", () => {
  it("honours length and always includes each chosen class", () => {
    for (let n = 0; n < 200; n++) {
      const pw = generatePassword({ ...DEFAULT_GEN, length: 12 });
      expect(pw).toHaveLength(12);
      expect(pw).toMatch(/[a-z]/);
      expect(pw).toMatch(/[A-Z]/);
      expect(pw).toMatch(/[0-9]/);
      expect(pw).toMatch(/[!@#$%^&*\-_=+?]/);
    }
  });
  it("avoids ambiguous characters when asked", () => {
    for (let n = 0; n < 200; n++) expect(generatePassword({ ...DEFAULT_GEN, length: 64 })).not.toMatch(/[0O1lI|`'"]/);
  });
  it("can use a single class and respects the minimum length", () => {
    const pw = generatePassword({ length: 1, lower: false, upper: false, digits: true, symbols: false, avoidAmbiguous: false });
    expect(pw).toMatch(/^[0-9]$/);
    expect(generatePassword({ ...DEFAULT_GEN, length: 2 })).toHaveLength(4); // never fewer chars than classes
  });
  it("caps length at 128 and refuses an empty selection", () => {
    expect(generatePassword({ ...DEFAULT_GEN, length: 500 })).toHaveLength(128);
    expect(() => generatePassword({ length: 16, lower: false, upper: false, digits: false, symbols: false, avoidAmbiguous: true })).toThrow(RangeError);
  });
  it("produces different passwords", () => {
    expect(new Set(Array.from({ length: 50 }, () => generatePassword(DEFAULT_GEN))).size).toBe(50);
  });
});

describe("estimateStrength", () => {
  it("flags short passwords regardless of variety", () => {
    expect(estimateStrength("aB3$").label).toBe("Too short");
    expect(estimateStrength("").score).toBe(0);
  });
  it("orders weak < fair < strong < excellent", () => {
    const weak = estimateStrength("password1").score;
    const fair = estimateStrength("Tr0ub4dor&3").score;
    const strong = estimateStrength("kX9#mQ2vTz!8pLd4").score;
    const great = estimateStrength(generatePassword({ ...DEFAULT_GEN, length: 32 })).score;
    expect(weak).toBeLessThan(strong);
    expect(fair).toBeLessThanOrEqual(strong);
    expect(great).toBe(4);
  });
  it("penalizes repetition and runs", () => {
    expect(estimateStrength("aaaaaaaaaaaaaaaa").bits).toBeLessThan(estimateStrength("abcdefghijklmnop").bits);
    expect(estimateStrength("abcdefghijklmnop").bits).toBeLessThan(estimateStrength("qzvkhwmdxjnbtgfp").bits);
  });
});

describe("charClass", () => {
  it("classifies for coloured rendering", () => {
    expect(charClass("a")).toBe("letter");
    expect(charClass("é")).toBe("letter");
    expect(charClass("7")).toBe("digit");
    expect(charClass("#")).toBe("symbol");
  });
});

describe("savedOrDefault", () => {
  it("shows the defaults when nothing is saved", () => {
    expect(savedOrDefault(null)).toEqual(DEFAULT_GEN);
    expect(savedOrDefault(undefined)).toEqual(DEFAULT_GEN);
    expect(DEFAULT_GEN.length).toBe(20);
  });
  it("shows what was saved, such as a length of 30", () => {
    const saved = { length: 30, lower: true, upper: false, digits: true, symbols: false, avoidAmbiguous: false };
    expect(savedOrDefault(saved)).toEqual(saved);
  });
  it("hands out a copy, never the object it was given", () => {
    const saved = { ...DEFAULT_GEN, length: 30 };
    expect(savedOrDefault(saved)).not.toBe(saved);
    expect(savedOrDefault(null)).not.toBe(DEFAULT_GEN);
  });
  it("moves a length outside the slider to its nearest end", () => {
    expect(savedOrDefault({ ...DEFAULT_GEN, length: 4 }).length).toBe(GEN_LENGTH_MIN);
    expect(savedOrDefault({ ...DEFAULT_GEN, length: 128 }).length).toBe(GEN_LENGTH_MAX);
    expect(savedOrDefault({ ...DEFAULT_GEN, length: Number.NaN }).length).toBe(DEFAULT_GEN.length);
  });
  it("falls back to the defaults when no character type is left on", () => {
    expect(savedOrDefault({ ...DEFAULT_GEN, length: 30, lower: false, upper: false, digits: false, symbols: false })).toEqual(DEFAULT_GEN);
  });
});

describe("canGenerate", () => {
  it("needs at least one character type", () => {
    expect(canGenerate(DEFAULT_GEN)).toBe(true);
    expect(canGenerate({ lower: false, upper: false, digits: false, symbols: true })).toBe(true);
    expect(canGenerate({ lower: false, upper: false, digits: false, symbols: false })).toBe(false);
  });
});
