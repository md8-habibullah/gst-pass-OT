import type { Theme } from "../backend";

/**
 * Colours: light, dark, or the device's own setting.
 *
 * The palette lives in `src/styles/tokens.css`. `<html data-theme="light|dark">` picks one on purpose; with no attribute the
 * stylesheet follows `prefers-color-scheme`. Nothing is stored in the web view: the choice is kept by the core in
 * `device.json` and applied here once the core has answered (and by the native window at start, for the first paint).
 */
export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement): void {
  if (theme === "light" || theme === "dark") root.dataset.theme = theme;
  else delete root.dataset.theme;
  if (typeof window !== "undefined" && root === window.document.documentElement) tellSystemBars(theme);
}

/**
 * Android draws the status and navigation bars itself, over the page, and picks their icon colour from the PHONE's setting.
 * A light page on a dark phone (or the other way round) would have icons that cannot be read. The app's activity offers
 * `HexisBars.setLook("light" | "dark")` (scripts/android/MainActivity.kt.tmpl); this tells it what the page really draws.
 * Anywhere else (desktop, a browser) there is no such object and nothing happens.
 */
let chosen: Theme = "system";
let watching = false;
function tellSystemBars(theme: Theme): void {
  chosen = theme;
  try {
    const bars = (window as unknown as { HexisBars?: { setLook?: (look: string) => void } }).HexisBars;
    if (!bars || typeof bars.setLook !== "function") return;
    const device = window.matchMedia("(prefers-color-scheme: light)");
    bars.setLook(resolveTheme(theme, device.matches));
    if (!watching) {
      watching = true;
      // "Device setting" follows the phone while the app is open (dusk, or a switch in the quick settings)
      device.addEventListener("change", () => tellSystemBars(chosen));
    }
  } catch {
    // the bridge is a nicety: a failure must never stop the page
  }
}

/** What is drawn right now: the chosen colours, or the device's own when the choice is "system". */
export function resolveTheme(theme: Theme, deviceIsLight: boolean): "light" | "dark" {
  return theme === "system" ? (deviceIsLight ? "light" : "dark") : theme;
}

export const THEME_LABEL: Record<Theme, string> = { system: "Device setting", light: "Light", dark: "Dark" };
