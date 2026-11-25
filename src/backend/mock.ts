// DEV-ONLY preview backend: lets the UI be designed and screenshotted in a normal browser.
// It is NOT secure (plain in-memory data, string-compare "password check"). Never bundled in production
// builds: src/backend/index.ts loads it behind import.meta.env.DEV and only outside Tauri.
import { DISGUISES } from "../lib/disguise";
import { MAX_FILE_BYTES, MAX_FILE_LABEL, mimeFor } from "../lib/files";
import { ImportError, parseImport, type ImportResult } from "../lib/importers";
import { checkDisplayName } from "../lib/profileName";
import { joinUrls, MAX_URLS, urlsOf } from "../lib/urls";
import { DEMO_NAME, DEMO_PASSWORD, demoSeeds } from "./mockDemo";
import { cleanFolder, folderProblem, FOLDER_SEP, isInside } from "../lib/folders";
import { MockGate, makeLongEquation } from "./mockGate";
import {
  BackendError,
  LIST_SORTS,
  FIELD_SCHEMA,
  isSecretKind,
  type AppSettings,
  type Backend,
  type BackupTargetInput,
  type BackupTargetView,
  type ClockStatus,
  type DeviceInfo,
  type DisguiseInput,
  type DisguiseView,
  type FileEntry,
  type CsvInspect,
  type ImportFormat,
  type ImportOptions,
  type ImportOutcome,
  type ImportPick,
  type ImportPreview,
  type GeneratorPrefs,
  type ItemDraft,
  type PlainExportFormat,
  type ItemSummary,
  type ItemType,
  type ItemView,
  type PrefsView,
  type FolderChange,
  type PurgePlan,
  type SearchHit,
  type FieldKind,
  type ProfileInfo,
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
  otherDevice?: boolean;
}
interface Stored {
  id: string;
  createdAt: number;
  updatedAt: number;
  version: number;
  deleted: boolean;
  deletedAt: number | null;
  archived: boolean;
  draft: ItemDraft;
  history: Snapshot[];
  changedElsewhere?: boolean;
}
interface MockTarget extends Omit<BackupTargetView, "hasSecret" | "health"> {
  secretAccessKey: string;
  queued: number;
  lastSuccessAt: number | null;
  lastCheckedAt: number | null;
  lastError: string | null;
  tested: boolean;
}
interface MockProfile {
  info: ProfileInfo;
  password: string;
  store: Map<string, Stored>;
  targets: MockTarget[];
  files: Map<string, { entry: FileEntry; bytes: Uint8Array }>;
  /** The generator's last settings (the core keeps them sealed in the vault and syncs them). */
  generator?: GeneratorPrefs;
  /** Folders made on purpose (the core keeps them with the preferences and syncs them). */
  folders?: string[];
}

const delay = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const clone = <T,>(v: T): T => structuredClone(v);
const MIN = 60_000;

const profiles: MockProfile[] = [];
let selectedId: string | null = null;
let unlocked = false;
const tokens = new Map<string, { id: string; version: number; expires: number }>();

function err(code: ConstructorParameters<typeof BackendError>[0], message: string): never {
  throw new BackendError(code, message);
}
function selected(): MockProfile {
  const p = profiles.find((x) => x.info.id === selectedId);
  if (!p) err("no_vault", "Choose an account first.");
  return p;
}
function requireUnlocked(): MockProfile {
  const p = selected();
  if (!unlocked) err("locked", "The vault is locked.");
  return p;
}
const items = () => requireUnlocked().store;
async function verify(password: string, p: MockProfile = selected()) {
  await delay(650); // stands in for Argon2id, which is slow on purpose
  if (password !== p.password) err("wrong_password", "That master password is not right.");
}
/**
 * The per-item re-prompt, as the core does it: a password that is sent is always checked; without one the item must not
 * have the `reprompt` flag (otherwise `password_required`).
 */
async function gateItem(s: Stored, password: string | undefined): Promise<void> {
  if (password !== undefined) return verify(password);
  if (s.draft.reprompt) err("password_required", "Enter your master password to do this with “" + s.draft.title + "”.");
}
function need(id: string): Stored {
  const s = items().get(id);
  if (!s) err("not_found", "That item no longer exists.");
  return s;
}
function active(id: string): Stored {
  const s = need(id);
  if (s.deleted) err("in_trash", "Restore this item from the trash to open it.");
  return s;
}
/** Six months, as the core counts them (183 days). */
const PURGE_GRACE_MS = 183 * 24 * 60 * MIN;
const isRecent = (deletedAt: number) => Date.now() - deletedAt < PURGE_GRACE_MS;
/** The sum shown for one trashed item; the answer stays here, never on the screen. */
let purgeSum: { id: string; answer: number } | null = null;

const prefsOf = (p: MockProfile): PrefsView => ({ generator: p.generator ? { ...p.generator } : null, folders: [...(p.folders ?? [])] });

/** Every folder that exists: made on purpose, with items in it, or a parent of those. */
function knownFolders(p: MockProfile): string[] {
  const set = new Set<string>();
  const add = (raw: string | null | undefined) => {
    const path = cleanFolder(raw); if (!path) return;
    const parts = path.split(FOLDER_SEP);
    for (let i = 1; i <= parts.length; i++) set.add(parts.slice(0, i).join(FOLDER_SEP));
  };
  (p.folders ?? []).forEach(add);
  for (const s of p.store.values()) if (!s.deleted) add(s.draft.folder);
  return [...set].sort();
}

/** Rename `from` to `to` for the folder, everything inside it, and the empty folders made on purpose. */
function remapFolders(p: MockProfile, from: string, to: string): number {
  const map = (f: string) => (f === from ? to : to + f.slice(from.length));
  let moved = 0;
  for (const s of p.store.values()) {
    if (s.deleted || !s.draft.folder || !isInside(s.draft.folder, from)) continue;
    setFolder(s, map(s.draft.folder)); moved += 1;
  }
  p.folders = [...new Set((p.folders ?? []).map((f) => (isInside(f, from) ? map(f) : f)))].sort();
  if (moved) touchQueue();
  return moved;
}

const NOTE_KEYS = ["notes", "body", "description"];
/** Which fields of this state hold every word, or `null`. Never compares a password, a code, a card number or a hidden value. */
function matchPieces(d: ItemDraft, words: string[], reprompt = d.reprompt): string[] | null {
  const pieces: { label: string; hay: string }[] = [{ label: "Title", hay: d.title.toLowerCase() }, { label: "Type", hay: d.type.toLowerCase() }];
  if (d.folder) pieces.push({ label: "Folder", hay: d.folder.toLowerCase() });
  for (const t of d.tags) pieces.push({ label: "Tag", hay: t.toLowerCase() });
  for (const spec of FIELD_SCHEMA[d.type]) {
    const value = d.fields[spec.key];
    if (value === undefined || value === "") continue;
    const kind: FieldKind = spec.kind;
    const searchable = kind === "text" || kind === "url" || kind === "paragraph" || (kind === "note" && NOTE_KEYS.includes(spec.key) && !reprompt);
    pieces.push({ label: spec.label, hay: `${spec.label}\n${searchable ? value : ""}`.toLowerCase() });
  }
  for (const c of d.customFields) pieces.push({ label: `Custom field: ${c.name}`, hay: `${c.name}\n${c.hidden ? "" : c.value}`.toLowerCase() });
  const labels: string[] = [];
  for (const w of words) {
    const found = pieces.find((p) => p.hay.includes(w));
    if (!found) return null;
    if (labels.length < 3 && !labels.includes(found.label)) labels.push(found.label);
  }
  return labels;
}
let syncRevision = 0;
/** DEV deep link only: pretends another device saved a different version of this item (what a diverged pull leaves behind). */
export function simulateOtherDeviceEdit(id: string): void {
  const s = need(id);
  const other = clone(s.draft);
  other.title = `${other.title} (other device)`.slice(0, 256);
  const top = s.history.reduce((m, h) => Math.max(m, h.version), 0);
  s.history.push({ version: top + 1, timestamp: Date.now() - 4 * MIN, event: "edited", changedFields: ["title"], restoredFrom: null, draft: other, otherDevice: true });
  s.changedElsewhere = true;
  syncRevision += 1;
}

