// The 11 launcher disguises. `assets/disguise/presets.json` is the single source for ids, names and tile colours: the Rust
// core embeds the same file to validate a choice, and the Android patch script reads it to write the manifest aliases.
import data from "../../assets/disguise/presets.json";

export interface DisguisePreset { id: string; name: string; description: string; background: string }
export const DISGUISES: readonly DisguisePreset[] = data.presets;
export const ORIGINAL_NAME = "Passman";

const tiles = import.meta.glob<string>("../../assets/disguise/*.svg", { eager: true, query: "?url", import: "default" });
const urlFor = (file: string) => tiles[`../../assets/disguise/${file}.svg`];

/** The full rounded tile, for previews. */
export const disguiseTile = (id: string): string | undefined => urlFor(id);

export interface DisguiseChoice { presetId: string | null; customName: string | null; lowercase: boolean }

/** The name shown in the launcher and in the app: custom name first, then the preset, then the original. */
export function disguiseName(c: DisguiseChoice): string {
  const base = c.customName?.trim() || DISGUISES.find((d) => d.id === c.presetId)?.name || ORIGINAL_NAME;
  return c.lowercase ? base.toLowerCase() : base;
}
