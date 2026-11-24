import { describe, expect, it } from "vitest";
import { MockGate, makeEquation } from "./mockGate";

// A tiny evaluator independent of the generator's own tree, so the answer is checked from the printed text.
function evaluate(text: string): number {
  const tokens = text.match(/\d+|[()+\-*/]/g)!;
  let i = 0;
  const factor = (): number => { const t = tokens[i++]!; if (t === "(") { const v = expr(); i++; return v; } return Number(t); };
  const term = (): number => { let v = factor(); while (tokens[i] === "*" || tokens[i] === "/") { const o = tokens[i++]; const r = factor(); if (o === "/") { if (v % r !== 0) throw new Error(`inexact ${text}`); v /= r; } else v *= r; } return v; };
  const expr = (): number => { let v = term(); while (tokens[i] === "+" || tokens[i] === "-") { const o = tokens[i++]; const r = term(); v = o === "+" ? v + r : v - r; } return v; };
  return expr();
}

describe("decoy equation", () => {
  it("always has a whole-number answer from 1 to 999 (one to three characters)", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 20_000; i++) {
      const { text, answer } = makeEquation();
      expect(Number.isInteger(answer)).toBe(true);
      expect(answer).toBeGreaterThanOrEqual(1);
      expect(answer).toBeLessThanOrEqual(999);
      expect(String(answer).length).toBeLessThanOrEqual(3);
      expect(evaluate(text)).toBe(answer);
      const nums = text.match(/\d+/g)!.map(Number);
      expect(nums.length === 2 || nums.length === 3).toBe(true);
      expect(nums.every((n) => n >= 1 && n <= 100)).toBe(true);
      for (const op of text.match(/[+\-*/]/g)!) seen.add(op);
    }
    expect([...seen].sort()).toEqual(["*", "+", "-", "/"].sort());
  });
});

describe("gate rules (preview mirror of the core)", () => {
  const gate = () => { const g = new MockGate(); g.seed("pin", "13579", { reversePanic: true, panicCode: "2468" }); return g; };

  it("treats 1 to 3 characters as math and never counts them", () => {
    const g = gate();
    for (let i = 0; i < 40; i++) g.submit("999"); // may be right once in a while, never a failed attempt
    expect(g.status().attemptsLeft).toBeNull();
    expect(g.status().open).toBe(false);
  });
  it("opens on the real credential", () => {
    const g = gate();
    expect(g.submit("13579")).toEqual({ kind: "opened" });
  });
  it("counts wrong credentials and wipes on the 15th", () => {
    const g = gate();
    let wiped = 0; g.onWipe = () => { wiped++; };
    for (let i = 1; i <= 14; i++) expect(g.submit("11111")).toMatchObject({ kind: "failed", attemptsLeft: 15 - i });
    expect(g.submit("11111")).toEqual({ kind: "wiped" });
    expect(wiped).toBe(1);
    expect(g.status().enabled).toBe(false);
  });
  it("wipes silently on the reversed credential and on the custom panic code", () => {
    expect(gate().submit("97531")).toEqual({ kind: "wiped" });
    expect(gate().submit("2468")).toEqual({ kind: "wiped" });
  });
  it("does not treat the reversed credential as panic when the option is off", () => {
    const g = new MockGate(); g.seed("pin", "13579", { reversePanic: false });
    expect(g.submit("97531")).toMatchObject({ kind: "failed" });
  });
  it("refuses a palindrome while reverse panic is on", () => {
    const g = new MockGate();
    expect(() => g.configure({ enabled: true, mode: "pin", credential: "12321", reversePanic: true, panicCode: null, biometric: false, panicButton: false, masterPassword: "x" })).toThrow(/same backwards/);
  });
});
