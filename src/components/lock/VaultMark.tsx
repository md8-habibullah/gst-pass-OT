import { motion } from "motion/react";

/**
 * The Passman mark: a brass ring with a keyhole. The ring has a gap while locked; `open` completes it
 * (the unlock moment, UI_DESIGN_SYSTEM.md §6) and turns it a quarter.
 */
export function VaultMark({ size = 64, open = false, busy = false }: { size?: number; open?: boolean; busy?: boolean }) {
  return (
    <motion.svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      role="img"
      aria-label="Passman"
      animate={{ rotate: open ? 90 : 0 }}
      transition={{ duration: 0.24, ease: [0.2, 0.8, 0.2, 1] }}
    >
      <circle cx="32" cy="32" r="28" fill="none" stroke="rgb(155 178 185 / 0.16)" strokeWidth="3" />
      <g transform="rotate(-62 32 32)">
        <motion.circle
          cx="32"
          cy="32"
          r="28"
          fill="none"
          stroke="var(--color-brass)"
          strokeWidth="3"
          strokeLinecap="round"
          initial={false}
          animate={{ pathLength: open ? 1 : 0.8 }}
          transition={{ duration: 0.24, ease: [0.2, 0.8, 0.2, 1] }}
          className={busy ? "animate-spin-slow" : ""}
          style={busy ? { transformOrigin: "32px 32px" } : undefined}
        />
      </g>
      <circle cx="32" cy="28" r="5.2" fill="var(--color-brass)" />
      <path d="M29.6 31.5h4.8l1.9 12.2h-8.6z" fill="var(--color-brass)" />
    </motion.svg>
  );
}
