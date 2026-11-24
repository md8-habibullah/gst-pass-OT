// Display-name rules for accounts. The Rust core enforces the same rules; this mirror gives instant feedback
// and powers the preview backend. Names are labels, not secrets and not login ids.

export const MAX_NAME_CHARS = 40;

export type NameCheck = { ok: true; name: string; key: string } | { ok: false; message: string };

/** NFKC, trimmed, whitespace runs collapsed, 1 to 40 characters, no control characters. */
export function checkDisplayName(raw: string, existing: { id: string; displayName: string }[] = [], selfId?: string): NameCheck {
  const name = raw.normalize("NFKC").replace(/\s+/gu, " ").trim();
  if (!name) return { ok: false, message: "Give this account a name." };
  if ([...name].length > MAX_NAME_CHARS) return { ok: false, message: `Use ${MAX_NAME_CHARS} characters or fewer.` };
  if (/[\p{Cc}\p{Cf}]/u.test(name)) return { ok: false, message: "That name contains characters that cannot be shown." };
  const key = name.toLowerCase();
  const clash = existing.find((p) => p.id !== selfId && p.displayName.normalize("NFKC").replace(/\s+/gu, " ").trim().toLowerCase() === key);
  if (clash) return { ok: false, message: `You already have an account called “${clash.displayName}”. Names must be different on this device.` };
  return { ok: true, name, key };
}
