import { invoke } from "@tauri-apps/api/core";
import {
  BackendError,
  type Backend,
  type AppSettings,
  type BackupTargetView,
  type BackupTestResult,
  type CsvInspect,
  type DisguiseView,
  type EditSession,
  type ErrorCode,
  type FileEntry,
  type GateOutcome,
  type GateStatus,
  type ExportedFile,
  type ImportOutcome,
  type ImportPick,
  type ImportPreview,
  type ItemDraft,
  type ItemSummary,
  type ItemView,
  type ProfileInfo,
  type ProfileList,
  type SuiteInfo,
  type TotpNow,
  type TrashEntry,
  type VaultStatus,
  type VersionEntry,
} from "./types";

const KNOWN: ReadonlySet<string> = new Set<ErrorCode>([
  "locked", "no_vault", "vault_exists", "wrong_password", "not_found", "in_trash", "not_in_trash",
  "version_conflict", "invalid_input", "confirmation_mismatch", "token_invalid", "storage", "crypto",
  "profile_name_taken", "unsupported_file", "network", "remote",
]);

/** The core rejects with the serialized Rust error ({code,message}) or, for framework errors, a string. */
export function normalizeError(e: unknown): BackendError {
  if (e instanceof BackendError) return e;
  if (e && typeof e === "object" && "code" in e) {
    const { code, message } = e as { code: unknown; message?: unknown };
    const c = typeof code === "string" && KNOWN.has(code) ? (code as ErrorCode) : "unknown";
    return new BackendError(c, typeof message === "string" ? message : "Something went wrong.");
  }
  return new BackendError("unknown", typeof e === "string" ? e : "Something went wrong.");
}

export type Call = <T = void>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

/** The whole contract (IPC_CONTRACT.md) over any transport: Tauri IPC in the app, HTTP in the dev e2e harness. */
export function makeBackend(call: Call, kind: Backend["kind"]): Backend {
  return {
    kind,
    suiteInfo: () => call<SuiteInfo>("crypto_suite_info"),
    status: () => call<VaultStatus>("vault_status"),

    listProfiles: () => call<ProfileList>("profile_list"),
    createProfile: (displayName, password) => call<ProfileInfo>("profile_create", { displayName, password }),
    selectProfile: (id) => call("profile_select", { id }),
    renameProfile: (id, displayName) => call<ProfileInfo>("profile_rename", { id, displayName }),
    removeProfile: (id, password, confirmation) => call("profile_remove", { id, password, confirmation }),
    importProfileFile: (displayName) => call<ProfileInfo | null>("profile_import_file", { displayName }),
    restoreProfileFromBackup: (displayName, input) => call<ProfileInfo>("profile_restore_from_backup", { displayName, input }),

    unlock: (password) => call("vault_unlock", { password }),
    lock: () => call("vault_lock"),
    changePassword: (oldPassword, newPassword) => call("vault_change_password", { oldPassword, newPassword }),
    exportVaultFile: (password) => call<{ path: string } | null>("vault_export_file", { password }),

    listItems: () => call<ItemSummary[]>("item_list"),
    getItem: (id) => call<ItemView>("item_get", { id }),
    revealItem: (id, password) => call<ItemDraft>("item_reveal", { id, password }),
    beginEdit: (id, password) => call<EditSession>("item_begin_edit", { id, password }),
    createItem: (draft) => call<ItemSummary>("item_create", { draft }),
    updateItem: (id, token, expectedVersion, draft) => call<ItemSummary>("item_update", { id, token, expectedVersion, draft }),
    trashItem: (id) => call("item_trash", { id }),
    restoreItem: (id) => call<ItemSummary>("item_restore", { id }),
    listTrash: () => call<TrashEntry[]>("trash_list"),
    purgeItem: (id, password, confirmation) => call("item_purge", { id, password, confirmation }),
    archiveItem: (id) => call<ItemSummary>("item_archive", { id }),
    unarchiveItem: (id) => call<ItemSummary>("item_unarchive", { id }),
    cloneItem: (id, password) => call<ItemSummary>("item_clone", { id, password }),

    listHistory: (id) => call<VersionEntry[]>("history_list", { id }),
    reviewOtherDevice: (id) => call<void>("item_review_other_device", { id }),
    getVersion: (id, version, password) => call<ItemDraft>("history_get", { id, version, password }),
    restoreVersion: (id, version, password) => call<ItemSummary>("history_restore", { id, version, password }),

    totpNow: (id) => call<TotpNow>("totp_now", { id }),

    listBackups: () => call<BackupTargetView[]>("backup_list"),
    testBackup: (input) => call<BackupTestResult>("backup_test", { input }),
    addBackup: (input) => call<BackupTargetView>("backup_add", { input }),
    updateBackup: (id, input) => call<BackupTargetView>("backup_update", { id, input }),
    removeBackup: (id) => call("backup_remove", { id }),
    setMasterBackup: (id) => call("backup_set_master", { id }),
    runBackup: (id) => call<BackupTargetView[]>("backup_run", { id }),
    syncFlush: () => call<{ complete: boolean }>("sync_flush"),
    revealBackupSecret: (id, password) => call<string>("backup_reveal_secret", { id, password }),

    openUrl: (url) => call("open_url", { url }),

    gateStatus: () => call<GateStatus>("gate_status"),
    gateSubmit: (input) => call<GateOutcome>("gate_submit", { input }),
    gateBiometric: () => call<GateOutcome>("gate_biometric"),
    gatePanic: () => call<GateOutcome>("gate_panic"),
    gateClose: () => call("gate_close"),
    gateConfigure: (input) => call<GateStatus>("gate_configure", { input }),
    appReset: () => call("app_reset", { confirmedSteps: 3 }),
    appSettingsGet: () => call<AppSettings>("app_settings_get"),
    appSettingsSet: (input) => call<AppSettings>("app_settings_set", { input }),
    activityPing: () => call("activity_ping"),
    screenSecure: (on) => call("screen_secure", { on }),
    importPick: () => call<ImportPick | null>("import_pick"),
    importPickKeyfile: () => call<{ handle: string; fileName: string } | null>("import_pick_keyfile"),
    inspectCsvHeaders: (handle) => call<CsvInspect>("inspect_csv_headers", { handle }),
    importPreview: (options) => call<ImportPreview>("import_preview", { options }),
    importCommit: (options) => call<ImportOutcome>("import_commit", { options }),
    importCancel: (handle) => call("import_cancel", { handle }),
    exportPlain: (format, password) => call<ExportedFile | null>("export_plain", { format, password }),
    fileList: (itemId) => call<FileEntry[]>("file_list", { itemId }),
    fileAdd: (itemId) => call<FileEntry[] | null>("file_add", { itemId }),
    fileRename: (id, name) => call<FileEntry>("file_rename", { id, name }),
    fileDelete: (id, password) => call("file_delete", { id, password }),
    fileRead: (id) => call<ArrayBuffer>("file_read", { id }),
    fileSaveCopy: (id, password) => call<{ path: string } | null>("file_save_copy", { id, password }),
    disguiseGet: () => call<DisguiseView>("disguise_get"),
    disguiseSet: (input) => call<DisguiseView>("disguise_set", { input }),
  };
}

const tauriCall: Call = async <T,>(cmd: string, args?: Record<string, unknown>) => {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw normalizeError(e);
  }
};

export const tauriBackend: Backend = makeBackend(tauriCall, "tauri");
