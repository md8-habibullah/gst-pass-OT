// DEV-ONLY preview backend: lets the UI be designed and screenshotted in a normal browser.
// It is NOT secure (plain in-memory data, string-compare "password check"). Never bundled in production
// builds: src/backend/index.ts loads it behind import.meta.env.DEV and only outside Tauri.
import {
  BackendError,
  FIELD_SCHEMA,
  type Backend,
  type ItemDraft,
  type ItemSummary,
  type ItemType,
  type ItemView,
  type TotpNow,
  type VersionEntry,
  type VersionEvent,
} from "./types";

interface Snapshot {
  version: number;
  timestamp: number;
  event: VersionEvent;
  changedFields: string[];
  restoredFrom: number | null;
  draft: ItemDraft;
}
interface Stored {
  id: string;
  createdAt: number;
  updatedAt: number;
  version: number;
  deleted: boolean;
  deletedAt: number | null;
  draft: ItemDraft;
  history: Snapshot[];
}

const delay = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const clone = <T,>(v: T): T => structuredClone(v);
const MIN = 60_000;
const DAY = 24 * 60 * MIN;

let vaultPassword: string | null = null;
let unlocked = false;
const store = new Map<string, Stored>();
const tokens = new Map<string, { id: string; version: number; expires: number }>();

function err(code: ConstructorParameters<typeof BackendError>[0], message: string): never {
  throw new BackendError(code, message);
}
function requireUnlocked() {
  if (!unlocked) err("locked", "The vault is locked.");
}
async function verify(password: string) {
  await delay(650); // stands in for Argon2id, which is slow on purpose
  if (password !== vaultPassword) err("wrong_password", "That master password is not right.");
}
function need(id: string): Stored {
  const s = store.get(id);
  if (!s) err("not_found", "That item no longer exists.");
  return s;
}
function active(id: string): Stored {
  const s = need(id);
  if (s.deleted) err("in_trash", "Restore this item from the trash to open it.");
  return s;
}

function hostOf(url: string): string | null {
  try {
    return new URL(url.includes("://") ? url : `https://${url}`).hostname.replace(/^www\./, "");
  } catch {
    return null;
  }
}
function summarize(s: Stored): ItemSummary {
  const d = s.draft;
  const f = d.fields;
  return {
    id: s.id,
    type: d.type,
    title: d.title,
    subtitle: f.username || f.account || (f.url ? hostOf(f.url) : null) || null,
    favorite: d.favorite,
    tags: d.tags,
    folder: d.folder,
    hasTotp: Boolean(f.totp),
    updatedAt: s.updatedAt,
    version: s.version,
  };
}

function diff(a: ItemDraft | null, b: ItemDraft): string[] {
  if (!a) return [];
  const out: string[] = [];
  if (a.title !== b.title) out.push("title");
  if (a.favorite !== b.favorite) out.push("favorite");
  if (a.folder !== b.folder) out.push("folder");
  if (a.tags.join("\u0000") !== b.tags.join("\u0000")) out.push("tags");
  for (const k of new Set([...Object.keys(a.fields), ...Object.keys(b.fields)])) {
    if ((a.fields[k] ?? "") !== (b.fields[k] ?? "")) out.push(k);
  }
  if (JSON.stringify(a.customFields) !== JSON.stringify(b.customFields)) out.push("customFields");
  return out;
}

function cleanDraft(d: ItemDraft): ItemDraft {
  const title = d.title.trim();
  if (!title) err("invalid_input", "Give the item a title.");
  const allowed = new Set(FIELD_SCHEMA[d.type].map((f) => f.key));
  const fields: Record<string, string> = {};
  for (const [k, v] of Object.entries(d.fields)) {
    if (!allowed.has(k)) err("invalid_input", `Unknown field “${k}”.`);
    if (v.trim() !== "") fields[k] = v;
  }
  if (fields.totp) parseTotp(fields.totp); // throws invalid_input when unusable
  return { ...d, title, fields, tags: d.tags.map((t) => t.trim()).filter(Boolean) };
}