function touchQueue() {
  for (const t of selected().targets) if (t.enabled) t.queued += 1;
}

function hostOf(urls: string): string | null {
  const url = urlsOf(urls)[0] ?? ""; // with several addresses, the first one
  try {
    return new URL(url.includes("://") ? url : `https://${url}`).hostname.replace(/^www\./, "");
  } catch {
    return null;
  }
}
/** The import's duplicate rule (the core's): same type and title and, for logins, the same username and website, ignoring case. */
function sameEntry(a: ItemDraft, b: ItemDraft): boolean {
  const lc = (s: string | undefined) => (s ?? "").trim().toLowerCase();
  if (a.type !== b.type || lc(a.title) !== lc(b.title)) return false;
  if (a.type !== "login") return true;
  return lc(a.fields.username) === lc(b.fields.username) && lc(hostOf(a.fields.url ?? "") ?? "") === lc(hostOf(b.fields.url ?? "") ?? "");
}
function subtitleOf(d: ItemDraft): string | null {
  const f = d.fields;
  switch (d.type) {
    case "login": return f.username || (f.url ? hostOf(f.url) : null) || null;
    case "totp": return f.account || f.issuer || null;
    case "card": {
      const last4 = (f.number ?? "").replace(/\D/g, "").slice(-4);
      return last4 ? `${f.brand ? `${f.brand} ` : ""}•••• ${last4}` : f.cardholder || null;
    }
    case "identity": return [f.firstName, f.lastName].filter(Boolean).join(" ") || f.email || null;
    case "sshKey": return f.fingerprint || f.publicKey?.trim().split(/\s+/)[2] || null;
    default: return null;
  }
}
function summarize(s: Stored): ItemSummary {
  const d = s.draft;
  return {
    id: s.id,
    type: d.type,
    title: d.title,
    subtitle: subtitleOf(d),
    favorite: d.favorite,
    reprompt: d.reprompt,
    tags: d.tags,
    folder: d.folder,
    hasTotp: Boolean(d.fields.totp),
    createdAt: s.createdAt,
    updatedAt: s.updatedAt,
    version: s.version,
    archived: s.archived,
  };
}

function diff(a: ItemDraft | null, b: ItemDraft): string[] {
  if (!a) return [];
  const out: string[] = [];
  if (a.title !== b.title) out.push("title");
  if (a.favorite !== b.favorite) out.push("favorite");
  if (a.reprompt !== b.reprompt) out.push("reprompt");
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
  if (fields.url !== undefined) {
    if (urlsOf(fields.url).length > MAX_URLS) err("invalid_input", `Use at most ${MAX_URLS} web addresses.`);
    const joined = joinUrls([fields.url]);
    if (joined) fields.url = joined; else delete fields.url;
  }
  if (fields.totp) parseTotp(fields.totp); // throws invalid_input when unusable
  if (fields.number && !/^[\d\s-]{8,23}$/.test(fields.number)) err("invalid_input", "A card number has 8 to 23 digits (spaces and dashes are fine).");
  if (fields.expMonth && !(Number(fields.expMonth) >= 1 && Number(fields.expMonth) <= 12)) err("invalid_input", "The expiry month is 1 to 12.");
  if (fields.expYear && !/^\d{4}$/.test(fields.expYear)) err("invalid_input", "The expiry year has four digits.");
  return { ...d, title, fields, folder: cleanFolder(d.folder), tags: d.tags.map((t) => t.trim()).filter(Boolean) };
}

function snapshot(s: Stored, event: VersionEvent, prev: ItemDraft | null, restoredFrom: number | null = null, ts = Date.now()) {
  s.history.push({ version: s.version, timestamp: ts, event, changedFields: diff(prev, s.draft), restoredFrom, draft: clone(s.draft) });
}

/** A folder change is a version of its own, as in the core. */
function setFolder(s: Stored, folder: string | null) {
  const prev = clone(s.draft);
  s.draft.folder = folder; s.version += 1; s.updatedAt = Date.now();
  snapshot(s, "edited", prev);
}

function seedItem(minsAgo: number, draft: ItemDraft, edits: { minsAgo: number; patch: Record<string, string>; title?: string }[] = []): Stored {
  const created = Date.now() - minsAgo * MIN;
  const s: Stored = { id: crypto.randomUUID(), createdAt: created, updatedAt: created, version: 1, deleted: false, deletedAt: null, archived: false, draft: clone(draft), history: [] };
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
  type: "login", title, favorite: false, reprompt: false, tags: [], folder: null, fields, customFields: [], ...extra,
});

