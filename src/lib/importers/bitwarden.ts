import { parseOtpInput } from "../otp";
import { blankDraft, ImportError, put, type ImportResult } from "./types";

interface BwField { name?: string | null; value?: string | null; type?: number }
interface BwItem {
  type?: number;
  name?: string | null;
  notes?: string | null;
  favorite?: boolean;
  reprompt?: number;
  folderId?: string | null;
  deletedDate?: string | null;
  fields?: BwField[] | null;
  login?: { uris?: { uri?: string | null }[] | null; username?: string | null; password?: string | null; totp?: string | null } | null;
  card?: { cardholderName?: string | null; brand?: string | null; number?: string | null; expMonth?: string | null; expYear?: string | null; code?: string | null } | null;
  identity?: Record<string, string | null | undefined> | null;
  sshKey?: { privateKey?: string | null; publicKey?: string | null; keyFingerprint?: string | null } | null;
}
interface BwExport { encrypted?: boolean; folders?: { id: string; name: string }[]; items?: BwItem[] }

export function looksLikeBitwarden(text: string): boolean {
  const t = text.trimStart();
  return t.startsWith("{") && /"items"\s*:/.test(t.slice(0, 200_000));
}

/** Maps a Bitwarden unencrypted JSON export to item drafts. Nothing is dropped silently. */
export function parseBitwarden(text: string): ImportResult {
  let data: BwExport;
  try {
    data = JSON.parse(text) as BwExport;
  } catch {
    throw new ImportError("That file is not valid JSON.");
  }
  if (data.encrypted) throw new ImportError("This Bitwarden export is password-protected. Export it again as an unencrypted .json file, import it here, then delete the file.");
  if (!Array.isArray(data.items)) throw new ImportError("This does not look like a Bitwarden export (no items found).");

  const folders = new Map((data.folders ?? []).map((f) => [f.id, f.name]));
  const out: ImportResult = { format: "Bitwarden JSON", drafts: [], skipped: [], warnings: [] };

  for (const it of data.items) {
    const name = (it.name ?? "").trim() || "(no name)";
    if (it.deletedDate) { out.skipped.push({ name, reason: "It was in Bitwarden's trash." }); continue; }
    const extra: { name: string; value: string; hidden: boolean }[] = [];
    let draft;

    switch (it.type) {
      case 1: {
        draft = blankDraft("login", name);
        const l = it.login ?? {};
        put(draft.fields, "username", l.username);
        put(draft.fields, "password", l.password);
        const uris = (l.uris ?? []).map((u) => u.uri?.trim() ?? "").filter(Boolean);
        put(draft.fields, "url", uris[0]);
        uris.slice(1).forEach((u, i) => extra.push({ name: `Website ${i + 2}`, value: u, hidden: false }));
        if (l.totp?.trim()) {
          const r = parseOtpInput(l.totp);
          if (r.kind === "ok") draft.fields.totp = r.account.value;
          else {
            extra.push({ name: "One-time code (not imported)", value: l.totp, hidden: true });
            out.warnings.push(`“${name}”: its one-time code could not be read${r.kind === "error" ? ` (${r.message})` : ""}. It was kept in a hidden custom field.`);
          }
        }
        put(draft.fields, "notes", it.notes);
        break;
      }
      case 2:
        draft = blankDraft("secureNote", name);
        put(draft.fields, "body", it.notes);
        break;
      case 3: {
        draft = blankDraft("card", name);
        const c = it.card ?? {};
        put(draft.fields, "cardholder", c.cardholderName);
        put(draft.fields, "brand", c.brand);
        put(draft.fields, "number", c.number);
        put(draft.fields, "expMonth", c.expMonth);
        put(draft.fields, "expYear", c.expYear);
        put(draft.fields, "code", c.code);
        put(draft.fields, "notes", it.notes);
        break;
      }
      case 4: {
        draft = blankDraft("identity", name);
        const i = it.identity ?? {};
        put(draft.fields, "salutation", i.title);
        put(draft.fields, "firstName", i.firstName);
        put(draft.fields, "middleName", i.middleName);
        put(draft.fields, "lastName", i.lastName);
        put(draft.fields, "email", i.email);
        put(draft.fields, "phone", i.phone);
        put(draft.fields, "company", i.company);
        put(draft.fields, "address1", i.address1);
        put(draft.fields, "address2", [i.address2, i.address3].filter((x) => x && x.trim()).join(", "));
        put(draft.fields, "city", i.city);
        put(draft.fields, "state", i.state);
        put(draft.fields, "postalCode", i.postalCode);
        put(draft.fields, "country", i.country);
        put(draft.fields, "nationalId", i.ssn);
        put(draft.fields, "passport", i.passportNumber);
        put(draft.fields, "license", i.licenseNumber);
        if (i.username?.trim()) extra.push({ name: "Username", value: i.username, hidden: false });
        put(draft.fields, "notes", it.notes);
        break;
      }
      case 5: {
        draft = blankDraft("sshKey", name);
        const k = it.sshKey ?? {};
        put(draft.fields, "publicKey", k.publicKey);
        put(draft.fields, "privateKey", k.privateKey);
        put(draft.fields, "fingerprint", k.keyFingerprint);
        put(draft.fields, "notes", it.notes);
        break;
      }
      default:
        out.skipped.push({ name, reason: `Unknown item type ${String(it.type)}.` });
        continue;
    }

    for (const f of it.fields ?? []) {
      const n = (f.name ?? "").trim();
      if (!n) continue;
      extra.push({ name: n.slice(0, 64), value: f.type === 2 ? String(f.value === "true") : (f.value ?? ""), hidden: f.type === 1 });
    }
    draft.customFields = extra.slice(0, 50);
    if (extra.length > 50) out.warnings.push(`“${name}” has more than 50 custom fields; the extra ones were left out.`);
    draft.favorite = Boolean(it.favorite);
    draft.reprompt = it.reprompt === 1;
    const folder = it.folderId ? folders.get(it.folderId) : null;
    draft.folder = folder ? folder.slice(0, 64) : null;
    out.drafts.push(draft);
  }
  return out;
}
