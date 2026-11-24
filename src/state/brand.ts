import { ORIGINAL_NAME } from "../lib/disguise";
import type { DisguiseView } from "../backend/types";
import { createStore, useStore } from "./store";

/**
 * What the app shows for itself: a name and, when a disguise icon is chosen, that icon. It follows the disguise so a
 * renamed app never says "Hexis" or draws the Hexis mark on screen. Every sentence that names the app takes the name
 * from here (`useBrandName()` in a component, `brandName()` elsewhere); nothing on screen hard-codes it.
 */
export const brand = createStore<{ name: string; presetId: string | null }>({ name: ORIGINAL_NAME, presetId: null });

export function setBrand(d: Pick<DisguiseView, "appliedName" | "presetId">): void {
  const name = d.appliedName.trim() || ORIGINAL_NAME;
  brand.set(() => ({ name, presetId: d.presetId }));
  document.title = name;
}

/** The name to print in a sentence: "Hexis", or whatever the disguise made of it. */
export const brandName = (): string => brand.get().name;
export const useBrandName = (): string => useStore(brand, (b) => b.name);
