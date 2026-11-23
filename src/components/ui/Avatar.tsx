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
        background: `linear-gradient(160deg, hsl(${h} 34% ${dim ? 17 : 25}%), hsl(${h} 38% ${dim ? 13 : 19}%))`,
        color: `hsl(${h} 62% ${dim ? 62 : 80}%)`,
        boxShadow: `inset 0 1px 0 hsl(${h} 50% 45% / 0.28), inset 0 0 0 1px hsl(${h} 40% 40% / 0.28)`,
      }}
    >
      {monogram(title)}
    </span>
  );
}
