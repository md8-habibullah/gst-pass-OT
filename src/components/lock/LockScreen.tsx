import { useRef, useState, type FormEvent } from "react";
import { motion } from "motion/react";
import { ArrowLeft, Eye, EyeOff, Fingerprint, Grid3x3 } from "lucide-react";
import { BackendError } from "../../backend";
import { app, createVault, enterApp, quickUnlockAvailable, unlock, unlockWithBiometric, unlockWithPin } from "../../state/app";
import { useStore } from "../../state/store";
import { Button, IconButton } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { PinPad } from "./PinPad";
import { StrengthMeter } from "./StrengthMeter";
import { VaultMark } from "./VaultMark";

const MIN_LENGTH = 8;

/** Lock screen and first-run vault creation. Plays the unlock moment before handing over to the app. */
export function LockScreen() {
  const phase = useStore(app, (s) => s.phase);
  const setup = phase === "setup";
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [understood, setUnderstood] = useState(false);
  const [reveal, setReveal] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shake, setShake] = useState(0);
  const [opening, setOpening] = useState(false);
  const [mode, setMode] = useState<"password" | "pin">("password");
  const input = useRef<HTMLInputElement>(null);
  const quick = quickUnlockAvailable();

  const mismatch = setup && confirm.length > 0 && confirm !== password;
  const canCreate = password.length >= MIN_LENGTH && confirm === password && understood;

  /** The unlock moment: the ring completes, the card recedes, then the app rises in. */
  function finish() {
    setOpening(true);
    window.setTimeout(enterApp, 430);
  }

  function fail(e: unknown) {
    setBusy(false);
    setShake((n) => n + 1);
    if (e instanceof BackendError && e.code === "wrong_password") setError("That master password is not right.");
    else setError(e instanceof Error ? e.message : "Something went wrong.");
    setPassword("");
    input.current?.focus();
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (busy || opening) return;
    if (setup ? !canCreate : !password) return;
    setBusy(true);
    setError(null);
    try {
      await (setup ? createVault(password) : unlock(password));
      setPassword("");
      setConfirm("");
      finish();
    } catch (err) {
      fail(err);
    }
  }

  async function viaQuick(run: () => Promise<void>) {
    await run();
    finish();
  }

  return (
    <main className="grid min-h-dvh place-items-center px-4 pt-[var(--safe-top)] pb-[var(--safe-bottom)]">
      <motion.div
        className="w-full max-w-[26rem]"
        initial={{ opacity: 0, y: 14 }}
        animate={opening ? { opacity: 0, scale: 0.97, y: 0, transition: { delay: 0.24, duration: 0.2 } } : { opacity: 1, scale: 1, y: 0, transition: { duration: 0.34, ease: [0.2, 0.8, 0.2, 1] } }}
      >
        <div key={shake} className={shake > 0 ? "animate-shake" : ""}>
          <div className="glass-strong rounded-xl px-6 pt-8 pb-7 pointer-coarse:px-7">
            <div className="mb-6 flex flex-col items-center gap-4 text-center">
              <VaultMark size={68} open={opening} busy={busy} />
              <div>
                <h1 className="font-display text-[2.1rem] leading-[1.12] font-semibold tracking-[-0.01em]">{setup ? "Create your vault" : "Passman"}</h1>
                <p className="mx-auto mt-2 max-w-[22rem] text-mist">
                  {setup ? "Your master password protects everything. It never leaves this device and cannot be recovered." : mode === "pin" ? "Enter your PIN." : "Your vault is locked."}
                </p>
              </div>
            </div>

            {mode === "pin" ? (
              <div className="flex flex-col items-center gap-5">
                <PinPad
                  onSubmit={(pin) => viaQuick(() => unlockWithPin(pin))}
                  onBiometric={quick.biometric ? () => void viaQuick(unlockWithBiometric).catch((e: unknown) => setError(e instanceof Error ? e.message : "Biometrics failed.")) : undefined}
                  disabled={opening}
                />
                <Button variant="ghost" icon={<ArrowLeft className="size-4" />} onClick={() => setMode("password")}>
                  Use master password
                </Button>
              </div>
            ) : (
              <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
                <Field label="Master password" error={setup ? null : error} hint={setup ? `At least ${MIN_LENGTH} characters. Longer is better.` : undefined}>
                  {({ id, describedBy, invalid }) => (
                    <div className="relative">
                      <TextInput
                        id={id}
                        ref={input}
                        data-autofocus
                        type={reveal ? "text" : "password"}
                        autoComplete={setup ? "new-password" : "current-password"}
                        value={password}
                        disabled={busy || opening}
                        invalid={invalid}
                        aria-describedby={describedBy}
                        onChange={(e) => setPassword(e.target.value)}
                        className="pr-11"
                        mono={reveal}
                      />
                      <IconButton label={reveal ? "Hide password" : "Show password"} size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => setReveal((r) => !r)}>
                        {reveal ? <EyeOff className="size-[17px]" /> : <Eye className="size-[17px]" />}
                      </IconButton>
                    </div>
                  )}
                </Field>

                {setup && (
                  <>
                    <StrengthMeter password={password} />
                    <Field label="Confirm master password" error={mismatch ? "The passwords do not match yet." : error}>
                      {({ id, describedBy, invalid }) => (
                        <TextInput id={id} type={reveal ? "text" : "password"} autoComplete="new-password" value={confirm} disabled={busy || opening} invalid={invalid} aria-describedby={describedBy} onChange={(e) => setConfirm(e.target.value)} mono={reveal} />
                      )}
                    </Field>
                    <label className="flex cursor-pointer items-start gap-3 rounded-sm py-1 text-[0.93em] text-mist pointer-coarse:py-2">
                      <input type="checkbox" checked={understood} onChange={(e) => setUnderstood(e.target.checked)} className="mt-0.5 size-4.5 shrink-0 accent-[var(--color-brass)] pointer-coarse:size-5.5" />
                      I understand that Passman cannot recover a forgotten master password.
                    </label>
                  </>
                )}

                <Button type="submit" variant="key" block busy={busy} disabled={busy || opening || (setup ? !canCreate : !password)} className="mt-1 min-h-11">
                  {busy ? (setup ? "Creating your vault" : "Unlocking") : setup ? "Create vault" : "Unlock"}
                </Button>
                {busy && <p className="-mt-1 text-center text-[0.86em] text-fog">Deriving your key with Argon2id. This takes a moment on purpose.</p>}

                {!setup && (quick.pin || quick.biometric) && (
                  <div className="mt-1 flex flex-col gap-2 border-t border-hairline pt-4">
                    <div className="flex gap-2">
                      {quick.pin && (
                        <Button block icon={<Grid3x3 className="size-4" />} onClick={() => setMode("pin")} disabled={busy}>
                          Use PIN
                        </Button>
                      )}
                      {quick.biometric && (
                        <Button block icon={<Fingerprint className="size-4" />} disabled={busy} onClick={() => { setError(null); void viaQuick(unlockWithBiometric).catch((e: unknown) => setError(e instanceof Error ? e.message : "Biometrics failed.")); }}>
                          Biometrics
                        </Button>
                      )}
                    </div>
                  </div>
                )}
              </form>
            )}
          </div>
        </div>
        {import.meta.env.DEV && !("__TAURI_INTERNALS__" in window) && (
          <p className="mt-4 text-center text-[0.8em] text-fog">Preview backend: in-memory demo data, not secure. Password: preview-vault-1, PIN: 246810.</p>
        )}
      </motion.div>
    </main>
  );
}
