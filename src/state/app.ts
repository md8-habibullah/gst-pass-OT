import { getBackend, type Backend, type BackupTargetInput, type ClipboardClear, type DeviceInfo, type GateOutcome, type GateStatus, type ItemSummary, type ListSort, type ProfileInfo, type SearchHit, type SuiteInfo, type Theme, type TrashEntry, type VaultStatus } from "../backend";
import { allFolders, childFolders, isInside, itemsIn, parentFolder, type FolderNode } from "../lib/folders";
import { setDisplayZone } from "../lib/format";
import { activeFilterCount, applyFilters, compareItems, NO_FILTERS, summaryMatches, type ListFilters } from "../lib/listView";
import { canGenerate, DEFAULT_GEN, savedOrDefault, type GenOptions } from "../lib/password";
import { applyTheme } from "../lib/theme";
import { setBrand } from "./brand";
import { createStore } from "./store";
import { setLockIntent, type LockFlow } from "./ui";

/**
 * gate       the optional Stage 1 app lock is closed; nothing about the accounts is shown yet
 * setup      no time zone has been chosen yet: the first page, mandatory
 * preparing  the first vault is being made (a moment of key derivation)
 * welcome    no account exists and the built-in vault could not be made: the plain create / import / restore choices
 * picker     several accounts exist and none is chosen
 * locked     one account is chosen; its master password is needed (the demo vault opens itself)
 * removed     "Remove Hexis from this device" is done: nothing is left to show but what to do next
 */
export type Phase = "boot" | "gate" | "setup" | "preparing" | "welcome" | "picker" | "locked" | "ready" | "failed" | "removed";

export type Filter =
  | "all" | "logins" | "cards" | "identities" | "ssh" | "notes" | "media" | "files" | "favorites" | "codes" | "archive" | "trash"
  | "generator" | "import" | "export" | "backup" | "settings";
export type { ClipboardClear };

export interface ToastAction {
  label: string;
  run: () => void;
}
export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  text: string;
  action?: ToastAction;
  /** How long it stays on screen. A notice never waits longer than {@link TOAST_MS} unless the caller says so (warnings do). */
  ttlMs: number;
}

/** A notice is on screen this long, then goes by itself; a swipe to either side (or the close button) removes it sooner. */
export const TOAST_MS = 5000;

/** What the deeper search found for one query (the core reads notes and every other field; the screen only gets ids and field names). */
export interface DeepSearch {
  query: string;
  oldVersions: boolean;
  hits: SearchHit[];
}

export interface AppState {
  phase: Phase;
  bootError: string | null;
  suite: SuiteInfo | null;
  /** Stage 1 app lock. Holds no secret: the equation, the attempts left and which mode to draw. */
  gate: GateStatus | null;
  /** Install-wide settings (time zone, first-run flag, online time check). `null` until the core answered. */
  device: DeviceInfo | null;
  /** Every account on this device, most recently used first. */
  profiles: ProfileInfo[];
  /** The account whose password is being asked for, or whose vault is open. */
  profile: ProfileInfo | null;
  items: ItemSummary[];
  trash: TrashEntry[];
  /** Folders made on purpose (so an empty one can exist). Items bring the rest; see `allFolders`. */
  folders: string[];
  /** The folder the list is showing, or `null` for the top level. */
  folder: string | null;
  filter: Filter;
  query: string;
  /** The extra list filters (types, tag, number of versions, ...). */
  filters: ListFilters;
  /** Search also looks in the versions an item had before. */
  includeOld: boolean;
  deep: DeepSearch | null;
  selectedId: string | null;
  /** True for the single render after unlocking so the shell can play the entrance sequence once. */
  justUnlocked: boolean;
  /** The vault just opened and "open in search" is on: the list puts the cursor in the search box once, then clears this. */
  searchOnOpen: boolean;
  toasts: Toast[];
  /** How long a copied secret stays on the clipboard. Kept by the core with the other device settings, never in the web view. */
  clipboardClear: ClipboardClear;
  /**
   * The password generator's settings, the same in every place that shows the generator. Remembered in the vault (and
   * synced with it); the defaults until the open vault answered.
   */
  generator: GenOptions;
  /** Last value of the core's pull counter. When it moves, a pull from the backup changed local data and the lists reload. */
  syncRevision: number;
  /** The open vault was opened with the account PIN: secrets and settings ask for the master password (`elevate` lifts it). */
  limited: boolean;
}

