// Rules for the Stage 1 credential, mirrored in the Rust core (which stays the authority). Used for live feedback in
// Settings and by the DEV preview backend. Nothing here ever sees a stored credential.
import type { GateMode } from "../backend/types";

export const MIN_CREDENTIAL = 4;
export const MAX_CREDENTIAL = 64;
/** The 16th wrong passcode erases the device. */
export const MAX_ATTEMPTS = 16;
/**
 * The wait after the Nth wrong passcode in a row: `[wrong attempts, seconds]`. Mirrors `gate::LOCK_STEPS` in the core
 * (which stays the authority). Attempt 16 is the last chance, so there is no step for it.
 */
export const LOCK_STEPS: ReadonlyArray<readonly [number, number]> = [
  [3, 90],
  [6, 5 * 60],
  [9, 45 * 60],
  [12, 8 * 3600],
  [15, 3 * 24 * 3600],
];
/** The sum as people read it: real multiplication, division and minus signs. */
export const prettyEquation = (t: string): string => t.replace(/\*/g, "×").replace(/\//g, "÷").replace(/-/g, "−");
export const lockSecondsAfter = (attempts: number): number | null => LOCK_STEPS.find(([n]) => n === attempts)?.[1] ?? null;
/** Input of at most this many characters is a casual math answer and is never checked as a credential. */
export const MATH_MAX_LENGTH = 3;

const chars = (s: string) => [...s];
export const reverseOf = (s: string) => chars(s).reverse().join("");
export const isPalindrome = (s: string) => s.length > 0 && s === reverseOf(s);

const NOUN: Record<GateMode, string> = { pin: "digits", password: "characters", grid: "dots" };
/** "4 digits", "9 characters", "1 dot": how many the person typed, in the words of their passcode type. */
export function typedCount(mode: GateMode, n: number): string {
  const noun = NOUN[mode];
  return `${n} ${n === 1 ? noun.replace(/s$/, "") : noun}`;
}

export function credentialProblem(mode: GateMode, value: string): string | null {
  const n = chars(value).length;
  if (n === 0) return null;
  if (mode === "pin" && !/^\d+$/.test(value)) return "A PIN uses digits only.";
  if (mode === "grid") {
    if (!/^[1-9]+$/.test(value)) return "A pattern uses the dots 1 to 9.";
    if (new Set(chars(value)).size !== n) return "A pattern cannot pass through the same dot twice.";
  }
  if (n < MIN_CREDENTIAL) return `Use at least ${MIN_CREDENTIAL} ${NOUN[mode]}.`;
  if (n > MAX_CREDENTIAL) return `Use at most ${MAX_CREDENTIAL} ${NOUN[mode]}.`;
  return null;
}

/** Problems with the reverse-panic and custom panic settings for a given credential. */
export function panicProblems(mode: GateMode, credential: string, reversePanic: boolean, panicCode: string): { reverse: string | null; panic: string | null } {
  const reverse = reversePanic && credential && isPalindrome(credential)
    ? "This reads the same backwards, so the reversed panic code would be the same as your passcode. Choose one that does not."
    : null;
  let panic: string | null = null;
  if (panicCode) {
    const own = credentialProblem(mode, panicCode);
    if (own) panic = own.replace("Use at least", "The panic code needs at least").replace("Use at most", "The panic code can have at most");
    else if (credential && panicCode === credential) panic = "The panic code must be different from your passcode.";
    else if (credential && reversePanic && panicCode === reverseOf(credential)) panic = "That is your passcode reversed, which already triggers panic.";
  }
  return { reverse, panic };
}
