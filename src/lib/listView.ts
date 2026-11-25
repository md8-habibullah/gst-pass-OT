// How the item list is ordered, filtered, searched and grouped. Pure functions over item summaries (titles, tags, dates,
// counts): nothing secret is in a summary, so this runs on the screen. The deeper search (notes, every other field, old
// versions) runs in the core, which is the only place that can read them; its answer is a list of ids and field names.
import { LIST_SORTS, type ItemSummary, type ItemType, type ListSort } from "../backend";
import { recencyGroup } from "./format";

export { LIST_SORTS };

export const SORT_LABEL: Record<ListSort, string> = {
  "updated-desc": "Recently changed",
  "updated-asc": "Changed longest ago",
  "created-desc": "Newest first",
  "created-asc": "Oldest first",
  "title-asc": "Title, A to Z",
  "title-desc": "Title, Z to A",
  type: "Type",
  "versions-desc": "Most versions",
  "versions-asc": "Fewest versions",
};

/** The type order used everywhere a list of types appears (see items/typeMeta.ts). */
const TYPE_RANK: Record<ItemType, number> = { login: 0, totp: 1, secureNote: 2, identity: 3, card: 4, sshKey: 5, media: 6 };

const text = (a: string, b: string): number => a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });

/** A stable comparer: ties fall back to the title, then the id, so the order never flickers between renders. */
export function compareItems(sort: ListSort): (a: ItemSummary, b: ItemSummary) => number {
  const tie = (a: ItemSummary, b: ItemSummary) => text(a.title, b.title) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  switch (sort) {
    case "updated-desc": return (a, b) => b.updatedAt - a.updatedAt || tie(a, b);
    case "updated-asc": return (a, b) => a.updatedAt - b.updatedAt || tie(a, b);
    case "created-desc": return (a, b) => b.createdAt - a.createdAt || tie(a, b);
    case "created-asc": return (a, b) => a.createdAt - b.createdAt || tie(a, b);
    case "title-asc": return (a, b) => tie(a, b);
    case "title-desc": return (a, b) => -text(a.title, b.title) || (a.id < b.id ? -1 : 1);
    case "type": return (a, b) => TYPE_RANK[a.type] - TYPE_RANK[b.type] || tie(a, b);
    case "versions-desc": return (a, b) => b.version - a.version || tie(a, b);
    case "versions-asc": return (a, b) => a.version - b.version || tie(a, b);
  }
}

// ---- filters ----

export type ChangedWithin = "any" | "day" | "week" | "month" | "year";
export const CHANGED_LABEL: Record<ChangedWithin, string> = { any: "Any time", day: "Last 24 hours", week: "Last 7 days", month: "Last 30 days", year: "Last year" };
const CHANGED_MS: Record<Exclude<ChangedWithin, "any">, number> = { day: 86_400_000, week: 7 * 86_400_000, month: 30 * 86_400_000, year: 365 * 86_400_000 };

export interface ListFilters {
  /** Empty means every type. */
  types: ItemType[];
  favoritesOnly: boolean;
  /** Only items that have a one-time code. */
  withCode: boolean;
  /** Only items that ask for the master password again. */
  protectedOnly: boolean;
  /** Only items carrying this tag. */
  tag: string | null;
  /** Only items whose latest version number is at least / at most this. */
  minVersion: number | null;
  maxVersion: number | null;
  changedWithin: ChangedWithin;
}

export const NO_FILTERS: ListFilters = { types: [], favoritesOnly: false, withCode: false, protectedOnly: false, tag: null, minVersion: null, maxVersion: null, changedWithin: "any" };

/** How many of the filters are switched on (shown on the filter button). */
export function activeFilterCount(f: ListFilters): number {
  return (
    (f.types.length ? 1 : 0) + (f.favoritesOnly ? 1 : 0) + (f.withCode ? 1 : 0) + (f.protectedOnly ? 1 : 0) + (f.tag ? 1 : 0) +
    (f.minVersion !== null ? 1 : 0) + (f.maxVersion !== null ? 1 : 0) + (f.changedWithin !== "any" ? 1 : 0)
  );
}

export function applyFilters(items: readonly ItemSummary[], f: ListFilters, now = Date.now()): ItemSummary[] {
  const since = f.changedWithin === "any" ? null : now - CHANGED_MS[f.changedWithin];
  return items.filter(
    (i) =>
      (f.types.length === 0 || f.types.includes(i.type)) &&
      (!f.favoritesOnly || i.favorite) &&
      (!f.withCode || i.hasTotp) &&
      (!f.protectedOnly || i.reprompt) &&
      (!f.tag || i.tags.includes(f.tag)) &&
      (f.minVersion === null || i.version >= f.minVersion) &&
      (f.maxVersion === null || i.version <= f.maxVersion) &&
      (since === null || i.updatedAt >= since),
  );
}

/** Every tag in use, most used first. */
export function tagsIn(items: readonly ItemSummary[]): string[] {
  const n = new Map<string, number>();
  for (const i of items) for (const t of i.tags) n.set(t, (n.get(t) ?? 0) + 1);
  return [...n].sort((a, b) => b[1] - a[1] || text(a[0], b[0])).map(([t]) => t);
}

// ---- the quick search over what a summary holds ----

/** Words are matched one by one, in any order: "github work" finds "GitHub" in the folder "Work". */
export function summaryMatches(i: ItemSummary, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const hay = [i.title, i.subtitle ?? "", i.folder ?? "", ...i.tags].join("\n").toLowerCase();
  return words.every((w) => hay.includes(w));
}

// ---- grouping ----

export interface Group<T> {
  name: string;
  items: T[];
}

/** Section headings that suit the order: time for "recently changed", letters for titles, types for "type", none otherwise (an empty name). */
export function groupItems(items: readonly ItemSummary[], sort: ListSort, typeLabel: (t: ItemType) => string, now = Date.now()): Group<ItemSummary>[] {
  const keyOf = (i: ItemSummary): string | null => {
    switch (sort) {
      case "updated-desc": return recencyGroup(i.updatedAt, now);
      case "title-asc":
      case "title-desc": {
        const c = [...i.title.trim()][0]?.toUpperCase() ?? "#";
        return /\p{L}/u.test(c) ? c : "#";
      }
      case "type": return typeLabel(i.type);
      default: return null;
    }
  };
  const out: Group<ItemSummary>[] = [];
  for (const i of items) {
    const key = keyOf(i) ?? "";
    const last = out[out.length - 1];
    if (last && last.name === key) last.items.push(i);
    else out.push({ name: key, items: [i] });
  }
  // The items arrive already sorted, so equal keys are next to each other and the headings come out in the sort's order.
  return out;
}
