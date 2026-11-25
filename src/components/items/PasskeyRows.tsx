import { Fingerprint } from "lucide-react";
import type { PasskeyInfo } from "../../backend";
import { exactTime } from "../../lib/format";
import { instanceKey } from "../../backend";
import { type Fields, PASSKEY, removeSlot, rowsOf, slotsOf } from "../../lib/instances";
import type { EditFields } from "./CredentialRows";

/** What the editor holds in place of a passkey: its public facts and `keep: true`. The core puts the real passkey back on save. */
export function keepMarker(text: string | undefined): PasskeyInfo | null {
  if (!text) return null;
  try {
    const v = JSON.parse(text) as Partial<PasskeyInfo> & { keep?: boolean };
    return v && v.keep === true && typeof v.credentialId === "string" && typeof v.rpId === "string" ? (v as PasskeyInfo) : null;
  } catch {
    return null;
  }
}

/** The one line a passkey gets everywhere: the website it belongs to, and the account name the website knows it by. */
export function PasskeyLine({ info }: { info: PasskeyInfo }) {
  const who = info.userName || info.displayName;
  return (
    <span className="flex min-w-0 items-start gap-3">
      <span className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-md bg-pane text-brass-ink shadow-e1" aria-hidden>
        <Fingerprint className="size-[18px]" />
      </span>
      <span className="min-w-0">
        <span className="block truncate font-medium">{info.rpName && info.rpName !== info.rpId ? `${info.rpName} (${info.rpId})` : info.rpId}</span>
        <span className="block truncate text-[0.93em] text-mist">{who ? `${who}, made ${exactTime(info.createdAt)}` : `Made ${exactTime(info.createdAt)}`}</span>
      </span>
    </span>
  );
}

/**
 * A login's passkeys in the editor. A passkey is made by a website (through the passkey provider or the scanner) and holds a
 * private key the screen never sees, so it cannot be typed or changed here: it can only be taken out.
 */
export function PasskeyRows({ fields, edit }: { fields: Fields; edit: EditFields }) {
  if (!slotsOf(fields, PASSKEY).length) return null;
  const rows = rowsOf(fields, PASSKEY).map((n) => ({ n, info: keepMarker(fields[instanceKey("passkey", n)]) }));
  return (
    <div role="group" aria-label="Passkeys" className="flex flex-col gap-3">
      <p className="text-[0.93em] font-medium text-mist">Passkeys</p>
      {rows.map(({ n, info }, i) => (
        <div key={n} className="flex items-center gap-3 rounded-lg bg-pane/70 p-3 shadow-e1">
          <div className="min-w-0 flex-1">{info ? <PasskeyLine info={info} /> : <span className="text-mist">A passkey</span>}</div>
          <button
            type="button"
            aria-label={`Remove passkey ${i + 1}${info ? ` for ${info.rpId}` : ""}`}
            onClick={() => edit((f) => removeSlot(f, PASSKEY, n))}
            className="pressable shrink-0 rounded-sm px-1.5 py-1 text-[0.86em] font-medium text-mist hover:text-ember pointer-coarse:min-h-10"
          >
            Remove
          </button>
        </div>
      ))}
      <p className="text-[0.86em] text-fog">Removing a passkey here does not remove it from the website. Do that on the website too.</p>
    </div>
  );
}
