import { motion } from "motion/react";
import type { ReactNode } from "react";

/**
 * Countdown ring. The arc shrinks linearly from `remainingMs/totalMs` to 0 over `remainingMs`; changing
 * `windowKey` restarts it (a new TOTP window, a new reveal, a new clipboard copy). Colour is set by the caller
 * (tone) and never the only signal: callers also show the seconds as text.
 */
export function CountdownRing({
  totalMs, remainingMs, windowKey, color, size = 44, stroke = 3.5, children, label,
}: {
  totalMs: number;
  remainingMs: number;
  windowKey: string | number;
  color: string;
  size?: number;
  stroke?: number;
  children?: ReactNode;
  label?: string;
}) {
  const r = (size - stroke) / 2;
  const start = Math.max(0, Math.min(1, remainingMs / totalMs));
  return (
    <span role={label ? "img" : undefined} aria-label={label} className="relative inline-grid shrink-0 place-items-center" style={{ width: size, height: size }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="absolute inset-0 -rotate-90" aria-hidden>
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="rgb(155 178 185 / 0.16)" strokeWidth={stroke} />
        <motion.circle
          key={windowKey}
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          strokeWidth={stroke}
          strokeLinecap="round"
          initial={{ pathLength: start }}
          animate={{ pathLength: 0 }}
          transition={{ duration: Math.max(0.05, remainingMs / 1000), ease: "linear" }}
          style={{ stroke: color, transition: "stroke 300ms" }}
        />
      </svg>
      {children}
    </span>
  );
}
