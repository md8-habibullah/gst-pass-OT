// Understands whatever a person pastes or scans into a one-time-code field and turns it into something the
// core accepts. This is only for friendly feedback ("GitHub, 6 digits, 30 s") and for splitting Google
// Authenticator exports; the Rust core validates again on save and stays the authority.

export type OtpAlgorithm = "SHA1" | "SHA256" | "SHA512";

export interface OtpAccount {
  /** What to store: an otpauth:// link or a clean base32 setup key. */
  value: string;
  issuer: string | null;
  account: string | null;
  digits: number;
  period: number;
  algorithm: OtpAlgorithm;
  /** Where the value came from, for the confirmation line. */
  origin: "link" | "key" | "web-link" | "export";
  /** The setup key is shorter than most services issue (still accepted). */
  shortKey: boolean;
}

export type OtpParse =
  | { kind: "empty" }
  | { kind: "ok"; account: OtpAccount }
  | { kind: "export"; accounts: OtpAccount[]; skipped: number }
  | { kind: "error"; message: string };

const B32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const MAX_INPUT = 8192;

/** Upper-cases and strips the separators people copy along (spaces, dashes, padding). */
export function cleanBase32(raw: string): string | null {
  const s = raw.replace(/[\s\-=]/g, "").toUpperCase();
  if (!s || ![...s].every((c) => B32.includes(c))) return null;
  return s;
}

