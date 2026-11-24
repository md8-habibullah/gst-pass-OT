// Every keyboard shortcut in one table. The handler (state/shortcuts.ts), the "?" overlay and the Help page in Settings all
// read this list, so what is shown is exactly what works.
//
// A combo is a string:
//   "n"            one key, no modifier
//   "mod+k"        Ctrl on Windows and Linux, Command on macOS
//   "mod+shift+l"  more modifiers
//   "g a"          two keys in a row (press g, release, press a within a second and a half)
// Single keys and chords are ignored while someone is typing in a field or a dialog is open; "mod" combos and F1 work everywhere.

export type ShortcutGroup = "General" | "Go to" | "Items" | "Selected item";

export interface Shortcut {
  id: string;
  group: ShortcutGroup;
  label: string;
  /** The first combo is the one shown in menus and tooltips; the rest are alternatives. */
  combos: string[];
}

export const SHORTCUTS: Shortcut[] = [
  { id: "help", group: "General", label: "Show keyboard shortcuts", combos: ["?", "mod+/", "F1"] },
  { id: "search", group: "General", label: "Search items", combos: ["/", "mod+k"] },
  { id: "new", group: "General", label: "New item", combos: ["n", "mod+n"] },
  { id: "settings", group: "General", label: "Open settings", combos: ["mod+,", "g s"] },
  { id: "lock", group: "General", label: "Lock the vault", combos: ["mod+l"] },
  { id: "lockApp", group: "General", label: "Lock the app", combos: ["mod+shift+l"] },
  { id: "escape", group: "General", label: "Close, clear or go back", combos: ["Escape"] },

  { id: "go.all", group: "Go to", label: "All items", combos: ["g a"] },
  { id: "go.logins", group: "Go to", label: "Logins", combos: ["g l"] },
  { id: "go.cards", group: "Go to", label: "Cards", combos: ["g c"] },
  { id: "go.identities", group: "Go to", label: "Identities", combos: ["g i"] },
  { id: "go.notes", group: "Go to", label: "Notes", combos: ["g n"] },
  { id: "go.ssh", group: "Go to", label: "SSH keys", combos: ["g k"] },
  { id: "go.codes", group: "Go to", label: "Codes", combos: ["g o"] },
  { id: "go.favorites", group: "Go to", label: "Favorites", combos: ["g f"] },
  { id: "go.archive", group: "Go to", label: "Archive", combos: ["g r"] },
  { id: "go.trash", group: "Go to", label: "Trash", combos: ["g t"] },
  { id: "go.files", group: "Go to", label: "Files", combos: ["g d"] },
  { id: "go.generator", group: "Go to", label: "Password generator", combos: ["g g"] },
  { id: "go.import", group: "Go to", label: "Import", combos: ["g m"] },
  { id: "go.export", group: "Go to", label: "Export", combos: ["g x"] },
  { id: "go.backup", group: "Go to", label: "Backup", combos: ["g u"] },
  { id: "go.help", group: "Go to", label: "Help and shortcuts", combos: ["g h"] },

  { id: "next", group: "Items", label: "Next item in the list", combos: ["j"] },
  { id: "prev", group: "Items", label: "Previous item in the list", combos: ["k"] },

  { id: "edit", group: "Selected item", label: "Edit", combos: ["e"] },
  { id: "copyUser", group: "Selected item", label: "Copy the username or email", combos: ["u"] },
  { id: "copyPass", group: "Selected item", label: "Copy the password, card number or private key", combos: ["p"] },
  { id: "open", group: "Selected item", label: "Open the website", combos: ["o"] },
  { id: "copyCode", group: "Selected item", label: "Copy the one-time code", combos: ["c"] },
  { id: "clone", group: "Selected item", label: "Make a copy", combos: ["mod+d"] },
  { id: "archive", group: "Selected item", label: "Archive or unarchive", combos: ["a"] },
  { id: "trash", group: "Selected item", label: "Move to the trash", combos: ["Delete"] },
];

export const GROUPS: ShortcutGroup[] = ["General", "Go to", "Items", "Selected item"];

/** How long the second key of a chord may take. */
export const CHORD_MS = 1500;

