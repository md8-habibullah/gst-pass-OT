import { useMemo } from "react";
import { app, counts, setFilter } from "../../state/app";
import { useStore } from "../../state/store";
import { CHIPS } from "./nav";

/** Horizontal type chips for the layouts without the full sidebar. */
export function FilterChips() {
  const filter = useStore(app, (s) => s.filter);
  const items = useStore(app, (s) => s.items);
  const trashCount = useStore(app, (s) => s.trash.length);
  const totals = useMemo(() => counts(items, trashCount), [items, trashCount]);
  return (
    <div className="no-scrollbar -mx-4 flex gap-2 overflow-x-auto px-4" role="tablist" aria-label="Item type">
      {CHIPS.map((c) => {
        const on = filter === c.filter;
        return (
          <button key={c.filter} type="button" role="tab" aria-selected={on} onClick={() => setFilter(c.filter)} className={`pressable min-h-10 shrink-0 rounded-full border px-4 text-[0.93em] font-medium ${on ? "border-brass-ink/60 bg-brass/15 text-brass-ink-hi" : "border-hairline bg-raised text-mist"}`}>
            {c.label}
            <span className="ml-1.5 text-fog">{totals[c.filter]}</span>
          </button>
        );
      })}
    </div>
  );
}
