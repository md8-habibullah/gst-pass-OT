import { hueFor, monogram } from "../../lib/format";

/** Monogram tile; the colour is derived from the title so each item keeps its identity. */
export function Avatar({ title, size = 40, dim }: { title: string; size?: number; dim?: boolean }) {
  const h = hueFor(title);
  return (
    <span
      aria-hidden
      className="inline-flex shrink-0 items-center justify-center rounded-lg font-semibold tracking-wide select-none"
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.38),
        // the lightness steps come from the palette (tokens.css): deep tiles with light letters, or pale tiles with dark letters
        background: `linear-gradient(160deg, hsl(${h} 34% var(${dim ? "--av-bg1-dim" : "--av-bg1"})), hsl(${h} 38% var(${dim ? "--av-bg2-dim" : "--av-bg2"})))`,
        color: `hsl(${h} 62% var(${dim ? "--av-fg-dim" : "--av-fg"}))`,
        boxShadow: `inset 0 1px 0 hsl(${h} 50% 45% / 0.28), inset 0 0 0 1px hsl(${h} 40% 40% / 0.28)`,
      }}
    >
      {monogram(title)}
    </span>
  );
}
