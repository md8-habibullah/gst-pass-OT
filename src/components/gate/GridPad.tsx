import { useRef, useState, type PointerEvent } from "react";
import { Delete, CornerDownLeft } from "lucide-react";

interface Pt { x: number; y: number }

/**
 * The numbered 3x3 grid: a keypad and a pattern lock in one.
 * A tap on a number types that digit; dragging across two or more numbers draws a line and, on release, sends the digits
 * in the order they were touched (for example 1, 2, 5, 8, 9 becomes "12589"). The core decides what the input means.
 */
export function GridPad({ disabled, onDigit, onBackspace, onPattern, onSubmit }: {
  disabled?: boolean;
  onDigit: (d: string) => void;
  onBackspace: () => void;
  onPattern: (digits: string) => void;
  onSubmit: () => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [path, setPath] = useState<string[]>([]);
  const [trail, setTrail] = useState<Pt[]>([]);
  const [cursor, setCursor] = useState<Pt | null>(null);
  const drag = useRef<{ digits: string[]; pts: Pt[] } | null>(null);

  const centreOf = (el: Element): Pt => {
    const b = box.current!.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    return { x: r.left - b.left + r.width / 2, y: r.top - b.top + r.height / 2 };
  };
  const nodeAt = (x: number, y: number) => document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-node]") ?? null;

  function down(e: PointerEvent<HTMLDivElement>) {
    if (disabled) return;
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-node]");
    if (!el) return;
    box.current!.setPointerCapture(e.pointerId);
    const d = el.dataset.node!;
    drag.current = { digits: [d], pts: [centreOf(el)] };
    setPath([d]); setTrail([centreOf(el)]); setCursor(null);
  }
  function move(e: PointerEvent<HTMLDivElement>) {
    const cur = drag.current;
    if (!cur) return;
    const b = box.current!.getBoundingClientRect();
    setCursor({ x: e.clientX - b.left, y: e.clientY - b.top });
    const el = nodeAt(e.clientX, e.clientY);
    const d = el?.dataset.node;
    if (el && d && !cur.digits.includes(d)) {
      cur.digits.push(d);
      cur.pts.push(centreOf(el));
      setPath([...cur.digits]); setTrail([...cur.pts]);
    }
  }
  function up() {
    const cur = drag.current;
    drag.current = null;
    setPath([]); setTrail([]); setCursor(null);
    if (!cur) return;
    if (cur.digits.length >= 2) onPattern(cur.digits.join(""));
    else onDigit(cur.digits[0]!);
  }

  const small = "pressable grid h-12 place-items-center rounded-full border border-hairline bg-raised/70 text-fog shadow-e1 hover:bg-hover disabled:opacity-40";
  return (
    <div className="flex flex-col items-center gap-3">
      <div
        ref={box}
        onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up}
        className="relative grid touch-none grid-cols-3 gap-x-7 gap-y-6 select-none"
        role="group" aria-label="Number grid"
      >
        {Array.from({ length: 9 }, (_, i) => {
          const d = String(i + 1);
          const on = path.includes(d);
          return (
            <button
              key={d} type="button" data-node={d} disabled={disabled} aria-label={`Number ${d}`}
              onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onDigit(d); } }}
              className={`grid size-[4.25rem] place-items-center rounded-full border text-[1.45em] font-medium shadow-e2 transition-colors duration-100 ${on ? "border-brass bg-brass/25 text-brass-hi" : "border-hairline-strong bg-raised hover:bg-hover"} disabled:opacity-40 pointer-coarse:size-[4.5rem]`}
            >{d}</button>
          );
        })}
        <svg className="pointer-events-none absolute inset-0 size-full overflow-visible" aria-hidden>
          {trail.length > 0 && (
            <polyline
              points={[...trail, ...(cursor && trail.length ? [cursor] : [])].map((p) => `${p.x},${p.y}`).join(" ")}
              fill="none" stroke="var(--color-brass)" strokeWidth={5} strokeLinecap="round" strokeLinejoin="round" opacity={0.85}
            />
          )}
        </svg>
      </div>
      <div className="grid w-full max-w-[16.5rem] grid-cols-3 gap-x-7">
        <button type="button" className={small} disabled={disabled} onClick={onBackspace} aria-label="Delete the last digit"><Delete className="mx-auto size-5" aria-hidden /></button>
        <button type="button" className={`${small} text-frost text-[1.25em]`} disabled={disabled} onClick={() => onDigit("0")} aria-label="Number 0">0</button>
        <button type="button" className={small} disabled={disabled} onClick={onSubmit} aria-label="Submit"><CornerDownLeft className="mx-auto size-5" aria-hidden /></button>
      </div>
    </div>
  );
}
