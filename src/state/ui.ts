import type { EditSession, ItemType } from "../backend";
import { createStore } from "./store";

/** What the item editor is currently doing. `new` starts with the type picker unless a type is given. */
export type EditorRequest =
  | { mode: "new"; type: ItemType | null; /** Field values to start with (the scanner passes a website or an authenticator export). */ prefill?: Record<string, string> }
  | { mode: "edit"; id: string; session: EditSession };

/** Where the lock screen should start when it next appears (set by "Add account" and the like). */
export type LockFlow = "create" | "import" | "restore";
export type LockIntent = LockFlow | "choose";

/** The pages of Settings. `null` is the list of categories (a phone) or the first page (a wide screen). */
export type SettingsPageId = "security" | "list" | "applock" | "passkeys" | "browsers" | "accounts" | "time" | "theme" | "appearance" | "data" | "help" | "about";

/** A folder dialog: put items in a folder, make one, rename one, or remove one. Opened from the list, the item menu and the item screen. */
export type FolderDialog =
  | { kind: "move"; ids: string[]; title: string; current: string | null }
  | { kind: "create"; parent: string | null }
  | { kind: "rename"; path: string }
  | { kind: "remove"; path: string };

/** The all-in-one scanner: opened by the camera button. `camera` starts the camera at once (phones); `choose` shows how to give it a picture or a text. */
export interface ScannerRequest {
  start: "camera" | "choose";
}

export interface UiState {
  editor: EditorRequest | null;
  scanner: ScannerRequest | null;
  folderDialog: FolderDialog | null;
  lockIntent: LockIntent | null;
  /** The keyboard shortcut overlay ("?"). */
  shortcutsOpen: boolean;
  settingsPage: SettingsPageId | null;
}

export const ui = createStore<UiState>({ editor: null, scanner: null, folderDialog: null, lockIntent: null, shortcutsOpen: false, settingsPage: null });

export const openFolderDialog = (folderDialog: FolderDialog) => ui.set((s) => ({ ...s, folderDialog }));
export const closeFolderDialog = () => ui.set((s) => ({ ...s, folderDialog: null }));

export const setShortcutsOpen = (shortcutsOpen: boolean) => ui.set((s) => ({ ...s, shortcutsOpen }));
export const setSettingsPage = (settingsPage: SettingsPageId | null) => ui.set((s) => ({ ...s, settingsPage }));

export const setLockIntent = (lockIntent: LockIntent | null) => ui.set((s) => ({ ...s, lockIntent }));

export const openNewItem = (type: ItemType | null = null, prefill?: Record<string, string>) => ui.set((s) => ({ ...s, editor: { mode: "new", type, ...(prefill ? { prefill } : {}) } }));
export const openScanner = (start: ScannerRequest["start"] = "choose") => ui.set((s) => ({ ...s, scanner: { start } }));
export const closeScanner = () => ui.set((s) => ({ ...s, scanner: null }));
export const openEditor = (id: string, session: EditSession) => ui.set((s) => ({ ...s, editor: { mode: "edit", id, session } }));
export const closeEditor = () => ui.set((s) => ({ ...s, editor: null }));
