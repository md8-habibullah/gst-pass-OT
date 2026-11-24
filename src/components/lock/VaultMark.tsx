import { useId } from "react";
import { motion } from "motion/react";
import { disguiseTile } from "../../lib/disguise";
import { brand } from "../../state/brand";
import { DUR, tween } from "../../lib/motion";
import { useStore } from "../../state/store";

/**
 * The Hexis mark: two solid pillars and a crossbar of two slanted layers, a silver one over a glass cyan one. The same
 * drawing as `src/assets/logo-mark.svg` (a test keeps the two in step). The unlock moment (UI_DESIGN_SYSTEM.md §6) closes
 * the gap between the layers. `busy` marks the screen as working for assistive tech; the mark itself stays still.
 * With a disguise icon chosen, the mark is that icon instead, so the Hexis H never shows on a renamed app.
 */
export function VaultMark({ size = 64, open = false, busy = false }: { size?: number; open?: boolean; busy?: boolean }) {
  const { name, presetId } = useStore(brand, (b) => b);
  const uid = useId().replace(/[^a-zA-Z0-9]/g, "");
  const tile = presetId ? disguiseTile(presetId) : undefined;
  if (tile) {
    return (
      <motion.img
        src={tile} width={size} height={size} alt={name} draggable={false}
        animate={{ scale: open ? 1.06 : 1 }} transition={tween(DUR.base)}
        aria-busy={busy || undefined}
      />
    );
  }
  const face = `${uid}face`, side = `${uid}side`, accent = `${uid}accent`;
  return (
    <motion.svg
      width={size}
      height={size}
      viewBox="0 0 256 256"
      role="img"
      aria-label={name}
      animate={{ scale: open ? 1.05 : 1 }}
      transition={tween(DUR.base)}
      aria-busy={busy || undefined}
    >
      <defs>
        <linearGradient id={face} x1="0" y1="0" x2="0" y2="1"><stop offset="0" style={{ stopColor: "var(--mark-face-top, #FFFFFF)" }} /><stop offset="1" style={{ stopColor: "var(--mark-face-bottom, #CBD5E1)" }} /></linearGradient>
        <linearGradient id={side} x1="0" y1="0" x2="0" y2="1"><stop offset="0" style={{ stopColor: "var(--mark-side-top, #94A3B8)" }} /><stop offset="1" style={{ stopColor: "var(--mark-side-bottom, #475569)" }} /></linearGradient>
        <linearGradient id={accent} x1="0" y1="0" x2="1" y2="1"><stop offset="0" stopColor="#38BDF8" /><stop offset="1" stopColor="#818CF8" /></linearGradient>
      </defs>
      <g transform="translate(-6 4)">
        <rect x="40" y="48" width="44" height="160" fill={`url(#${face})`} />
        <polygon points="40,48 84,48 96,40 52,40" style={{ fill: "var(--mark-cap, #FFFFFF)" }} />
        <polygon points="84,48 96,40 96,200 84,208" fill={`url(#${side})`} />
        <rect x="172" y="48" width="44" height="160" fill={`url(#${face})`} />
        <polygon points="172,48 216,48 228,40 184,40" style={{ fill: "var(--mark-cap, #FFFFFF)" }} />
        <polygon points="216,48 228,40 228,200 216,208" fill={`url(#${side})`} />
        <motion.g animate={{ y: open ? -14 : 0 }} transition={tween(DUR.base)}>
          <polygon points="84,140 172,124 172,160 84,176" fill={`url(#${accent})`} fillOpacity="0.92" />
          <polygon points="84,140 172,124 172,127 84,143" fill="#FFFFFF" fillOpacity="0.45" />
        </motion.g>
        <polygon points="84,84 172,68 172,108 84,124" fill={`url(#${face})`} />
      </g>
    </motion.svg>
  );
}