function snapshot(s: Stored, event: VersionEvent, prev: ItemDraft | null, restoredFrom: number | null = null, ts = Date.now()) {
  s.history.push({ version: s.version, timestamp: ts, event, changedFields: diff(prev, s.draft), restoredFrom, draft: clone(s.draft) });
}

function seedItem(minsAgo: number, draft: ItemDraft, edits: { minsAgo: number; patch: Partial<ItemDraft["fields"]>; title?: string }[] = []): Stored {
  const created = Date.now() - minsAgo * MIN;
  const s: Stored = { id: crypto.randomUUID(), createdAt: created, updatedAt: created, version: 1, deleted: false, deletedAt: null, draft: clone(draft), history: [] };
  snapshot(s, "created", null, null, created);
  for (const e of edits.sort((a, b) => b.minsAgo - a.minsAgo)) {
    const prev = clone(s.draft);
    s.draft.fields = { ...s.draft.fields, ...e.patch };
    if (e.title) s.draft.title = e.title;
    s.version += 1;
    s.updatedAt = Date.now() - e.minsAgo * MIN;
    snapshot(s, "edited", prev, null, s.updatedAt);
  }
  return s;
}

const login = (title: string, fields: Record<string, string>, extra: Partial<ItemDraft> = {}): ItemDraft => ({
  type: "login", title, favorite: false, tags: [], folder: null, fields, customFields: [], ...extra,
});

function seed() {
  store.clear();
  const items: Stored[] = [
    seedItem(60 * 24 * 90, login("GitHub", { username: "marlowe-dev", password: "kX9#mQ2vTz!8pLd4", url: "https://github.com/login", totp: "otpauth://totp/GitHub:marlowe-dev?secret=JBSWY3DPEHPK3PXP&issuer=GitHub&period=30&digits=6", notes: "Personal account. Recovery codes are in the Recovery codes note." }, { favorite: true, tags: ["work", "dev"] }), [
      { minsAgo: 60 * 24 * 41, patch: { password: "t7Rw-Nq4!zYb2uHc" } },
      { minsAgo: 60 * 26, patch: { password: "kX9#mQ2vTz!8pLd4", url: "https://github.com/login" } },
    ]),
    seedItem(60 * 24 * 12, login("Fastmail", { username: "marlowe@fastmail.example", password: "violet-anchor-Quench-71", url: "https://app.fastmail.com", totp: "JBSWY3DPEHPK3PXP" }, { tags: ["personal"] })),
    seedItem(60 * 5, login("Hetzner Cloud", { username: "ops@marlowe.example", password: "Hq4$vN8wLr2^TmZx", url: "https://console.hetzner.cloud", totp: "otpauth://totp/Hetzner:ops?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Hetzner&period=30&digits=6" }, { tags: ["work", "infra"] }), [
      { minsAgo: 60 * 5, patch: { notes: "Project: edge-cluster. Billing alerts go to ops@." } },
    ]),
    seedItem(60 * 24 * 3, login("Northline Bank", { username: "4417 2208", password: "7!rBv-ka39Q#sdLm", url: "https://online.northline.example", notes: "Branch PIN is not stored here." }, { favorite: true, tags: ["finance"] })),
    seedItem(60 * 24 * 21, login("Cloudflare", { username: "marlowe@fastmail.example", password: "Zp3&yLk0!vWm8RtB", url: "https://dash.cloudflare.com", totp: "otpauth://totp/Cloudflare:marlowe?secret=MFRGGZDFMZTWQ2LKNNWG23TPOBYXE43U&issuer=Cloudflare&digits=6&period=30" }, { tags: ["work", "infra"] })),
    seedItem(60 * 24 * 160, { type: "secureNote", title: "Home Wi-Fi", favorite: false, tags: ["home"], folder: null, fields: { body: "Network: Linden-5G\nPassword: marble-orchard-kettle-84\nRouter admin: 192.168.1.1 (admin / see router label)" }, customFields: [] }),
    seedItem(60 * 24 * 30, { type: "secureNote", title: "Recovery codes", favorite: false, tags: ["work"], folder: null, fields: { body: "GitHub\n8f2a-91cc-d04e\n3b7d-62aa-1f90\ne1c5-77b8-0a4d\n\nCloudflare\n9d31-aa20-5be7" }, customFields: [] }),
    seedItem(60 * 24 * 7, { type: "totp", title: "Google", favorite: false, tags: ["personal"], folder: null, fields: { issuer: "Google", account: "marlowe@example.com", totp: "otpauth://totp/Google:marlowe@example.com?secret=KRSXG5CTMVRXEZLUKRSXG5CTMVRXEZLU&issuer=Google" }, customFields: [] }),
    seedItem(60 * 24 * 2, login("Proxmox lab", { username: "root@pam", password: "Cx8^pQe2!tVn7MsH", url: "https://pve.lab.example:8006" }, { tags: ["infra"], customFields: [{ name: "Node", value: "pve-02", hidden: false }, { name: "API token", value: "pve!ci=3f8a1c2e-77d4-4b1a-9e0c-5d2b8f6a1e44", hidden: true }] })),
    seedItem(60 * 24 * 55, { type: "media", title: "Passport scan", favorite: false, tags: ["documents"], folder: null, fields: { description: "Front page, scanned 2025. Encrypted files can be attached once media support ships." }, customFields: [] }),
    seedItem(60 * 30, login("Atlas Registry", { username: "marlowe", password: "Wd6!kRm3^Ys9TqLz", url: "https://registry.atlas.example" })),
  ];
  const old1 = seedItem(60 * 24 * 200, login("Old VPN", { username: "mdev", password: "o1d-vpn-pass-0000", url: "https://vpn.legacy.example" }));
  const old2 = seedItem(60 * 24 * 120, login("Gym membership", { username: "marlowe.d", password: "Lift-Heavy-22!", url: "https://members.ironyard.example" }));
  for (const [s, mins] of [[old1, 60 * 24 * 19], [old2, 60 * 24 * 4]] as const) {
    const prev = clone(s.draft);
    s.version += 1; s.deleted = true; s.deletedAt = Date.now() - mins * MIN; s.updatedAt = s.deletedAt;
    snapshot(s, "trashed", prev, null, s.deletedAt);
    items.push(s);
  }
  for (const s of items) store.set(s.id, s);
}

