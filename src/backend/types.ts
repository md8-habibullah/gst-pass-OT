// Mirrors ai-docs/IPC_CONTRACT.md. Change the contract first, then this file and the Rust command layer.

export type ItemType = "login" | "card" | "identity" | "sshKey" | "secureNote" | "totp" | "media";
/** paragraph = non-secret multi-line text (for example an SSH public key). */
export type FieldKind = "text" | "url" | "password" | "note" | "totp" | "paragraph" | "passkey";

export interface SuiteInfo {
  version: number;
  kem: string;
  kdf: string;
  aead: string;
  kdfDefaults: { mCostKiB: number; tCost: number; pCost: number };
  nonceBytes: number;
  keyBytes: number;
}

export interface ProfileInfo {
  id: string;
  displayName: string;
  createdAt: number;
  lastUsedAt: number | null;
  /** The built-in demo vault ("Hexis123"). Absent for every other account. */
  demo?: boolean;
  /** The account has a quick-unlock PIN. Absent when it has none. */
  hasPin?: boolean;
  /** Its vault is still open in memory behind the PIN: the PIN opens it right now. Absent otherwise. */
  resumable?: boolean;
}

/** A PIN is 4 to 64 characters, the same as the app lock's credential (the core checks it, too). */
export const PIN_MIN = 4;
export const PIN_MAX = 64;
/** Wrong PINs in a row before the open vault is closed and only the master password works. */
export const PIN_TRIES = 5;

/** Light or dark colours; `system` follows the device's own setting. */
export type Theme = "system" | "light" | "dark";
/** Seconds a copied secret stays on the clipboard. */
export type ClipboardClear = 15 | 30 | 60;

/** The orders the item list offers. The core keeps the chosen default (`device.json`); the screen does the sorting. */
export const LIST_SORTS = ["updated-desc", "updated-asc", "created-desc", "created-asc", "title-asc", "title-desc", "type", "versions-desc", "versions-asc"] as const;
export type ListSort = (typeof LIST_SORTS)[number];

/**
 * What the core keeps about this install in `device.json` (not secret; survives an erase). The screen itself keeps nothing:
 * no local storage, no cookies, so nothing of the app is left in the web view when it is removed.
 */
export interface DeviceInfo {
  /** IANA name chosen on the first screen; `null` until then. */
  timeZone: string | null;
  /** The first-run welcome was seen. */
  setupDone: boolean;
  /** Ask a few HTTPS servers for the real time. */
  timeCheck: boolean;
  theme: Theme;
  clipboardClear: ClipboardClear;
  /** After the vault opens, the cursor goes to the search box (and the keyboard comes up on a phone). */
  openInSearch: boolean;
  /** How the item list is ordered until another order is picked. */
  listSort: ListSort;
  /** Hexis may ask an app's website, once, whether that app may use its passkeys (Android). On unless switched off. */
  passkeyAppCheck: boolean;
}

/** The core's clock: UTC checked online, not the device date. */
export interface ClockStatus {
  /** The time was checked online since this device started. */
  verified: boolean;
  enabled: boolean;
  /** Checked UTC minus this device's own clock, in milliseconds (positive: the device is behind). */
  skewMs: number | null;
  checkedAgoMs: number | null;
  sources: number;
  /** Why the last check failed, in words. */
  problem: string | null;
  /** UTC milliseconds the core uses as "now". */
  nowMs: number;
}

export interface ProfileList {
  profiles: ProfileInfo[];
  selectedId: string | null;
  lastUsedId: string | null;
}

/** Describes the selected profile's vault. */
export interface VaultStatus {
  exists: boolean;
  unlocked: boolean;
  profile: ProfileInfo | null;
  /** Items that failed authentication at unlock and were skipped (the preview backend omits it). */
  damagedItems?: number;
  /** Counts up whenever a pull from the backup changed local data; the UI reloads its lists when it changes. */
  syncRevision?: number;
  /** A pull from the backup is running right now (started by unlocking or by Back up now). */
  reconciling?: boolean;
  /** Objects on the backup that failed authentication during a pull and were ignored (never applied). */
  remoteDamaged?: number;
  /** Locked, but still open in memory behind the account's PIN (`unlockPin` opens it). */
  resumable?: boolean;
  /** Opened with the PIN only: secrets and settings ask for the master password (`elevate` lifts it). */
  limited?: boolean;
}

