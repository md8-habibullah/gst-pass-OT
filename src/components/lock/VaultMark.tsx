import { motion } from "motion/react";
import { disguiseTile } from "../../lib/disguise";
import { brand } from "../../state/brand";
import { useStore } from "../../state/store";

/**
 * The Passman mark: a brass ring with a keyhole. The ring has a gap while locked; `open` completes it
 * (the unlock moment, UI_DESIGN_SYSTEM.md §6) and turns it a quarter.
 * With a disguise icon chosen, the mark is that icon instead, so the keyhole never shows on a renamed app.
 */
export function VaultMark({ size = 64, open = false, busy = false }: { size?: number; open?: boolean; busy?: boolean }) {
  const { name, presetId } = useStore(brand, (b) => b);
  const tile = presetId ? disguiseTile(presetId) : undefined;
  if (tile) {
    return (
      <motion.img
        src={tile} width={size} height={size} alt={name} draggable={false}
        animate={{ scale: open ? 1.06 : 1 }} transition={{ duration: 0.24, ease: [0.2, 0.8, 0.2, 1] }}
        className={busy ? "animate-pulse" : ""}
      />
    );
  }
  return (
    <motion.svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      role="img"
      aria-label={name}
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
