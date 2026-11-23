import { useEffect, useMemo, useRef } from "react";
import { LayoutGroup } from "motion/react";
import { Plus, Search, X } from "lucide-react";
import { recencyGroup, type RecencyGroup } from "../../lib/format";
import { useLayout } from "../../lib/layout";
import { app, counts, select, setFilter, setQuery, visibleItems, type Filter } from "../../state/app";
import { useStore } from "../../state/store";
import { openNewItem } from "../../state/ui";
import { Button, IconButton } from "../ui/Button";
import { TextInput } from "../ui/Inputs";
import { TITLE } from "../shell/nav";
import { ItemRow } from "./ItemRow";

const GROUP_ORDER: RecencyGroup[] = ["Today", "Last 7 days", "Last 30 days", "Older"];
const CHIPS: { filter: Filter; label: string }[] = [
  { filter: "all", label: "All" },
  { filter: "logins", label: "Logins" },
  { filter: "notes", label: "Notes" },
  { filter: "media", label: "Media" },
  { filter: "favorites", label: "Favorites" },
];

function EmptyState({ query, filter, empty }: { query: string; filter: Filter; empty: boolean }) {
  if (query) {
    return (
      <div className="px-8 pt-16 text-center">
        <p className="font-display text-xl">Nothing matches “{query}”</p>
        <p className="mt-2 text-mist">Search looks at titles, usernames, tags and folders.</p>
      </div>
    );
  }
  return (
    <div className="px-8 pt-16 text-center">
      <p className="font-display text-xl">{empty ? "Your vault is empty" : `No ${TITLE[filter].toLowerCase()} yet`}</p>
      <p className="mt-2 text-mist">Add your first login, one-time code or secure note. Everything is encrypted on this device.</p>
      <Button variant="key" className="mt-5" icon={<Plus className="size-4" />} onClick={() => openNewItem()}>
        New item
      </Button>
    </div>
  );
}

/** Search, filter chips (compact) and the grouped item list. */
export function ListPane() {
  const layout = useLayout();
  const compact = layout === "compact";
  const items = useStore(app, (s) => s.items);
  const trashCount = useStore(app, (s) => s.trash.length);
  const filter = useStore(app, (s) => s.filter);
  const query = useStore(app, (s) => s.query);
  const selectedId = useStore(app, (s) => s.selectedId);
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);

  const visible = useMemo(() => visibleItems(items, filter, query), [items, filter, query]);
  const groups = useMemo(() => {
    const now = Date.now();
    const map = new Map<RecencyGroup, typeof visible>();
    for (const i of visible) {
      const g = recencyGroup(i.updatedAt, now);
      map.set(g, [...(map.get(g) ?? []), i]);
    }
    return GROUP_ORDER.filter((g) => map.has(g)).map((g) => ({ name: g, items: map.get(g)! }));
  }, [visible]);
  const totals = useMemo(() => counts(items, trashCount), [items, trashCount]);

  // "/" and Ctrl/Cmd+K focus search from anywhere that is not a text field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement | null)?.closest("input,textarea,[contenteditable]");
      if ((e.key === "/" && !typing) || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k")) {
        e.preventDefault();
        search.current?.focus();
        search.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  function onListKey(e: React.KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const rows = [...(list.current?.querySelectorAll<HTMLElement>("[data-item-row]") ?? [])];
    const at = rows.findIndex((r) => r === document.activeElement);
    const next = rows[Math.max(0, Math.min(rows.length - 1, at + (e.key === "ArrowDown" ? 1 : -1)))];
    next?.focus();
    next?.click();
  }

  return (
    <section aria-label="Items" className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-between gap-3 px-4 pt-[calc(1rem+var(--safe-top))] pb-3 min-[720px]:pt-4">
        <h2 className="font-display text-[1.45rem] leading-none font-semibold tracking-[-0.005em]">{TITLE[filter]}</h2>
        {!compact && (
          <Button variant="key" icon={<Plus className="size-4" />} onClick={() => openNewItem()}>
            New
          </Button>
        )}
      </div>
      <div className="px-4 pb-3">
        <div className="relative">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-fog" aria-hidden />
          <TextInput
            ref={search}
            type="search"
            aria-label="Search items"
            placeholder="Search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="pr-10 pl-9"
          />
          {!query && !compact && (
            <kbd aria-hidden className="pointer-events-none absolute top-1/2 right-2.5 grid h-5 min-w-5 -translate-y-1/2 place-items-center rounded-sm border border-hairline-strong bg-raised px-1 font-mono text-[0.72rem] leading-none text-fog">/</kbd>
          )}
          {query && (
            <IconButton label="Clear search" size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => { setQuery(""); search.current?.focus(); }}>
              <X className="size-4" />
            </IconButton>
          )}
        </div>
        {compact && (
          <div className="no-scrollbar -mx-4 mt-3 flex gap-2 overflow-x-auto px-4" role="tablist" aria-label="Item type">
            {CHIPS.map((c) => {
              const on = filter === c.filter;
              return (
                <button key={c.filter} type="button" role="tab" aria-selected={on} onClick={() => setFilter(c.filter)} className={`pressable min-h-10 shrink-0 rounded-full border px-4 text-[0.93em] font-medium ${on ? "border-brass/60 bg-brass/15 text-brass-hi" : "border-hairline bg-raised text-mist"}`}>
                  {c.label}
                  <span className="ml-1.5 text-fog">{totals[c.filter]}</span>
                </button>
              );
            })}
          </div>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-28 min-[720px]:pb-6" onKeyDown={onListKey}>
        {visible.length === 0 ? (
          <EmptyState query={query.trim()} filter={filter} empty={items.length === 0} />
        ) : (
          <LayoutGroup id="list">
            <ul ref={list} role="listbox" aria-label={TITLE[filter]} className="flex flex-col gap-0.5">
              {groups.map((g) => (
                <li key={g.name} role="presentation" className="contents">
                  <p className="sticky top-0 z-10 bg-pane/90 px-3 pt-3 pb-1.5 text-[0.86em] font-medium text-fog backdrop-blur-sm">{g.name}</p>
                  <ul role="presentation" className="flex flex-col gap-0.5">
                    {g.items.map((i) => (
                      <ItemRow key={i.id} item={i} selected={i.id === selectedId} roomy={compact} onSelect={() => select(i.id)} />
                    ))}
                  </ul>
                </li>
              ))}
            </ul>
          </LayoutGroup>
        )}
      </div>
    </section>
  );
}
