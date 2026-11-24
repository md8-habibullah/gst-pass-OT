import type { ReactNode } from "react";
import { isMacPlatform, keycaps } from "../../lib/shortcuts";

/** One key, drawn as a key: a quiet raised face with a heavier lower edge. */
export function Keycap({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <kbd aria-hidden className={`inline-grid h-6 min-w-6 place-items-center rounded-[6px] border border-b-2 border-hairline-strong bg-raised px-1.5 font-mono text-[0.78rem] leading-none text-frost ${className}`}>
      {children}
    </kbd>
  );
}

/** A combo as keycaps: "mod+k" is Ctrl K (Command K on a Mac), "g a" is G then A. The screen-reader text says the same in words. */
export function Combo({ combo }: { combo: string }) {
  const mac = isMacPlatform();
  const caps = keycaps(combo, mac);
  return (
    <span className="inline-flex items-center gap-1" aria-label={caps.join(" ")}>
      {caps.map((c, i) => (c === "then" ? <span key={i} className="px-0.5 text-[0.8em] text-fog">then</span> : <Keycap key={i}>{c}</Keycap>))}
    </span>
  );
}
