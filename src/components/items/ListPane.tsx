import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { LayoutGroup } from "motion/react";
import { Archive, ChevronRight, Folder, FolderPlus, Pencil, Plus, Search, SlidersHorizontal, Star, Trash2, X } from "lucide-react";
import { TYPE_LABEL } from "../../backend";
import { trail, type FolderNode } from "../../lib/folders";
import { plural, TYPE_NOUN } from "../../lib/format";
import { useLayout } from "../../lib/layout";
import { activeFilterCount, groupItems, NO_FILTERS } from "../../lib/listView";
import { app, be, consumeSearchOnOpen, openFolder, select, setDeep, setFilters, setQuery, visibleList, type Filter } from "../../state/app";
import { useStore } from "../../state/store";
import { openFolderDialog, openNewItem } from "../../state/ui";
import { Button, IconButton } from "../ui/Button";
import { Keycap } from "../ui/Keycap";
import { TextInput } from "../ui/Inputs";
import { AccountBar } from "../shell/AccountMenu";
import { FilterChips } from "../shell/FilterChips";
import { FILTER_TYPE, TITLE } from "../shell/nav";
import { ItemRow } from "./ItemRow";
import { ListTools, OldVersionsTick } from "./ListTools";

function EmptyState({ query, filter, empty, filtered }: { query: string; filter: Filter; empty: boolean; filtered: boolean }) {
  if (query || filtered) {
    return (
      <div className="px-8 pt-16 text-center">
        <p className="font-display text-xl">{query ? `Nothing matches “${query}”` : "Nothing matches these filters"}</p>
        <p className="mt-2 text-mist">
          {query
            ? "Search looks at titles, websites, usernames, notes, field names, tags and folders. Passwords and card numbers are never searched."
            : "Loosen a filter to see more."}
        </p>
        {filtered && <Button variant="quiet" className="mt-5" onClick={() => setFilters(NO_FILTERS)}>Clear filters</Button>}
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

function FolderRow({ node, roomy }: { node: FolderNode; roomy: boolean }) {
  const detail = [node.total > 0 ? plural(node.total, "item") : "Empty", node.folders > 0 ? plural(node.folders, "folder") : null].filter(Boolean).join(", ");
  return (
    <li role="presentation">
      <button
        type="button"
        data-folder-row
        onClick={() => openFolder(node.path)}
        className={`pressable flex w-full items-center gap-3 rounded-lg py-1 pr-3 pl-3 text-left hover:bg-raised/60 ${roomy ? "min-h-[4.5rem]" : "min-h-[3.75rem]"}`}
      >
        <span aria-hidden className={`grid shrink-0 place-items-center rounded-full bg-raised text-brass-ink ${roomy ? "size-11" : "size-[38px]"}`}><Folder className="size-[18px]" /></span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold">{node.name}</span>
          <span className="block truncate text-[0.93em] text-mist">{detail}</span>
        </span>
        <ChevronRight className="size-4 shrink-0 text-fog" aria-hidden />
      </button>
    </li>
  );
}

/** Where you are inside the folders, one button per level, and what can be done to the open folder. */
function Breadcrumb({ folder, rootLabel }: { folder: string; rootLabel: string }) {
  const steps = trail(folder);
  return (
    <div className="flex items-center gap-1">
      <nav aria-label="Folder path" className="no-scrollbar -ml-2 flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto">
        <button type="button" onClick={() => openFolder(null)} className="pressable min-h-9 shrink-0 rounded-sm px-2 text-[0.93em] text-mist hover:bg-raised hover:text-frost pointer-coarse:min-h-11">{rootLabel}</button>
        {steps.map((t, i) => {
          const last = i === steps.length - 1;
          return (
            <Fragment key={t.path}>
              <ChevronRight className="size-3.5 shrink-0 text-fog" aria-hidden />
              <button type="button" aria-current={last ? "page" : undefined} onClick={() => openFolder(t.path)} className={`pressable min-h-9 shrink-0 rounded-sm px-2 text-[0.93em] pointer-coarse:min-h-11 ${last ? "font-semibold text-frost" : "text-mist hover:bg-raised hover:text-frost"}`}>{t.name}</button>
            </Fragment>
          );
        })}
      </nav>
      <IconButton label={`Rename folder ${trail(folder).at(-1)?.name ?? ""}`} size="sm" onClick={() => openFolderDialog({ kind: "rename", path: folder })}><Pencil className="size-4" /></IconButton>
      <IconButton label="Remove this folder" size="sm" tone="danger" onClick={() => openFolderDialog({ kind: "remove", path: folder })}><Trash2 className="size-4" /></IconButton>
    </div>
  );
}

/**
 * The deeper search: the core reads notes and every other field and answers with ids and field names. It runs a moment
 * after typing stops, and again when the items change or "include earlier versions" is switched.
 */
function useDeepSearch() {
  const query = useStore(app, (s) => s.query);
  const includeOld = useStore(app, (s) => s.includeOld);
  const items = useStore(app, (s) => s.items);
  useEffect(() => {
    const q = query.trim();
    if (!q) { if (app.get().deep) setDeep(null); return; }
    let live = true;
    const timer = setTimeout(() => {
      be().searchItems(q, includeOld).then(
        (hits) => { if (live) setDeep({ query: q, oldVersions: includeOld, hits }); },
        () => { if (live) setDeep(null); },
      );
    }, 160);
    return () => { live = false; clearTimeout(timer); };
  }, [query, includeOld, items]);
}

/** Where a hit was found, in words ("Found in Website, Notes"), for the row. Titles are the row itself and need no note. */
function hitNote(matched: string[], oldVersion: number | null): string | undefined {
  const where = matched.filter((m) => m !== "Title");
  if (oldVersion !== null) return `Earlier version ${oldVersion}${where.length ? `: ${where.join(", ")}` : ""}`;
  return where.length ? `Found in ${where.join(", ")}` : undefined;
}

/** Search, filters and order, then the folders and the item list. */
export function ListPane() {
  const layout = useLayout();
  const compact = layout === "compact";
  const items = useStore(app, (s) => s.items);
  const filter = useStore(app, (s) => s.filter);
  const query = useStore(app, (s) => s.query);
  const filters = useStore(app, (s) => s.filters);
  const folder = useStore(app, (s) => s.folder);
  const folders = useStore(app, (s) => s.folders);
  const deep = useStore(app, (s) => s.deep);
  const includeOld = useStore(app, (s) => s.includeOld);
  const device = useStore(app, (s) => s.device);
  const searchOnOpen = useStore(app, (s) => s.searchOnOpen);
  const selectedId = useStore(app, (s) => s.selectedId);
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const [tools, setTools] = useState(false);
  const sort = device?.listSort ?? "updated-desc";
  const filtered = activeFilterCount(filters);

  useDeepSearch();

  // "Open in search": the cursor goes to the search box once after the vault opens (on a phone the keyboard comes up with it).
  useEffect(() => {
    if (!searchOnOpen) return;
    const timer = setTimeout(() => { search.current?.focus(); consumeSearchOnOpen(); }, 120);
    return () => clearTimeout(timer);
  }, [searchOnOpen]);

  const model = useMemo(
    () => visibleList({ items, filter, query, filters, folder, folders, deep, includeOld, device }),
    [items, filter, query, filters, folder, folders, deep, includeOld, device],
  );
  const groups = useMemo(() => groupItems(model.items, sort, (t) => TYPE_LABEL[t]), [model.items, sort]);
  const nothing = model.items.length === 0 && model.folders.length === 0;
  const rootLabel = TITLE[filter];

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
        <div className="flex items-center gap-2">
          <IconButton label={folder ? `Add folder inside ${trail(folder).at(-1)?.name ?? "this folder"}` : "Add folder"} onClick={() => openFolderDialog({ kind: "create", parent: folder })}>
            <FolderPlus className="size-[18px]" />
          </IconButton>
          {!compact && filter !== "archive" && (
            <Button variant="key" icon={<Plus className="size-4" />} onClick={() => openNewItem(FILTER_TYPE[filter] ?? null)}>
              New
            </Button>
          )}
          <AccountBar />
        </div>
      </div>
      <div className="px-4 pb-3">
        <div className="flex items-center gap-2">
          <div className="relative min-w-0 flex-1">
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
          <div className="relative shrink-0">
            <IconButton label={filtered ? `Order and filters, ${filtered} on` : "Order and filters"} aria-expanded={tools} tone={tools || filtered ? "key" : "default"} onClick={() => setTools((v) => !v)}>
              <SlidersHorizontal className="size-[18px]" />
            </IconButton>
            {filtered > 0 && !tools && <span aria-hidden className="pointer-events-none absolute -top-1 -right-1 grid size-4 place-items-center rounded-full bg-brass-ink text-[0.66rem] leading-none font-semibold text-marine">{filtered}</span>}
          </div>
        </div>
        {(query.trim() || tools) && <div className="mt-1.5"><OldVersionsTick /></div>}
        {tools && <ListTools onClose={() => setTools(false)} />}
        {layout !== "wide" && <div className="mt-3"><FilterChips /></div>}
        {folder && !model.flat && <div className="mt-2"><Breadcrumb folder={folder} rootLabel={rootLabel} /></div>}
        {model.flat && !nothing && <p role="status" className="mt-2 text-[0.86em] text-fog">{plural(model.items.length, "item")} found, from every folder</p>}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-28 min-[720px]:pb-6" onKeyDown={onListKey}>
        {nothing ? (
          folder && !model.flat ? (
            <div className="px-8 pt-14 text-center">
              <Folder className="mx-auto size-8 text-fog" aria-hidden strokeWidth={1.4} />
              <p className="mt-3 font-display text-xl">This folder is empty</p>
              <p className="mt-2 text-mist">Put items in it with “Move to folder” in an item’s menu, or when you edit an item.</p>
            </div>
          ) : (
            <EmptyState query={query.trim()} filter={filter} empty={items.length === 0} filtered={filtered > 0} />
          )
        ) : (
          <LayoutGroup id="list">
            {model.folders.length > 0 && (
              <>
                <p className="px-3 pt-3 pb-1.5 text-[0.86em] font-medium text-fog">Folders</p>
                <ul role="list" aria-label="Folders" className="flex flex-col gap-0.5">
                  {model.folders.map((f) => <FolderRow key={f.path} node={f} roomy={compact} />)}
                </ul>
              </>
            )}
            <ul ref={list} role="listbox" aria-label={TITLE[filter]} className="flex flex-col gap-0.5">
              {groups.map((g) => (
                <li key={g.name} role="presentation" className="contents">
                  {(g.name || model.folders.length > 0) && <p className="sticky top-0 z-10 bg-pane/90 px-3 pt-3 pb-1.5 text-[0.86em] font-medium text-fog">{g.name || "Items"}</p>}
                  <ul role="presentation" className="flex flex-col gap-0.5">
                    {g.items.map((i) => {
                      const hit = model.hits.get(i.id);
                      const where = hit ? hitNote(hit.matched, hit.oldVersion) : undefined;
                      const note = model.flat && i.folder ? [i.folder.split("/").join(" / "), where].filter(Boolean).join(" · ") : where;
                      return <ItemRow key={i.id} item={i} selected={i.id === selectedId} roomy={compact} note={note} showType={filter === "all" || filter === "favorites" || filter === "archive"} onSelect={() => select(i.id)} />;
                    })}
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
