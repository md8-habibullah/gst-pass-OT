import { invoke } from "@tauri-apps/api/core";
import {
  BackendError,
  type Backend,
  type EditSession,
  type ErrorCode,
  type ItemDraft,
  type ItemSummary,
  type ItemView,
  type SuiteInfo,
  type TotpNow,
  type TrashEntry,
  type VaultStatus,
  type VersionEntry,
} from "./types";

const KNOWN: ReadonlySet<string> = new Set<ErrorCode>([
  "locked", "no_vault", "vault_exists", "wrong_password", "not_found", "in_trash", "not_in_trash",
  "version_conflict", "invalid_input", "confirmation_mismatch", "token_invalid", "storage", "crypto",
]);

/** Tauri rejects with the serialized Rust error ({code,message}) or, for framework errors, a string. */
function normalize(e: unknown): BackendError {
  if (e instanceof BackendError) return e;
  if (e && typeof e === "object" && "code" in e) {
    const { code, message } = e as { code: unknown; message?: unknown };
    const c = typeof code === "string" && KNOWN.has(code) ? (code as ErrorCode) : "unknown";
    return new BackendError(c, typeof message === "string" ? message : "Something went wrong.");
  }
  return new BackendError("unknown", typeof e === "string" ? e : "Something went wrong.");
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw normalize(e);
  }
}

export const tauriBackend: Backend = {
  kind: "tauri",
  suiteInfo: () => call<SuiteInfo>("crypto_suite_info"),
  status: () => call<VaultStatus>("vault_status"),
  createVault: (password) => call("vault_create", { password }),
  unlock: (password) => call("vault_unlock", { password }),
  lock: () => call("vault_lock"),
  changePassword: (oldPassword, newPassword) => call("vault_change_password", { oldPassword, newPassword }),

  listItems: () => call<ItemSummary[]>("item_list"),
  getItem: (id) => call<ItemView>("item_get", { id }),
  revealItem: (id, password) => call<ItemDraft>("item_reveal", { id, password }),
  beginEdit: (id, password) => call<EditSession>("item_begin_edit", { id, password }),
  createItem: (draft) => call<ItemSummary>("item_create", { draft }),
  updateItem: (id, token, expectedVersion, draft) =>
    call<ItemSummary>("item_update", { id, token, expectedVersion, draft }),
  trashItem: (id) => call("item_trash", { id }),
  restoreItem: (id) => call<ItemSummary>("item_restore", { id }),
  listTrash: () => call<TrashEntry[]>("trash_list"),
  purgeItem: (id, password, confirmation) => call("item_purge", { id, password, confirmation }),

  listHistory: (id) => call<VersionEntry[]>("history_list", { id }),
  getVersion: (id, version, password) => call<ItemDraft>("history_get", { id, version, password }),
  restoreVersion: (id, version, password) => call<ItemSummary>("history_restore", { id, version, password }),

  totpNow: (id) => call<TotpNow>("totp_now", { id }),
};
