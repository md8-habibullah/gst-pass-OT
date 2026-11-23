import { Image, KeyRound, Settings, StickyNote, Star, Timer, Trash2, Vault, type LucideIcon } from "lucide-react";
import type { Filter } from "../../state/app";

export interface NavEntry {
  filter: Filter;
  label: string;
  icon: LucideIcon;
}

export const NAV: NavEntry[] = [
  { filter: "all", label: "All items", icon: Vault },
  { filter: "logins", label: "Logins", icon: KeyRound },
  { filter: "codes", label: "Codes", icon: Timer },
  { filter: "notes", label: "Notes", icon: StickyNote },
  { filter: "media", label: "Media", icon: Image },
  { filter: "favorites", label: "Favorites", icon: Star },
  { filter: "trash", label: "Trash", icon: Trash2 },
  { filter: "settings", label: "Settings", icon: Settings },
];

export const TITLE: Record<Filter, string> = {
  all: "All items",
  logins: "Logins",
  codes: "Codes",
  notes: "Notes",
  media: "Media",
  favorites: "Favorites",
  trash: "Trash",
  settings: "Settings",
};

/** The four tabs of the compact bottom navigation. */
export const TABS: { filter: Filter; label: string; icon: LucideIcon }[] = [
  { filter: "all", label: "Vault", icon: Vault },
  { filter: "codes", label: "Codes", icon: Timer },
  { filter: "trash", label: "Trash", icon: Trash2 },
  { filter: "settings", label: "Settings", icon: Settings },
];

/** Filters whose main content is a list of items with a detail pane. */
export const ITEM_FILTERS: ReadonlySet<Filter> = new Set<Filter>(["all", "logins", "notes", "media", "favorites"]);
