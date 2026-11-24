/** Presentation maths for the live one-time-code ring. The secret never reaches the UI; only code + timing. */

export type RingTone = "fresh" | "low" | "critical";

export function ringFraction(validUntilMs: number, period: number, now: number): number {
  const remaining = Math.max(0, validUntilMs - now);
  return Math.min(1, remaining / (period * 1000));
}

export function secondsLeft(validUntilMs: number, now: number): number {
  return Math.max(0, Math.ceil((validUntilMs - now) / 1000));
}

export function ringTone(secondsRemaining: number): RingTone {
  if (secondsRemaining <= 5) return "critical";
  if (secondsRemaining <= 10) return "low";
  return "fresh";
}

export const TONE_COLOR: Record<RingTone, string> = {
  fresh: "var(--color-verdigris)",
  low: "var(--color-brass-ink)",
  critical: "var(--color-ember)",
};
