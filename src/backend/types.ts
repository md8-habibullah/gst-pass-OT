// Mirrors ai-docs/IPC_CONTRACT.md. Change the contract first, then this file and the Rust command layer.

export type ItemType = "login" | "totp" | "secureNote" | "media";
export type FieldKind = "text" | "url" | "password" | "note" | "totp";

export interface SuiteInfo {
  version: number;
  kem: string;
  kdf: string;
  aead: string;
  kdfDefaults: { mCostKiB: number; tCost: number; pCost: number };
  nonceBytes: number;
  keyBytes: number;
}

export interface VaultStatus {
  exists: boolean;
  unlocked: boolean;
  capabilities: { quickUnlock: boolean; biometric: boolean };
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

export type VersionEvent = "created" | "edited" | "trashed" | "restored" | "restoredFromVersion";

export interface VersionEntry {
  version: number;
  timestamp: number;
  event: VersionEvent;
  changedFields: string[];
  restoredFrom: number | null;
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
  | "unknown";

export class BackendError extends Error {
  readonly code: ErrorCode;
  constructor(code: ErrorCode, message: string) {
    super(message);
    this.name = "BackendError";
    this.code = code;
  }
}

/** Everything the UI may ask of the core. Implemented by the Tauri adapter and the dev-only preview backend. */
export interface Backend {
  readonly kind: "tauri" | "preview";
  suiteInfo(): Promise<SuiteInfo>;
  status(): Promise<VaultStatus>;
  createVault(password: string): Promise<void>;
  unlock(password: string): Promise<void>;
  lock(): Promise<void>;
  changePassword(oldPassword: string, newPassword: string): Promise<void>;

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

  listHistory(id: string): Promise<VersionEntry[]>;
  getVersion(id: string, version: number, password: string): Promise<ItemDraft>;
  restoreVersion(id: string, version: number, password: string): Promise<ItemSummary>;

  totpNow(id: string): Promise<TotpNow>;
}

/** Field schemas per item type (must match the contract and the Rust validation). */
export interface FieldSpec {
  key: string;
  label: string;
  kind: FieldKind;
  placeholder?: string;
}

export const FIELD_SCHEMA: Record<ItemType, FieldSpec[]> = {
  login: [
    { key: "username", label: "Username", kind: "text", placeholder: "name@example.com" },
    { key: "password", label: "Password", kind: "password" },
    { key: "url", label: "Website", kind: "url", placeholder: "https://example.com" },
    { key: "totp", label: "One-time code", kind: "totp", placeholder: "Paste an otpauth:// link or the setup key" },
    { key: "notes", label: "Notes", kind: "note" },
  ],
  totp: [
    { key: "issuer", label: "Service", kind: "text", placeholder: "GitHub" },
    { key: "account", label: "Account", kind: "text", placeholder: "name@example.com" },
    { key: "totp", label: "One-time code", kind: "totp", placeholder: "Paste an otpauth:// link or the setup key" },
  ],
  secureNote: [{ key: "body", label: "Note", kind: "note" }],
  media: [{ key: "description", label: "Description", kind: "note" }],
};

export const TYPE_LABEL: Record<ItemType, string> = {
  login: "Login",
  totp: "Code only",
  secureNote: "Secure note",
  media: "Media",
};