/** A tiny but valid PDF so the in-app PDF preview can be reviewed without a real file. */
function miniPdf(text: string): Uint8Array {
  const enc = new TextEncoder();
  const stream = `BT /F1 26 Tf 60 720 Td (${text}) Tj ET`;
  const objs = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 420 800] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
    `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  let out = "%PDF-1.4\n";
  const offsets: number[] = [];
  objs.forEach((o, i) => { offsets.push(out.length); out += `${i + 1} 0 obj\n${o}\nendobj\n`; });
  const xref = out.length;
  out += `xref\n0 ${objs.length + 1}\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, "0")} 00000 n \n`).join("")}trailer\n<< /Size ${objs.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return enc.encode(out);
}

function seedFiles(p: MockProfile) {
  p.files.clear();
  const add = (name: string, bytes: Uint8Array, minsAgo: number, itemId: string | null = null) => {
    const entry: FileEntry = { id: crypto.randomUUID(), name, size: bytes.length, mime: mimeFor(name), itemId, addedAt: Date.now() - minsAgo * MIN };
    p.files.set(entry.id, { entry, bytes });
  };
  const enc = new TextEncoder();
  add("recovery-notes.txt", enc.encode("Recovery notes\n\nSpare router login is on the label under the unit.\nBackup phone: ask Dana.\n"), 30);
  add("lakehouse-floorplan.svg", enc.encode('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 200"><rect width="320" height="200" fill="#16323d"/><rect x="30" y="30" width="150" height="140" fill="none" stroke="#e2b259" stroke-width="4"/><rect x="180" y="70" width="110" height="100" fill="none" stroke="#e2b259" stroke-width="4"/><circle cx="105" cy="100" r="22" fill="#52d3b8"/></svg>'), 60 * 5);
  add("insurance-receipt.pdf", miniPdf("Insurance receipt - paid in full"), 60 * 48);
  add("archive-2024.zip", enc.encode("PK\u0005\u0006".padEnd(22, "\0")), 60 * 24 * 9);
}

function seedInto(p: MockProfile) {
  p.store.clear();
  seedFiles(p);
  const items: Stored[] = [
    seedItem(60 * 24 * 90, login("GitHub", { username: "marlowe-dev", password: "kX9#mQ2vTz!8pLd4", url: "https://github.com/login", totp: "otpauth://totp/GitHub:marlowe-dev?secret=JBSWY3DPEHPK3PXP&issuer=GitHub&period=30&digits=6", notes: "Personal account. Recovery codes are in the Recovery codes note." }, { favorite: true, tags: ["work", "dev"] }), [
      { minsAgo: 60 * 24 * 41, patch: { password: "t7Rw-Nq4!zYb2uHc" } },
      { minsAgo: 60 * 26, patch: { password: "kX9#mQ2vTz!8pLd4", url: "https://github.com/login" } },
    ]),
    seedItem(60 * 24 * 12, login("Fastmail", { username: "marlowe@fastmail.example", password: "violet-anchor-Quench-71", url: "https://app.fastmail.com", totp: "ONSWG4TFOQYTEMZU" }, { tags: ["personal"] })),
    seedItem(60 * 5, login("Hetzner Cloud", { username: "ops@marlowe.example", password: "Hq4$vN8wLr2^TmZx", url: "https://console.hetzner.cloud", totp: "otpauth://totp/Hetzner:ops?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Hetzner&period=30&digits=6" }, { tags: ["work", "infra"] }), [
      { minsAgo: 60 * 5, patch: { notes: "Project: edge-cluster. Billing alerts go to ops@." } },
    ]),
    seedItem(60 * 24 * 3, login("Northline Bank", { username: "4417 2208", password: "7!rBv-ka39Q#sdLm", url: "https://online.northline.example", notes: "Branch PIN is not stored here." }, { favorite: true, tags: ["finance"] })),
    seedItem(60 * 24 * 21, login("Cloudflare", { username: "marlowe@fastmail.example", password: "Zp3&yLk0!vWm8RtB", url: "https://dash.cloudflare.com", totp: "otpauth://totp/Cloudflare:marlowe?secret=MFRGGZDFMZTWQ2LKNNWG23TPOBYXE43U&issuer=Cloudflare&digits=6&period=30" }, { tags: ["work", "infra"] })),
    seedItem(60 * 24 * 160, { type: "secureNote", title: "Home Wi-Fi", favorite: false, reprompt: false, tags: ["home"], folder: null, fields: { body: "Network: Linden-5G\nPassword: marble-orchard-kettle-84\nRouter admin: 192.168.1.1 (admin / see router label)" }, customFields: [] }),
    seedItem(60 * 24 * 30, { type: "secureNote", title: "Recovery codes", favorite: false, reprompt: false, tags: ["work"], folder: null, fields: { body: "GitHub\n8f2a-91cc-d04e\n3b7d-62aa-1f90\ne1c5-77b8-0a4d\n\nCloudflare\n9d31-aa20-5be7" }, customFields: [] }),
    seedItem(60 * 24 * 7, { type: "totp", title: "Google", favorite: false, reprompt: false, tags: ["personal"], folder: null, fields: { issuer: "Google", account: "marlowe@example.com", totp: "otpauth://totp/Google:marlowe@example.com?secret=KRSXG5CTMVRXEZLUKRSXG5CTMVRXEZLU&issuer=Google" }, customFields: [] }),
    seedItem(60 * 24 * 2, login("Proxmox lab", { username: "root@pam", password: "Cx8^pQe2!tVn7MsH", url: "https://pve.lab.example:8006" }, { tags: ["infra"], customFields: [{ name: "Node", value: "pve-02", hidden: false }, { name: "API token", value: "pve!ci=3f8a1c2e-77d4-4b1a-9e0c-5d2b8f6a1e44", hidden: true }] })),
    seedItem(60 * 24 * 55, { type: "media", title: "Passport scan", favorite: false, reprompt: false, tags: ["documents"], folder: null, fields: { description: "Front page, scanned 2025. Encrypted files can be attached once media support ships." }, customFields: [] }),
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
  const extra: Stored[] = [
    seedItem(60 * 24 * 6, { type: "card", title: "Northline Visa", favorite: false, reprompt: false, tags: ["finance"], folder: null, fields: { cardholder: "M. Marlowe", brand: "Visa", number: "4111 1111 1111 4242", expMonth: "08", expYear: "2029", code: "913", pin: "4821" }, customFields: [] }),
    seedItem(60 * 24 * 40, { type: "identity", title: "Personal details", favorite: false, reprompt: false, tags: ["home"], folder: null, fields: { salutation: "Mx", firstName: "Marlowe", lastName: "Dean", email: "marlowe@example.com", phone: "+44 20 7946 0000", address1: "14 Linden Row", city: "Bristol", postalCode: "BS1 4DJ", country: "United Kingdom", passport: "563201948" }, customFields: [] }),
    seedItem(60 * 24 * 9, { type: "sshKey", title: "Deploy key (edge-cluster)", favorite: false, reprompt: false, tags: ["infra"], folder: null, fields: { publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ3x9bK2deploy marlowe@edge", privateKey: "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n-----END OPENSSH PRIVATE KEY-----", fingerprint: "SHA256:Qm3vN8wLr2TmZxHq4vN8wLr2TmZxHq4vN8w" }, customFields: [] }),
  ];
  const archived = seedItem(60 * 24 * 300, login("Old Slack workspace", { username: "marlowe", password: "sl4ck-0ld-pass!", url: "https://oldteam.slack.example" }));
  archived.archived = true;
  extra.push(archived);
  for (const s of [...items, ...extra]) p.store.set(s.id, s);
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
  const k = await crypto.subtle.importKey("raw", key.buffer as ArrayBuffer, { name: "HMAC", hash: alg }, false, ["sign"]);
  const at = async (c: number) => {
    const msg = new ArrayBuffer(8);
    new DataView(msg).setBigUint64(0, BigInt(Math.max(0, c)));
    const h = new Uint8Array(await crypto.subtle.sign("HMAC", k, msg));
    const o = h[h.length - 1]! & 0x0f;
    const bin = ((h[o]! & 0x7f) << 24) | (h[o + 1]! << 16) | (h[o + 2]! << 8) | h[o + 3]!;
    return String(bin % 10 ** digits).padStart(digits, "0");
  };
  const [code, previous, next] = [await at(counter), await at(counter - 1), await at(counter + 1)];
  const validUntilMs = (counter + 1) * period * 1000;
  return { code, previous, next, period, digits, validUntilMs, secondsRemaining: Math.max(0, Math.ceil((validUntilMs - nowMs) / 1000)) };
}

function view(s: Stored): ItemView {
  const d = s.draft;
  return {
    id: s.id, type: d.type, title: d.title, favorite: d.favorite, reprompt: d.reprompt, tags: d.tags, folder: d.folder,
    fields: FIELD_SCHEMA[d.type]
      .filter((spec) => d.fields[spec.key])
      .map((spec) => {
        const secret = isSecretKind(spec.kind);
        return { key: spec.key, label: spec.label, kind: spec.kind, secret, value: secret ? null : (d.fields[spec.key] ?? null), hasValue: Boolean(d.fields[spec.key]) };
      }),
    customFields: d.customFields.map((c) => ({ name: c.name, hidden: c.hidden, value: c.hidden ? null : c.value })),
    attachments: [], createdAt: s.createdAt, updatedAt: s.updatedAt, version: s.version,
    changedElsewhere: s.changedElsewhere === true,
  };
}

function consumeToken(token: string, id: string, expectedVersion: number) {
  const t = tokens.get(token);
  tokens.delete(token);
  if (!t || t.id !== id || t.expires < Date.now()) err("token_invalid", "Edit session expired. Open the item for editing again.");
  if (t.version !== expectedVersion) err("version_conflict", "This item changed since you opened it.");
}

// ---- device settings and the demo vault (same rules as the core; see storage/device.rs and storage/demo.rs) ----
const FRESH_DEVICE: DeviceInfo = { timeZone: null, setupDone: false, timeCheck: true, theme: "system", clipboardClear: 30, openInSearch: false, listSort: "updated-desc" };
let device: DeviceInfo = { ...FRESH_DEVICE };
let demoId: string | null = null;
const profileInfo = (p: MockProfile): ProfileInfo => ({ ...p.info, ...(p.info.id === demoId ? { demo: true } : {}) });
function seedDemo(p: MockProfile) {
  p.store.clear();
  for (const d of demoSeeds()) {
    const s = seedItem(d.minsAgo, d.draft);
    if (d.editedMinsAgo !== undefined && d.editedMinsAgo < d.minsAgo) {
      const prev = clone(s.draft);
      s.version += 1; s.updatedAt = Date.now() - d.editedMinsAgo * MIN;
      snapshot(s, "edited", prev, null, s.updatedAt);
    }
    p.store.set(s.id, s);
  }
}

function toView(t: MockTarget): BackupTargetView {
  const { secretAccessKey, queued, lastSuccessAt, lastCheckedAt, lastError, tested, ...rest } = t;
  const lagging = queued > 0 && lastSuccessAt !== null && Date.now() - lastSuccessAt > 5 * MIN;
  const state = !t.enabled ? "disabled" : lastError ? "failing" : !tested && lastSuccessAt === null ? "untested" : lagging ? "lagging" : "healthy";
  return { ...rest, hasSecret: secretAccessKey.length > 0, health: { state, queued, lastSuccessAt, lastCheckedAt, lastError, conditionalWrites: tested ? "strong" : "unknown" } };
}
function checkTargetInput(i: BackupTargetInput, keepSecret = false) {
  if (!i.name.trim()) err("invalid_input", "Give this backup a name.");
  if (!i.bucket.trim()) err("invalid_input", "Enter the bucket name.");
  if (!i.accessKeyId.trim()) err("invalid_input", "Enter the access key ID.");
  if (!keepSecret && !i.secretAccessKey) err("invalid_input", "Enter the secret access key.");
  if (i.endpoint && !/^https:\/\/|^http:\/\/(localhost|127\.0\.0\.1)/i.test(i.endpoint.trim())) err("invalid_input", "The endpoint must start with https://.");
}

/** Preview-only stand-in for the core's native picker. The input stays in the page so a test can feed it files. */
function pickFilesInBrowser(): Promise<File[] | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file"; input.multiple = true; input.hidden = true; input.dataset.mockPicker = "1";
    input.addEventListener("change", () => { const files = [...(input.files ?? [])]; input.remove(); resolve(files.length ? files : null); });
    input.addEventListener("cancel", () => { input.remove(); resolve(null); });
    document.body.append(input);
    input.click();
  });
}

const imports = new Map<string, { result: ImportResult | null; name: string; text: string }>();
function importFor(o: ImportOptions): { result: ImportResult } {
  const f = imports.get(o.handle);
  if (!f) err("not_found", "That file is no longer open. Choose it again.");
  if (o.format === "keepass-kdbx") err("invalid_input", "KeePass files are read by the real app only (this preview cannot open them).");
  try { return { result: f.result ?? parseImport(f.text) }; } catch (e) { return err("unsupported_file", e instanceof ImportError ? e.message : "That file could not be read."); }
}

function clockView(): ClockStatus {
  return { verified: device.timeCheck, enabled: device.timeCheck, skewMs: device.timeCheck ? 120 : null, checkedAgoMs: device.timeCheck ? 4_000 : null, sources: device.timeCheck ? 3 : 0, problem: null, nowMs: Date.now() };
}

export const gate = new MockGate();
gate.onWipe = () => { profiles.length = 0; selectedId = null; unlocked = false; tokens.clear(); demoId = null; };
let appSettings: AppSettings = { lockAfterMinutes: 5, lockAfterLeaveSeconds: 300 };
// "After leaving the app": the preview follows the same rule as the core (the vault and, when on, the gate lock when the time away runs out).
let leaveTimer: ReturnType<typeof setTimeout> | undefined;
let leaveDeadline = 0;
function lockForAway() { leaveDeadline = 0; unlocked = false; tokens.clear(); gate.close(); }
let disguise: DisguiseInput = { presetId: null, customName: null, lowercase: false };
function disguiseView(): DisguiseView {
  const preset = DISGUISES.find((d) => d.id === disguise.presetId);
  const base = disguise.customName ?? preset?.name ?? "Hexis";
  return { ...disguise, appliedName: disguise.lowercase ? base.toLowerCase() : base, note: "Preview only: nothing outside this page changes." };
}

export const previewBackend: Backend = {
  kind: "preview",
  async suiteInfo() {
    return { version: 1, kem: "ML-KEM-1024+X25519", kdf: "Argon2id", aead: "XChaCha20-Poly1305", kdfDefaults: { mCostKiB: 65536, tCost: 4, pCost: 4 }, nonceBytes: 24, keyBytes: 32 };
  },
  async status() {
    await delay(120);
    const p = profiles.find((x) => x.info.id === selectedId) ?? null;
    return { exists: p !== null, unlocked: p !== null && unlocked, profile: p ? profileInfo(p) : null, syncRevision, reconciling: false };
  },

  async listProfiles() {
    await delay(60);
    const sorted = [...profiles].sort((a, b) => (b.info.lastUsedAt ?? b.info.createdAt) - (a.info.lastUsedAt ?? a.info.createdAt));
    return { profiles: sorted.map(profileInfo), selectedId, lastUsedId: sorted.find((p) => p.info.lastUsedAt !== null)?.info.id ?? null };
  },
  async createProfile(displayName, password) {
    const check = checkDisplayName(displayName, profiles.map((p) => p.info));
    if (!check.ok) err(check.message.startsWith("You already") ? "profile_name_taken" : "invalid_input", check.message);
    if (password.length < 8) err("invalid_input", "Use at least 8 characters.");
    await delay(900);
    const p: MockProfile = { info: { id: crypto.randomUUID(), displayName: check.name, createdAt: Date.now(), lastUsedAt: Date.now() }, password, store: new Map(), targets: [], files: new Map() };
    profiles.push(p);
    selectedId = p.info.id; unlocked = true; tokens.clear();
    return profileInfo(p);
  },
  async selectProfile(id) {
    const p = profiles.find((x) => x.info.id === id);
    if (!p) err("not_found", "That account no longer exists on this device.");
    if (selectedId !== id) { unlocked = false; tokens.clear(); }
    selectedId = id;
  },
  async renameProfile(id, displayName) {
    const p = requireUnlocked();
    if (p.info.id !== id) err("locked", "Unlock this account to rename it.");
    const check = checkDisplayName(displayName, profiles.map((x) => x.info), id);
    if (!check.ok) err(check.message.startsWith("You already") ? "profile_name_taken" : "invalid_input", check.message);
    p.info.displayName = check.name;
    return profileInfo(p);
  },
  async removeProfile(id, password, confirmation) {
    const p = profiles.find((x) => x.info.id === id);
    if (!p) err("not_found", "That account no longer exists on this device.");
    await verify(password, p);
    const c = confirmation.trim();
    if (c !== "CONFIRM" && c !== p.info.displayName) err("confirmation_mismatch", "Type the account name or CONFIRM exactly.");
    profiles.splice(profiles.indexOf(p), 1);
    if (demoId === id) demoId = null;
    if (selectedId === id) { selectedId = null; unlocked = false; tokens.clear(); }
  },
  async importProfileFile(displayName) {
    const check = checkDisplayName(displayName, profiles.map((p) => p.info));
    if (!check.ok) err(check.message.startsWith("You already") ? "profile_name_taken" : "invalid_input", check.message);
    await delay(400);
    const p: MockProfile = { info: { id: crypto.randomUUID(), displayName: check.name, createdAt: Date.now(), lastUsedAt: null }, password: PREVIEW_PASSWORD, store: new Map(), targets: [], files: new Map() };
    seedInto(p);
    profiles.push(p);
    selectedId = p.info.id; unlocked = false;
    return profileInfo(p);
  },
  async restoreProfileFromBackup(displayName, input) {
    checkTargetInput(input);
    const check = checkDisplayName(displayName, profiles.map((p) => p.info));
    if (!check.ok) err(check.message.startsWith("You already") ? "profile_name_taken" : "invalid_input", check.message);
    await delay(1200);
    if (/fail|bad/i.test(input.bucket)) err("remote", "The bucket answered “Access denied”. Check the access key and the bucket name.");
    const p: MockProfile = { info: { id: crypto.randomUUID(), displayName: check.name, createdAt: Date.now(), lastUsedAt: null }, password: PREVIEW_PASSWORD, store: new Map(), targets: [], files: new Map() };
    seedInto(p);
    profiles.push(p);
    selectedId = p.info.id; unlocked = false;
    return profileInfo(p);
  },

  async unlock(password) {
    const p = selected();
    await verify(password, p);
    unlocked = true;
    p.info.lastUsedAt = Date.now();
  },
  async lock() { unlocked = false; tokens.clear(); },
  async changePassword(oldPassword, newPassword) {
    const p = requireUnlocked(); await verify(oldPassword, p);
    if (newPassword.length < 8) err("invalid_input", "Use at least 8 characters.");
    p.password = newPassword;
  },
  async exportVaultFile(password) {
    const p = requireUnlocked(); await verify(password, p);
    await delay(300);
    return { path: `~/Documents/${p.info.displayName}-${new Date().toISOString().slice(0, 10)}.hexis` };
  },

  async listItems() {
    await delay(60);
    return [...items().values()].filter((s) => !s.deleted).map(summarize).sort((a, b) => b.updatedAt - a.updatedAt);
  },
  async getItem(id) { items(); await delay(50); return view(active(id)); },
  async revealItem(id, password) { const s = active(id); await gateItem(s, password); return clone(s.draft); },
  async beginEdit(id, password) {
    const s = active(id); await gateItem(s, password);
    const token = crypto.randomUUID();
    tokens.set(token, { id, version: s.version, expires: Date.now() + 10 * MIN });
    return { token, draft: clone(s.draft), version: s.version };
  },
  async createItem(draft) {
    const store = items(); await delay(150);
    const d = cleanDraft(draft); const now = Date.now();
    const s: Stored = { id: crypto.randomUUID(), createdAt: now, updatedAt: now, version: 1, deleted: false, deletedAt: null, archived: false, draft: d, history: [] };
    snapshot(s, "created", null); store.set(s.id, s); touchQueue();
    return summarize(s);
  },
  async updateItem(id, token, expectedVersion, draft) {
    items(); await delay(150);
    const s = active(id); consumeToken(token, id, expectedVersion);
    if (s.version !== expectedVersion) err("version_conflict", "This item changed since you opened it.");
    const next = cleanDraft(draft);
    if (next.type !== s.draft.type) err("invalid_input", "An item cannot change type.");
    const prev = clone(s.draft);
    s.draft = next; s.version += 1; s.updatedAt = Date.now();
    snapshot(s, "edited", prev); touchQueue();
    return summarize(s);
  },
  async trashItem(id) {
    items(); await delay(100); const s = active(id);
    s.version += 1; s.deleted = true; s.deletedAt = Date.now(); s.updatedAt = s.deletedAt;
    snapshot(s, "trashed", s.draft); touchQueue();
  },
  async restoreItem(id) {
    items(); await delay(100); const s = need(id);
    if (!s.deleted) err("not_in_trash", "That item is not in the trash.");
    s.version += 1; s.deleted = false; s.deletedAt = null; s.updatedAt = Date.now();
    snapshot(s, "restored", s.draft); touchQueue();
    return summarize(s);
  },
  async listTrash() {
    await delay(60);
    return [...items().values()].filter((s) => s.deleted).map((s) => ({ id: s.id, title: s.draft.title, deletedAt: s.deletedAt ?? s.updatedAt })).sort((a, b) => b.deletedAt - a.deletedAt);
  },
  async purgePlan(id) {
    const s = need(id); await delay(60);
    if (!s.deleted) err("not_in_trash", "Only items in the trash can be deleted forever.");
    const deletedAt = s.deletedAt ?? s.updatedAt;
    const plan: PurgePlan = { deletedAt, recent: isRecent(deletedAt), olderAt: deletedAt + PURGE_GRACE_MS, sum: null };
    purgeSum = null;
    if (plan.recent) {
      const eq = makeLongEquation();
      purgeSum = { id, answer: eq.answer };
      plan.sum = eq.text;
    }
    return plan;
  },
  async purgeItem(id, password, answer) {
    const store = items(); const s = need(id);
    if (!s.deleted) err("not_in_trash", "Only items in the trash can be deleted forever.");
    await verify(password);
    // the sum is checked after the password and used up by any attempt, as in the core
    if (isRecent(s.deletedAt ?? s.updatedAt)) {
      const sum = purgeSum; purgeSum = null;
      const typed = (answer ?? "").trim();
      if (!sum || sum.id !== id || !/^\d{1,9}$/.test(typed) || Number(typed) !== sum.answer) err("wrong_answer", "That is not the answer. Try the new sum.");
    }
    store.delete(id); touchQueue();
  },
  // ---- search and folders, with the core's rules (storage/vault_search.rs, vault_folders.rs) ----
  async searchItems(query, oldVersions): Promise<SearchHit[]> {
    const store = items(); await delay(80);
    if ([...query].length > 200) err("invalid_input", "That search is too long.");
    const words = query.toLowerCase().split(/\s+/).filter(Boolean).slice(0, 8);
    if (words.length === 0) return [];
    const hits: SearchHit[] = [];
    for (const s of store.values()) {
      if (s.deleted) continue;
      const now = matchPieces(s.draft, words);
      if (now) { hits.push({ id: s.id, matched: now, oldVersion: null }); continue; }
      if (!oldVersions) continue;
      for (const h of [...s.history].sort((a, b) => b.version - a.version)) {
        if (h.version === s.version) continue;
        const old = matchPieces(h.draft, words, s.draft.reprompt);
        if (old) { hits.push({ id: s.id, matched: old, oldVersion: h.version }); break; }
      }
    }
    return hits;
  },
  async createFolder(path): Promise<FolderChange> {
    const p = requireUnlocked(); await delay(40);
    const problem = folderProblem(path); if (problem) err("invalid_input", problem);
    const clean = cleanFolder(path) as string;
    const known = knownFolders(p);
    const spelled = known.find((f) => f.toLowerCase() === clean.toLowerCase()) ?? clean;
    p.folders = [...new Set([...(p.folders ?? []), spelled])].sort();
    return { path: spelled, moved: 0, prefs: prefsOf(p) };
  },
  async renameFolder(from, to): Promise<FolderChange> {
    const p = requireUnlocked(); await delay(60);
    const a = cleanFolder(from); const b = cleanFolder(to);
    if (!a) err("invalid_input", "Choose a folder.");
    if (!b) err("invalid_input", "Give the folder a name.");
    const problem = folderProblem(b); if (problem) err("invalid_input", problem);
    const known = knownFolders(p);
    if (!known.includes(a)) err("not_found", "That folder no longer exists.");
    if (a === b) return { path: null, moved: 0, prefs: prefsOf(p) };
    if (isInside(b, a) && b !== a && b.toLowerCase() !== a.toLowerCase()) err("invalid_input", "A folder cannot be moved inside itself.");
    const target = known.find((f) => f.toLowerCase() === b.toLowerCase()) ?? b;
    return { path: null, moved: remapFolders(p, a, target), prefs: prefsOf(p) };
  },
  async removeFolder(path): Promise<FolderChange> {
    const p = requireUnlocked(); await delay(60);
    const a = cleanFolder(path); if (!a) err("invalid_input", "Choose a folder.");
    if (!knownFolders(p).includes(a)) err("not_found", "That folder no longer exists.");
    const at = a.lastIndexOf(FOLDER_SEP);
    const parent = at < 0 ? null : a.slice(0, at);
    // what was inside moves up one level: `a/x` becomes `parent/x`, and `a` itself goes
    let moved = 0;
    for (const s of p.store.values()) {
      if (s.deleted || !s.draft.folder || !isInside(s.draft.folder, a)) continue;
      const rest = s.draft.folder === a ? "" : s.draft.folder.slice(a.length + 1);
      const next = [parent, rest].filter(Boolean).join(FOLDER_SEP) || null;
      setFolder(s, next); moved += 1;
    }
    p.folders = (p.folders ?? []).flatMap((f) => (f === a ? [] : isInside(f, a) ? [[parent, f.slice(a.length + 1)].filter(Boolean).join(FOLDER_SEP)] : [f]));
    if (moved) touchQueue();
    return { path: null, moved, prefs: prefsOf(p) };
  },
  async moveItems(ids, folder) {
    const store = items(); await delay(60);
    const target = cleanFolder(folder);
    const found = ids.map((id) => { const s = store.get(id); if (!s) err("not_found", "That item no longer exists."); if (s.deleted) err("in_trash", "Restore this item from the trash to open it."); return s; });
    let n = 0;
    for (const s of found) {
      if ((s.draft.folder ?? null) === target) continue;
      setFolder(s, target); n += 1;
    }
    if (n) touchQueue();
    return n;
  },
  async archiveItem(id) {
    items(); await delay(80); const s = active(id);
    if (!s.archived) { s.archived = true; s.version += 1; s.updatedAt = Date.now(); snapshot(s, "archived", s.draft); touchQueue(); }
    return summarize(s);
  },
  async unarchiveItem(id) {
    items(); await delay(80); const s = active(id);
    if (s.archived) { s.archived = false; s.version += 1; s.updatedAt = Date.now(); snapshot(s, "unarchived", s.draft); touchQueue(); }
    return summarize(s);
  },
  async cloneItem(id, password) {
    const store = items(); const src = active(id); await gateItem(src, password);
    const now = Date.now();
    const d = clone(src.draft); d.title = `${d.title} (copy)`.slice(0, 256);
    const s: Stored = { id: crypto.randomUUID(), createdAt: now, updatedAt: now, version: 1, deleted: false, deletedAt: null, archived: false, draft: d, history: [] };
    snapshot(s, "created", null); store.set(s.id, s); touchQueue();
    return summarize(s);
  },

  async listHistory(id) {
    items(); await delay(60); const s = active(id);
    return s.history.map<VersionEntry>(({ version, timestamp, event, changedFields, restoredFrom, otherDevice }) => ({ version, timestamp, event, changedFields, restoredFrom, otherDevice: otherDevice === true })).sort((a, b) => b.timestamp - a.timestamp || b.version - a.version);
  },
  async reviewOtherDevice(id) { const s = active(id); s.changedElsewhere = false; },
  async syncFlush() { return { complete: true }; },
  async syncReconnect() { /* the preview has no network */ },
  async screenSecure() { /* nothing to block in a browser preview */ },
  async getVersion(id, version, password) {
    const s = active(id); await gateItem(s, password);
    const snap = s.history.find((h) => h.version === version);
    if (!snap) err("not_found", "That version does not exist.");
    return clone(snap.draft);
  },
  async restoreVersion(id, version, password) {
    const s = active(id); await gateItem(s, password);
    const snap = s.history.find((h) => h.version === version);
    if (!snap) err("not_found", "That version does not exist.");
    const prev = clone(s.draft);
    s.draft = clone(snap.draft); s.version = Math.max(s.version, ...s.history.map((h) => h.version)) + 1; s.updatedAt = Date.now();
    s.changedElsewhere = false;
    snapshot(s, "restoredFromVersion", prev, version); touchQueue();
    return summarize(s);
  },

  async totpNow(id, password) {
    items(); const s = active(id); await gateItem(s, password);
    const raw = s.draft.fields.totp;
    if (!raw) err("invalid_input", "This item has no one-time code.");
    return totpAt(raw, Date.now());
  },

  async listBackups() {
    const p = requireUnlocked(); await delay(80);
    return [...p.targets].sort((a, b) => Number(b.master) - Number(a.master)).map(toView);
  },
  async testBackup(input) {
    checkTargetInput(input, true);
    await delay(1100);
    if (/fail|bad/i.test(input.bucket) || /fail/i.test(input.endpoint ?? "")) return { ok: false, message: "The bucket answered “Access denied” (403). Check the access key, the secret and the bucket name.", conditionalWrites: "unknown" };
    return { ok: true, message: "Connected. Wrote, read and removed a test object.", conditionalWrites: "strong" };
  },
  async addBackup(input) {
    const p = requireUnlocked(); checkTargetInput(input);
    if (p.targets.some((t) => t.name.trim().toLowerCase() === input.name.trim().toLowerCase())) err("invalid_input", "You already have a backup with that name.");
    await delay(300);
    const master = input.master || p.targets.length === 0;
    if (master) p.targets.forEach((t) => (t.master = false));
    const t: MockTarget = { id: crypto.randomUUID(), name: input.name.trim(), endpoint: input.endpoint?.trim() || null, region: input.region.trim() || "auto", bucket: input.bucket.trim(), prefix: input.prefix.trim(), accessKeyId: input.accessKeyId.trim(), secretAccessKey: input.secretAccessKey, pathStyle: input.pathStyle, enabled: input.enabled, master, shareInMaster: input.shareInMaster, queued: [...p.store.values()].length, lastSuccessAt: null, lastCheckedAt: null, lastError: null, tested: false };
    p.targets.push(t);
    return toView(t);
  },
  async updateBackup(id, input) {
    const p = requireUnlocked(); checkTargetInput(input, true);
    const t = p.targets.find((x) => x.id === id);
    if (!t) err("not_found", "That backup no longer exists.");
    await delay(250);
    if (input.master) p.targets.forEach((x) => (x.master = false));
    Object.assign(t, { name: input.name.trim(), endpoint: input.endpoint?.trim() || null, region: input.region.trim() || "auto", bucket: input.bucket.trim(), prefix: input.prefix.trim(), accessKeyId: input.accessKeyId.trim(), pathStyle: input.pathStyle, enabled: input.enabled, master: input.master || t.master, shareInMaster: input.shareInMaster });
    if (input.secretAccessKey) t.secretAccessKey = input.secretAccessKey;
    return toView(t);
  },
  async removeBackup(id) {
    const p = requireUnlocked(); await delay(150);
    const i = p.targets.findIndex((x) => x.id === id);
    if (i < 0) err("not_found", "That backup no longer exists.");
    const wasMaster = p.targets[i]!.master;
    p.targets.splice(i, 1);
    if (wasMaster && p.targets[0]) p.targets[0].master = true;
  },
  async setMasterBackup(id) {
    const p = requireUnlocked();
    if (!p.targets.some((t) => t.id === id)) err("not_found", "That backup no longer exists.");
    p.targets.forEach((t) => (t.master = t.id === id));
  },
  async runBackup(id) {
    const p = requireUnlocked(); await delay(1400);
    for (const t of p.targets) {
      if (id !== null && t.id !== id) continue;
      if (!t.enabled) continue;
      t.lastCheckedAt = Date.now();
      if (/fail|bad/i.test(t.bucket)) { t.lastError = "Access denied (403) while uploading."; continue; }
      t.lastError = null; t.tested = true; t.queued = 0; t.lastSuccessAt = Date.now();
    }
    return p.targets.map(toView);
  },
  async revealBackupSecret(id, password) {
    const p = requireUnlocked(); await verify(password, p);
    const t = p.targets.find((x) => x.id === id);
    if (!t) err("not_found", "That backup no longer exists.");
    return t.secretAccessKey;
  },

  // ---- import and export (preview: the old browser-side parsers stand in for the core's Rust importer) ----
  async importPick() {
    const [file] = (await pickFilesInBrowser()) ?? [];
    if (!file) return null;
    const head = new Uint8Array(await file.slice(0, 4).arrayBuffer());
    const kdbx = head[0] === 0x03 && head[1] === 0xd9 && head[2] === 0xa2 && head[3] === 0x9a;
    const handle = crypto.randomUUID();
    let format: ImportFormat | "unknown" = "unknown";
    let result: ImportResult | null = null;
    if (kdbx) format = "keepass-kdbx";
    else {
      try { result = parseImport(await file.text()); format = result.format === "Bitwarden JSON" ? "bitwarden-json" : "chrome-csv"; } catch { /* unknown */ }
    }
    imports.set(handle, { result, name: file.name, text: kdbx ? "" : await file.text() });
    const pick: ImportPick = { handle, fileName: file.name, size: file.size, format, needsPassword: kdbx, needsKeyfile: false };
    return pick;
  },
  async importPickKeyfile() {
    const [file] = (await pickFilesInBrowser()) ?? [];
    return file ? { handle: crypto.randomUUID(), fileName: file.name } : null;
  },
  async inspectCsvHeaders(handle) {
    const f = imports.get(handle);
    if (!f) err("not_found", "That file is no longer open. Choose it again.");
    const rows = f.text.split(/\r?\n/).filter(Boolean).map((l) => l.split(","));
    const out: CsvInspect = { delimiter: ",", headers: rows[0] ?? [], sample: rows.slice(1, 6) };
    return out;
  },
  async importPreview(options) {
    const { result } = importFor(options);
    const p = requireUnlocked();
    const existing = [...p.store.values()].filter((x) => !x.deleted);
    const dup = (d: ItemDraft) => existing.some((x) => sameEntry(x.draft, d));
    const preview: ImportPreview = {
      entries: result.drafts.length, totpSeeds: result.drafts.filter((d) => d.fields.totp).length, attachments: 0,
      duplicates: result.drafts.filter(dup).length, skipped: result.skipped.length, warnings: result.warnings, titles: result.drafts.slice(0, 50).map((d) => d.title),
    };
    return preview;
  },
  async importCommit(options) {
    const { result } = importFor(options);
    const p = requireUnlocked();
    const out: ImportOutcome = { added: 0, appended: 0, skipped: 0, failed: 0, warnings: [] };
    for (const d of result.drafts) {
      const match = [...p.store.values()].find((x) => !x.deleted && sameEntry(x.draft, d));
      if (match && options.mode === "skip_duplicates") { out.skipped++; continue; }
      if (match && options.mode === "append_version") {
        const prev = clone(match.draft); match.draft = cleanDraft(d); match.version += 1; match.updatedAt = Date.now();
        snapshot(match, "edited", prev); out.appended++; continue;
      }
      await previewBackend.createItem(d); out.added++;
    }
    out.skipped += result.skipped.length;
    imports.delete(options.handle); touchQueue();
    return out;
  },
  async importCancel(handle) { imports.delete(handle); },
  async exportPlain(format: PlainExportFormat, password) {
    const p = requireUnlocked(); await verify(password, p);
    const rows = [...p.store.values()].filter((x) => !x.deleted);
    const text = format === "csv"
      ? ["name,url,username,password,totp,notes", ...rows.map((x) => [x.draft.title, x.draft.fields.url, x.draft.fields.username, x.draft.fields.password, x.draft.fields.totp, x.draft.fields.notes].map((v) => `"${(v ?? "").replace(/"/g, '""')}"`).join(","))].join("\n")
      : JSON.stringify({ encrypted: false, folders: [], items: rows.map((x) => ({ type: 1, name: x.draft.title, login: { username: x.draft.fields.username, password: x.draft.fields.password } })) }, null, 2);
    const name = format === "csv" ? "vault-export.csv" : "vault-export.json";
    const url = URL.createObjectURL(new Blob([text]));
    const a = document.createElement("a"); a.href = url; a.download = name; a.click();
    window.setTimeout(() => URL.revokeObjectURL(url), 2000);
    return { path: name, warnings: [] };
  },

  // ---- encrypted drive (preview: plain bytes in memory; the real core streams 1 MiB sealed chunks) ----
  async fileList(itemId) {
    const p = requireUnlocked();
    return [...p.files.values()].map((f) => f.entry).filter((e) => e.itemId === itemId).sort((a, b) => b.addedAt - a.addedAt).map(clone);
  },
  async fileAdd(itemId) {
    const p = requireUnlocked();
    if (itemId !== null) need(itemId);
    const picked = await pickFilesInBrowser();
    if (picked === null) return null;
    const tooBig = picked.find((f) => f.size > MAX_FILE_BYTES);
    if (tooBig) err("invalid_input", `${tooBig.name} is larger than ${MAX_FILE_LABEL}.`);
    const added: FileEntry[] = [];
    for (const f of picked) {
      const entry: FileEntry = { id: crypto.randomUUID(), name: f.name, size: f.size, mime: f.type || mimeFor(f.name), itemId, addedAt: Date.now() };
      p.files.set(entry.id, { entry, bytes: new Uint8Array(await f.arrayBuffer()) });
      added.push(clone(entry));
    }
    return added;
  },
  async fileRename(id, name) {
    const p = requireUnlocked(); const f = p.files.get(id);
    if (!f) err("not_found", "That file no longer exists.");
    if (!name.trim() || name.length > 200) err("invalid_input", "Use 1 to 200 characters for the file name.");
    f.entry.name = name.trim();
    return clone(f.entry);
  },
  async fileDelete(id, password) {
    const p = requireUnlocked(); await verify(password, p);
    if (!p.files.delete(id)) err("not_found", "That file no longer exists.");
  },
  async fileRead(id) {
    const f = requireUnlocked().files.get(id);
    if (!f) err("not_found", "That file no longer exists.");
    return f.bytes.slice().buffer;
  },
  async fileSaveCopy(id, password) {
    const p = requireUnlocked(); await verify(password, p);
    const f = p.files.get(id);
    if (!f) err("not_found", "That file no longer exists.");
    const url = URL.createObjectURL(new Blob([f.bytes.slice().buffer]));
    const a = document.createElement("a"); a.href = url; a.download = f.entry.name; a.click();
    window.setTimeout(() => URL.revokeObjectURL(url), 2000);
    return { path: f.entry.name };
  },

  // ---- app-level preview of the gate, inactivity lock and disguise (same rules as the core; see mockGate.ts) ----
  async gateStatus() { return gate.status(); },
  async gateSubmit(input) { await delay(160); return gate.submit(input); },
  async gateBiometric() { await delay(500); return gate.biometric(); },
  async gatePanic() { try { return gate.panic(); } catch (e) { return err("invalid_input", e instanceof Error ? e.message : "Not available."); } },
  async gateClose() { gate.close(); unlocked = false; tokens.clear(); },
  async gateConfigure(input) {
    const p = requireUnlocked(); await verify(input.masterPassword, p);
    try { return gate.configure(input); } catch (e) { return err("invalid_input", e instanceof Error ? e.message : "Could not save the app lock."); }
  },
  async appResetChallenge() { return gate.resetChallenge(); },
  async appReset(answer) { if (!gate.reset(answer)) err("wrong_answer", "That is not the answer. Try the new sum."); },
  async appPrepareUninstall(answer) {
    if (!gate.reset(answer)) err("wrong_answer", "That is not the answer. Try the new sum.");
    // Nothing is made again, not even the device file: the real core removes the data folder and the app closes.
    device = { ...FRESH_DEVICE };
  },
  async appSettingsGet() { return { ...appSettings }; },
  async appSettingsSet(input) { appSettings = { ...input }; return { ...appSettings }; },
  async activityPing() { /* the preview never times out */ },
  async appLeave() {
    clearTimeout(leaveTimer);
    const ms = appSettings.lockAfterLeaveSeconds * 1000;
    if (ms === 0) { lockForAway(); return; }
    leaveDeadline = Date.now() + ms;
    leaveTimer = setTimeout(lockForAway, ms);
  },
  async appReturn() {
    clearTimeout(leaveTimer);
    const ranOut = leaveDeadline !== 0 && Date.now() >= leaveDeadline;
    leaveDeadline = 0;
    if (ranOut) lockForAway();
  },
  // ---- the vault's preferences: the same limits as the core (storage/backup/prefs.rs) ----
  async prefsGet(): Promise<PrefsView> {
    return prefsOf(requireUnlocked());
  },
  async prefsSetGenerator(g): Promise<PrefsView> {
    const p = requireUnlocked();
    if (!Number.isInteger(g.length) || g.length < 4 || g.length > 128) err("invalid_input", "The password length must be between 4 and 128.");
    if (!(g.lower || g.upper || g.digits || g.symbols)) err("invalid_input", "Select at least one character type.");
    p.generator = { length: g.length, lower: g.lower, upper: g.upper, digits: g.digits, symbols: g.symbols, avoidAmbiguous: g.avoidAmbiguous };
    return prefsOf(p);
  },
  // ---- device settings, the clock and the demo vault ----
  async deviceGet() { return { ...device }; },
  async deviceSetTimeZone(zone) {
    try { new Intl.DateTimeFormat("en", { timeZone: zone.trim() }); } catch { err("invalid_input", "Choose a time zone from the list, for example Asia/Dhaka or Europe/Berlin."); }
    device = { ...device, timeZone: zone.trim() };
    return { ...device };
  },
  async deviceFinishSetup() { device = { ...device, setupDone: true }; return { ...device }; },
  async deviceSetTimeCheck(on) { device = { ...device, timeCheck: on }; return { ...device }; },
  async deviceSetTheme(theme) {
    if (theme !== "system" && theme !== "light" && theme !== "dark") err("invalid_input", "Choose light, dark or system.");
    device = { ...device, theme };
    return { ...device };
  },
  async deviceSetClipboardClear(seconds) {
    if (seconds !== 15 && seconds !== 30 && seconds !== 60) err("invalid_input", "Choose 15, 30 or 60 seconds.");
    device = { ...device, clipboardClear: seconds };
    return { ...device };
  },
  async deviceSetOpenInSearch(on) { device = { ...device, openInSearch: on === true }; return { ...device }; },
  async deviceSetListSort(order) {
    if (!(LIST_SORTS as readonly string[]).includes(order)) err("invalid_input", "Choose one of the orders in the list.");
    device = { ...device, listSort: order };
    return { ...device };
  },
  async clockStatus() { return clockView(); },
  async clockSync() {
    if (!device.timeCheck) err("invalid_input", "The online time check is switched off. Switch it on first.");
    await delay(700);
    return clockView();
  },
  async demoSpawn() {
    if (profiles.length > 0) err("invalid_input", "A vault already exists on this device.");
    await delay(500);
    const p: MockProfile = { info: { id: crypto.randomUUID(), displayName: DEMO_NAME, createdAt: Date.now(), lastUsedAt: Date.now() }, password: DEMO_PASSWORD, store: new Map(), targets: [], files: new Map() };
    seedDemo(p);
    profiles.push(p);
    demoId = p.info.id;
    selectedId = p.info.id; unlocked = true; tokens.clear();
    return profileInfo(p);
  },
  async demoOpen() {
    const p = profiles.find((x) => x.info.id === demoId);
    if (!p) err("not_found", "That account no longer exists on this device.");
    if (selectedId !== p.info.id) tokens.clear();
    selectedId = p.info.id; unlocked = true;
    p.info.lastUsedAt = Date.now();
  },

  async disguiseGet() { return disguiseView(); },
  async disguiseSet(input) {
    const name = input.customName?.trim() ?? "";
    if (input.customName !== null && (name.length === 0 || name.length > 30)) err("invalid_input", "Use 1 to 30 characters for the name.");
    if (input.presetId !== null && !DISGUISES.some((d) => d.id === input.presetId)) err("invalid_input", "That icon is not one of the choices.");
    disguise = { presetId: input.presetId, customName: input.customName === null ? null : name, lowercase: input.lowercase };
    return disguiseView();
  },

  async openUrl(url) {
    if (!/^https?:\/\//i.test(url)) err("invalid_input", "Only web links can be opened.");
    window.open(url, "_blank", "noopener,noreferrer");
  },
};

