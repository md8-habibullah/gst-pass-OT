import { describe, expect, it } from "vitest";
import { MAX_ATTEMPTS, lockSecondsAfter } from "../lib/gateRules";
import { MockGate, makeEquation, makeLongEquation } from "./mockGate";

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
  it("counts wrong credentials and wipes on the 16th", () => {
    const g = gate();
    let wiped = 0; g.onWipe = () => { wiped++; };
    let t = 1_000_000; g.now = () => t;
    for (let i = 1; i < MAX_ATTEMPTS; i++) {
      expect(g.submit("11111")).toMatchObject({ kind: "failed", attemptsLeft: MAX_ATTEMPTS - i });
      t += 4 * 24 * 3600 * 1000; // let any wait run out
    }
    expect(g.status().finalAttempt).toBe(true);
    expect(g.submit("11111")).toEqual({ kind: "wiped" });
    expect(wiped).toBe(1);
    expect(g.status().enabled).toBe(false);
  });
  it("has the waits 1.5 min, 5 min, 45 min, 8 h and 3 days after the 3rd, 6th, 9th, 12th and 15th wrong attempt", () => {
    expect(MAX_ATTEMPTS).toBe(16);
    const waits = Array.from({ length: 16 }, (_, i) => lockSecondsAfter(i + 1));
    expect(waits.map((w, i) => (w === null ? null : i + 1))).toEqual([null, null, 3, null, null, 6, null, null, 9, null, null, 12, null, null, 15, null]);
    expect([3, 6, 9, 12, 15].map(lockSecondsAfter)).toEqual([90, 300, 2700, 28_800, 259_200]);
  });
  it("refuses attempts during a wait without counting them, and takes them again when it ends", () => {
    const g = gate();
    let t = 5_000_000; g.now = () => t;
    for (let i = 0; i < 3; i++) g.submit("11111");
    expect(g.status().attemptsLeft).toBe(MAX_ATTEMPTS - 3);
    expect(g.status().lockedForMs).toBe(90_000);
    // neither a wrong code nor the right one is looked at while the wait runs
    expect(g.submit("11111")).toEqual({ kind: "locked", retryAfterMs: 90_000, clockPending: false });
    t += 30_000;
    expect(g.submit("13579")).toEqual({ kind: "locked", retryAfterMs: 60_000, clockPending: false });
    expect(g.status().attemptsLeft).toBe(MAX_ATTEMPTS - 3);
    // short input is still just a sum during a wait
    expect(g.submit("1")).toMatchObject({ kind: "math" });
    t += 60_000;
    expect(g.status().lockedForMs).toBeNull();
    expect(g.submit("13579")).toEqual({ kind: "opened" });
  });
  it("warns before the last attempt, and only then", () => {
    const g = gate();
    let t = 0; g.now = () => t;
    for (let i = 1; i <= 14; i++) { g.submit("11111"); t += 4 * 24 * 3600 * 1000; expect(g.status().finalAttempt).toBe(false); }
    g.submit("11111"); // the 15th wrong attempt
    t += 4 * 24 * 3600 * 1000;
    expect(g.status().attemptsLeft).toBe(1);
    expect(g.status().finalAttempt).toBe(true);
  });
  it("looks at nothing during a wait, not even the panic codes", () => {
    const g = gate();
    for (let i = 0; i < 3; i++) g.submit("11111");
    expect(g.submit("97531")).toEqual({ kind: "locked", retryAfterMs: 90_000, clockPending: false }); // refused like any attempt: nothing is looked at
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

describe("erase-everything sum", () => {
  it("is a long sum with a whole-number answer from 100 to 99,999", () => {
    for (let i = 0; i < 3_000; i++) {
      const { text, answer } = makeLongEquation();
      expect(Number.isInteger(answer)).toBe(true);
      expect(answer).toBeGreaterThanOrEqual(100);
      expect(answer).toBeLessThanOrEqual(99_999);
      expect(evaluate(text)).toBe(answer);
    }
  });
  it("erases only on the right answer and uses the sum up either way", () => {
    const g = new MockGate(); g.seed("pin", "13579");
    let wiped = 0; g.onWipe = () => { wiped++; };
    const sum = g.resetChallenge();
    const answer = String(evaluate(sum));
    expect(g.reset(String(Number(answer) + 1))).toBe(false);
    expect(wiped).toBe(0);
    expect(g.reset(answer)).toBe(false); // that sum is gone; the right answer to it no longer counts
    expect(wiped).toBe(0);
    const next = g.resetChallenge();
    expect(g.reset("abc")).toBe(false);
    g.resetChallenge();
    expect(g.reset(String(evaluate(next)))).toBe(false); // the answer to an older sum
    const last = g.resetChallenge();
    expect(g.reset(` ${evaluate(last)} `)).toBe(true);
    expect(wiped).toBe(1);
    expect(g.status().enabled).toBe(false);
  });
  it("needs a sum to be asked for first", () => {
    const g = new MockGate(); g.seed("pin", "13579");
    expect(g.reset("1234")).toBe(false);
    expect(g.status().enabled).toBe(true);
  });
});