export const app = createStore<AppState>({
  phase: "boot",
  bootError: null,
  suite: null,
  gate: null,
  device: null,
  profiles: [],
  profile: null,
  items: [],
  trash: [],
  folders: [],
  folder: null,
  filter: "all",
  query: "",
  filters: NO_FILTERS,
  includeOld: false,
  deep: null,
  selectedId: null,
  justUnlocked: false,
  searchOnOpen: false,
  toasts: [],
  syncRevision: 0,
  generator: { ...DEFAULT_GEN },
  clipboardClear: 30,
  limited: false,
});

/**
 * Whether to call a vault "demo" on screen. Only until the first-run welcome was read: after that (and after any erase,
 * which keeps that flag) the built-in vault is shown like any other, so a reset does not announce itself.
 */
export const isLabelledDemo = (device: DeviceInfo | null, p: ProfileInfo | null): boolean => p?.demo === true && device?.setupDone === false;

const patch = (p: Partial<AppState>) => app.set((s) => ({ ...s, ...p }));

/** The device settings were read or changed: the time zone, the colours and the clipboard time all follow them. */
function adoptDevice(device: DeviceInfo): void {
  setDisplayZone(device.timeZone);
  applyTheme(device.theme);
  patch({ device, clipboardClear: device.clipboardClear });
}

let backend: Backend | null = null;
export function be(): Backend {
  if (!backend) throw new Error("backend not ready");
  return backend;
}

const EMPTY_VAULT: Partial<AppState> = {
  items: [], trash: [], folders: [], folder: null, selectedId: null, query: "", filters: NO_FILTERS, includeOld: false, deep: null, filter: "all",
  toasts: [], justUnlocked: false, searchOnOpen: false, generator: { ...DEFAULT_GEN }, limited: false,
};

/** Re-reads the account list and decides which lock-screen view to start from. No vault at all: make the built-in one. */
async function settle(status: VaultStatus): Promise<void> {
  const list = await be().listProfiles();
  if (list.profiles.length === 0) {
    patch({ profiles: [], profile: null });
    await spawnDemo();
    return;
  }
  const profile = status.profile ?? list.profiles.find((p) => p.id === list.selectedId) ?? null;
  const phase: Phase = status.unlocked && profile ? "ready" : profile ? "locked" : "picker";
  patch({ profiles: list.profiles, profile, phase });
}

/** The first page: no time zone is known yet, so nothing else is shown until one is chosen. */
const needsZone = () => !app.get().device?.timeZone;

/** After Stage 1 is open (or was never on): the time zone page on a first run, otherwise the account list or a password prompt. */
async function enterAccounts(): Promise<void> {
  if (needsZone()) {
    patch({ phase: "setup" });
    return;
  }
  await continueToAccounts();
}

async function continueToAccounts(): Promise<void> {
  await settle(await be().status());
  if (app.get().phase === "ready") await refresh();
  // One account and nothing selected yet: go straight to its password prompt.
  const s = app.get();
  if (s.phase === "picker" && s.profiles.length === 1) await selectProfile(s.profiles[0]!.id);
}

export async function boot(): Promise<void> {
  try {
    backend = await getBackend();
    const [suite, gate, disguise, device] = await Promise.all([backend.suiteInfo(), backend.gateStatus(), backend.disguiseGet(), backend.deviceGet()]);
    setBrand(disguise);
    patch({ suite, gate });
    adoptDevice(device);
    if (gate.enabled && !gate.open) {
      patch({ phase: "gate" });
      return;
    }
    await enterAccounts();
  } catch (e) {
    patch({ phase: "failed", bootError: e instanceof Error ? e.message : String(e) });
  }
}

/** Reads the device settings again (zone, setup, time check), for a change made outside the screens. */
export async function refreshDevice(): Promise<void> {
  adoptDevice(await be().deviceGet());
}

export async function refreshProfiles(): Promise<void> {
  const list = await be().listProfiles();
  const current = app.get().profile;
  patch({ profiles: list.profiles, profile: current ? (list.profiles.find((p) => p.id === current.id) ?? null) : null });
}

