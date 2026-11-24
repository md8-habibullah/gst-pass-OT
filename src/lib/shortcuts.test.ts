import { describe, expect, it } from "vitest";
import { GROUPS, SHORTCUTS, keycaps, matchKey, type KeyLike } from "./shortcuts";

const key = (k: string, mods: Partial<KeyLike> = {}): KeyLike => ({ key: k, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods });
const idle = { typing: false, pending: null, isMac: false } as const;

describe("shortcut table", () => {
  it("has no combo bound twice and every id once", () => {
    const combos = SHORTCUTS.flatMap((s) => s.combos);
    expect(new Set(combos).size).toBe(combos.length);
    const ids = SHORTCUTS.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
  it("puts every shortcut in a known group", () => {
    for (const s of SHORTCUTS) expect(GROUPS).toContain(s.group);
  });
  it("is generous: well over thirty shortcuts", () => {
    expect(SHORTCUTS.length).toBeGreaterThan(30);
  });
});

describe("matchKey", () => {
  it("runs single keys when idle and ignores them while typing", () => {
    expect(matchKey(key("n"), idle)).toMatchObject({ kind: "run", id: "new" });
    expect(matchKey(key("n"), { ...idle, typing: true })).toBeNull();
    expect(matchKey(key("j"), idle)).toMatchObject({ id: "next" });
  });
  it("treats ? as one key even though it needs Shift", () => {
    expect(matchKey(key("?", { shiftKey: true }), idle)).toMatchObject({ id: "help" });
  });
  it("runs mod combos everywhere, with Ctrl on Linux and Command on a Mac", () => {
    expect(matchKey(key("k", { ctrlKey: true }), { ...idle, typing: true })).toMatchObject({ id: "search" });
    expect(matchKey(key("k", { metaKey: true }), idle)).toBeNull();
    expect(matchKey(key("k", { metaKey: true }), { ...idle, isMac: true })).toMatchObject({ id: "search" });
    expect(matchKey(key(",", { ctrlKey: true }), idle)).toMatchObject({ id: "settings" });
    expect(matchKey(key("L", { ctrlKey: true, shiftKey: true }), idle)).toMatchObject({ id: "lockApp" });
    expect(matchKey(key("l", { ctrlKey: true }), idle)).toMatchObject({ id: "lock" });
  });
  it("opens help with F1 and leaves the field with Escape even while typing", () => {
    expect(matchKey(key("F1"), { ...idle, typing: true })).toMatchObject({ id: "help" });
    expect(matchKey(key("Escape"), { ...idle, typing: true })).toMatchObject({ id: "escape" });
  });
  it("does the two-key chords: g then a letter", () => {
    expect(matchKey(key("g"), idle)).toEqual({ kind: "prefix", prefix: "g" });
    expect(matchKey(key("a"), { ...idle, pending: "g" })).toMatchObject({ kind: "run", id: "go.all" });
    expect(matchKey(key("t"), { ...idle, pending: "g" })).toMatchObject({ id: "go.trash" });
    expect(matchKey(key("g"), { ...idle, typing: true })).toBeNull();
  });
  it("a wrong second key cancels the chord and still works as itself", () => {
    expect(matchKey(key("n"), { ...idle, pending: "g" })).toMatchObject({ id: "go.notes" });
    expect(matchKey(key("z"), { ...idle, pending: "g" })).toBeNull();
    expect(matchKey(key("j"), { ...idle, pending: "g" })).toMatchObject({ id: "next" });
  });
  it("ignores combinations with Alt or the other system key, and unknown keys", () => {
    expect(matchKey(key("n", { altKey: true }), idle)).toBeNull();
    expect(matchKey(key("n", { metaKey: true }), idle)).toBeNull();
    expect(matchKey(key("q"), idle)).toBeNull();
  });
});

describe("keycaps", () => {
  it("writes combos the way they are pressed", () => {
    expect(keycaps("mod+k", false)).toEqual(["Ctrl", "K"]);
    expect(keycaps("mod+k", true)).toEqual(["⌘", "K"]);
    expect(keycaps("mod+shift+l", false)).toEqual(["Ctrl", "Shift", "L"]);
    expect(keycaps("g a", false)).toEqual(["G", "then", "A"]);
    expect(keycaps("mod+,", false)).toEqual(["Ctrl", ","]);
    expect(keycaps("Escape", false)).toEqual(["Esc"]);
    expect(keycaps("?", false)).toEqual(["?"]);
    expect(keycaps("F1", false)).toEqual(["F1"]);
  });
});
