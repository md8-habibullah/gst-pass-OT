import { describe, expect, it } from "vitest";
import { allFolders, childFolders, cleanFolder, folderName, folderProblem, isInside, itemsIn, joinFolder, parentFolder, trail } from "./folders";

const at = (folder: string | null) => ({ folder });

describe("folder paths", () => {
  it("cleans stray spaces and empty parts", () => {
    expect(cleanFolder(" Work // Clients/ ")).toBe("Work/Clients");
    expect(cleanFolder("")).toBeNull();
    expect(cleanFolder("  / / ")).toBeNull();
    expect(cleanFolder(null)).toBeNull();
  });

  it("names the parent and the last part", () => {
    expect(parentFolder("Work/Clients/Acme")).toBe("Work/Clients");
    expect(parentFolder("Work")).toBeNull();
    expect(folderName("Work/Clients")).toBe("Clients");
    expect(joinFolder("Work", " Clients ")).toBe("Work/Clients");
    expect(joinFolder(null, "Work")).toBe("Work");
  });

  it("knows what is inside what, and does not mistake a prefix for a parent", () => {
    expect(isInside("Work/Clients", "Work")).toBe(true);
    expect(isInside("Work", "Work")).toBe(true);
    expect(isInside("Workshop", "Work")).toBe(false);
    expect(isInside("Home", "Work")).toBe(false);
  });

  it("explains a path that cannot be used", () => {
    expect(folderProblem("Work")).toBeNull();
    expect(folderProblem("   ")).toMatch(/name/);
    expect(folderProblem("a/b/c/d/e/f/g")).toMatch(/6 levels/);
    expect(folderProblem("x".repeat(65))).toMatch(/64/);
    expect(folderProblem("a\u0007b")).toMatch(/control/);
    expect(folderProblem(["a".repeat(60), "b".repeat(60), "c".repeat(60)].join("/"))).toMatch(/128/);
  });
});

describe("the folder tree", () => {
  const items = [at("Work/Clients"), at("Work/Clients"), at("Work"), at("Home"), at(null), at("Work/Tools/Cli")];

  it("is the folders made on purpose plus the ones items sit in, with every parent", () => {
    expect(allFolders(["Archive 2020"], items)).toEqual(["Archive 2020", "Home", "Work", "Work/Clients", "Work/Tools", "Work/Tools/Cli"]);
    expect(allFolders([], [])).toEqual([]);
  });

  it("lists the folders directly inside one, with counts", () => {
    const all = allFolders([], items);
    const top = childFolders(all, items, null);
    expect(top.map((f) => f.path)).toEqual(["Home", "Work"]);
    const work = top.find((f) => f.path === "Work")!;
    expect(work).toMatchObject({ name: "Work", items: 1, total: 4, folders: 2 });
    expect(childFolders(all, items, "Work").map((f) => f.path)).toEqual(["Work/Clients", "Work/Tools"]);
    expect(childFolders(all, items, "Work/Clients")).toEqual([]);
  });

  it("an empty folder made on purpose is listed with no items", () => {
    const all = allFolders(["Plans"], items);
    expect(childFolders(all, items, null).find((f) => f.path === "Plans")).toMatchObject({ items: 0, total: 0, folders: 0 });
  });

  it("items directly in a folder, or at the top level", () => {
    expect(itemsIn(items, null)).toHaveLength(1);
    expect(itemsIn(items, "Work")).toHaveLength(1);
    expect(itemsIn(items, "Work/Clients")).toHaveLength(2);
    expect(itemsIn(items, "Nope")).toHaveLength(0);
  });

  it("a breadcrumb trail", () => {
    expect(trail(null)).toEqual([]);
    expect(trail("Work/Clients")).toEqual([{ name: "Work", path: "Work" }, { name: "Clients", path: "Work/Clients" }]);
  });
});