export interface ItemSummary {
  id: string;
  type: ItemType;
  title: string;
  subtitle: string | null;
  favorite: boolean;
  tags: string[];
  folder: string | null;
  hasTotp: boolean;
  /** Keys of the item's one-time codes in order (`totp`, `totp#2` …): what `totpNow` takes. */
  codeKeys: string[];
  /** The login holds at least one passkey. */
  hasPasskey: boolean;
  createdAt: number;
  updatedAt: number;
  version: number;
  archived: boolean;
  /** Revealing, copying, editing or cloning this item asks for the master password every time. */
  reprompt: boolean;
}

/** What the screen may know about a passkey: everything but the private key, which never leaves the core. */
export interface PasskeyInfo {
  credentialId: string;
  rpId: string;
  rpName: string;
  userName: string;
  displayName: string;
  alg: number;
  createdAt: number;
}

export interface FieldView {
  key: string;
  label: string;
  kind: FieldKind;
  secret: boolean;
  value: string | null;
  hasValue: boolean;
  /** Only on a passkey field. */
  passkey?: PasskeyInfo;
}

/** A login the scanner thinks a scanned code (or a passkey) belongs to. */
export interface LoginSuggestion {
  id: string;
  title: string;
  subtitle: string | null;
  /** Why, strongest first: `account` (same email or user name), `site` (same website), `name` (same name). */
  reasons: Array<"account" | "site" | "name">;
  score: number;
  /** How many one-time codes and passkeys it holds now. */
  codes: number;
  passkeys: number;
  /** This exact code is on it already. */
  hasThisCode: boolean;
  hasSitePasskey: boolean;
  /** No slot is left for another code or passkey. */
  full: boolean;
  reprompt: boolean;
}

/** What a scanned or pasted text is. Nothing in it is secret: the login names are the ones the list already shows. */
export type ScanReply =
  | { kind: "totp"; issuer: string | null; account: string; digits: number; period: number; suggestions: LoginSuggestion[] }
  | { kind: "migration" }
  | { kind: "fido" }
  | { kind: "link"; host: string }
  | { kind: "text" }
  | { kind: "invalid"; message: string };

/** Where a scanned code goes: an existing login, or a new one. */
export type ScanChoice = { existing: string } | { new: { title: string | null } };

export interface CustomFieldView {
  name: string;
  hidden: boolean;
  value: string | null;
}

export interface AttachmentView {
  id: string;
  name: string;
  mime: string;
  size: number;
}

export interface ItemView {
  id: string;
  type: ItemType;
  title: string;
  favorite: boolean;
  reprompt: boolean;
  tags: string[];
  folder: string | null;
  fields: FieldView[];
  customFields: CustomFieldView[];
  attachments: AttachmentView[];
  createdAt: number;
  updatedAt: number;
  version: number;
  /** Another device saved a different version of this item; a copy of it is in the history. Cleared by reviewing it. */
  changedElsewhere?: boolean;
}

export interface CustomFieldDraft {
  name: string;
  value: string;
  hidden: boolean;
}

export interface ItemDraft {
  type: ItemType;
  title: string;
  favorite: boolean;
  /** Ask for the master password again before this item is shown, copied, edited or cloned. */
  reprompt: boolean;
  tags: string[];
  folder: string | null;
  fields: Record<string, string>;
  customFields: CustomFieldDraft[];
}

export interface TrashEntry {
  id: string;
  title: string;
  deletedAt: number;
}

export type VersionEvent = "created" | "edited" | "trashed" | "restored" | "restoredFromVersion" | "archived" | "unarchived";

export interface VersionEntry {
  version: number;
  timestamp: number;
  event: VersionEvent;
  changedFields: string[];
  restoredFrom: number | null;
  /** This entry is a copy that came from another device when the backups were compared. */
  otherDevice?: boolean;
}

export interface TotpNow {
  code: string;
  /** The code of the window before this one and of the window after it (withheld together with `code`). */
  previous: string;
  next: string;
  period: number;
  digits: number;
  secondsRemaining: number;
  validUntilMs: number;
}

export interface EditSession {
  token: string;
  draft: ItemDraft;
  version: number;
}

