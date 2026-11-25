import { describe, expect, it } from "vitest";
import { instanceKey, instancesOf, parseFieldKey, specOf } from "../backend/types";
import { addSlot, CODE, compactSlots, groupSignIns, removeSlot, rowsOf, SIGN_IN, slotFilled, slotsOf, withoutBlanks } from "./instances";

describe("field keys", () => {
  it("has one spelling for every instance", () => {
    expect(parseFieldKey("totp")).toEqual({ base: "totp", n: 1 });
    expect(parseFieldKey("totp#3")).toEqual({ base: "totp", n: 3 });
    for (const bad of ["totp#", "totp#1", "totp#0", "totp#02", "totp#-2", "totp#2a", "totp#1000"]) expect(parseFieldKey(bad), bad).toBeNull();
    expect(instanceKey("totp", 1)).toBe("totp");
    expect(instanceKey("totp", 4)).toBe("totp#4");
  });

  it("lets only a login's sign-ins and codes repeat, as the core does", () => {
    for (const k of ["username#2", "password#10", "totp#3"]) expect(specOf("login", k), k).not.toBeNull();
    for (const k of ["url#2", "notes#2", "totp#11", "username#1", "nope#2"]) expect(specOf("login", k), k).toBeNull();
    expect(specOf("totp", "totp#2")).toBeNull();
    expect(specOf("card", "number#2")).toBeNull();
  });

  it("lists instances in number order, so #10 follows #9", () => {
    const f = { "totp#10": "c", totp: "a", "totp#9": "b", other: "x" };
    expect(instancesOf(f, "totp").map((x) => x.key)).toEqual(["totp", "totp#9", "totp#10"]);
  });
});

describe("slots", () => {
  it("shows the first, empty slot when there are none", () => {
    expect(rowsOf({}, SIGN_IN)).toEqual([1]);
    expect(rowsOf({ title: "x" }, CODE)).toEqual([1]);
    expect(rowsOf({ "username#3": "", password: "p" }, SIGN_IN)).toEqual([1, 3]);
  });

  it("adds a blank slot at the lowest free number and stops at the limit", () => {
    let f: Record<string, string> = { username: "a", password: "p" };
    f = addSlot(f, SIGN_IN)!;
    expect(slotsOf(f, SIGN_IN)).toEqual([1, 2]);
    expect(f["username#2"]).toBe("");
    expect(slotFilled(f, SIGN_IN, 2)).toBe(false);
    f = addSlot({ ...f, "username#2": "b" }, SIGN_IN)!;
    expect(slotsOf(f, SIGN_IN)).toEqual([1, 2, 3]);
    // a gap is reused
    const gap = addSlot({ username: "a", "username#3": "c" }, SIGN_IN)!;
    expect(slotsOf(gap, SIGN_IN)).toEqual([1, 2, 3]);
    // an empty code list gets its second slot, the first being the shown blank one
    expect(slotsOf(addSlot({}, CODE)!, CODE)).toEqual([2]);
    let full: Record<string, string> = { totp: "a" };
    for (let i = 2; i <= 10; i++) full[`totp#${i}`] = "x";
    expect(addSlot(full, CODE)).toBeNull();
    full = removeSlot(full, CODE, 5);
    expect(addSlot(full, CODE)).not.toBeNull();
  });

  it("removes both fields of a sign-in and nothing else", () => {
    const f = { username: "a", password: "p", "username#2": "b", "password#2": "q", url: "u" };
    expect(removeSlot(f, SIGN_IN, 2)).toEqual({ username: "a", password: "p", url: "u" });
    expect(f["username#2"]).toBe("b"); // the input is not changed
  });

  it("compacts to 1, 2, 3 …, dropping blanks, keeping order and every other field", () => {
    const f = {
      username: "",
      password: "",
      "username#2": "second",
      "password#2": "pw2",
      "username#3": "",
      "username#5": "fifth",
      url: "u",
      notes: "n",
    };
    expect(compactSlots(f, SIGN_IN)).toEqual({ username: "second", password: "pw2", "username#2": "fifth", url: "u", notes: "n" });
    expect(compactSlots({ totp: "", "totp#2": "B", "totp#4": "D" }, CODE)).toEqual({ totp: "B", "totp#2": "D" });
    expect(compactSlots({ totp: "A" }, CODE)).toEqual({ totp: "A" });
  });

  it("ignores blanks when asking whether anything changed", () => {
    expect(withoutBlanks({ a: "x", b: "", "totp#2": "" })).toEqual({ a: "x" });
  });
});

describe("groupSignIns", () => {
  const f = (...keys: string[]) => keys.map((key) => ({ key }));
  it("pairs each username with its password and keeps the rest where it was", () => {
    const out = groupSignIns(f("username", "username#2", "password", "password#2", "url", "totp", "totp#2", "notes"));
    expect(out.map((x) => x.key)).toEqual(["username", "password", "username#2", "password#2", "url", "totp", "totp#2", "notes"]);
  });
  it("copes with a sign-in that has only one half and with no sign-ins at all", () => {
    expect(groupSignIns(f("password#3", "username", "url")).map((x) => x.key)).toEqual(["username", "password#3", "url"]);
    expect(groupSignIns(f("url", "notes")).map((x) => x.key)).toEqual(["url", "notes"]);
  });
});
