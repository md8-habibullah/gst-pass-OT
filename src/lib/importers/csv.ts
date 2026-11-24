import { parseOtpInput } from "../otp";
import { blankDraft, ImportError, put, type ImportResult } from "./types";

/** RFC 4180 parser: quotes, doubled quotes, CRLF and newlines inside quoted cells. */
export function parseCsvRows(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let quoted = false;
  const s = text.replace(/^﻿/, "");
  for (let i = 0; i < s.length; i++) {
    const c = s[i]!;
    if (quoted) {
      if (c === '"') {
        if (s[i + 1] === '"') { cell += '"'; i++; } else quoted = false;
      } else cell += c;
    } else if (c === '"' && cell === "") quoted = true;
    else if (c === ",") { row.push(cell); cell = ""; }
    else if (c === "\n" || c === "\r") {
      if (c === "\r" && s[i + 1] === "\n") i++;
      row.push(cell); cell = "";
      if (row.some((x) => x !== "")) rows.push(row);
      row = [];
    } else cell += c;
  }
  if (quoted) throw new ImportError("That CSV has an unclosed quote, so it may be cut off.");
  row.push(cell);
  if (row.some((x) => x !== "")) rows.push(row);
  return rows;
}

const ALIASES: Record<string, string[]> = {
  name: ["name", "title", "login_name", "account", "site"],
  url: ["url", "login_uri", "website", "web site", "uri", "origin", "hostname"],
  username: ["username", "login_username", "user", "login", "email", "user name"],
  password: ["password", "login_password", "pass"],
  notes: ["notes", "note", "extra", "comments", "comment"],
  totp: ["totp", "login_totp", "otpauth", "otp", "one-time password", "2fa"],
  folder: ["folder", "grouping", "group", "category"],
  favorite: ["favorite", "fav"],
};

export function looksLikeCsv(text: string): boolean {
  const first = text.replace(/^﻿/, "").split(/\r?\n/, 1)[0]?.toLowerCase() ?? "";
  return first.includes(",") && ["password", "login_password"].some((h) => first.split(",").map((x) => x.replace(/"/g, "").trim()).includes(h));
}

/** Browser and password-manager CSV exports (Chrome, Edge, Firefox, Bitwarden, 1Password, LastPass, KeePass). */
export function parseCsv(text: string): ImportResult {
  const rows = parseCsvRows(text);
  if (rows.length < 2) throw new ImportError("That CSV has no rows to import.");
  const header = rows[0]!.map((h) => h.trim().toLowerCase());
  const col = (key: keyof typeof ALIASES) => header.findIndex((h) => ALIASES[key]!.includes(h));
  const idx = { name: col("name"), url: col("url"), username: col("username"), password: col("password"), notes: col("notes"), totp: col("totp"), folder: col("folder"), favorite: col("favorite") };
  if (idx.password < 0 || (idx.username < 0 && idx.url < 0 && idx.name < 0)) throw new ImportError("This CSV needs at least a password column and one of name, url or username.");
  const typeCol = header.indexOf("type");

  const out: ImportResult = { format: "CSV", drafts: [], skipped: [], warnings: [] };
  const cell = (r: string[], i: number) => (i >= 0 ? (r[i] ?? "") : "");
  rows.slice(1).forEach((r, n) => {
    const url = cell(r, idx.url).trim();
    const host = (() => { try { return new URL(url.includes("://") ? url : `https://${url}`).hostname.replace(/^www\./, ""); } catch { return ""; } })();
    const name = cell(r, idx.name).trim() || host || cell(r, idx.username).trim() || `Row ${n + 2}`;
    if (typeCol >= 0 && /note/i.test(cell(r, typeCol)) && !cell(r, idx.password)) {
      const d = blankDraft("secureNote", name);
      put(d.fields, "body", cell(r, idx.notes));
      out.drafts.push(d);
      return;
    }
    if (!cell(r, idx.password) && !cell(r, idx.username) && !url) { out.skipped.push({ name, reason: "The row has no username, password or website." }); return; }
    const d = blankDraft("login", name);
    put(d.fields, "username", cell(r, idx.username));
    put(d.fields, "password", cell(r, idx.password));
    put(d.fields, "url", url);
    put(d.fields, "notes", cell(r, idx.notes));
    const totp = cell(r, idx.totp).trim();
    if (totp) {
      const p = parseOtpInput(totp);
      if (p.kind === "ok") d.fields.totp = p.account.value;
      else {
        d.customFields.push({ name: "One-time code (not imported)", value: totp, hidden: true });
        out.warnings.push(`“${name}”: its one-time code could not be read. It was kept in a hidden custom field.`);
      }
    }
    const folder = cell(r, idx.folder).trim();
    d.folder = folder ? folder.slice(0, 64) : null;
    d.favorite = /^(1|true|yes)$/i.test(cell(r, idx.favorite).trim());
    out.drafts.push(d);
  });
  return out;
}
