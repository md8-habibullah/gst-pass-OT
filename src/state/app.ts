import { getBackend, type Backend, type ItemSummary, type SuiteInfo, type TrashEntry, type VaultStatus } from "../backend";
import { createStore } from "./store";

export type Phase = "boot" | "setup" | "locked" | "ready" | "failed";
export type Filter = "all" | "logins" | "codes" | "notes" | "media" | "favorites" | "trash" | "settings";
export type ClipboardClear = 15 | 30 | 60;

export interface ToastAction {
  label: string;
  run: () => void;
}
export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  text: string;
  action?: ToastAction;
  /** When set, a ring counts down this long (clipboard auto-clear). */
  countdownMs?: number;
  ttlMs: number;
}

export interface AppState {
  phase: Phase;
  bootError: string | null;
  capabilities: VaultStatus["capabilities"];
  suite: SuiteInfo | null;
  items: ItemSummary[];
  trash: TrashEntry[];
  filter: Filter;
  query: string;
  selectedId: string | null;
  /** True for the single render after unlocking so the shell can play the entrance sequence once. */
  justUnlocked: boolean;
  toasts: Toast[];
  clipboardClear: ClipboardClear;
}

const PREF_KEY = "passman.prefs.v1";

function loadPrefs(): Pick<AppState, "clipboardClear"> {
  try {
    const raw = JSON.parse(localStorage.getItem(PREF_KEY) ?? "{}") as { clipboardClear?: number };
    if (raw.clipboardClear === 15 || raw.clipboardClear === 30 || raw.clipboardClear === 60) return { clipboardClear: raw.clipboardClear };
  } catch {
    /* storage unavailable or corrupt: use defaults */
  }
  return { clipboardClear: 30 };
}

export const app = createStore<AppState>({
  phase: "boot",
  bootError: null,
  capabilities: { quickUnlock: false, biometric: false },
  suite: null,
  items: [],
  trash: [],
  filter: "all",
  query: "",
  selectedId: null,
  justUnlocked: false,
  toasts: [],
  ...loadPrefs(),
});

const patch = (p: Partial<AppState>) => app.set((s) => ({ ...s, ...p }));

let backend: Backend | null = null;
export function be(): Backend {
  if (!backend) throw new Error("backend not ready");
  return backend;
}

export async function boot(): Promise<void> {
  try {
    backend = await getBackend();
    const [status, suite] = await Promise.all([backend.status(), backend.suiteInfo()]);
    patch({ suite, capabilities: status.capabilities, phase: status.unlocked ? "ready" : status.exists ? "locked" : "setup" });
    if (status.unlocked) await refresh();
  } catch (e) {
    patch({ phase: "failed", bootError: e instanceof Error ? e.message : String(e) });
  }
}

export async function refresh(): Promise<void> {
  const b = be();
  const [items, trash] = await Promise.all([b.listItems(), b.listTrash()]);
  app.set((s) => ({
    ...s,
    items,
    trash,
    selectedId: s.selectedId && (items.some((i) => i.id === s.selectedId) || trash.some((t) => t.id === s.selectedId)) ? s.selectedId : null,
  }));
}

/** Loads the vault contents after a successful unlock but stays on the lock screen so it can play its exit. */
async function preload(): Promise<void> {
  await refresh();
  const damaged = (await be().status()).damagedItems ?? 0;
  if (damaged > 0) {
    pushToast({
      kind: "error",
      ttlMs: 15_000,
      text: `${damaged === 1 ? "One item" : `${damaged} items`} failed its integrity check and ${damaged === 1 ? "was" : "were"} left out. ${damaged === 1 ? "It" : "They"} may be damaged or tampered with. Everything else is intact.`,
    });
  }
}

/** Called by the lock screen once its unlock animation has finished. */
export function enterApp(): void {
  patch({ phase: "ready", justUnlocked: true, filter: "all", query: "", selectedId: null });
}

export async function createVault(password: string): Promise<void> {
  await be().createVault(password);
  await preload();
}

export async function unlock(password: string): Promise<void> {
  await be().unlock(password);
  await preload();
}

/** Quick unlock (Phase 3 in the core): only the preview backend implements these today. */
export async function unlockWithPin(pin: string): Promise<void> {
  const b = be();
  if (!b.quickUnlockPin) throw new Error("PIN unlock is not available yet.");
  await b.quickUnlockPin(pin);
  await preload();
}

export async function unlockWithBiometric(): Promise<void> {
  const b = be();
  if (!b.quickUnlockBiometric) throw new Error("Biometric unlock is not available yet.");
  await b.quickUnlockBiometric();
  await preload();
}

export function quickUnlockAvailable(): { pin: boolean; biometric: boolean } {
  const { capabilities } = app.get();
  const b = backend;
  return {
    pin: Boolean(capabilities.quickUnlock && b?.quickUnlockPin),
    biometric: Boolean(capabilities.biometric && b?.quickUnlockBiometric),
  };
}

export async function lockVault(): Promise<void> {
  await be().lock();
  app.set((s) => ({ ...s, phase: "locked", items: [], trash: [], selectedId: null, query: "", filter: "all", toasts: [], justUnlocked: false }));
}

export const consumeUnlockMoment = () => patch({ justUnlocked: false });
export const select = (id: string | null) => patch({ selectedId: id });
export const setFilter = (filter: Filter) => patch({ filter, selectedId: null });
export const setQuery = (query: string) => patch({ query });

export function setClipboardClear(sec: ClipboardClear) {
  patch({ clipboardClear: sec });
  try {
    localStorage.setItem(PREF_KEY, JSON.stringify({ clipboardClear: sec }));
  } catch {
    /* preference simply will not persist */
  }
}

// ---- toasts ----
let toastSeq = 0;
export function pushToast(t: Omit<Toast, "id" | "ttlMs"> & { ttlMs?: number }): number {
  const id = ++toastSeq;
  const toast: Toast = { ttlMs: t.countdownMs ?? 4200, ...t, id };
  app.set((s) => ({ ...s, toasts: [...s.toasts.slice(-2), toast] }));
  return id;
}
export const dismissToast = (id: number) => app.set((s) => ({ ...s, toasts: s.toasts.filter((t) => t.id !== id) }));

// ---- derived views ----
const FILTER_MATCH: Record<Exclude<Filter, "trash" | "settings">, (i: ItemSummary) => boolean> = {
  all: () => true,
  logins: (i) => i.type === "login",
  codes: (i) => i.hasTotp,
  notes: (i) => i.type === "secureNote",
  media: (i) => i.type === "media",
  favorites: (i) => i.favorite,
};

export function visibleItems(items: ItemSummary[], filter: Filter, query: string): ItemSummary[] {
  if (filter === "trash" || filter === "settings") return [];
  const q = query.trim().toLowerCase();
  const match = FILTER_MATCH[filter];
  return items
    .filter(match)
    .filter((i) => !q || i.title.toLowerCase().includes(q) || (i.subtitle ?? "").toLowerCase().includes(q) || i.tags.some((t) => t.toLowerCase().includes(q)) || (i.folder ?? "").toLowerCase().includes(q))
    .sort((a, b) => b.updatedAt - a.updatedAt);
}

export function counts(items: ItemSummary[], trashCount: number): Record<Filter, number> {
  return {
    all: items.length,
    logins: items.filter(FILTER_MATCH.logins).length,
    codes: items.filter(FILTER_MATCH.codes).length,
    notes: items.filter(FILTER_MATCH.notes).length,
    media: items.filter(FILTER_MATCH.media).length,
    favorites: items.filter(FILTER_MATCH.favorites).length,
    trash: trashCount,
    settings: 0,
  };
}
