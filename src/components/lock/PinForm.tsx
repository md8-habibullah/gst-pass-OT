import { useRef, useState, type FormEvent } from "react";
import { Eye, EyeOff } from "lucide-react";
import { BackendError } from "../../backend";
import { refreshProfiles, unlockWithPin } from "../../state/app";
import { Button, IconButton } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";

/**
 * The account PIN, typed where the master password normally goes. It shows a vault that is still open in memory behind the
 * PIN (after Lock, or after switching to another account), and only for as long as the lock timer allows. It opens the vault
 * limited: secrets and settings still ask for the master password. `onGone` says why the PIN is no longer an option.
 */
export function PinForm({ busyExternal, onUnlocked, onGone }: { busyExternal: boolean; onUnlocked: () => void; onGone: (why: string) => void }) {
  const [pin, setPin] = useState("");
  const [reveal, setReveal] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shake, setShake] = useState(0);
  const input = useRef<HTMLInputElement>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (busy || busyExternal || !pin) return;
    setBusy(true);
    setError(null);
    try {
      await unlockWithPin(pin);
      setPin("");
      onUnlocked();
    } catch (err) {
      setBusy(false);
      setPin("");
      if (err instanceof BackendError && (err.code === "pin_locked" || err.code === "locked" || err.code === "no_vault")) {
        // the open vault is gone (too many tries, the timer ran out, the app was closed): only the master password works now
        await refreshProfiles().catch(() => undefined);
        onGone(err.code === "pin_locked" ? "Too many wrong PINs. Use your master password." : "This account locked completely in the meantime. Use your master password.");
        return;
      }
      setShake((n) => n + 1);
      setError(err instanceof Error ? err.message : "Something went wrong.");
      input.current?.focus();
    }
  }

  return (
    <div key={shake} className={shake > 0 ? "animate-shake" : ""}>
      <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
        <Field label="PIN" error={error}>
          {({ id, describedBy, invalid }) => (
            <div className="relative">
              <TextInput id={id} ref={input} data-autofocus type={reveal ? "text" : "password"} autoComplete="off" value={pin} disabled={busy || busyExternal} invalid={invalid} aria-describedby={describedBy} onChange={(e) => setPin(e.target.value)} className="pr-11" mono={reveal} />
              <IconButton label={reveal ? "Hide PIN" : "Show PIN"} size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => setReveal((r) => !r)}>
                {reveal ? <EyeOff className="size-[17px]" /> : <Eye className="size-[17px]" />}
              </IconButton>
            </div>
          )}
        </Field>
        <Button type="submit" variant="key" block busy={busy} disabled={busy || busyExternal || !pin} className="mt-1 min-h-11">
          {busy ? "Checking" : "Open with PIN"}
        </Button>
        <p className="-mt-1 text-center text-[0.86em] text-fog">Secrets and settings will still ask for your master password.</p>
      </form>
    </div>
  );
}
