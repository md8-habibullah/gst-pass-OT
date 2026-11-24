import { useEffect, useRef, useState, type PointerEvent } from "react";
import { Delete, CornerDownLeft } from "lucide-react";

interface Pt { x: number; y: number }

/** A finger is inside a node when it is within this share of the node's radius of its centre. */
const HIT = 0.85;

/**
 * The numbered 3x3 grid: a keypad and a pattern lock in one.
 * A tap on a number types that digit; dragging across two or more numbers draws a line and, on release, sends the digits
 * in the order they were touched (for example 1, 2, 5, 8, 9 becomes "12589"). The core decides what the input means.
 *
 * It stays still on purpose: no fades, no dimming while a code is being checked, no glow. The line follows the finger
 * through a requestAnimationFrame write to one SVG attribute; React only re-renders when a number is entered. The node
 * centres are measured once when the finger goes down, so a drag never reads layout.
 */
export function GridPad({ disabled, onDigit, onBackspace, onPattern, onSubmit }: {
  disabled?: boolean;
  onDigit: (d: string) => void;
  onBackspace: () => void;
  onPattern: (digits: string) => void;
  onSubmit: () => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const line = useRef<SVGPolylineElement>(null);
  const centres = useRef<[string, Pt][]>([]);
  const radius = useRef(0);
  const drag = useRef<{ digits: string[]; pts: Pt[] } | null>(null);
  const frame = useRef(0);
  const [path, setPath] = useState<string[]>([]);

  useEffect(() => () => cancelAnimationFrame(frame.current), []);

  function measure() {
    const b = box.current!.getBoundingClientRect();
    centres.current = [...box.current!.querySelectorAll<HTMLElement>("[data-node]")].map((el) => {
      const r = el.getBoundingClientRect();
      radius.current = r.width / 2;
      return [el.dataset.node!, { x: r.left - b.left + r.width / 2, y: r.top - b.top + r.height / 2 }];
    });
  }
  function paint(cursor: Pt | null) {
    const cur = drag.current;
    if (!cur || !line.current) return;
    line.current.setAttribute("points", (cursor ? [...cur.pts, cursor] : cur.pts).map((p) => `${p.x},${p.y}`).join(" "));
  }

  function down(e: PointerEvent<HTMLDivElement>) {
    if (disabled) return;
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-node]");
    if (!el) return;
    box.current!.setPointerCapture(e.pointerId);
    measure();
    const d = el.dataset.node!;
    const c = centres.current.find(([n]) => n === d)![1];
    drag.current = { digits: [d], pts: [c] };
    setPath([d]);
    paint(null);
  }
  function move(e: PointerEvent<HTMLDivElement>) {
    const cur = drag.current;
    if (!cur) return;
    const b = box.current!.getBoundingClientRect();
    const at = { x: e.clientX - b.left, y: e.clientY - b.top };
    for (const [d, c] of centres.current) {
      if (cur.digits.includes(d)) continue;
      if (Math.hypot(c.x - at.x, c.y - at.y) <= radius.current * HIT) {
        cur.digits.push(d);
        cur.pts.push(c);
        setPath([...cur.digits]);
        break;
      }
    }
    cancelAnimationFrame(frame.current);
    frame.current = requestAnimationFrame(() => paint(at));
  }
  function up() {
    const cur = drag.current;
    drag.current = null;
    cancelAnimationFrame(frame.current);
    line.current?.setAttribute("points", "");
    setPath([]);
    if (!cur) return;
    if (cur.digits.length >= 2) onPattern(cur.digits.join(""));
    else onDigit(cur.digits[0]!);
  }

  const small = "pressable grid h-12 place-items-center rounded-full border border-hairline bg-raised/70 text-fog hover:bg-hover";
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
              key={d} type="button" data-node={d} aria-disabled={disabled || undefined} aria-label={`Number ${d}`}
              onKeyDown={(e) => { if (!disabled && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); onDigit(d); } }}
              className={`grid size-[4.25rem] place-items-center rounded-full border text-[1.45em] font-medium transition-colors duration-75 pointer-coarse:size-[4.5rem] ${on ? "border-brass bg-raised text-brass-hi" : "border-hairline-strong bg-raised hover:bg-hover"}`}
            >{d}</button>
          );
        })}
        <svg className="pointer-events-none absolute inset-0 size-full overflow-visible" aria-hidden>
          <polyline ref={line} points="" fill="none" stroke="var(--color-brass)" strokeWidth={3} strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </div>
      <div className="grid w-full max-w-[16.5rem] grid-cols-3 gap-x-7">
        <button type="button" className={small} onClick={() => !disabled && onBackspace()} aria-label="Delete the last digit"><Delete className="mx-auto size-5" aria-hidden /></button>
        <button type="button" className={`${small} text-frost text-[1.25em]`} onClick={() => !disabled && onDigit("0")} aria-label="Number 0">0</button>
        <button type="button" className={small} onClick={() => !disabled && onSubmit()} aria-label="Submit"><CornerDownLeft className="mx-auto size-5" aria-hidden /></button>
      </div>
    </div>
  );
}
