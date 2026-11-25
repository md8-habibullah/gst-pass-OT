import { Archive, CloudUpload, Download, Files, Fingerprint, Image, KeyRound, CreditCard, Settings, StickyNote, Star, Terminal, Timer, Trash2, Upload, Vault, Wand2, type LucideIcon } from "lucide-react";
import type { ItemType } from "../../backend";
import type { Filter } from "../../state/app";

export interface NavEntry {
  filter: Filter;
  label: string;
  icon: LucideIcon;
}

/** Sidebar groups, top to bottom. A hairline separates the groups. */
export const NAV_GROUPS: NavEntry[][] = [
  [
    { filter: "all", label: "All items", icon: Vault },
    { filter: "logins", label: "Logins", icon: KeyRound },
    { filter: "notes", label: "Notes", icon: StickyNote },
    { filter: "identities", label: "Identities", icon: Fingerprint },
    { filter: "cards", label: "Cards", icon: CreditCard },
    { filter: "ssh", label: "SSH keys", icon: Terminal },
    { filter: "media", label: "Media", icon: Image },
    { filter: "files", label: "Files", icon: Files },
  ],
  [
    { filter: "codes", label: "Codes", icon: Timer },
    { filter: "favorites", label: "Favorites", icon: Star },
    { filter: "archive", label: "Archive", icon: Archive },
    { filter: "trash", label: "Trash", icon: Trash2 },
  ],
  [
    { filter: "generator", label: "Generator", icon: Wand2 },
    { filter: "import", label: "Import", icon: Download },
    { filter: "export", label: "Export", icon: Upload },
    { filter: "backup", label: "Backup", icon: CloudUpload },
  ],
  [{ filter: "settings", label: "Settings", icon: Settings }],
];

/** The icon rail (medium layout) shows the places people go most; the rest sit in the list pane's type chips. */
export const RAIL: NavEntry[][] = [
  [
    { filter: "all", label: "All items", icon: Vault },
    { filter: "codes", label: "Codes", icon: Timer },
    { filter: "archive", label: "Archive", icon: Archive },
    { filter: "trash", label: "Trash", icon: Trash2 },
  ],
  [
    { filter: "generator", label: "Generator", icon: Wand2 },
    { filter: "import", label: "Import", icon: Download },
    { filter: "export", label: "Export", icon: Upload },
    { filter: "backup", label: "Backup", icon: CloudUpload },
  ],
  [{ filter: "settings", label: "Settings", icon: Settings }],
];

export const TITLE: Record<Filter, string> = {
  all: "All items",
  logins: "Logins",
  cards: "Cards",
  identities: "Identities",
  ssh: "SSH keys",
  notes: "Notes",
  media: "Media",
  files: "Files",
  favorites: "Favorites",
  codes: "Codes",
  archive: "Archive",
  trash: "Trash",
  generator: "Generator",
  import: "Import",
  export: "Export",
  backup: "Backup",
  settings: "Settings",
};

/** Filters whose main content is a list of items with a detail pane. */
export const ITEM_FILTERS: ReadonlySet<Filter> = new Set<Filter>(["all", "logins", "cards", "identities", "ssh", "notes", "media", "favorites", "archive"]);

/** Full-width views without a list/detail split. */
export const WIDE_VIEWS: ReadonlySet<Filter> = new Set<Filter>(["codes", "files", "generator", "import", "export", "backup", "settings"]);

/** The four tabs of the compact bottom navigation. Everything else is reached from these. */
export const TABS: { filter: Filter; label: string; icon: LucideIcon; owns: ReadonlySet<Filter> }[] = [
  { filter: "all", label: "Vault", icon: Vault, owns: new Set<Filter>([...ITEM_FILTERS, "trash"]) },
  { filter: "codes", label: "Codes", icon: Timer, owns: new Set<Filter>(["codes"]) },
  { filter: "generator", label: "Generator", icon: Wand2, owns: new Set<Filter>(["generator"]) },
  { filter: "settings", label: "Settings", icon: Settings, owns: new Set<Filter>(["settings", "files", "import", "export", "backup"]) },
];

/** Type chips in the list pane (compact and medium layouts), then the two special lists at the end. */
export const CHIPS: { filter: Filter; label: string }[] = [
  { filter: "all", label: "All" },
  { filter: "logins", label: "Logins" },
  { filter: "notes", label: "Notes" },
  { filter: "identities", label: "Identities" },
  { filter: "cards", label: "Cards" },
  { filter: "ssh", label: "SSH keys" },
  { filter: "media", label: "Media" },
  { filter: "favorites", label: "Favorites" },
  { filter: "archive", label: "Archive" },
  { filter: "trash", label: "Trash" },
];

/** The item type a list filter is about, so "New" starts on the right form. */
export const FILTER_TYPE: Partial<Record<Filter, ItemType>> = {
  logins: "login",
  cards: "card",
  identities: "identity",
  ssh: "sshKey",
  notes: "secureNote",
  media: "media",
  codes: "totp",
};

/** Sub-pages reached from Settings on the compact layout (they get a back button). */
export const SETTINGS_CHILDREN: ReadonlySet<Filter> = new Set<Filter>(["import", "export", "backup", "files"]);