export async function refresh(): Promise<void> {
  const b = be();
  // The remembered generator settings ride along; they must never get in the way of showing the vault.
  const [items, trash, prefs] = await Promise.all([b.listItems(), b.listTrash(), b.prefsGet().catch(() => null)]);
  app.set((s) => {
    const folders = prefs?.folders ?? s.folders;
    return {
    ...s,
    items,
    trash,
    folders,
    // The folder being shown was removed or renamed somewhere else (another device, a pull): back to the top level.
    folder: s.folder && allFolders(folders, items).includes(s.folder) ? s.folder : null,
    // A change made a moment ago and not yet saved is newer than what the vault answers.
    generator: prefs && !genPending && genSaving === 0 ? savedOrDefault(prefs.generator) : s.generator,
    selectedId: s.selectedId && (items.some((i) => i.id === s.selectedId) || trash.some((t) => t.id === s.selectedId)) ? s.selectedId : null,
    };
  });
}

// ---- the generator's remembered settings ----

/** The latest change that has not reached the vault yet, and how many saves are on their way. */
let genPending: GenOptions | null = null;
let genSaving = 0;
let genTimer: ReturnType<typeof setTimeout> | undefined;
const GEN_SAVE_DELAY_MS = 600;

/**
 * The generator's settings changed. Every place that shows the generator has them at once; the vault gets them a moment
 * later, so dragging the slider is one save and not fifty. Settings the generator cannot use (no character type) are not kept.
 */
export function rememberGenerator(next: GenOptions): void {
  if (!canGenerate(next)) return;
  genPending = next;
  patch({ generator: next });
  clearTimeout(genTimer);
  genTimer = setTimeout(() => void flushGenerator(), GEN_SAVE_DELAY_MS);
}

/** Saves a change that is still waiting (when the generator closes, or before the vault locks). Never throws: a missed save only means the next change tries again. */
export async function flushGenerator(): Promise<void> {
  clearTimeout(genTimer);
  const g = genPending;
  if (!g) return;
  genPending = null;
  genSaving += 1;
  try {
    await be().prefsSetGenerator(g);
  } catch {
    /* locked, or the core refused: the settings stay on screen and the next change saves again */
  } finally {
    genSaving -= 1;
  }
}

/** Loads the vault contents after a successful unlock but stays on the lock screen so it can play its exit. */
async function preload(): Promise<void> {
  await refresh();
  const status = await be().status();
  // keep the demo flag even from an older core that does not put it on the status
  const known = app.get().profiles.find((p) => p.id === status.profile?.id);
  patch({ profile: status.profile ? { ...status.profile, ...(known?.demo ? { demo: true } : {}) } : app.get().profile, syncRevision: status.syncRevision ?? 0, limited: status.limited === true });
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
  patch({ phase: "ready", justUnlocked: true, searchOnOpen: app.get().device?.openInSearch === true, filter: "all", query: "", folder: null, filters: NO_FILTERS, includeOld: false, deep: null, selectedId: null });
}

// ---- accounts ----

export async function selectProfile(id: string): Promise<void> {
  await be().selectProfile(id);
  // The account that was open is kept behind its PIN (or wiped) by the core, and the chosen one may be waiting behind its own.
  await refreshProfiles();
  const profile = app.get().profiles.find((p) => p.id === id) ?? null;
  patch({ ...EMPTY_VAULT, profile, phase: "locked" });
}

/** Switch to another account. The one that is open is kept behind its PIN, or locked when it has none. */
export async function switchAccount(id: string): Promise<void> {
  await flushGenerator();
  await selectProfile(id);
}

/** Log out: every vault is wiped, those waiting behind a PIN too, and the account list shows. Nothing is deleted. */
export async function logOut(): Promise<void> {
  await flushGenerator();
  await be().lockAll();
  await refreshProfiles();
  const s = app.get();
  patch({ ...EMPTY_VAULT, profile: null, phase: s.profiles.length === 0 ? "welcome" : "picker" });
  if (s.profiles.length === 1) await selectProfile(s.profiles[0]!.id);
}

/** The account's PIN on the lock screen: it shows the vault that is still open behind it, limited. */
export async function unlockWithPin(pin: string): Promise<void> {
  await be().unlockPin(pin);
  await preload();
}

/** The master password, typed in a vault opened with the PIN: secrets and settings work until it locks again. */
export async function elevate(password: string): Promise<void> {
  await be().elevate(password);
  patch({ limited: false });
}

