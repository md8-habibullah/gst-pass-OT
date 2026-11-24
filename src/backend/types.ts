// Mirrors ai-docs/IPC_CONTRACT.md. Change the contract first, then this file and the Rust command layer.

export type ItemType = "login" | "card" | "identity" | "sshKey" | "secureNote" | "totp" | "media";
/** paragraph = non-secret multi-line text (for example an SSH public key). */
export type FieldKind = "text" | "url" | "password" | "note" | "totp" | "paragraph";

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
  updatedAt: number;
  version: number;
  archived: boolean;
}

export interface FieldView {
  key: string;
  label: string;
  kind: FieldKind;
  secret: boolean;
  value: string | null;
  hasValue: boolean;
}

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
  /** `null` until a failed attempt; then 15 minus the attempts used. */
  attemptsLeft: number | null;
  streak: number;
}
export type GateOutcome =
  | { kind: "math"; correct: boolean; equation: string; streak: number }
  | { kind: "opened" }
  | { kind: "failed"; attemptsLeft: number; equation: string }
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
export interface AppSettings {
  lockAfterMinutes: 1 | 5 | 15 | 30;
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
export type ImportMode = "skip_duplicates" | "append_version" | "import_all";
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
  lock(): Promise<void>;
  changePassword(oldPassword: string, newPassword: string): Promise<void>;
  /** Native save dialog opened by the core. `null` when the user cancels. */
  exportVaultFile(password: string): Promise<{ path: string } | null>;

  listItems(): Promise<ItemSummary[]>;
  getItem(id: string): Promise<ItemView>;
  revealItem(id: string, password: string): Promise<ItemDraft>;
  beginEdit(id: string, password: string): Promise<EditSession>;
  createItem(draft: ItemDraft): Promise<ItemSummary>;
  updateItem(id: string, token: string, expectedVersion: number, draft: ItemDraft): Promise<ItemSummary>;
  trashItem(id: string): Promise<void>;
  restoreItem(id: string): Promise<ItemSummary>;
  listTrash(): Promise<TrashEntry[]>;
  purgeItem(id: string, password: string, confirmation: string): Promise<void>;
  archiveItem(id: string): Promise<ItemSummary>;
  unarchiveItem(id: string): Promise<ItemSummary>;
  cloneItem(id: string, password: string): Promise<ItemSummary>;
  /** Dismisses the "changed on another device" note after the person has looked at it. */
  reviewOtherDevice(id: string): Promise<void>;

  listHistory(id: string): Promise<VersionEntry[]>;
  getVersion(id: string, version: number, password: string): Promise<ItemDraft>;
  restoreVersion(id: string, version: number, password: string): Promise<ItemSummary>;

  totpNow(id: string): Promise<TotpNow>;

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
  /** The manual reset, after the third confirmation. */
  appReset(): Promise<void>;
  appSettingsGet(): Promise<AppSettings>;
  appSettingsSet(input: AppSettings): Promise<AppSettings>;
  /** Tells the core the person is active, so the inactivity timer restarts. */
  activityPing(): Promise<void>;
  /** Android: blocks screenshots and screen recording while a decrypted file is on screen (FLAG_SECURE). No-op elsewhere. */
  screenSecure(on: boolean): Promise<void>;
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
}

/** Field schemas per item type (must match the contract and the Rust validation). */
export interface FieldSpec {
  key: string;
  label: string;
  kind: FieldKind;
  placeholder?: string;
}

const TOTP_SPEC: FieldSpec = {
  key: "totp",
  label: "One-time code",
  kind: "totp",
  placeholder: "Paste a link or setup key, or scan a QR code",
};

export const FIELD_SCHEMA: Record<ItemType, FieldSpec[]> = {
  login: [
    { key: "username", label: "Username", kind: "text", placeholder: "name@example.com" },
    { key: "password", label: "Password", kind: "password" },
    { key: "url", label: "Website", kind: "url", placeholder: "https://example.com" },
    TOTP_SPEC,
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
export const isSecretKind = (k: FieldKind): boolean => k === "password" || k === "note" || k === "totp";
