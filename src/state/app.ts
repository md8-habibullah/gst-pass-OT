import { getBackend, type Backend, type BackupTargetInput, type GateOutcome, type GateStatus, type ItemSummary, type ProfileInfo, type SuiteInfo, type TrashEntry, type VaultStatus } from "../backend";
import { setBrand } from "./brand";
import { createStore } from "./store";

/**
 * gate     the optional Stage 1 app lock is closed; nothing about the accounts is shown yet
 * welcome  no account exists on this device yet
 * picker   several accounts exist and none is chosen
 * locked   one account is chosen; its master password is needed (always, never skipped by Stage 1)
 */
export type Phase = "boot" | "gate" | "welcome" | "picker" | "locked" | "ready" | "failed";

export type Filter =
  | "all" | "logins" | "cards" | "identities" | "ssh" | "notes" | "media" | "files" | "favorites" | "codes" | "archive" | "trash"
  | "generator" | "import" | "export" | "backup" | "settings";
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
  suite: SuiteInfo | null;
  /** Stage 1 app lock. Holds no secret: the equation, the attempts left and which mode to draw. */
  gate: GateStatus | null;
  /** Every account on this device, most recently used first. */
  profiles: ProfileInfo[];
  /** The account whose password is being asked for, or whose vault is open. */
  profile: ProfileInfo | null;
  items: ItemSummary[];
  trash: TrashEntry[];
  filter: Filter;
  query: string;
  selectedId: string | null;
  /** True for the single render after unlocking so the shell can play the entrance sequence once. */
  justUnlocked: boolean;
  toasts: Toast[];
  clipboardClear: ClipboardClear;
  /** Last value of the core's pull counter. When it moves, a pull from the backup changed local data and the lists reload. */
  syncRevision: number;
}

const PREF_KEY = "hexis.prefs.v1";

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
  suite: null,
  gate: null,
  profiles: [],
  profile: null,
  items: [],
  trash: [],
  filter: "all",
  query: "",
  selectedId: null,
  justUnlocked: false,
  toasts: [],
  syncRevision: 0,
  ...loadPrefs(),
});

const patch = (p: Partial<AppState>) => app.set((s) => ({ ...s, ...p }));

let backend: Backend | null = null;
export function be(): Backend {
  if (!backend) throw new Error("backend not ready");
  return backend;
}

const EMPTY_VAULT: Partial<AppState> = { items: [], trash: [], selectedId: null, query: "", filter: "all", toasts: [], justUnlocked: false };

/** Re-reads the account list and decides which lock-screen view to start from. */
async function settle(status: VaultStatus): Promise<void> {
  const list = await be().listProfiles();
  const profile = status.profile ?? list.profiles.find((p) => p.id === list.selectedId) ?? null;
  const phase: Phase = status.unlocked && profile ? "ready" : list.profiles.length === 0 ? "welcome" : profile ? "locked" : "picker";
  patch({ profiles: list.profiles, profile, phase });
}

/** After Stage 1 is open (or was never on): decide between first run, the account list and a password prompt. */
async function enterAccounts(): Promise<void> {
  await settle(await be().status());
  if (app.get().phase === "ready") await refresh();
  // One account and nothing selected yet: go straight to its password prompt.
  const s = app.get();
  if (s.phase === "picker" && s.profiles.length === 1) await selectProfile(s.profiles[0]!.id);
}

export async function boot(): Promise<void> {
  try {
    backend = await getBackend();
    const [suite, gate, disguise] = await Promise.all([backend.suiteInfo(), backend.gateStatus(), backend.disguiseGet()]);
    setBrand(disguise);
    patch({ suite, gate });
    if (gate.enabled && !gate.open) {
      patch({ phase: "gate" });
      return;
    }
    await enterAccounts();
  } catch (e) {
    patch({ phase: "failed", bootError: e instanceof Error ? e.message : String(e) });
  }
}