/** DEV deep links: create a profile that already holds the demo items (and a second, empty one for the picker). */
export async function seedPreview(): Promise<void> {
  if (profiles.length > 0) return;
  device = { ...device, setupDone: true, timeZone: device.timeZone ?? "Europe/London" };
  const a: MockProfile = { info: { id: crypto.randomUUID(), displayName: "Marlowe", createdAt: Date.now() - 40 * 86_400_000, lastUsedAt: Date.now() - 3_600_000 }, password: PREVIEW_PASSWORD, store: new Map(), targets: [], files: new Map() };
  seedInto(a);
  a.targets.push(
    { id: crypto.randomUUID(), name: "Cloudflare R2", endpoint: "https://0123abcd.r2.cloudflarestorage.example", region: "auto", bucket: "hexis-main", prefix: "marlowe/", accessKeyId: "AKIAPREVIEWR2KEY", secretAccessKey: "preview-secret-r2", pathStyle: false, enabled: true, master: true, shareInMaster: true, queued: 0, lastSuccessAt: Date.now() - 2 * MIN, lastCheckedAt: Date.now() - 2 * MIN, lastError: null, tested: true },
    { id: crypto.randomUUID(), name: "Home MinIO", endpoint: "https://minio.home.example", region: "us-east-1", bucket: "vault-mirror", prefix: "", accessKeyId: "minio-marlowe", secretAccessKey: "preview-secret-minio", pathStyle: true, enabled: true, master: false, shareInMaster: false, queued: 3, lastSuccessAt: Date.now() - 49 * MIN, lastCheckedAt: Date.now() - 6 * MIN, lastError: null, tested: true },
    { id: crypto.randomUUID(), name: "Backblaze B2", endpoint: "https://s3.us-west-004.backblazeb2.example", region: "us-west-004", bucket: "hexis-offsite", prefix: "", accessKeyId: "004preview", secretAccessKey: "preview-secret-b2", pathStyle: false, enabled: true, master: false, shareInMaster: true, queued: 9, lastSuccessAt: Date.now() - 3 * 3_600_000, lastCheckedAt: Date.now() - 5 * MIN, lastError: "Access denied (403) while uploading.", tested: true },
  );
  const b: MockProfile = { info: { id: crypto.randomUUID(), displayName: "Atlas Registry (work)", createdAt: Date.now() - 12 * 86_400_000, lastUsedAt: Date.now() - 2 * 86_400_000 }, password: PREVIEW_PASSWORD, store: new Map(), targets: [], files: new Map() };
  const c: MockProfile = { info: { id: crypto.randomUUID(), displayName: "Family", createdAt: Date.now() - 80 * 86_400_000, lastUsedAt: null }, password: PREVIEW_PASSWORD, store: new Map(), targets: [], files: new Map() };
  profiles.push(a, b, c);
  selectedId = a.info.id;
}

export const PREVIEW_PASSWORD = "preview-vault-1";
export type { ItemType };
