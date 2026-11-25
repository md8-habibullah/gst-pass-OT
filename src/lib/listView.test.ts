import { describe, expect, it } from "vitest";
import type { ItemSummary } from "../backend";
import { activeFilterCount, applyFilters, compareItems, groupItems, LIST_SORTS, NO_FILTERS, SORT_LABEL, summaryMatches, tagsIn } from "./listView";

const NOW = new Date("2026-06-15T12:00:00").getTime();
const DAY = 86_400_000;

let n = 0;
function item(over: Partial<ItemSummary> = {}): ItemSummary {
  n += 1;
  return { id: `id-${String(n).padStart(3, "0")}`, type: "login", title: `Item ${n}`, subtitle: null, favorite: false, tags: [], folder: null, hasTotp: false, createdAt: NOW - 100 * DAY, updatedAt: NOW - DAY, version: 1, archived: false, reprompt: false, ...over };
}

describe("sorting", () => {
  const a = item({ title: "alpha", updatedAt: NOW - 3 * DAY, createdAt: NOW - 30 * DAY, version: 5, type: "card" });
  const b = item({ title: "Bravo", updatedAt: NOW - 1 * DAY, createdAt: NOW - 20 * DAY, version: 1, type: "login" });
  const c = item({ title: "charlie", updatedAt: NOW - 2 * DAY, createdAt: NOW - 10 * DAY, version: 3, type: "secureNote" });
  const titles = (sort: (typeof LIST_SORTS)[number]) => [a, b, c].slice().sort(compareItems(sort)).map((i) => i.title);

  it("every order the core accepts has a label and a comparer", () => {
    for (const s of LIST_SORTS) {
      expect(SORT_LABEL[s]).toBeTruthy();
      expect(() => [a, b, c].sort(compareItems(s))).not.toThrow();
    }
  });
  it("by when it changed", () => {
    expect(titles("updated-desc")).toEqual(["Bravo", "charlie", "alpha"]);
    expect(titles("updated-asc")).toEqual(["alpha", "charlie", "Bravo"]);
  });
  it("by when it was made", () => {
    expect(titles("created-desc")).toEqual(["charlie", "Bravo", "alpha"]);
    expect(titles("created-asc")).toEqual(["alpha", "Bravo", "charlie"]);
  });
  it("by title, ignoring case", () => {
    expect(titles("title-asc")).toEqual(["alpha", "Bravo", "charlie"]);
    expect(titles("title-desc")).toEqual(["charlie", "Bravo", "alpha"]);
  });
  it("by version count, ties by title", () => {
    expect(titles("versions-desc")).toEqual(["alpha", "charlie", "Bravo"]);
    expect(titles("versions-asc")).toEqual(["Bravo", "charlie", "alpha"]);
    const x = item({ title: "x", version: 2 }), y = item({ title: "a", version: 2 });
    expect([x, y].sort(compareItems("versions-desc")).map((i) => i.title)).toEqual(["a", "x"]);
  });
  it("by type, in the order of the new-item list", () => {
    expect(titles("type")).toEqual(["Bravo", "charlie", "alpha"]);
  });
  it("numbers in titles sort as numbers", () => {
    const list = [item({ title: "Server 10" }), item({ title: "Server 2" })];
    expect(list.sort(compareItems("title-asc")).map((i) => i.title)).toEqual(["Server 2", "Server 10"]);
  });
});

