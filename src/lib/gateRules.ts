// Rules for the Stage 1 credential, mirrored in the Rust core (which stays the authority). Used for live feedback in
// Settings and by the DEV preview backend. Nothing here ever sees a stored credential.
import type { GateMode } from "../backend/types";

export const MIN_CREDENTIAL = 4;
export const MAX_CREDENTIAL = 64;
export const MAX_ATTEMPTS = 15;
/** Input of at most this many characters is a casual math answer and is never checked as a credential. */
export const MATH_MAX_LENGTH = 3;

const chars = (s: string) => [...s];
export const reverseOf = (s: string) => chars(s).reverse().join("");
export const isPalindrome = (s: string) => s.length > 0 && s === reverseOf(s);

const NOUN: Record<GateMode, string> = { pin: "digits", password: "characters", grid: "dots" };

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