export type ErrorCode =
  | "locked"
  | "no_vault"
  | "vault_exists"
  | "wrong_password"
  | "not_found"
  | "in_trash"
  | "not_in_trash"
  | "version_conflict"
  | "invalid_input"
  | "confirmation_mismatch"
  | "token_invalid"
  | "storage"
  | "crypto"
  | "profile_name_taken"
  | "unsupported_file"
  | "network"
  | "remote"
  /** The item asks for the master password again and none was sent. */
  | "password_required"
  /** The PIN was wrong. The message says how many tries are left. */
  | "wrong_pin"
  /** Too many wrong PINs: the vault was closed and only the master password opens it. */
  | "pin_locked"
  /** The vault was opened with the PIN; this action needs the master password (`elevate` first). */
  | "master_required"
  /** The answer to the sum on the last "erase everything" step was wrong. */
  | "wrong_answer"
  | "unknown";

export class BackendError extends Error {
  readonly code: ErrorCode;
  constructor(code: ErrorCode, message: string) {
    super(message);
    this.name = "BackendError";
    this.code = code;
  }
}

// ---- backup targets (IPC_CONTRACT.md §6) ----
export interface BackupTargetInput {
  name: string;
  endpoint: string | null;
  region: string;
  bucket: string;
  prefix: string;
  accessKeyId: string;
  /** Empty on update keeps the stored secret. */
  secretAccessKey: string;
  pathStyle: boolean;
  enabled: boolean;
  master: boolean;
  shareInMaster: boolean;
}
export type HealthState = "untested" | "healthy" | "lagging" | "failing" | "disabled";
export type ConditionalWrites = "strong" | "weak" | "unknown";
export interface TargetHealth {
  state: HealthState;
  queued: number;
  lastSuccessAt: number | null;
  lastCheckedAt: number | null;
  lastError: string | null;
  conditionalWrites: ConditionalWrites;
}
export interface BackupTargetView {
  id: string;
  name: string;
  endpoint: string | null;
  region: string;
  bucket: string;
  prefix: string;
  accessKeyId: string;
  hasSecret: boolean;
  pathStyle: boolean;
  enabled: boolean;
  master: boolean;
  shareInMaster: boolean;
  health: TargetHealth;
}
export interface BackupTestResult {
  ok: boolean;
  message: string;
  conditionalWrites: ConditionalWrites;
}

// ---- app-level: Stage 1 gate, disguise, inactivity lock (FEATURE_QUEUE.md "IPC additions") ----
export type GateMode = "pin" | "password" | "grid";
export interface GateStatus {
  enabled: boolean;
  /** Stage 1 passed in this run: the vault list may be shown. */
  open: boolean;
  mode: GateMode;
  /** A fingerprint reader can be used on this device. */
  biometricAvailable: boolean;
  /** Switched on in Settings and available. */
  biometric: boolean;
  panicButton: boolean;
  /** The decoy equation to show while the gate is closed, for example "(48 + 27) - 19". */
  equation: string | null;
  /** `null` until a failed attempt; then 16 minus the attempts used. */
  attemptsLeft: number | null;
  /** Milliseconds until a passcode may be typed again, while a wait after wrong attempts is running. */
  lockedForMs: number | null;
  /** The same moment as checked UTC milliseconds ("try again at 14:35" without trusting the device date). `null` while unchecked. */
  lockedUntilMs: number | null;
  /** A wait is running but its end cannot be judged until the time was checked online since the device restarted. */
  clockPending: boolean;
  /** The next wrong passcode erases the device: the screen warns before it is sent. */
  finalAttempt: boolean;
  streak: number;
}
export type GateOutcome =
  | { kind: "math"; correct: boolean; equation: string; streak: number }
  | { kind: "opened" }
  /** `lockedForMs` is set when this failure started a wait. */
  | { kind: "failed"; attemptsLeft: number; equation: string; lockedForMs: number | null }
  /** A wait is running: the attempt was neither counted nor checked. */
  | { kind: "locked"; retryAfterMs: number; clockPending: boolean }
  | { kind: "wiped" };
export interface GateConfigInput {
  enabled: boolean;
  mode: GateMode;
  credential: string | null;
  reversePanic: boolean;
  /** `null` keeps the stored one, `""` removes it. */
  panicCode: string | null;
  biometric: boolean;
  panicButton: boolean;
  masterPassword: string;
}
/** The inactivity choices, in minutes. */
export const LOCK_AFTER_MINUTES = [1, 5, 15, 30, 60, 240] as const;
/** The "after leaving the app" choices, in seconds (0 = at once). */
export const LOCK_AFTER_LEAVE_SECONDS = [0, 15, 30, 60, 300, 900, 1800, 3600, 14400] as const;
/**
 * The password generator's last settings. Kept sealed in the vault, copied to every backup target and picked up by the
 * person's other devices, so the Generator page and the item editor open with what was chosen last.
 */