// ---- TOTP (real maths, for realistic rings in the preview) ----
const B32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
function b32decode(input: string): Uint8Array {
  const clean = input.toUpperCase().replace(/[\s=-]/g, "");
  let bits = 0, value = 0;
  const out: number[] = [];
  for (const ch of clean) {
    const i = B32.indexOf(ch);
    if (i < 0) err("invalid_input", "That setup key is not valid base32.");
    value = (value << 5) | i; bits += 5;
    if (bits >= 8) { out.push((value >>> (bits - 8)) & 0xff); bits -= 8; }
  }
  if (out.length < 10) err("invalid_input", "That setup key is too short.");
  return Uint8Array.from(out);
}
function parseTotp(raw: string) {
  const v = raw.trim();
  if (v.toLowerCase().startsWith("otpauth://")) {
    let u: URL;
    try { u = new URL(v); } catch { err("invalid_input", "That otpauth link is not valid."); }
    if (u.hostname !== "totp") err("invalid_input", "Only time-based codes are supported.");
    const secret = u.searchParams.get("secret");
    if (!secret) err("invalid_input", "The otpauth link has no secret.");
    return {
      key: b32decode(secret),
      period: Number(u.searchParams.get("period") ?? 30) || 30,
      digits: Number(u.searchParams.get("digits") ?? 6) || 6,
      alg: ({ SHA1: "SHA-1", SHA256: "SHA-256", SHA512: "SHA-512" } as Record<string, string>)[(u.searchParams.get("algorithm") ?? "SHA1").toUpperCase()] ?? "SHA-1",
    };
  }
  return { key: b32decode(v), period: 30, digits: 6, alg: "SHA-1" };
}
async function totpAt(raw: string, nowMs: number): Promise<TotpNow> {
  const { key, period, digits, alg } = parseTotp(raw);
  const counter = Math.floor(nowMs / 1000 / period);
  const msg = new ArrayBuffer(8);
  new DataView(msg).setBigUint64(0, BigInt(counter));
  const k = await crypto.subtle.importKey("raw", key.buffer as ArrayBuffer, { name: "HMAC", hash: alg }, false, ["sign"]);
  const h = new Uint8Array(await crypto.subtle.sign("HMAC", k, msg));
  const o = h[h.length - 1]! & 0x0f;
  const bin = ((h[o]! & 0x7f) << 24) | (h[o + 1]! << 16) | (h[o + 2]! << 8) | h[o + 3]!;
  const code = String(bin % 10 ** digits).padStart(digits, "0");
  const validUntilMs = (counter + 1) * period * 1000;
  return { code, period, digits, validUntilMs, secondsRemaining: Math.max(0, Math.ceil((validUntilMs - nowMs) / 1000)) };
}

