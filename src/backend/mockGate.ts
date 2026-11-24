// DEV-ONLY preview of the Stage 1 gate, written to the same rules as the Rust core so the screens can be reviewed in a
// normal browser. Credentials are kept in memory in the clear: this is a design aid and says so; the real gate stores
// Argon2id hashes only. Never imported by production builds (reached only through mock.ts behind import.meta.env.DEV).
import { MATH_MAX_LENGTH, MAX_ATTEMPTS, credentialProblem, panicProblems, reverseOf } from "../lib/gateRules";
import type { GateConfigInput, GateMode, GateOutcome, GateStatus } from "./types";

export interface Equation { text: string; answer: number }

type Node = { n: number } | { op: "+" | "-" | "*" | "/"; l: Node; r: Node };
const OPS = ["+", "-", "*", "/"] as const;

function evalNode(x: Node): number | null {
  if ("n" in x) return x.n;
  const l = evalNode(x.l);
  const r = evalNode(x.r);
  if (l === null || r === null) return null;
  let v: number;
  switch (x.op) {
    case "+": v = l + r; break;
    case "-": v = l - r; break;
    case "*": v = l * r; break;
    default: if (r === 0 || l % r !== 0) return null; v = l / r;
  }
  return v >= 1 && v <= 999 ? v : null; // every step stays a whole number from 1 to 999, never negative or zero
}

function show(x: Node, top = true): string {
  if ("n" in x) return String(x.n);
  const inner = `${show(x.l, false)} ${x.op} ${show(x.r, false)}`;
  return top ? inner : `(${inner})`;
}

/** Two or three numbers from 1 to 100, mixed operators, brackets where needed, every result a whole number from 1 to 999. */
export function makeEquation(rand: () => number = Math.random): Equation {
  const int = (a: number, b: number) => a + Math.floor(rand() * (b - a + 1));
  const leaf = (): Node => ({ n: int(1, 100) });
  const op = () => OPS[int(0, 3)]!;
  for (let i = 0; i < 20_000; i++) {
    const three = rand() < 0.5;
    let tree: Node;
    if (!three) tree = { op: op(), l: leaf(), r: leaf() };
    else if (rand() < 0.5) tree = { op: op(), l: { op: op(), l: leaf(), r: leaf() }, r: leaf() };
    else tree = { op: op(), l: leaf(), r: { op: op(), l: leaf(), r: leaf() } };
    const answer = evalNode(tree);
    if (answer !== null) return { text: show(tree), answer };
  }
  return { text: "7 + 8", answer: 15 }; // unreachable in practice; keeps the function total
}

interface Config {
  enabled: boolean; mode: GateMode; credential: string; reversePanic: boolean; panicCode: string; biometric: boolean; panicButton: boolean;
}

export class MockGate {
  private cfg: Config = { enabled: false, mode: "pin", credential: "", reversePanic: false, panicCode: "", biometric: false, panicButton: false };
  private open = false;
  private attempts = 0;
  private streak = 0;
  private eq = makeEquation();

  onWipe: () => void = () => undefined;

  status(): GateStatus {
    return {
      enabled: this.cfg.enabled, open: !this.cfg.enabled || this.open, mode: this.cfg.mode,
      biometricAvailable: true, biometric: this.cfg.enabled && this.cfg.biometric, panicButton: this.cfg.enabled && this.cfg.panicButton,
      equation: this.cfg.enabled && !this.open ? this.eq.text : null,
      attemptsLeft: this.attempts > 0 ? MAX_ATTEMPTS - this.attempts : null, streak: this.streak,
    };
  }

  private next(): string { this.eq = makeEquation(); return this.eq.text; }

  private wipe(): GateOutcome {
    this.cfg = { enabled: false, mode: "pin", credential: "", reversePanic: false, panicCode: "", biometric: false, panicButton: false };
    this.open = false; this.attempts = 0; this.streak = 0;
    this.onWipe();
    return { kind: "wiped" };
  }

  submit(input: string): GateOutcome {
    if (!this.cfg.enabled || this.open) return { kind: "opened" };
    const n = [...input].length;
    if (n === 0) return { kind: "math", correct: false, equation: this.eq.text, streak: this.streak };
    if (n <= MATH_MAX_LENGTH) {
      const ok = /^\d+$/.test(input) && Number(input) === this.eq.answer;
      this.streak = ok ? this.streak + 1 : this.streak;
      return { kind: "math", correct: ok, equation: this.next(), streak: this.streak };
    }
    const c = this.cfg;
    if (input === c.credential) { this.open = true; this.attempts = 0; return { kind: "opened" }; }
    if ((c.reversePanic && input === reverseOf(c.credential)) || (c.panicCode && input === c.panicCode)) return this.wipe();
    this.attempts += 1;
    if (this.attempts >= MAX_ATTEMPTS) return this.wipe();
    return { kind: "failed", attemptsLeft: MAX_ATTEMPTS - this.attempts, equation: this.next() };
  }

  biometric(): GateOutcome {
    if (this.cfg.enabled && this.cfg.biometric) this.open = true;
    return { kind: "opened" };
  }
  panic(): GateOutcome {
    if (!this.cfg.enabled || !this.cfg.panicButton) throw new Error("The panic trigger is not switched on.");
    return this.wipe();
  }
  close() { this.open = false; this.next(); }
  reset() { this.wipe(); }

  configure(i: GateConfigInput): GateStatus {
    const credential = i.credential ?? this.cfg.credential;
    if (i.enabled) {
      const problem = credentialProblem(i.mode, credential);
      if (!credential || problem) throw new Error(problem ?? "Choose a passcode.");
      const panicCode = i.panicCode === null ? this.cfg.panicCode : i.panicCode;
      const p = panicProblems(i.mode, credential, i.reversePanic, panicCode);
      if (p.reverse || p.panic) throw new Error(p.reverse ?? p.panic ?? "");
      this.cfg = { enabled: true, mode: i.mode, credential, reversePanic: i.reversePanic, panicCode, biometric: i.biometric, panicButton: i.panicButton };
      this.open = true; // you are inside the app while configuring it
    } else {
      this.cfg = { ...this.cfg, enabled: false };
    }
    return this.status();
  }

  /** DEV deep link helper. */
  seed(mode: GateMode, credential: string, extra: Partial<Config> = {}) {
    this.cfg = { enabled: true, mode, credential, reversePanic: true, panicCode: "", biometric: true, panicButton: false, ...extra };
    this.open = false; this.attempts = 0; this.next();
  }
}