export interface GeneratorPrefs {
  /** 4 to 128 (the slider offers 8 to 64). */
  length: number;
  lower: boolean;
  upper: boolean;
  digits: boolean;
  symbols: boolean;
  avoidAmbiguous: boolean;
}
/** What the vault remembers for the person. `generator` is `null` until something was saved. */
export interface PrefsView {
  generator: GeneratorPrefs | null;
  /** Folders made on purpose (an empty folder can exist), as paths like "Work/Clients". Folders that items sit in are not listed here. */
  folders: string[];
}
/** One item the search found: which fields matched (names, never values), and the old version if only that matched. */
export interface SearchHit {
  id: string;
  matched: string[];
  /** `n`: the item as it is now does not match, but its version `n` did (only when old versions were searched). */
  oldVersion: number | null;
}
/** What the screen needs before an item is erased for good. */
export interface PurgePlan {
  deletedAt: number;
  /** Trashed less than six months ago: the master password and the sum are both needed. */
  recent: boolean;
  /** When the six months are over. */
  olderAt: number;
  /** The sum to solve (its answer stays in the core). `null` for an old item. */
  sum: string | null;
}
/** What a folder command did: the folder it made, how many items it changed, and the folders now. */
export interface FolderChange {
  path: string | null;
  moved: number;
  prefs: PrefsView;
}
/** `supported`: Android 14 or later with the system's Credential Manager. `enabled`: the provider is on (Android's list still asks the person to choose Hexis). */
export interface PasskeyProviderStatus {
  supported: boolean;
  enabled: boolean;
}

export interface AppSettings {
  lockAfterMinutes: (typeof LOCK_AFTER_MINUTES)[number];
  /** Phones and tablets: how long the app may stay open after the person leaves it. */
  lockAfterLeaveSeconds: (typeof LOCK_AFTER_LEAVE_SECONDS)[number];
}
export interface DisguiseInput {
  /** `null` is the original icon and name. */
  presetId: string | null;
  customName: string | null;
  lowercase: boolean;
}
export interface DisguiseView extends DisguiseInput {
  appliedName: string;
  /** What the launcher could and could not do on this platform. */
  note: string | null;
}

// ---- encrypted drive (FEATURE_QUEUE.md, Q3) ----
export interface FileEntry {
  id: string;
  name: string;
  size: number;
  mime: string | null;
  /** The item it is attached to, or `null` for the standalone Files section. */
  itemId: string | null;
  addedAt: number;
}

// ---- import and export (FEATURE_QUEUE.md, Q4). The webview never learns a file path: the core keeps handle -> path. ----
export type ImportFormat =
  | "keepass-kdbx" | "keepass-xml" | "bitwarden-json" | "bitwarden-csv" | "proton-json" | "proton-zip" | "proton-csv"
  | "1password-1pux" | "1password-csv" | "lastpass-csv" | "dashlane-csv" | "dashlane-json" | "nordpass-csv" | "enpass-csv" | "enpass-json"
  | "chrome-csv" | "firefox-csv" | "edge-csv" | "brave-csv" | "safari-csv" | "aegis-json" | "2fas-json" | "otpauth-list" | "google-migration" | "custom-csv";
export interface ImportPick { handle: string; fileName: string; size: number; format: ImportFormat | "unknown"; needsPassword: boolean; needsKeyfile: boolean }
export interface CsvMapping { title?: string; username?: string; password?: string; url?: string; totp?: string; notes?: string; folder?: string }
export interface CsvInspect { delimiter: string; headers: string[]; sample: string[][] }
export type ImportMode = "skip_duplicates" | "append_version" | "import_all" | "merge";
export interface ImportOptions { handle: string; format: ImportFormat; mode: ImportMode; password?: string; keyfileHandle?: string; mapping?: CsvMapping }
export interface ImportPreview { entries: number; totpSeeds: number; attachments: number; duplicates: number; skipped: number; warnings: string[]; titles: string[] }
export interface ImportOutcome { added: number; appended: number; skipped: number; failed: number; /** Plain sentences about what was not carried over (attachments, drive unavailable); never file content. */ warnings: string[] }
export interface ExportedFile { path: string; /** Absent when empty. For example "2 items could not be read and are not in the file." */ warnings?: string[] }
export type PlainExportFormat = "bitwarden-json" | "csv";

