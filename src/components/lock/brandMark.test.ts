import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// The mark exists twice on purpose: as a file (README, site, favicon, icons) and as JSX (so the unlock moment can
// animate its layers). These tests keep the two drawings from drifting apart.
const svg = readFileSync(new URL("../../assets/logo-mark.svg", import.meta.url), "utf8");
const logo = readFileSync(new URL("../../assets/logo.svg", import.meta.url), "utf8");
const jsx = readFileSync(new URL("./VaultMark.tsx", import.meta.url), "utf8");

const pointsOf = (text: string) => [...text.matchAll(/points="([^"]+)"/g)].map((m) => m[1]);
const rectsOf = (text: string) => [...text.matchAll(/<rect x="(\d+)" y="(\d+)" width="(\d+)" height="(\d+)"/g)].map((m) => m.slice(1).join(","));

describe("the Hexis mark", () => {
  it("has the same polygons and pillars in the SVG file and in the component", () => {
    expect(pointsOf(svg).length).toBe(7);
    expect(pointsOf(jsx).sort()).toEqual(pointsOf(svg).sort());
    expect(rectsOf(jsx)).toEqual(rectsOf(svg));
  });

  it("uses the brand palette and nothing else", () => {
    const palette = new Set(["#0B0E14", "#F8FAFC", "#38BDF8", "#818CF8", "#FFFFFF", "#CBD5E1", "#94A3B8", "#475569"]);
    for (const text of [svg, logo, jsx]) {
      for (const hex of text.match(/#[0-9A-Fa-f]{6}\b/g) ?? []) expect(palette.has(hex.toUpperCase()), hex).toBe(true);
    }
  });

  it("is a transparent mark, and a full icon with a background", () => {
    expect(svg).not.toMatch(/<path/); // no ground behind the mark
    expect(logo).toMatch(/id="hx-squircle"[^>]*fill="#0B0E14"/);
  });

  it("is named for the product", () => {
    expect(svg).toContain("<title");
    expect(svg).toContain("Hexis");
    expect(logo).toContain("Hexis");
  });
});