function view(s: Stored): ItemView {
  const d = s.draft;
  return {
    id: s.id, type: d.type, title: d.title, favorite: d.favorite, tags: d.tags, folder: d.folder,
    fields: FIELD_SCHEMA[d.type]
      .filter((spec) => d.fields[spec.key])
      .map((spec) => {
        const secret = spec.kind === "password" || spec.kind === "note" || spec.kind === "totp";
        return { key: spec.key, label: spec.label, kind: spec.kind, secret, value: secret ? null : (d.fields[spec.key] ?? null), hasValue: true };
      }),
    customFields: d.customFields.map((c) => ({ name: c.name, hidden: c.hidden, value: c.hidden ? null : c.value })),
    attachments: [], createdAt: s.createdAt, updatedAt: s.updatedAt, version: s.version,
  };
}

function consumeToken(token: string, id: string, expectedVersion: number) {
  const t = tokens.get(token);
  tokens.delete(token);
  if (!t || t.id !== id || t.expires < Date.now()) err("token_invalid", "Edit session expired. Open the item for editing again.");
  if (t.version !== expectedVersion) err("version_conflict", "This item changed since you opened it.");
}

export const previewBackend: Backend = {
  kind: "preview",
  async suiteInfo() {
    return { version: 1, kem: "ML-KEM-1024+X25519", kdf: "Argon2id", aead: "XChaCha20-Poly1305", kdfDefaults: { mCostKiB: 65536, tCost: 4, pCost: 4 }, nonceBytes: 24, keyBytes: 32 };
  },
  async status() {
    await delay(120);
    return { exists: vaultPassword !== null, unlocked, capabilities: { quickUnlock: true, biometric: true } };
  },
  async createVault(password) {
    if (vaultPassword !== null) err("vault_exists", "A vault already exists.");
    if (password.length < 8) err("invalid_input", "Use at least 8 characters.");
    await delay(900);
    vaultPassword = password; unlocked = true; seed();
  },
  async unlock(password) {
    if (vaultPassword === null) err("no_vault", "There is no vault yet.");
    await verify(password);
    unlocked = true;
  },
  async lock() { unlocked = false; tokens.clear(); },
  async changePassword(oldPassword, newPassword) {
    requireUnlocked(); await verify(oldPassword);
    if (newPassword.length < 8) err("invalid_input", "Use at least 8 characters.");
    vaultPassword = newPassword;
  },

  async listItems() {
    requireUnlocked(); await delay(60);
    return [...store.values()].filter((s) => !s.deleted).map(summarize).sort((a, b) => b.updatedAt - a.updatedAt);
  },
  async getItem(id) { requireUnlocked(); await delay(50); return view(active(id)); },
  async revealItem(id, password) { requireUnlocked(); const s = active(id); await verify(password); return clone(s.draft); },
  async beginEdit(id, password) {
    requireUnlocked(); const s = active(id); await verify(password);
    const token = crypto.randomUUID();
    tokens.set(token, { id, version: s.version, expires: Date.now() + 10 * MIN });
    return { token, draft: clone(s.draft), version: s.version };
  },
  async createItem(draft) {
    requireUnlocked(); await delay(150);
    const d = cleanDraft(draft); const now = Date.now();
    const s: Stored = { id: crypto.randomUUID(), createdAt: now, updatedAt: now, version: 1, deleted: false, deletedAt: null, draft: d, history: [] };
    snapshot(s, "created", null); store.set(s.id, s);
    return summarize(s);
  },
  async updateItem(id, token, expectedVersion, draft) {
    requireUnlocked(); await delay(150);
    const s = active(id); consumeToken(token, id, expectedVersion);
    if (s.version !== expectedVersion) err("version_conflict", "This item changed since you opened it.");
    const next = cleanDraft(draft);
    if (next.type !== s.draft.type) err("invalid_input", "An item cannot change type.");
    const prev = clone(s.draft);
    s.draft = next; s.version += 1; s.updatedAt = Date.now();
    snapshot(s, "edited", prev);
    return summarize(s);
  },
  async trashItem(id) {
    requireUnlocked(); await delay(100); const s = active(id);
    s.version += 1; s.deleted = true; s.deletedAt = Date.now(); s.updatedAt = s.deletedAt;
    snapshot(s, "trashed", s.draft);
  },
  async restoreItem(id) {
    requireUnlocked(); await delay(100); const s = need(id);
    if (!s.deleted) err("not_in_trash", "That item is not in the trash.");
    s.version += 1; s.deleted = false; s.deletedAt = null; s.updatedAt = Date.now();
    snapshot(s, "restored", s.draft);
    return summarize(s);
  },
  async listTrash() {
    requireUnlocked(); await delay(60);
    return [...store.values()].filter((s) => s.deleted).map((s) => ({ id: s.id, title: s.draft.title, deletedAt: s.deletedAt ?? s.updatedAt })).sort((a, b) => b.deletedAt - a.deletedAt);
  },
  async purgeItem(id, password, confirmation) {
    requireUnlocked(); const s = need(id);
    if (!s.deleted) err("not_in_trash", "Only items in the trash can be deleted forever.");
    await verify(password);
    const c = confirmation.trim();
    if (c !== "CONFIRM" && c !== s.draft.title.trim()) err("confirmation_mismatch", "Type the item name or CONFIRM exactly.");
    store.delete(id);
  },

  async listHistory(id) {
    requireUnlocked(); await delay(60); const s = active(id);
    return s.history.map<VersionEntry>(({ version, timestamp, event, changedFields, restoredFrom }) => ({ version, timestamp, event, changedFields, restoredFrom })).sort((a, b) => b.version - a.version);
  },
  async getVersion(id, version, password) {
    requireUnlocked(); const s = active(id); await verify(password);
    const snap = s.history.find((h) => h.version === version);
    if (!snap) err("not_found", "That version does not exist.");
    return clone(snap.draft);
  },
  async restoreVersion(id, version, password) {
    requireUnlocked(); const s = active(id); await verify(password);
    const snap = s.history.find((h) => h.version === version);
    if (!snap) err("not_found", "That version does not exist.");
    const prev = clone(s.draft);
    s.draft = clone(snap.draft); s.version += 1; s.updatedAt = Date.now();
    snapshot(s, "restoredFromVersion", prev, version);
    return summarize(s);
  },

  async totpNow(id) {
    requireUnlocked(); const s = active(id);
    const raw = s.draft.fields.totp;
    if (!raw) err("invalid_input", "This item has no one-time code.");
    return totpAt(raw, Date.now());
  },
};

export const PREVIEW_PASSWORD = "preview-vault-1";
export type { ItemType };
