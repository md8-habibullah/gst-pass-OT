import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { applyTheme, resolveTheme } from "./theme";

const root = () => ({ dataset: {} as Record<string, string | undefined> }) as unknown as HTMLElement;

describe("theme", () => {
  it("a chosen theme is written on the page, 'system' leaves the choice to the device", () => {
    const el = root();
    applyTheme("light", el);
    expect(el.dataset.theme).toBe("light");
    applyTheme("dark", el);
    expect(el.dataset.theme).toBe("dark");
    applyTheme("system", el);
    expect("theme" in el.dataset).toBe(false);
  });

  it("'system' resolves to what the device says, a chosen theme to itself", () => {
    expect(resolveTheme("system", true)).toBe("light");
    expect(resolveTheme("system", false)).toBe("dark");
    expect(resolveTheme("dark", true)).toBe("dark");
    expect(resolveTheme("light", false)).toBe("light");
  });

  it("the page and the Android activity agree on the bridge: same object, same method, only light or dark", () => {
    const page = readFileSync(new URL("./theme.ts", import.meta.url), "utf8");
    const kotlin = readFileSync(new URL("../../scripts/android/MainActivity.kt.tmpl", import.meta.url), "utf8");
    expect(page).toContain("HexisBars");
    expect(page).toContain("setLook");
    expect(kotlin).toContain('addJavascriptInterface(BarsBridge(), "HexisBars")');
    expect(kotlin).toContain("fun setLook(look: String)");
    expect(kotlin).toContain("@JavascriptInterface");
  });

  it("with no bridge (desktop, browser, a test) applying a theme is quiet", () => {
    expect(() => applyTheme("dark", root())).not.toThrow();
  });
});