export async function refreshProfiles(): Promise<void> {
  const list = await be().listProfiles();
  const current = app.get().profile;
  patch({ profiles: list.profiles, profile: current ? (list.profiles.find((p) => p.id === current.id) ?? null) : null });
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
  const status = await be().status();
  patch({ profile: status.profile ?? app.get().profile, syncRevision: status.syncRevision ?? 0 });
  const damaged = status.damagedItems ?? 0;
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

// ---- accounts ----

export async function selectProfile(id: string): Promise<void> {
  await be().selectProfile(id);
  const profile = app.get().profiles.find((p) => p.id === id) ?? null;
  patch({ ...EMPTY_VAULT, profile, phase: "locked" });
}

/** Back to the account chooser (locks whatever is open). */
export async function showPicker(): Promise<void> {
  if (backend) await backend.lock();
  await refreshProfiles();
  const s = app.get();
  patch({ ...EMPTY_VAULT, profile: null, phase: s.profiles.length === 0 ? "welcome" : "picker" });
}

/** Creates the account and its vault; leaves it unlocked. The lock screen then continues (backup step, unlock moment). */
export async function createProfile(displayName: string, password: string): Promise<void> {
  const info = await be().createProfile(displayName, password);
  await refreshProfiles();
  patch({ profile: info });
  await preload();
}

/** Opens the native file dialog in the core and adds the chosen vault file as a new account. False if cancelled. */
export async function importProfileFile(displayName: string): Promise<boolean> {
  const info = await be().importProfileFile(displayName);
  if (!info) return false;
  await refreshProfiles();
  patch({ ...EMPTY_VAULT, profile: info, phase: "locked" });
  return true;
}

export async function restoreFromBackup(displayName: string, input: BackupTargetInput): Promise<void> {
  const info = await be().restoreProfileFromBackup(displayName, input);
  await refreshProfiles();
  patch({ ...EMPTY_VAULT, profile: info, phase: "locked" });
}

export async function renameProfile(id: string, displayName: string): Promise<void> {
  const info = await be().renameProfile(id, displayName);
  await refreshProfiles();
  if (app.get().profile?.id === id) patch({ profile: info });
}

export async function removeProfile(id: string, password: string, confirmation: string): Promise<void> {
  await be().removeProfile(id, password, confirmation);
  await refreshProfiles();
  const s = app.get();
  if (s.profile?.id === id || s.profiles.length === 0) patch({ ...EMPTY_VAULT, profile: null, phase: s.profiles.length === 0 ? "welcome" : "picker" });
}

export async function unlock(password: string): Promise<void> {
  await be().unlock(password);
  await preload();
}

// ---- Stage 1 gate ----

const merge = (g: GateStatus | null, p: Partial<GateStatus>): GateStatus | null => (g ? { ...g, ...p } : g);

/** A wipe (panic, the 15th failure, or the manual reset) ends here: the app looks freshly installed and says nothing. */
async function afterWipe(): Promise<void> {
  const b = be();
  const [gate, disguise] = await Promise.all([b.gateStatus(), b.disguiseGet()]);
  setBrand(disguise);
  patch({ ...EMPTY_VAULT, profiles: [], profile: null, gate, phase: "welcome" });
}

async function applyGateOutcome(out: GateOutcome): Promise<void> {
  const b = be();
  if (out.kind === "opened") {
    patch({ gate: await b.gateStatus() });
    await enterAccounts();
  } else if (out.kind === "wiped") {
    await afterWipe();
  } else if (out.kind === "math") {
    patch({ gate: merge(app.get().gate, { equation: out.equation, streak: out.streak }) });
  } else {
    patch({ gate: merge(app.get().gate, { equation: out.equation, attemptsLeft: out.attemptsLeft }) });
  }
}

export async function submitGate(input: string): Promise<GateOutcome> {
  const out = await be().gateSubmit(input);
  await applyGateOutcome(out);
  return out;
}
export async function gateWithBiometric(): Promise<GateOutcome> {
  const out = await be().gateBiometric();
  await applyGateOutcome(out);
  return out;
}
export async function gatePanic(): Promise<void> {
  await applyGateOutcome(await be().gatePanic());
}
export async function resetAppData(): Promise<void> {
  await be().appReset();
  await afterWipe();
}
export async function refreshGate(): Promise<GateStatus> {
  const gate = await be().gateStatus();
  patch({ gate });
  return gate;
}
/** "Lock app": back to Stage 1 when it is on, otherwise to the vault's password prompt. */
export async function lockApp(): Promise<void> {
  await be().gateClose();
  const gate = await be().gateStatus();
  patch({ ...EMPTY_VAULT, gate, phase: gate.enabled && !gate.open ? "gate" : "locked" });
}
let remoteDamagedShown = 0;
/**
 * The core locked the vault by itself (inactivity)? Bring the screen in line with it. Also picks up what a pull from
 * the backup changed. Returns true while a pull is still running, so the caller can look again soon.
 */
export async function syncAfterCoreLock(): Promise<boolean> {
  const b = be();
  const before = app.get().phase;
  if (before === "picker" || before === "locked" || before === "welcome") {
    // No vault is open here. The only thing the core can have changed is Stage 1: its inactivity timer closes the gate.
    if (app.get().gate?.enabled) {
      const gate = await b.gateStatus();
      if (gate.enabled && !gate.open && app.get().phase === before) patch({ ...EMPTY_VAULT, gate, phase: "gate" });
    }
    return false;
  }
  const [status, gate] = await Promise.all([b.status(), b.gateStatus()]);
  if (app.get().phase !== "ready") return false;
  if (!status.unlocked) {
    patch({ ...EMPTY_VAULT, gate, phase: gate.enabled && !gate.open ? "gate" : "locked" });
    remoteDamagedShown = 0;
    return false;
  }
  // A pull from the backup changed local data (another device saved something): show it without a manual refresh.
  const rev = status.syncRevision ?? 0;
  if (rev !== app.get().syncRevision) {
    patch({ syncRevision: rev });
    await refresh();
  }
  // Objects on the backup that failed their integrity check are never applied; say so once per increase.
  const bad = status.remoteDamaged ?? 0;
  if (bad > remoteDamagedShown) {
    pushToast({
      kind: "error",
      ttlMs: 15_000,
      text: `${bad === 1 ? "One object" : `${bad} objects`} on your backup failed its integrity check and ${bad === 1 ? "was" : "were"} ignored. Your local data is untouched. The backup may be damaged or tampered with.`,
    });
  }
  remoteDamagedShown = bad;
  return status.reconciling === true;
}

/** Locks the open vault and returns to its password prompt. */
export async function lockVault(): Promise<void> {
  await be().lock();
  patch({ ...EMPTY_VAULT, phase: "locked" });
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
type ListFilter = Exclude<Filter, "trash" | "files" | "generator" | "import" | "export" | "backup" | "settings">;

/** Which filters show a list of items, and the rule each one applies. Archived items only appear in "archive". */
const FILTER_MATCH: Record<ListFilter, (i: ItemSummary) => boolean> = {
  all: (i) => !i.archived,
  logins: (i) => !i.archived && i.type === "login",
  cards: (i) => !i.archived && i.type === "card",
  identities: (i) => !i.archived && i.type === "identity",
  ssh: (i) => !i.archived && i.type === "sshKey",
  notes: (i) => !i.archived && i.type === "secureNote",
  media: (i) => !i.archived && i.type === "media",
  favorites: (i) => !i.archived && i.favorite,
  codes: (i) => !i.archived && i.hasTotp,
  archive: (i) => i.archived,
};

const isListFilter = (f: Filter): f is ListFilter => f in FILTER_MATCH;

export function visibleItems(items: ItemSummary[], filter: Filter, query: string): ItemSummary[] {
  if (!isListFilter(filter)) return [];
  const q = query.trim().toLowerCase();
  const match = FILTER_MATCH[filter];
  return items
    .filter(match)
    .filter((i) => !q || i.title.toLowerCase().includes(q) || (i.subtitle ?? "").toLowerCase().includes(q) || i.tags.some((t) => t.toLowerCase().includes(q)) || (i.folder ?? "").toLowerCase().includes(q))
    .sort((a, b) => b.updatedAt - a.updatedAt);
}

export function counts(items: ItemSummary[], trashCount: number): Record<Filter, number> {
  const n = (f: ListFilter) => items.filter(FILTER_MATCH[f]).length;
  return {
    all: n("all"), logins: n("logins"), cards: n("cards"), identities: n("identities"), ssh: n("ssh"), notes: n("notes"),
    media: n("media"), favorites: n("favorites"), codes: n("codes"), archive: n("archive"), trash: trashCount,
    files: 0, generator: 0, import: 0, export: 0, backup: 0, settings: 0,
  };
}