/** Everything the UI may ask of the core. Implemented by the Tauri adapter and the dev-only preview backend. */
export interface Backend {
  readonly kind: "tauri" | "preview";
  suiteInfo(): Promise<SuiteInfo>;
  status(): Promise<VaultStatus>;

  // accounts (profiles)
  listProfiles(): Promise<ProfileList>;
  createProfile(displayName: string, password: string): Promise<ProfileInfo>;
  selectProfile(id: string): Promise<void>;
  renameProfile(id: string, displayName: string): Promise<ProfileInfo>;
  removeProfile(id: string, password: string, confirmation: string): Promise<void>;
  /** Native open dialog opened by the core. `null` when the user cancels. */
  importProfileFile(displayName: string): Promise<ProfileInfo | null>;
  restoreProfileFromBackup(displayName: string, input: BackupTargetInput): Promise<ProfileInfo>;

  unlock(password: string): Promise<void>;
  /**
   * The Lock button. An account with a PIN stays open in memory out of reach (its PIN shows it again); one without is
   * wiped. Read `status().resumable` afterwards to see which it was.
   */
  lock(): Promise<void>;
  /** "Log out": every open vault is wiped, those waiting behind a PIN too. The saved accounts stay. */
  lockAll(): Promise<void>;
  /** The selected account's PIN, while its vault is still open behind it (`wrong_pin`, `pin_locked`, `locked`). */
  unlockPin(pin: string): Promise<void>;
  /** Type the master password in a vault opened with the PIN: secrets and settings work again until it locks. */
  elevate(password: string): Promise<void>;
  /** Give the open account a PIN, or change it. Needs the master password. */
  setAccountPin(password: string, pin: string): Promise<ProfileInfo>;
  /** Take the PIN away. Needs the master password. */
  removeAccountPin(password: string): Promise<ProfileInfo>;
  changePassword(oldPassword: string, newPassword: string): Promise<void>;
  /** Native save dialog opened by the core. `null` when the user cancels. */
  exportVaultFile(password: string): Promise<{ path: string } | null>;

  listItems(): Promise<ItemSummary[]>;
  getItem(id: string): Promise<ItemView>;
  /** The master password is needed only when the item has `reprompt` (otherwise leave it out); without it `password_required`. */
  revealItem(id: string, password?: string): Promise<ItemDraft>;
  beginEdit(id: string, password?: string): Promise<EditSession>;
  createItem(draft: ItemDraft): Promise<ItemSummary>;
  updateItem(id: string, token: string, expectedVersion: number, draft: ItemDraft): Promise<ItemSummary>;
  trashItem(id: string): Promise<void>;
  restoreItem(id: string): Promise<ItemSummary>;
  listTrash(): Promise<TrashEntry[]>;
  /** The master password is always needed; for an item trashed less than six months ago, also the answer to the sum `purgePlan` showed. */
  purgeItem(id: string, password: string, answer?: string): Promise<void>;
  /** What erasing this trashed item for good will ask for. Each call makes a new sum. */
  purgePlan(id: string): Promise<PurgePlan>;
  /** The deep search: titles, addresses, notes, field names, tags and folders, never a password. With `oldVersions`, earlier versions too. */
  searchItems(query: string, oldVersions: boolean): Promise<SearchHit[]>;
  createFolder(path: string): Promise<FolderChange>;
  renameFolder(from: string, to: string): Promise<FolderChange>;
  /** What was inside moves up one level. */
  removeFolder(path: string): Promise<FolderChange>;
  /** Moves items into a folder (`null`: out of every folder). Answers with how many changed. */
  moveItems(ids: string[], folder: string | null): Promise<number>;
  archiveItem(id: string): Promise<ItemSummary>;
  unarchiveItem(id: string): Promise<ItemSummary>;
  cloneItem(id: string, password?: string): Promise<ItemSummary>;
  /** Dismisses the "changed on another device" note after the person has looked at it. */
  reviewOtherDevice(id: string): Promise<void>;

  listHistory(id: string): Promise<VersionEntry[]>;
  getVersion(id: string, version: number, password?: string): Promise<ItemDraft>;
  restoreVersion(id: string, version: number, password?: string): Promise<ItemSummary>;

  /** An item with `reprompt` needs the master password for its one-time code too. */
  /** The current code. `key` picks one of several codes on the item (`totp`, `totp#2` …); omitted, the first. */
  totpNow(id: string, password?: string, key?: string): Promise<TotpNow>;