/** Give the open account a PIN, or change it (master password needed). */
export async function setAccountPin(password: string, pin: string): Promise<void> {
  const info = await be().setAccountPin(password, pin);
  await refreshProfiles();
  patch({ profile: { ...(app.get().profile ?? info), ...info } });
}

/** Take the PIN away (master password needed). */
export async function removeAccountPin(password: string): Promise<void> {
  await be().removeAccountPin(password);
  await refreshProfiles();
}

/** Back to the account chooser (locks whatever is open). */
export async function showPicker(): Promise<void> {
  if (backend) await backend.lock(); // kept behind its PIN when the account has one
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
  if (s.profiles.length === 0) {
    // No vault is left: the built-in one takes its place, as on a first start.
    patch({ ...EMPTY_VAULT, profile: null });
    await spawnDemo();
  } else if (s.profile?.id === id) {
    patch({ ...EMPTY_VAULT, profile: null, phase: "picker" });
  }
}

export async function unlock(password: string): Promise<void> {
  await be().unlock(password);
  await preload();
}

// ---- device settings, first run and the built-in vault ----

/** Makes the built-in vault and opens it. A failure falls back to the plain create / import / restore screen. */
async function spawnDemo(): Promise<void> {
  patch({ ...EMPTY_VAULT, phase: "preparing", profile: null });
  try {
    const info = await be().demoSpawn();
    await refreshProfiles();
    patch({ profile: info });
    await preload();
    enterApp();
  } catch {
    // Nothing half-made stays behind (the core rolls back); the app must never be stuck on a splash.
    patch({ phase: "welcome", profile: null });
  }
}

/** The lock screen opens the built-in vault without a password: this is the core part of it. */
export async function openDemo(): Promise<void> {
  await be().demoOpen();
  await preload();
}

/** Saves the zone every time is shown in. Everything the app stores stays UTC. */
export async function chooseTimeZone(zone: string): Promise<void> {
  adoptDevice(await be().deviceSetTimeZone(zone));
}

/** The first page: remember the zone, then carry on to the accounts. An install that already had vaults is not a first run. */
export async function confirmTimeZone(zone: string): Promise<void> {
  await chooseTimeZone(zone);
  const list = await be().listProfiles();
  if (list.profiles.length > 0 && !app.get().device?.setupDone) adoptDevice(await be().deviceFinishSetup());
  await continueToAccounts();
}

/** The one-time welcome over the demo was read. `go` carries on to a way of getting a real vault. */
export async function finishFirstRun(go: LockFlow | null = null): Promise<void> {
  adoptDevice(await be().deviceFinishSetup());
  if (go) {
    setLockIntent(go);
    await showPicker();
  }
}

export async function setTimeCheck(on: boolean): Promise<void> {
  adoptDevice(await be().deviceSetTimeCheck(on));
}

// ---- Stage 1 gate ----

const merge = (g: GateStatus | null, p: Partial<GateStatus>): GateStatus | null => (g ? { ...g, ...p } : g);

