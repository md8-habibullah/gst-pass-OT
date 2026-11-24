import { useState, type FormEvent } from "react";
import { ArrowLeft, Cloud, Eye, EyeOff, SkipForward } from "lucide-react";
import { BackendError, type BackupTargetInput } from "../../backend";
import { be, createProfile, pushToast } from "../../state/app";
import { TargetForm } from "../backup/TargetForm";
import { Button, IconButton } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { NameField, useNameCheck } from "./NameField";
import { StrengthMeter } from "./StrengthMeter";

const MIN_LENGTH = 8;

/** Step 1 of creating an account: name and master password. The vault is created and unlocked on submit. */
export function CreateFlow({ onBack, onCreated, onFinish }: { onBack: () => void; onCreated: () => void; onFinish: () => void }) {
  const [step, setStep] = useState<"details" | "backup" | "backupForm">("details");

  return step === "details" ? (
    <Details onBack={onBack} onCreated={() => { onCreated(); setStep("backup"); }} />
  ) : step === "backup" ? (
    <BackupChoice onSkip={onFinish} onSetup={() => setStep("backupForm")} />
  ) : (
    <BackupSetup onBack={() => setStep("backup")} onDone={onFinish} />
  );
}

function Details({ onBack, onCreated }: { onBack: () => void; onCreated: () => void }) {
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [understood, setUnderstood] = useState(false);
  const [reveal, setReveal] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const check = useNameCheck(name);
  const mismatch = confirm.length > 0 && confirm !== password;
  const ready = check.ok && password.length >= MIN_LENGTH && confirm === password && understood;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!ready || busy || !check.ok) return;
    setBusy(true);
    setError(null);
    try {
      await createProfile(check.name, password);
      setPassword(""); setConfirm("");
      onCreated();
    } catch (err) {
      setBusy(false);
      setError(err instanceof BackendError || err instanceof Error ? err.message : "Could not create the vault.");
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
      <button type="button" onClick={onBack} disabled={busy} className="pressable -mt-1 mb-1 inline-flex min-h-9 items-center gap-1.5 self-start rounded-sm text-[0.93em] text-mist hover:text-frost">
        <ArrowLeft className="size-4" aria-hidden /> Back
      </button>
      {error && <p role="alert" className="rounded-lg border border-ember/40 bg-danger/10 px-4 py-3 text-[0.95em]">{error}</p>}
      <NameField value={name} onChange={setName} disabled={busy} autoFocus />
      <Field label="Master password" hint={`At least ${MIN_LENGTH} characters. Longer is better.`}>
        {({ id, describedBy }) => (
          <div className="relative">
            <TextInput id={id} type={reveal ? "text" : "password"} autoComplete="new-password" value={password} disabled={busy} aria-describedby={describedBy} onChange={(e) => setPassword(e.target.value)} className="pr-11" mono={reveal} />
            <IconButton label={reveal ? "Hide password" : "Show password"} size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => setReveal((r) => !r)}>
              {reveal ? <EyeOff className="size-[17px]" /> : <Eye className="size-[17px]" />}
            </IconButton>
          </div>
        )}
      </Field>
      <StrengthMeter password={password} />
      <Field label="Confirm master password" error={mismatch ? "The passwords do not match yet." : null}>
        {({ id, describedBy, invalid }) => <TextInput id={id} type={reveal ? "text" : "password"} autoComplete="new-password" value={confirm} disabled={busy} invalid={invalid} aria-describedby={describedBy} onChange={(e) => setConfirm(e.target.value)} mono={reveal} />}
      </Field>
      <label className="flex cursor-pointer items-start gap-3 rounded-sm py-1 text-[0.93em] text-mist pointer-coarse:py-2">
        <input type="checkbox" checked={understood} onChange={(e) => setUnderstood(e.target.checked)} className="mt-0.5 size-4.5 shrink-0 accent-[var(--color-brass)] pointer-coarse:size-5.5" />
        I understand that Passman cannot recover a forgotten master password.
      </label>
      <Button type="submit" variant="key" block busy={busy} disabled={!ready} className="mt-1 min-h-11">
        {busy ? "Creating your vault" : "Create vault"}
      </Button>
      {busy && <p className="-mt-1 text-center text-[0.86em] text-fog">Deriving your key with Argon2id. This takes a moment on purpose.</p>}
    </form>
  );
}

function BackupChoice({ onSkip, onSetup }: { onSkip: () => void; onSetup: () => void }) {
  return (
    <div className="flex flex-col gap-4">
      <div>
        <h2 className="font-display text-[1.45rem] leading-tight font-semibold">Back up this vault?</h2>
        <p className="mt-2 text-mist">A backup copies your encrypted vault to a bucket you own, so losing this device does not lose your passwords. It works with Amazon S3, Cloudflare R2, Backblaze B2, MinIO and other S3-compatible services.</p>
      </div>
      <Button variant="key" block className="min-h-11" icon={<Cloud className="size-4" />} onClick={onSetup}>Set up a backup</Button>
      <Button block className="min-h-11" icon={<SkipForward className="size-4" />} onClick={onSkip}>Skip for now</Button>
      <p className="text-center text-[0.86em] text-fog">You can add backups any time from Backup in the sidebar.</p>
    </div>
  );
}

function BackupSetup({ onBack, onDone }: { onBack: () => void; onDone: () => void }) {
  async function save(input: BackupTargetInput) {
    const t = await be().addBackup({ ...input, master: true });
    pushToast({ kind: "success", text: `“${t.name}” is now your master backup.` });
    void be().runBackup(t.id).catch(() => undefined);
    onDone();
  }
  return (
    <div className="flex flex-col gap-4">
      <div>
        <h2 className="font-display text-[1.45rem] leading-tight font-semibold">Add your first backup</h2>
        <p className="mt-2 text-[0.93em] text-mist">This becomes your master backup. The keys are stored sealed inside your vault, so you never have to enter them again on this device.</p>
      </div>
      <TargetForm mode="add" submitLabel="Save and back up" defaultMaster onSubmit={save} onCancel={onBack} cancelLabel="Back" />
      <Button variant="ghost" onClick={onDone}>Skip for now</Button>
    </div>
  );
}