  /** Say what a scanned or pasted text is and which logins it may belong to. `bareKey`: typed or pasted text, so a bare setup key counts. */
  scanClassify(text: string, bareKey?: boolean): Promise<ScanReply>;
  /** Put a scanned one-time code on the chosen login (asks for the master password where that login does) or make a login for it. */
  scanApply(text: string, choice: ScanChoice, password?: string): Promise<ItemSummary>;

  // backups
  listBackups(): Promise<BackupTargetView[]>;
  testBackup(input: BackupTargetInput): Promise<BackupTestResult>;
  addBackup(input: BackupTargetInput): Promise<BackupTargetView>;
  updateBackup(id: string, input: BackupTargetInput): Promise<BackupTargetView>;
  removeBackup(id: string): Promise<void>;
  setMasterBackup(id: string): Promise<void>;
  runBackup(id: string | null): Promise<BackupTargetView[]>;
  /** Sends whatever is still queued (bounded to a few seconds). Called when a phone app goes to the background. */
  syncFlush(): Promise<{ complete: boolean }>;
  /** The device came back online: pull what other devices changed and send what was queued. Answers at once. */
  syncReconnect(): Promise<void>;
  revealBackupSecret(id: string, password: string): Promise<string>;

  /** Opens an http(s) link in the system browser (validated in the core). */
  openUrl(url: string): Promise<void>;

  // app-level (no vault needs to be open)
  gateStatus(): Promise<GateStatus>;
  gateSubmit(input: string): Promise<GateOutcome>;
  gateBiometric(): Promise<GateOutcome>;
  /** The discreet lock-screen trigger. Only works when it is switched on; same wipe as a panic credential. */
  gatePanic(): Promise<GateOutcome>;
  /** Back to Stage 1 and lock the open vault. */
  gateClose(): Promise<void>;
  gateConfigure(input: GateConfigInput): Promise<GateStatus>;
  /** A long sum for the last "erase everything" step. The answer stays in the core. */
  appResetChallenge(): Promise<string>;
  /** The manual reset, after the third confirmation and the answer to the sum (`wrong_answer`; ask for a new sum). */
  appReset(answer: string): Promise<void>;
  /**
   * "Remove Hexis from this device": the same three confirmations and the sum as `appReset`, then everything Hexis keeps
   * on this device is removed (vaults, settings, the web view's leftovers) and the app closes. Nothing new is made.
   * S3 backups are not touched. Not available while the app lock is closed.
   */
  appPrepareUninstall(answer: string): Promise<void>;
  appSettingsGet(): Promise<AppSettings>;
  appSettingsSet(input: AppSettings): Promise<AppSettings>;
  /** Tells the core the person is active, so the inactivity timer restarts. */
  activityPing(): Promise<void>;
  /** Phones and tablets: the app went to the background (starts the "after leaving the app" timer). */
  appLeave(): Promise<void>;
  /** The app is back on screen (locks now when the time away ran out). */
  appReturn(): Promise<void>;
  /** Android: blocks screenshots and screen recording while a decrypted file is on screen (FLAG_SECURE). No-op elsewhere. */
  screenSecure(on: boolean): Promise<void>;
  // passkey provider (Android 14 and later; everywhere else `supported` is false and nothing else works)
  /** Whether this phone can use Hexis as a passkey provider, and whether the provider is switched on. */
  passkeyProviderStatus(): Promise<PasskeyProviderStatus>;
  /** Switch the provider on or off. Needs the master password to have opened this vault (not only the PIN). */
  passkeyProviderSet(on: boolean): Promise<PasskeyProviderStatus>;
  /** Open Android's own passkey settings, where Hexis is chosen as a provider. */
  passkeyProviderOpenSettings(): Promise<void>;
  // import / export
  /** Native picker in the core; it sniffs the format from the content. `null` when cancelled. */
  importPick(): Promise<ImportPick | null>;
  importPickKeyfile(): Promise<{ handle: string; fileName: string } | null>;
  inspectCsvHeaders(handle: string): Promise<CsvInspect>;
  importPreview(options: ImportOptions): Promise<ImportPreview>;
  importCommit(options: ImportOptions): Promise<ImportOutcome>;
  importCancel(handle: string): Promise<void>;
  /** A readable copy of the vault (re-prompt, native save dialog, owner-only file). `null` when cancelled. */
  exportPlain(format: PlainExportFormat, password: string): Promise<ExportedFile | null>;