export interface KeyLike {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

export type Match =
  | { kind: "run"; id: string; combo: string }
  | { kind: "prefix"; prefix: string }
  | null;

interface Parsed { mod: boolean; shift: boolean; key: string }

function parse(combo: string): Parsed {
  const parts = combo.split("+");
  // a literal "+" or "," at the end survives the split: "mod+," -> ["mod", ","]
  const key = parts[parts.length - 1] || "+";
  const mods = parts.slice(0, -1);
  return { mod: mods.includes("mod"), shift: mods.includes("shift"), key };
}

const TABLE = (() => {
  const single = new Map<string, { id: string; combo: string }>();
  const chords = new Map<string, { id: string; combo: string }>();
  const prefixes = new Set<string>();
  for (const s of SHORTCUTS) {
    for (const combo of s.combos) {
      if (combo.includes(" ")) {
        const [a, b] = combo.split(" ") as [string, string];
        chords.set(`${a} ${b}`, { id: s.id, combo });
        prefixes.add(a);
      } else {
        const p = parse(combo);
        single.set(`${p.mod ? "mod+" : ""}${p.shift ? "shift+" : ""}${p.key.length === 1 ? p.key.toLowerCase() : p.key}`, { id: s.id, combo });
      }
    }
  }
  return { single, chords, prefixes };
})();

/**
 * Decides what one key press means. `typing`: the target is a text field or a dialog is open. `pending`: the first key of a
 * chord, if one was pressed a moment ago. Modifier-less keys and chords do nothing while typing; "mod" combos and F1 always work.
 */
export function matchKey(e: KeyLike, opts: { typing: boolean; pending: string | null; isMac: boolean }): Match {
  const mod = opts.isMac ? e.metaKey : e.ctrlKey;
  const other = opts.isMac ? e.ctrlKey : e.metaKey;
  if (other || e.altKey) return null;
  const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;

  if (mod) {
    const hit = TABLE.single.get(`mod+${e.shiftKey ? "shift+" : ""}${k}`);
    return hit ? { kind: "run", ...hit } : null;
  }
  if (k === "F1") {
    const hit = TABLE.single.get("F1");
    return hit ? { kind: "run", ...hit } : null;
  }
  if (k === "Escape") {
    // Escape is also how a field is left, so it works while typing
    const hit = TABLE.single.get("Escape");
    return hit ? { kind: "run", ...hit } : null;
  }
  if (opts.typing) return null;
  if (opts.pending) {
    const hit = TABLE.chords.get(`${opts.pending} ${k}`);
    if (hit) return { kind: "run", ...hit };
    // a wrong second key cancels the chord; fall through so it can still act as a single key
  }
  if (!e.shiftKey || e.key.length === 1) {
    if (TABLE.prefixes.has(k) && !e.shiftKey) return { kind: "prefix", prefix: k };
    // "?" needs Shift on most layouts: the key itself is the match, so Shift is not part of the lookup
    const hit = TABLE.single.get(k);
    if (hit) return { kind: "run", ...hit };
  }
  return null;
}

/** The keycaps to draw for a combo: "mod+k" -> ["Ctrl", "K"] (["⌘", "K"] on macOS), "g a" -> ["G", "then", "A"]. */
export function keycaps(combo: string, isMac: boolean): string[] {
  if (combo.includes(" ")) {
    const [a, b] = combo.split(" ") as [string, string];
    return [a.toUpperCase(), "then", b.toUpperCase()];
  }
  const p = parse(combo);
  const out: string[] = [];
  if (p.mod) out.push(isMac ? "⌘" : "Ctrl");
  if (p.shift) out.push(isMac ? "⇧" : "Shift");
  out.push(NAMED[p.key] ?? (p.key.length === 1 ? p.key.toUpperCase() : p.key));
  return out;
}

const NAMED: Record<string, string> = { Escape: "Esc", Delete: "Del", ArrowDown: "↓", ArrowUp: "↑" };

export const isMacPlatform = (): boolean => typeof navigator !== "undefined" && /Mac|iPhone|iPad/i.test(navigator.platform || navigator.userAgent);

export const shortcutById = (id: string): Shortcut | undefined => SHORTCUTS.find((s) => s.id === id);
