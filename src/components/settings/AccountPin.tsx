import { useState, type FormEvent } from "react";
import { KeyRound } from "lucide-react";
import { BackendError, PIN_MAX, PIN_MIN } from "../../backend";
import { app, pushToast, removeAccountPin, setAccountPin } from "../../state/app";
import { useStore } from "../../state/store";
import { reprompt } from "../reprompt/Reprompt";
import { Button } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";
import { Row } from "./parts";

/** Why this PIN cannot be used yet, or `null`. The core checks the same rules. */
export function pinProblem(pin: string): string | null {
  const n = [...pin].length;
  if (n < PIN_MIN) return `Use at least ${PIN_MIN} characters.`;
  if (n > PIN_MAX) return `Use at most ${PIN_MAX} characters.`;
  return null;
}

function PinSheet({ open, onClose, changing }: { open: boolean; onClose: () => void; changing: boolean }) {
  const [password, setPassword] = useState("");
  const [pin, setPin] = useState("");
  const [again, setAgain] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const reset = () => { setPassword(""); setPin(""); setAgain(""); setError(null); setBusy(false); };
  const close = () => { if (busy) return; reset(); onClose(); };
  const problem = pin ? pinProblem(pin) : null;
  const valid = password.length > 0 && pin.length > 0 && !problem && pin === again;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!valid || busy) return;
    setBusy(true); setError(null);
    try {
      await setAccountPin(password, pin);
      reset(); onClose();
      pushToast({ kind: "success", text: changing ? "PIN changed." : "PIN set. After Lock, or when you switch accounts, it opens this account again until the lock timer ends." });
    } catch (err) {
      setBusy(false);
      setError(err instanceof BackendError && err.code === "wrong_password" ? "That is not your master password." : err instanceof Error ? err.message : "Could not save the PIN.");
    }
  }

  return (
    <Sheet open={open} onClose={close} title={changing ? "Change PIN" : "Set a PIN"} variant="dialog" size="sm" dismissible={!busy} titleIcon={<KeyRound className="size-5 text-brass-ink" aria-hidden />}>
      <form onSubmit={submit} className="flex flex-col gap-4 pt-1">
        <Field label="Master password" error={error}>
          {({ id, describedBy, invalid }) => <TextInput id={id} data-autofocus type="password" autoComplete="current-password" value={password} invalid={invalid} aria-describedby={describedBy} disabled={busy} onChange={(e) => setPassword(e.target.value)} />}
        </Field>
        <Field label="New PIN" hint={`${PIN_MIN} to ${PIN_MAX} characters. A short word or number you can type quickly.`} error={problem}>
          {({ id, describedBy, invalid }) => <TextInput id={id} type="password" autoComplete="off" value={pin} invalid={invalid} aria-describedby={describedBy} disabled={busy} onChange={(e) => setPin(e.target.value)} />}
        </Field>
        <Field label="Repeat the PIN" error={again && again !== pin ? "The PINs do not match yet." : null}>
          {({ id, describedBy, invalid }) => <TextInput id={id} type="password" autoComplete="off" value={again} invalid={invalid} aria-describedby={describedBy} disabled={busy} onChange={(e) => setAgain(e.target.value)} />}
        </Field>
        <p className="text-[0.86em] text-fog">The PIN never unlocks Hexis after it was closed, and it never replaces the master password. It only brings back a vault that is still open on this device.</p>
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={close} disabled={busy}>Cancel</Button>
          <Button type="submit" variant="key" busy={busy} disabled={!valid}>{changing ? "Change PIN" : "Set PIN"}</Button>
        </div>
      </form>
    </Sheet>
  );
}

/** The selected account's quick-unlock PIN (middle layer: app lock, then PIN, then master password). */
export function AccountPinRow() {
  const profile = useStore(app, (s) => s.profile);
  const [sheet, setSheet] = useState(false);
  if (!profile || profile.demo) return null;
  const has = profile.hasPin === true;

  async function remove() {
    const done = await reprompt<boolean>({
      reason: `Remove the PIN of ${profile?.displayName ?? "this account"}.`,
      because: "After that, only the master password opens this account again.",
      confirmLabel: "Remove PIN",
      tone: "danger",
      run: async (pw) => { await removeAccountPin(pw); return true; },
    });
    if (done) pushToast({ kind: "info", text: "PIN removed." });
  }

  return (
    <>
      <Row
        title="Account PIN"
        description={has
          ? "Opens this account again after Lock or after you switch accounts, until the lock timer ends. Secrets, exports and backup settings still ask for the master password."
          : "A short code that reopens this account after Lock or after you switch accounts, until the lock timer ends. Without one, those need the master password."}
      >
        <div className="flex flex-wrap gap-2">
          <Button icon={<KeyRound className="size-4" />} onClick={() => setSheet(true)}>{has ? "Change" : "Set PIN"}</Button>
          {has && <Button variant="ghost" onClick={() => void remove()}>Remove</Button>}
        </div>
      </Row>
      <PinSheet open={sheet} onClose={() => setSheet(false)} changing={has} />
    </>
  );
}