describe("filters", () => {
  const all = [
    item({ title: "a", type: "login", favorite: true, hasTotp: true, tags: ["work"], version: 4, updatedAt: NOW - 2 * 3_600_000 }),
    item({ title: "b", type: "card", tags: ["work", "money"], version: 1, updatedAt: NOW - 3 * DAY }),
    item({ title: "c", type: "login", reprompt: true, version: 7, updatedAt: NOW - 40 * DAY }),
    item({ title: "d", type: "secureNote", version: 2, updatedAt: NOW - 400 * DAY }),
  ];
  const run = (over: Partial<typeof NO_FILTERS>) => applyFilters(all, { ...NO_FILTERS, ...over }, NOW).map((i) => i.title);

  it("no filter keeps everything", () => {
    expect(run({})).toEqual(["a", "b", "c", "d"]);
    expect(activeFilterCount(NO_FILTERS)).toBe(0);
  });
  it("by type, several at once", () => {
    expect(run({ types: ["card"] })).toEqual(["b"]);
    expect(run({ types: ["card", "secureNote"] })).toEqual(["b", "d"]);
  });
  it("favorites, with a code, asking again", () => {
    expect(run({ favoritesOnly: true })).toEqual(["a"]);
    expect(run({ withCode: true })).toEqual(["a"]);
    expect(run({ protectedOnly: true })).toEqual(["c"]);
  });
  it("by tag", () => { expect(run({ tag: "work" })).toEqual(["a", "b"]); expect(run({ tag: "money" })).toEqual(["b"]); });
  it("by version count, above and below, both limits included", () => {
    expect(run({ minVersion: 4 })).toEqual(["a", "c"]);
    expect(run({ maxVersion: 2 })).toEqual(["b", "d"]);
    expect(run({ minVersion: 2, maxVersion: 4 })).toEqual(["a", "d"]);
    expect(run({ minVersion: 8 })).toEqual([]);
  });
  it("by how recently it changed", () => {
    expect(run({ changedWithin: "day" })).toEqual(["a"]);
    expect(run({ changedWithin: "week" })).toEqual(["a", "b"]);
    expect(run({ changedWithin: "month" })).toEqual(["a", "b"]);
    expect(run({ changedWithin: "year" })).toEqual(["a", "b", "c"]);
  });
  it("filters combine, and each one counts", () => {
    const f = { ...NO_FILTERS, types: ["login" as const], minVersion: 5, tag: null };
    expect(applyFilters(all, f, NOW).map((i) => i.title)).toEqual(["c"]);
    expect(activeFilterCount({ ...NO_FILTERS, types: ["login"], favoritesOnly: true, minVersion: 1, maxVersion: 9, changedWithin: "week" })).toBe(5);
  });
  it("tags, most used first", () => { expect(tagsIn(all)).toEqual(["work", "money"]); });
});

describe("the quick search over a summary", () => {
  const i = item({ title: "GitHub", subtitle: "me@example.com", folder: "Work/Dev", tags: ["code"] });
  it("finds words in any order, in the title, the subtitle, the folder and the tags", () => {
    expect(summaryMatches(i, "github")).toBe(true);
    expect(summaryMatches(i, "EXAMPLE.com")).toBe(true);
    expect(summaryMatches(i, "dev github")).toBe(true);
    expect(summaryMatches(i, "code work")).toBe(true);
    expect(summaryMatches(i, "github gitlab")).toBe(false);
    expect(summaryMatches(i, "   ")).toBe(true);
  });
});

describe("headings", () => {
  const label = (t: string) => t;
  it("time headings for recently changed, in order", () => {
    const list = [item({ updatedAt: NOW - 60_000 }), item({ updatedAt: NOW - 3 * DAY }), item({ updatedAt: NOW - 200 * DAY })].sort(compareItems("updated-desc"));
    expect(groupItems(list, "updated-desc", label, NOW).map((g) => g.name)).toEqual(["Today", "Last 7 days", "Older"]);
  });
  it("letters for titles, # for anything that is not a letter", () => {
    const list = [item({ title: "Apple" }), item({ title: "avocado" }), item({ title: "Bank" }), item({ title: "7-Eleven" })].sort(compareItems("title-asc"));
    expect(groupItems(list, "title-asc", label, NOW).map((g) => [g.name, g.items.length])).toEqual([["#", 1], ["A", 2], ["B", 1]]);
  });
  it("types for the type order", () => {
    const list = [item({ type: "card" }), item({ type: "login" })].sort(compareItems("type"));
    expect(groupItems(list, "type", (t) => t.toUpperCase(), NOW).map((g) => g.name)).toEqual(["LOGIN", "CARD"]);
  });
  it("no headings for the other orders", () => {
    const list = [item(), item()];
    const g = groupItems(list, "versions-desc", label, NOW);
    expect(g).toHaveLength(1);
    expect(g[0]!.name).toBe("");
  });
});