export function toBase32(bytes: Uint8Array): string {
  let bits = 0;
  let value = 0;
  let out = "";
  for (const b of bytes) {
    value = (value << 8) | b;
    bits += 8;
    while (bits >= 5) {
      out += B32[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += B32[(value << (5 - bits)) & 31];
  return out;
}

function intParam(v: string | null, fallback: number, min: number, max: number): number | null {
  if (v === null || v === "") return fallback;
  if (!/^\d+$/.test(v)) return null;
  const n = Number(v);
  return n >= min && n <= max ? n : null;
}

function labelParts(label: string): { issuer: string | null; account: string | null } {
  let decoded = label;
  try {
    decoded = decodeURIComponent(label);
  } catch {
    /* keep the raw label */
  }
  decoded = decoded.replace(/^\/+/, "");
  const i = decoded.indexOf(":");
  if (i > 0) return { issuer: decoded.slice(0, i).trim() || null, account: decoded.slice(i + 1).trim() || null };
  return { issuer: null, account: decoded.trim() || null };
}

function parseOtpauth(link: string): OtpParse {
  let url: URL;
  try {
    url = new URL(link);
  } catch {
    return { kind: "error", message: "That otpauth link is not valid." };
  }
  const type = url.hostname.toLowerCase();
  if (type === "hotp") return { kind: "error", message: "Counter-based (HOTP) codes are not supported yet. Most services use time-based codes." };
  if (type !== "totp") return { kind: "error", message: "Only time-based one-time codes are supported." };
  if ((url.searchParams.get("encoder") ?? "").toLowerCase() === "steam") return { kind: "error", message: "Steam Guard codes are not supported yet." };
  const secret = cleanBase32(url.searchParams.get("secret") ?? "");
  if (!secret) return { kind: "error", message: url.searchParams.has("secret") ? "The secret in that link is not a valid setup key." : "That link has no secret in it." };
  const digits = intParam(url.searchParams.get("digits"), 6, 6, 8);
  if (digits === null) return { kind: "error", message: "Digits must be 6, 7 or 8." };
  const period = intParam(url.searchParams.get("period"), 30, 1, 86400);
  if (period === null) return { kind: "error", message: "The period must be between 1 and 86400 seconds." };
  const alg = (url.searchParams.get("algorithm") ?? "SHA1").toUpperCase().replace("-", "");
  if (alg !== "SHA1" && alg !== "SHA256" && alg !== "SHA512") return { kind: "error", message: "That link uses an unsupported algorithm." };
  const { issuer: fromLabel, account } = labelParts(url.pathname);
  const issuer = (url.searchParams.get("issuer") ?? fromLabel)?.trim() || null;
  return {
    kind: "ok",
    account: { value: link, issuer, account, digits, period, algorithm: alg, origin: "link", shortKey: secret.length < 16 },
  };
}

// ---- Google Authenticator "Export accounts" (otpauth-migration://offline?data=…) ----

function b64decode(s: string): Uint8Array | null {
  try {
    const norm = s.replace(/-/g, "+").replace(/_/g, "/").replace(/\s/g, "");
    const bin = atob(norm + "=".repeat((4 - (norm.length % 4)) % 4));
    return Uint8Array.from(bin, (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
}

class Reader {
  pos = 0;
  constructor(readonly buf: Uint8Array) {}
  get done() { return this.pos >= this.buf.length; }
  varint(): number {
    let result = 0;
    let shift = 0;
    for (;;) {
      if (this.pos >= this.buf.length || shift > 49) throw new Error("bad varint");
      const b = this.buf[this.pos++]!;
      result += (b & 0x7f) * 2 ** shift;
      if ((b & 0x80) === 0) return result;
      shift += 7;
    }
  }
  bytes(): Uint8Array {
    const n = this.varint();
    if (this.pos + n > this.buf.length) throw new Error("truncated");
    const out = this.buf.subarray(this.pos, this.pos + n);
    this.pos += n;
    return out;
  }
  skip(wire: number) {
    if (wire === 0) this.varint();
    else if (wire === 1) this.pos += 8;
    else if (wire === 2) this.bytes();
    else if (wire === 5) this.pos += 4;
    else throw new Error("bad wire type");
  }
}

const utf8 = new TextDecoder("utf-8", { fatal: false });

function parseMigration(link: string): OtpParse {
  let data: string | null;
  try {
    data = new URL(link).searchParams.get("data");
  } catch {
    data = null;
  }
  const bytes = data ? b64decode(data) : null;
  if (!bytes || bytes.length === 0) return { kind: "error", message: "That Google Authenticator export link is damaged or incomplete." };
  const accounts: OtpAccount[] = [];
  let skipped = 0;
  try {
    const top = new Reader(bytes);
    while (!top.done) {
      const tag = top.varint();
      if (tag >> 3 === 1 && (tag & 7) === 2) {
        const r = new Reader(top.bytes());
        let secret: Uint8Array = new Uint8Array();
        let name = "";
        let issuer = "";
        let alg = 1;
        let digitsEnum = 1;
        let type = 2;
        while (!r.done) {
          const t = r.varint();
          const f = t >> 3;
          const w = t & 7;
          if (f === 1 && w === 2) secret = r.bytes();
          else if (f === 2 && w === 2) name = utf8.decode(r.bytes());
          else if (f === 3 && w === 2) issuer = utf8.decode(r.bytes());
          else if (f === 4 && w === 0) alg = r.varint();
          else if (f === 5 && w === 0) digitsEnum = r.varint();
          else if (f === 6 && w === 0) type = r.varint();
          else r.skip(w);
        }
        const algorithm: OtpAlgorithm | null = alg === 1 || alg === 0 ? "SHA1" : alg === 2 ? "SHA256" : alg === 3 ? "SHA512" : null;
        const digits = digitsEnum === 2 ? 8 : digitsEnum === 3 ? 7 : 6;
        if (type !== 2 || !algorithm || secret.length === 0) {
          skipped++;
          continue;
        }
        const b32 = toBase32(secret);
        const label = issuer && !name.startsWith(`${issuer}:`) ? `${issuer}:${name}` : name;
        const q = new URLSearchParams({ secret: b32 });
        if (issuer) q.set("issuer", issuer);
        if (algorithm !== "SHA1") q.set("algorithm", algorithm);
        if (digits !== 6) q.set("digits", String(digits));
        accounts.push({
          value: `otpauth://totp/${encodeURIComponent(label)}?${q.toString()}`,
          issuer: issuer || null,
          account: (issuer && name.startsWith(`${issuer}:`) ? name.slice(issuer.length + 1) : name) || null,
          digits,
          period: 30,
          algorithm,
          origin: "export",
          shortKey: b32.length < 16,
        });
      } else {
        top.skip(tag & 7);
      }
    }
  } catch {
    return { kind: "error", message: "That Google Authenticator export link could not be read." };
  }
  if (accounts.length === 0) return { kind: "error", message: skipped ? "That export only contains counter-based or unsupported codes." : "That export link has no accounts in it." };
  return { kind: "export", accounts, skipped };
}

/** Parse anything a person might paste, drop in from a QR code, or type. */
export function parseOtpInput(raw: string): OtpParse {
  let s = raw.trim().replace(/^["'<`]+|["'>`]+$/g, "").trim();
  if (!s) return { kind: "empty" };
  if (s.length > MAX_INPUT) return { kind: "error", message: "That is far too long for a setup key or link." };
  const lower = s.toLowerCase();

  if (lower.startsWith("otpauth-migration://")) return parseMigration(s);
  if (lower.startsWith("otpauth://")) return parseOtpauth(s);
  if (lower.startsWith("steam://")) return { kind: "error", message: "Steam Guard codes are not supported yet." };

  // A web link that carries the secret, for example https://example.com/2fa?secret=ABCD…
  if (/^https?:\/\//i.test(s)) {
    try {
      const u = new URL(s);
      const key = u.searchParams.get("secret") ?? u.searchParams.get("key");
      const clean = key ? cleanBase32(key) : null;
      if (clean) {
        return {
          kind: "ok",
          account: { value: clean, issuer: u.searchParams.get("issuer") ?? u.hostname.replace(/^www\./, ""), account: null, digits: 6, period: 30, algorithm: "SHA1", origin: "web-link", shortKey: clean.length < 16 },
        };
      }
    } catch {
      /* fall through to the error below */
    }
    return { kind: "error", message: "That web link does not contain a setup key. Paste the key or the otpauth link instead." };
  }

  // "secret=ABCD…", "key: ABCD EFGH …" and similar copy-paste leftovers.
  s = s.replace(/^(secret|key|setup key|code)\s*[:=]\s*/i, "");
  const clean = cleanBase32(s);
  if (!clean) return { kind: "error", message: "That does not look like a setup key (letters A to Z and digits 2 to 7) or an otpauth link." };
  if (clean.length < 8) return { kind: "error", message: "That setup key is too short." };
  return { kind: "ok", account: { value: clean, issuer: null, account: null, digits: 6, period: 30, algorithm: "SHA1", origin: "key", shortKey: clean.length < 16 } };
}

/** One-line, secret-free description for the field's confirmation text. */
export function describeOtp(a: OtpAccount): string {
  const who = [a.issuer, a.account].filter(Boolean).join(", ");
  const spec = `${a.digits} digits, every ${a.period} s${a.algorithm !== "SHA1" ? `, ${a.algorithm}` : ""}`;
  return who ? `${who} (${spec})` : spec;
}
