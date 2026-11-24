import { ORIGINAL_NAME } from "../lib/disguise";
import type { DisguiseView } from "../backend/types";
import { createStore } from "./store";

/**
 * What the app shows for itself: a name and, when a disguise icon is chosen, that icon. It follows the disguise so a
 * renamed app never says "Passman" or draws the keyhole on screen.
 */
export const brand = createStore<{ name: string; presetId: string | null }>({ name: ORIGINAL_NAME, presetId: null });

export function setBrand(d: Pick<DisguiseView, "appliedName" | "presetId">): void {
  const name = d.appliedName.trim() || ORIGINAL_NAME;
  brand.set(() => ({ name, presetId: d.presetId }));
  document.title = name;
}
