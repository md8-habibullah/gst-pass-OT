import { describe, expect, it } from "vitest";
import type { ItemSummary } from "../backend";
import { NO_FILTERS } from "../lib/listView";
import { visibleList, type AppState } from "./app";

let n = 0;
const item = (over: Partial<ItemSummary> = {}): ItemSummary => ({
  id: `id-${++n}`, type: "login", title: "Item", subtitle: null, favorite: false, tags: [], folder: null, hasTotp: false,
  createdAt: 1000, updatedAt: 1000, version: 1, archived: false, reprompt: false, ...over,
});

type View = Pick<AppState, "items" | "filter" | "query" | "filters" | "folder" | "folders" | "deep" | "includeOld" | "device">;
const view = (over: Partial<View>): View => ({ items: [], filter: "all", query: "", filters: NO_FILTERS, folder: null, folders: [], deep: null, includeOld: false, device: null, ...over });
const titles = (xs: ItemSummary[]) => xs.map((i) => i.title);

describe("the folder view", () => {
  const items = [
    item({ title: "Bank", folder: "Money" }),
    item({ title: "Broker", folder: "Money/Savings" }),
    item({ title: "Mail" }),
    item({ title: "Wi-Fi", folder: "Home" }),
  ];

  it("shows the folders first and only the items with no folder at the top level", () => {
    const m = visibleList(view({ items, device: null }));
    expect(m.flat).toBe(false);
    expect(m.folders.map((f) => f.path)).toEqual(["Home", "Money"]);
    expect(titles(m.items)).toEqual(["Mail"]);
  });

  it("counts what is inside, subfolders included", () => {
    const money = visibleList(view({ items })).folders.find((f) => f.path === "Money")!;
    expect(money).toMatchObject({ items: 1, total: 2, folders: 1 });
  });

  it("opens a folder: its subfolders and its own items", () => {
    const m = visibleList(view({ items, folder: "Money" }));
    expect(m.folders.map((f) => f.path)).toEqual(["Money/Savings"]);
    expect(titles(m.items)).toEqual(["Bank"]);
  });

  it("keeps a folder that was made on purpose even though it is empty (All items only)", () => {
    expect(visibleList(view({ items, folders: ["Later"] })).folders.map((f) => f.path)).toContain("Later");
    expect(visibleList(view({ items, folders: ["Later"], filter: "logins" })).folders.map((f) => f.path)).not.toContain("Later");
  });

  it("searching looks through every folder and is flat", () => {
    const m = visibleList(view({ items, query: "b" }));
    expect(m.flat).toBe(true);
    expect(m.folders).toEqual([]);
    expect(titles(m.items).sort()).toEqual(["Bank", "Broker"]);
  });

  it("a filter looks through every folder too", () => {
    const m = visibleList(view({ items: [...items, item({ title: "Fav", favorite: true, folder: "Home" })], filters: { ...NO_FILTERS, favoritesOnly: true } }));
    expect(m.flat).toBe(true);
    expect(titles(m.items)).toEqual(["Fav"]);
  });
});

describe("order", () => {
  const items = [item({ title: "b", updatedAt: 1, version: 3 }), item({ title: "a", updatedAt: 3, version: 1 }), item({ title: "c", updatedAt: 2, version: 2 })];
  const sorted = (listSort: string) => titles(visibleList(view({ items, device: { listSort } as AppState["device"] })).items);

  it("follows the device setting", () => {
    expect(sorted("updated-desc")).toEqual(["a", "c", "b"]);
    expect(sorted("updated-asc")).toEqual(["b", "c", "a"]);
    expect(sorted("title-asc")).toEqual(["a", "b", "c"]);
    expect(sorted("title-desc")).toEqual(["c", "b", "a"]);
    expect(sorted("versions-desc")).toEqual(["b", "c", "a"]);
    expect(sorted("versions-asc")).toEqual(["a", "c", "b"]);
  });

  it("starts with the most recently changed when the device says nothing", () => {
    expect(titles(visibleList(view({ items })).items)).toEqual(["a", "c", "b"]);
  });
});

describe("the deeper search", () => {
  const a = item({ title: "Gym" });
  const b = item({ title: "Cabin" });
  const base = { items: [a, b], query: "zebra" };

  it("adds what the core found in notes and other fields", () => {
    const deep = { query: "zebra", oldVersions: false, hits: [{ id: b.id, matched: ["Notes"], oldVersion: null }] };
    const m = visibleList(view({ ...base, deep }));
    expect(titles(m.items)).toEqual(["Cabin"]);
    expect(m.hits.get(b.id)?.matched).toEqual(["Notes"]);
  });

  it("ignores an answer to another question (another query, or the old-versions tick changed)", () => {
    const hits = [{ id: b.id, matched: ["Notes"], oldVersion: null }];
    expect(visibleList(view({ ...base, deep: { query: "zeb", oldVersions: false, hits } })).items).toEqual([]);
    expect(visibleList(view({ ...base, includeOld: true, deep: { query: "zebra", oldVersions: false, hits } })).items).toEqual([]);
  });

  it("never shows an item the other filters leave out, even when the core found it", () => {
    const deep = { query: "zebra", oldVersions: false, hits: [{ id: b.id, matched: ["Notes"], oldVersion: null }] };
    const m = visibleList(view({ ...base, deep, filter: "favorites" }));
    expect(m.items).toEqual([]);
  });
});
