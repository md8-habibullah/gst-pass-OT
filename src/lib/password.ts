// Password generation and strength estimation. Randomness comes from crypto.getRandomValues with
// rejection sampling (no modulo bias). The strength meter is guidance only; the core enforces the minimum.

export interface GenOptions {
  length: number;
  lower: boolean;
  upper: boolean;
  digits: boolean;
  symbols: boolean;
  avoidAmbiguous: boolean;
}

export const DEFAULT_GEN: GenOptions = { length: 20, lower: true, upper: true, digits: true, symbols: true, avoidAmbiguous: true };

const SETS = {
  lower: "abcdefghijklmnopqrstuvwxyz",
  upper: "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
  digits: "0123456789",
  symbols: "!@#$%^&*-_=+?",
} as const;
const AMBIGUOUS = /[0O1lI|`'"]/g;

/** Uniform integer in [0, max) without modulo bias. */
export function randomBelow(max: number, rng: (a: Uint32Array) => Uint32Array = (a) => crypto.getRandomValues(a)): number {
  if (!Number.isInteger(max) || max <= 0 || max > 0x1_0000_0000) throw new RangeError("max out of range");
  const limit = Math.floor(0x1_0000_0000 / max) * max;
  const buf = new Uint32Array(1);
  for (;;) {
    const v = rng(buf)[0]!;
    if (v < limit) return v % max;
  }
}

export function generatePassword(opts: GenOptions, rng?: (a: Uint32Array) => Uint32Array): string {
  const classes = (Object.keys(SETS) as (keyof typeof SETS)[])
    .filter((k) => opts[k])
    .map((k) => (opts.avoidAmbiguous ? SETS[k].replace(AMBIGUOUS, "") : SETS[k]));
  if (classes.length === 0) throw new RangeError("Select at least one character type.");
  const length = Math.min(128, Math.max(classes.length, Math.floor(opts.length)));
  const pool = classes.join("");
  const out: string[] = classes.map((c) => c[randomBelow(c.length, rng)]!); // one of each chosen class
  while (out.length < length) out.push(pool[randomBelow(pool.length, rng)]!);
  for (let i = out.length - 1; i > 0; i--) {
    const j = randomBelow(i + 1, rng);
    [out[i], out[j]] = [out[j]!, out[i]!];
  }
  return out.join("");
}

export type CharClass = "letter" | "digit" | "symbol";
export function charClass(ch: string): CharClass {
  if (/\p{L}/u.test(ch)) return "letter";
  if (/\p{N}/u.test(ch)) return "digit";
  return "symbol";
}

export interface Strength {
  bits: number;
  score: 0 | 1 | 2 | 3 | 4;
  label: "Too short" | "Weak" | "Fair" | "Strong" | "Excellent";
}

/** Rough entropy estimate: pool size from character classes, discounted for repeats and runs. */
export function estimateStrength(pw: string): Strength {
  if (pw.length === 0) return { bits: 0, score: 0, label: "Too short" };
  const chars = [...pw];
  let pool = 0;
  if (/\p{Ll}/u.test(pw)) pool += 26;
  if (/\p{Lu}/u.test(pw)) pool += 26;
  if (/\p{N}/u.test(pw)) pool += 10;
  if (/[^\p{L}\p{N}]/u.test(pw)) pool += 33;
  let effective = 0;
  const seen = new Map<string, number>();
  for (let i = 0; i < chars.length; i++) {
    const c = chars[i]!;
    const n = (seen.get(c) ?? 0) + 1;
    seen.set(c, n);
    const prev = chars[i - 1];
    const run = prev !== undefined && (c.codePointAt(0)! - prev.codePointAt(0)! === 1 || c === prev);
    effective += n > 1 ? 0.35 : run ? 0.4 : 1;
  }
  const bits = Math.round(effective * Math.log2(Math.max(pool, 2)));
  if (chars.length < 8) return { bits, score: 0, label: "Too short" };
  if (bits < 40) return { bits, score: 1, label: "Weak" };
  if (bits < 60) return { bits, score: 2, label: "Fair" };
  if (bits < 85) return { bits, score: 3, label: "Strong" };
  return { bits, score: 4, label: "Excellent" };
}