  // encrypted drive: the core picks, reads, encrypts and writes files itself, so file bytes never travel as JSON
  fileList(itemId: string | null): Promise<FileEntry[]>;
  /** Native picker in the core (several files). `null` when cancelled. Files over 50 MB are refused with a clear message. */
  fileAdd(itemId: string | null): Promise<FileEntry[] | null>;
  fileRename(id: string, name: string): Promise<FileEntry>;
  fileDelete(id: string, password: string): Promise<void>;
  /** Decrypted bytes for the in-app preview only. */
  fileRead(id: string): Promise<ArrayBuffer>;
  /** Native save dialog in the core; decrypts straight to the chosen file. `null` when cancelled. */
  fileSaveCopy(id: string, password: string): Promise<{ path: string } | null>;
  disguiseGet(): Promise<DisguiseView>;
  disguiseSet(input: DisguiseInput): Promise<DisguiseView>;

  /** The vault's own preferences (needs an open vault). */
  prefsGet(): Promise<PrefsView>;
  /** Remembers the generator's settings (4 to 128 characters, at least one character type) and queues them for every backup target. Saving what is already saved changes nothing. */
  prefsSetGenerator(generator: GeneratorPrefs): Promise<PrefsView>;

  // device settings, the trusted clock and the demo vault
  deviceGet(): Promise<DeviceInfo>;
  deviceSetTimeZone(zone: string): Promise<DeviceInfo>;
  /** The first-run welcome was seen. */
  deviceFinishSetup(): Promise<DeviceInfo>;
  deviceSetTimeCheck(on: boolean): Promise<DeviceInfo>;
  deviceSetTheme(theme: Theme): Promise<DeviceInfo>;
  deviceSetClipboardClear(seconds: ClipboardClear): Promise<DeviceInfo>;
  deviceSetOpenInSearch(on: boolean): Promise<DeviceInfo>;
  /** Settings > Passkeys: allow or stop the one website request that checks an app. Off, apps get no passkeys. */
  deviceSetPasskeyAppCheck(on: boolean): Promise<DeviceInfo>;
  deviceSetListSort(order: ListSort): Promise<DeviceInfo>;
  clockStatus(): Promise<ClockStatus>;
  /** Checks the time online now (up to about ten seconds). A failure is in `problem`, not an error. */
  clockSync(): Promise<ClockStatus>;
  /** Makes the demo vault when the app holds no vault; leaves it selected and unlocked. */
  demoSpawn(): Promise<ProfileInfo>;
  /** Selects and unlocks the demo vault with no typed password. */
  demoOpen(): Promise<void>;
}

/** Field schemas per item type (must match the contract and the Rust validation). */
export interface FieldSpec {
  key: string;
  label: string;
  kind: FieldKind;
  placeholder?: string;
  /** Stored more than once as `key`, `key#2` … `key#10` (a login's sign-ins and codes). */
  repeats?: boolean;
}

/** How many sign-ins and how many one-time codes one login holds (mirrors MAX_INSTANCES in the core). */
export const MAX_INSTANCES = 10;

/** `totp` is `{ base: "totp", n: 1 }`, `totp#3` is `{ base: "totp", n: 3 }`; any other spelling is not a stored key. */
export function parseFieldKey(key: string): { base: string; n: number } | null {
  const i = key.indexOf("#");
  if (i < 0) return { base: key, n: 1 };
  const rest = key.slice(i + 1);
  if (!/^[1-9]\d{0,2}$/.test(rest) || Number(rest) < 2) return null;
  return { base: key.slice(0, i), n: Number(rest) };
}

/** The stored key of instance `n` of a field. */
export const instanceKey = (base: string, n: number): string => (n <= 1 ? base : `${base}#${n}`);

/** Every stored instance of one field, lowest number first. */
export function instancesOf(fields: Record<string, string>, base: string): { n: number; key: string; value: string }[] {
  const out: { n: number; key: string; value: string }[] = [];
  for (const [key, value] of Object.entries(fields)) {
    const p = parseFieldKey(key);
    if (p && p.base === base) out.push({ n: p.n, key, value });
  }
  return out.sort((a, b) => a.n - b.n);
}

