import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// The palette exists in four places in tokens.css (the dark @theme, the scoped dark island, the light palette for a chosen
// theme and the same light palette for "follow the device"). These tests keep the copies identical and keep every text
// colour readable on every surface in BOTH palettes (WCAG 2.x contrast).
const css = readFileSync(new URL("./tokens.css", import.meta.url), "utf8");

/** The text between the braces that follow `opener` (nesting counted). */
function blockAfter(opener: string, from = 0): string {
  const at = css.indexOf(opener, from);
  if (at < 0) throw new Error(`${opener} not found in tokens.css`);
  let depth = 0;
  for (let i = css.indexOf("{", at); i < css.length; i++) {
    if (css[i] === "{") depth++;
    if (css[i] === "}" && --depth === 0) return css.slice(css.indexOf("{", at) + 1, i);
  }
  throw new Error(`${opener} is not closed`);
}

function declarations(block: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of block.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) out[m[1]!] = m[2]!.trim().replace(/\s+/g, " ");
  return out;
}

const theme = declarations(blockAfter("@theme"));
const rootBlock = declarations(blockAfter(":root {"));
const dark = { ...theme, ...rootBlock };
const island = declarations(blockAfter('[data-palette="dark"]'));
const lightChosen = declarations(blockAfter(':root[data-theme="light"]'));
const lightFollow = declarations(blockAfter(":root:not([data-theme])"));

const PALETTE_KEYS = Object.keys(island).filter((k) => k !== "color-scheme");

describe("the palettes in tokens.css", () => {
  it("the dark island repeats the dark palette exactly", () => {
    for (const key of PALETTE_KEYS) expect(island[key], key).toBe(dark[key]);
    // and nothing in the dark palette that changes with the theme is missing from it
    const themed = Object.keys(lightChosen).filter((k) => !k.startsWith("--color-scheme"));
    for (const key of themed) expect(island[key], `${key} is missing from the dark island`).toBeDefined();
  });

  it("the light palette for a chosen theme and for 'follow the device' are the same", () => {
    expect(lightFollow).toEqual(lightChosen);
  });

  it("the light palette redefines every colour the dark one has, and nothing that is not there", () => {
    for (const key of Object.keys(lightChosen)) expect(dark[key], `${key} has no dark counterpart`).toBeDefined();
    for (const key of Object.keys(theme).filter((k) => k.startsWith("--color-") && !["--color-white", "--color-transparent"].includes(k))) {
      expect(lightChosen[key], `${key} has no light value`).toBeDefined();
    }
    // the same variables are themed in both palettes
    expect(Object.keys(island).sort()).toEqual([...Object.keys(lightChosen)].sort());
  });

  it("every light value differs from its dark one, except the few that are meant to be shared", () => {
    const shared = new Set(["--color-on-brass", "--color-danger", "--shadow-key-down"]);
    for (const key of Object.keys(lightChosen)) {
      if (shared.has(key)) continue;
      expect(lightChosen[key], `${key} is the same in both palettes`).not.toBe(dark[key]);
    }
  });
});

// ---- contrast

type Rgb = [number, number, number];
function parse(value: string): { rgb: Rgb; alpha: number } {
  const hex = /^#([0-9a-f]{6})$/i.exec(value);
  if (hex) return { rgb: [0, 2, 4].map((i) => parseInt(hex[1]!.slice(i, i + 2), 16)) as Rgb, alpha: 1 };
  const fn = /^rgb\((\d+) (\d+) (\d+)(?: \/ ([\d.]+))?\)$/.exec(value);
  if (fn) return { rgb: [Number(fn[1]), Number(fn[2]), Number(fn[3])], alpha: fn[4] === undefined ? 1 : Number(fn[4]) };
  throw new Error(`cannot read the colour ${value}`);
}
function over(top: string, alpha: number, ground: Rgb): Rgb {
  const t = parse(top).rgb;
  return [0, 1, 2].map((i) => Math.round(t[i]! * alpha + ground[i]! * (1 - alpha))) as Rgb;
}
const lin = (c: number) => ((c /= 255) <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const luminance = (c: Rgb) => 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
const ratio = (a: Rgb, b: Rgb) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
};