/** A wipe (panic, the 16th failure, or the manual reset) ends here: the app looks freshly installed and says nothing. */
async function afterWipe(): Promise<void> {
  const b = be();
  const [gate, disguise, device] = await Promise.all([b.gateStatus(), b.disguiseGet(), b.deviceGet()]);
  setBrand(disguise);
  patch({ ...EMPTY_VAULT, profiles: [], profile: null, gate });
  adoptDevice(device);
  // The time zone survives the erase, so the new start has no setup page: it opens straight into the built-in vault.
  if (needsZone()) patch({ phase: "setup" });
  else await spawnDemo();
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
  } else if (out.kind === "locked") {
    // A wait is running: nothing was counted. Read the whole status so the countdown and the attempts are exact.
    patch({ gate: merge(await b.gateStatus(), { lockedForMs: out.retryAfterMs, clockPending: out.clockPending }) });
  } else if (out.lockedForMs !== null) {
    // This failure started a wait: read the whole status so the end time (checked UTC) and the attempts are exact.
    patch({ gate: await b.gateStatus() });
  } else {
    patch({
      gate: merge(app.get().gate, {
        equation: out.equation,
        attemptsLeft: out.attemptsLeft,
        lockedForMs: out.lockedForMs,
        finalAttempt: out.attemptsLeft <= 1,
      }),
    });
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
/** A new long sum for the last "erase everything" step. */
export const resetChallenge = (): Promise<string> => be().appResetChallenge();
/** Erases this device when `answer` is right; `wrong_answer` (and a used-up sum) otherwise. */
export async function resetAppData(answer: string): Promise<void> {
  await be().appReset(answer);
  await afterWipe();
}
/**
 * "Remove Hexis from this device": the same confirmations as the erase, but nothing is made again afterwards. The core
 * removes the vaults, the settings and the web view's leftovers and then closes the app (a computer) or waits for the
 * system's uninstall (a phone). The colours stay as chosen until then.
 */
export async function prepareUninstall(answer: string): Promise<void> {
  await be().appPrepareUninstall(answer);
  // Nothing is read from the core after this: it has nothing left to answer with. The screen only says what to do next,
  // and the activity watcher stops with the phase, so no call goes to the core again.
  patch({ ...EMPTY_VAULT, profiles: [], profile: null, gate: null, phase: "removed" });
}
export async function refreshGate(): Promise<GateStatus> {
  const gate = await be().gateStatus();
  patch({ gate });
  return gate;
}
/** "Lock app": back to Stage 1 when it is on, otherwise to the vault's password prompt. */
export async function lockApp(): Promise<void> {
  await be().gateClose();
  await refreshProfiles().catch(() => undefined); // closing the app lock wipes every vault, the ones behind a PIN too
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
    // locked by the core's timer: nothing waits behind a PIN any more, and the lock screen must not offer it
    await refreshProfiles().catch(() => undefined);
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
  await flushGenerator();
  await be().lock();
  // an account with a PIN now waits behind it: the lock screen asks for the PIN first
  await refreshProfiles();
  patch({ ...EMPTY_VAULT, phase: "locked" });
}

export const consumeUnlockMoment = () => patch({ justUnlocked: false });
export const consumeSearchOnOpen = () => patch({ searchOnOpen: false });
export const select = (id: string | null) => patch({ selectedId: id });
export const setFilter = (filter: Filter) => patch({ filter, folder: null, selectedId: null });
export const setQuery = (query: string) => patch({ query });
export const setFilters = (filters: ListFilters) => patch({ filters });
export const setIncludeOld = (includeOld: boolean) => patch({ includeOld });
export const setDeep = (deep: DeepSearch | null) => patch({ deep });
/** Opens a folder in the list (`null`: the top level). */
export const openFolder = (folder: string | null) => patch({ folder, selectedId: null });

// ---- folders ----
// The core keeps the folder list with the vault's other synced settings; each call answers with the list as it is now.

/** Makes a folder (and the ones above it). Answers with its path. A refusal (too deep, too long) is thrown with its words. */
export async function createFolder(path: string): Promise<string> {
  const change = await be().createFolder(path);
  patch({ folders: change.prefs.folders });
  return change.path ?? path;
}

/** Renames a folder, or moves it under another: everything inside follows. */
export async function renameFolder(from: string, to: string): Promise<number> {
  const change = await be().renameFolder(from, to);
  const here = app.get().folder;
  patch({ folders: change.prefs.folders, folder: here && isInside(here, from) ? to + here.slice(from.length) : here });
  await refresh();
  return change.moved;
}

/** Removes the folder only: what was inside moves up one level and nothing is deleted. */
export async function removeFolder(path: string): Promise<number> {
  const change = await be().removeFolder(path);
  const here = app.get().folder;
  patch({ folders: change.prefs.folders, folder: here && isInside(here, path) ? parentFolder(path) : here });
  await refresh();
  return change.moved;
}

/** Puts items in a folder (`null`: no folder). */
export async function moveItems(ids: string[], folder: string | null): Promise<number> {
  const n = await be().moveItems(ids, folder);
  await refresh();
  return n;
}

/**
 * How long a copied secret stays on the clipboard. The core keeps it (`device.json`), never the web view: the screen stores
 * nothing, so nothing of it is left behind when Hexis is removed.
 */
export async function setClipboardClear(sec: ClipboardClear): Promise<void> {
  const before = app.get().clipboardClear;
  patch({ clipboardClear: sec });
  try {
    adoptDevice(await be().deviceSetClipboardClear(sec));
  } catch (e) {
    patch({ clipboardClear: before });
    pushToast({ kind: "error", text: e instanceof Error ? e.message : "The clipboard time could not be saved." });
  }
}

/** "Open in search": after the vault opens, the cursor is in the search box (the keyboard comes up on a phone). The core keeps the choice. */
export async function setOpenInSearch(on: boolean): Promise<void> {
  try {
    adoptDevice(await be().deviceSetOpenInSearch(on));
  } catch (e) {
    pushToast({ kind: "error", text: e instanceof Error ? e.message : "The setting could not be saved." });
  }
}

/** Settings > Passkeys: may Hexis ask an app's website whether that app may use its passkeys? The core keeps the choice. */
export async function setPasskeyAppCheck(on: boolean): Promise<void> {
  const before = app.get().device;
  if (before) patch({ device: { ...before, passkeyAppCheck: on } });
  try {
    adoptDevice(await be().deviceSetPasskeyAppCheck(on));
  } catch (e) {
    if (before) patch({ device: before });
    pushToast({ kind: "error", text: e instanceof Error ? e.message : "The setting could not be saved." });
  }
}

/** The order the item list starts in. The core keeps the choice; the list follows it at once. */
export async function setListSort(order: ListSort): Promise<void> {
  const before = app.get().device;
  if (before) patch({ device: { ...before, listSort: order } });
  try {
    adoptDevice(await be().deviceSetListSort(order));
  } catch (e) {
    if (before) patch({ device: before });
    pushToast({ kind: "error", text: e instanceof Error ? e.message : "The order could not be saved." });
  }
}

/** Light, dark or the device's own setting. The colours change at once; the core keeps the choice in `device.json`. */
export async function setTheme(theme: Theme): Promise<void> {
  const before = app.get().device?.theme ?? "system";
  applyTheme(theme);
  try {
    adoptDevice(await be().deviceSetTheme(theme));
  } catch (e) {
    applyTheme(before);
    pushToast({ kind: "error", text: e instanceof Error ? e.message : "The colour choice could not be saved." });
  }
}

// ---- toasts ----
let toastSeq = 0;
export function pushToast(t: Omit<Toast, "id" | "ttlMs"> & { ttlMs?: number }): number {
  const id = ++toastSeq;
  const toast: Toast = { ...t, ttlMs: t.ttlMs ?? TOAST_MS, id };
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

/** What the list shows: folders first (when browsing), then items in the chosen order. */
export interface ListModel {
  /** Folders directly inside the open one. Empty while searching or filtering, which look through everything. */
  folders: FolderNode[];
  items: ItemSummary[];
  /** Searching or filtering: every folder is looked through and the list is flat. */
  flat: boolean;
  /** What the deeper search found, by item id (field names for the row's note). */
  hits: Map<string, SearchHit>;
}

/** The one place that decides what the list holds, so the screen and the keyboard shortcuts agree. */
export function visibleList(s: Pick<AppState, "items" | "filter" | "query" | "filters" | "folder" | "folders" | "deep" | "includeOld" | "device">): ListModel {
  const hits = new Map<string, SearchHit>();
  if (!isListFilter(s.filter)) return { folders: [], items: [], flat: false, hits };
  const order = compareItems(s.device?.listSort ?? "updated-desc");
  const base = applyFilters(s.items.filter(FILTER_MATCH[s.filter]), s.filters);
  const q = s.query.trim();
  const flat = q !== "" || activeFilterCount(s.filters) > 0;
  if (flat) {
    const deepFits = s.deep !== null && s.deep.query === q && s.deep.oldVersions === s.includeOld;
    if (q && deepFits) for (const h of s.deep!.hits) hits.set(h.id, h);
    const items = (q ? base.filter((i) => summaryMatches(i, q) || hits.has(i.id)) : base).sort(order);
    return { folders: [], items, flat, hits };
  }
  // Browsing. The folders made on purpose only show with "All items"; a type or a list of favorites shows the folders that hold matching items.
  const known = allFolders(s.filter === "all" ? s.folders : [], base);
  return { folders: childFolders(known, base, s.folder), items: itemsIn(base, s.folder).sort(order), flat, hits };
}

export function counts(items: ItemSummary[], trashCount: number): Record<Filter, number> {
  const n = (f: ListFilter) => items.filter(FILTER_MATCH[f]).length;
  return {
    all: n("all"), logins: n("logins"), cards: n("cards"), identities: n("identities"), ssh: n("ssh"), notes: n("notes"),
    media: n("media"), favorites: n("favorites"), codes: n("codes"), archive: n("archive"), trash: trashCount,
    files: 0, generator: 0, import: 0, export: 0, backup: 0, settings: 0,
  };
}