/** The schema entry a stored key belongs to (`totp#2` belongs to `totp` where the type repeats it). */
export function specOf(type: ItemType, key: string): FieldSpec | null {
  const p = parseFieldKey(key);
  if (!p) return null;
  const spec = FIELD_SCHEMA[type].find((f) => f.key === p.base);
  if (!spec) return null;
  return p.n === 1 || (spec.repeats && p.n <= MAX_INSTANCES) ? spec : null;
}

const TOTP_SPEC: FieldSpec = {
  key: "totp",
  label: "One-time code",
  kind: "totp",
  placeholder: "Paste a link or setup key, or scan a QR code",
};
const REPEATING_TOTP: FieldSpec = { ...TOTP_SPEC, repeats: true };
/** Passkeys are made by websites through the passkey provider and by the scanner, never typed; the editor can only remove one. */
const PASSKEY_SPEC: FieldSpec = { key: "passkey", label: "Passkey", kind: "passkey", repeats: true };

export const FIELD_SCHEMA: Record<ItemType, FieldSpec[]> = {
  login: [
    { key: "username", label: "Username", kind: "text", placeholder: "name@example.com", repeats: true },
    { key: "password", label: "Password", kind: "password", repeats: true },
    { key: "url", label: "Website", kind: "url", placeholder: "https://example.com" },
    REPEATING_TOTP,
    PASSKEY_SPEC,
    { key: "notes", label: "Notes", kind: "note" },
  ],
  card: [
    { key: "cardholder", label: "Name on card", kind: "text", placeholder: "As printed on the card" },
    { key: "brand", label: "Brand", kind: "text", placeholder: "Visa, Mastercard, Amex" },
    { key: "number", label: "Card number", kind: "password", placeholder: "1234 5678 9012 3456" },
    { key: "expMonth", label: "Expiry month", kind: "text", placeholder: "MM" },
    { key: "expYear", label: "Expiry year", kind: "text", placeholder: "YYYY" },
    { key: "code", label: "Security code", kind: "password", placeholder: "CVV / CVC" },
    { key: "pin", label: "Card PIN", kind: "password" },
    { key: "notes", label: "Notes", kind: "note" },
  ],
  identity: [
    { key: "salutation", label: "Title", kind: "text", placeholder: "Mr, Ms, Dr" },
    { key: "firstName", label: "First name", kind: "text" },
    { key: "middleName", label: "Middle name", kind: "text" },
    { key: "lastName", label: "Last name", kind: "text" },
    { key: "email", label: "Email", kind: "text", placeholder: "name@example.com" },
    { key: "phone", label: "Phone", kind: "text" },
    { key: "company", label: "Company", kind: "text" },
    { key: "address1", label: "Address", kind: "text" },
    { key: "address2", label: "Address line 2", kind: "text" },
    { key: "city", label: "City", kind: "text" },
    { key: "state", label: "State or region", kind: "text" },
    { key: "postalCode", label: "Postal code", kind: "text" },
    { key: "country", label: "Country", kind: "text" },
    { key: "nationalId", label: "National ID number", kind: "password" },
    { key: "passport", label: "Passport number", kind: "password" },
    { key: "license", label: "Licence number", kind: "password" },
    { key: "notes", label: "Notes", kind: "note" },
  ],
  sshKey: [
    { key: "publicKey", label: "Public key", kind: "paragraph", placeholder: "ssh-ed25519 AAAA… you@host" },
    { key: "privateKey", label: "Private key", kind: "note", placeholder: "-----BEGIN OPENSSH PRIVATE KEY-----" },
    { key: "passphrase", label: "Key passphrase", kind: "password" },
    { key: "fingerprint", label: "Fingerprint", kind: "text", placeholder: "SHA256:…" },
    { key: "notes", label: "Notes", kind: "note" },
  ],
  secureNote: [{ key: "body", label: "Note", kind: "note" }],
  totp: [
    { key: "issuer", label: "Service", kind: "text", placeholder: "GitHub" },
    { key: "account", label: "Account", kind: "text", placeholder: "name@example.com" },
    TOTP_SPEC,
  ],
  media: [{ key: "description", label: "Description", kind: "note" }],
};

export const TYPE_LABEL: Record<ItemType, string> = {
  login: "Login",
  card: "Card",
  identity: "Identity",
  sshKey: "SSH key",
  secureNote: "Secure note",
  totp: "Code only",
  media: "Media",
};

/** Is this field's value secret (sealed until the master password is re-entered)? Mirrors the Rust field kinds. */
export const isSecretKind = (k: FieldKind): boolean => k === "password" || k === "note" || k === "totp" || k === "passkey";