const palettes: Record<string, Record<string, string>> = { dark, light: { ...dark, ...lightChosen } };

describe.each(Object.entries(palettes))("the %s palette", (_name, p) => {
  const c = (token: string): Rgb => parse(p[`--color-${token}`]!).rgb;
  const surfaces = ["marine", "pane", "raised", "hover"] as const;

  // text on every surface it can sit on: at least 4.5 (3 for large or non-text marks)
  it.each([
    ["frost", 7],
    ["mist", 4.5],
    ["fog", 4.5],
    ["brass-ink", 4.5],
    ["brass-ink-hi", 4.5],
    ["verdigris", 4.5],
    ["ember", 4.5],
  ] as const)("%s is readable on every surface (at least %s)", (token, min) => {
    for (const s of surfaces) expect(ratio(c(token), c(s)), `${token} on ${s}`).toBeGreaterThanOrEqual(min);
  });

  it("the label on the key button is readable in every state of the button", () => {
    for (const fill of ["brass", "brass-hi", "brass-lo"]) expect(ratio(c("on-brass"), c(fill)), `on ${fill}`).toBeGreaterThanOrEqual(4.5);
  });

  it("white text on the danger button is readable", () => {
    expect(ratio([255, 255, 255], c("danger"))).toBeGreaterThanOrEqual(4.5);
  });

  it("error text stays readable on the red wash behind an error message", () => {
    for (const s of ["pane", "raised"] as const) {
      const wash = over(p["--color-danger"]!, 0.1, c(s));
      expect(ratio(c("ember"), wash), `ember on the wash over ${s}`).toBeGreaterThanOrEqual(4.5);
      expect(ratio(c("frost"), wash), `frost on the wash over ${s}`).toBeGreaterThanOrEqual(7);
    }
  });

  it("the accent washes (selected rows, switches) keep their text readable", () => {
    for (const s of ["pane", "raised"] as const) {
      for (const a of [0.15, 0.2, 0.25]) {
        const wash = over(p["--color-brass"]!, a, c(s));
        expect(ratio(c("brass-ink"), wash), `brass-ink on ${a} over ${s}`).toBeGreaterThanOrEqual(4.5);
        expect(ratio(c("frost"), wash), `frost on ${a} over ${s}`).toBeGreaterThanOrEqual(7);
      }
    }
  });

  it("lines and marks that carry meaning reach 3:1 on the panes (focus ring, selection edge, dots)", () => {
    for (const s of ["marine", "pane", "raised"] as const) {
      for (const token of ["brass-ink", "verdigris", "ember"]) expect(ratio(c(token), c(s)), `${token} on ${s}`).toBeGreaterThanOrEqual(3);
    }
  });

  it("each surface step is a visible step from the one before (ground, pane, raised, hover)", () => {
    // not WCAG, a sanity check against a palette that flattens into one grey
    expect(ratio(c("marine"), c("pane"))).toBeGreaterThan(1.03);
    expect(ratio(c("pane"), c("raised"))).toBeGreaterThan(1.03);
    expect(ratio(c("raised"), c("hover"))).toBeGreaterThan(1.03);
  });

  it("the hairline is a quiet line, not a border", () => {
    const line = parse(p["--color-hairline"]!);
    expect(line.alpha).toBeGreaterThan(0.05);
    expect(line.alpha).toBeLessThanOrEqual(0.15);
    const strong = parse(p["--color-hairline-strong"]!);
    expect(strong.alpha).toBeGreaterThan(line.alpha);
  });
});

describe("what the page needs before any script runs", () => {
  it("tells the browser both schemes exist, so form controls and scrollbars follow the page", () => {
    const html = readFileSync(new URL("../../index.html", import.meta.url), "utf8");
    expect(html).toMatch(/<meta name="color-scheme" content="light dark"/);
    expect(dark["color-scheme"] ?? css.match(/:root \{\s*color-scheme: dark;/)?.[0]).toBeTruthy();
    expect(css).toMatch(/:root\[data-theme="light"\][^{]*\{\s*color-scheme: light;/);
  });
});
