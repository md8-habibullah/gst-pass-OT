import { useMemo, useRef } from "react";
import { LayoutGroup } from "motion/react";
import { Archive, Plus, Search, Star, X } from "lucide-react";
import { recencyGroup, type RecencyGroup } from "../../lib/format";
import { useLayout } from "../../lib/layout";
import { app, select, setQuery, visibleItems, type Filter } from "../../state/app";
import { useStore } from "../../state/store";
import { openNewItem } from "../../state/ui";
import { Button, IconButton } from "../ui/Button";
import { Keycap } from "../ui/Keycap";
import { TextInput } from "../ui/Inputs";
import { FilterChips } from "../shell/FilterChips";
import { FILTER_TYPE, TITLE } from "../shell/nav";
import { TYPE_NOUN } from "../../lib/format";
import { ItemRow } from "./ItemRow";

const GROUP_ORDER: RecencyGroup[] = ["Today", "Last 7 days", "Last 30 days", "Older"];

function EmptyState({ query, filter, empty }: { query: string; filter: Filter; empty: boolean }) {
  if (query) {
    return (
      <div className="px-8 pt-16 text-center">
        <p className="font-display text-xl">Nothing matches “{query}”</p>
        <p className="mt-2 text-mist">Search looks at titles, usernames, tags and folders.</p>
      </div>
    );
  }
  if (filter === "archive") {
    return (
      <div className="px-8 pt-16 text-center">
        <Archive className="mx-auto size-8 text-fog" aria-hidden strokeWidth={1.4} />
        <p className="mt-3 font-display text-xl">Nothing archived</p>
        <p className="mt-2 text-mist">Archive items you rarely need. They stay encrypted, keep their history, and disappear from your lists and codes.</p>
      </div>
    );
  }
  if (filter === "favorites") {
    return (
      <div className="px-8 pt-16 text-center">
        <Star className="mx-auto size-8 text-fog" aria-hidden strokeWidth={1.4} />
        <p className="mt-3 font-display text-xl">No favorites yet</p>
        <p className="mt-2 text-mist">Turn on Favorite when you edit an item to keep it here.</p>
      </div>
    );
  }
  const type = FILTER_TYPE[filter] ?? null;
  return (
    <div className="px-8 pt-16 text-center">
      <p className="font-display text-xl">{empty ? "Your vault is empty" : type ? `No ${TITLE[filter].toLowerCase()} yet` : "Nothing here yet"}</p>
      <p className="mt-2 text-mist">
        {type ? `Add your first ${TYPE_NOUN[type]}. Everything is encrypted on this device.` : "Add a login, card, identity, SSH key, note or one-time code. Everything is encrypted on this device."}
      </p>
      <Button variant="key" className="mt-5" icon={<Plus className="size-4" />} onClick={() => openNewItem(type)}>
        {type ? `New ${TYPE_NOUN[type]}` : "New item"}
      </Button>
    </div>
  );
}

/** Search, filter chips (compact) and the grouped item list. */
export function ListPane() {
  const layout = useLayout();
  const compact = layout === "compact";
  const items = useStore(app, (s) => s.items);
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
          filter !== "archive" && (
            <Button variant="key" icon={<Plus className="size-4" />} onClick={() => openNewItem(FILTER_TYPE[filter] ?? null)}>
              New
            </Button>
          )
        )}
      </div>
      <div className="px-4 pb-3">
        <div className="relative">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-fog" aria-hidden />
          <TextInput
            ref={search}
            type="search"
            data-search
            aria-label="Search items"
            placeholder="Search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="pr-10 pl-9"
          />
          {!query && !compact && (
            <Keycap className="pointer-events-none absolute top-1/2 right-2.5 -translate-y-1/2">/</Keycap>
          )}
          {query && (
            <IconButton label="Clear search" size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => { setQuery(""); search.current?.focus(); }}>
              <X className="size-4" />
            </IconButton>
          )}
        </div>
        {layout !== "wide" && <div className="mt-3"><FilterChips /></div>}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-28 min-[720px]:pb-6" onKeyDown={onListKey}>
        {visible.length === 0 ? (
          <EmptyState query={query.trim()} filter={filter} empty={items.length === 0} />
        ) : (
          <LayoutGroup id="list">
            <ul ref={list} role="listbox" aria-label={TITLE[filter]} className="flex flex-col gap-0.5">
              {groups.map((g) => (
                <li key={g.name} role="presentation" className="contents">
                  <p className="sticky top-0 z-10 bg-pane/90 px-3 pt-3 pb-1.5 text-[0.86em] font-medium text-fog">{g.name}</p>
                  <ul role="presentation" className="flex flex-col gap-0.5">
                    {g.items.map((i) => (
                      <ItemRow key={i.id} item={i} selected={i.id === selectedId} roomy={compact} showType={filter === "all" || filter === "favorites" || filter === "archive"} onSelect={() => select(i.id)} />
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
