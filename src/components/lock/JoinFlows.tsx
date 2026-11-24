import { useState, type FormEvent } from "react";
import { ArrowLeft, FolderOpen } from "lucide-react";
import { BackendError, type BackupTargetInput } from "../../backend";
import { importProfileFile, pushToast, restoreFromBackup } from "../../state/app";
import { TargetForm } from "../backup/TargetForm";
import { Button } from "../ui/Button";
import { NameField, useNameCheck } from "./NameField";

function Back({ onBack, disabled }: { onBack: () => void; disabled?: boolean }) {
  return (
    <button type="button" onClick={onBack} disabled={disabled} className="pressable -mt-1 mb-1 inline-flex min-h-9 items-center gap-1.5 self-start rounded-sm text-[0.93em] text-mist hover:text-frost">
      <ArrowLeft className="size-4" aria-hidden /> Back
    </button>
  );
}

/** Adds an existing Passman vault file as a new account. The core opens the file dialog. */
export function ImportFlow({ onBack, onDone }: { onBack: () => void; onDone: () => void }) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const check = useNameCheck(name);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!check.ok || busy) return;
    setBusy(true);
    setError(null);
    try {
      const added = await importProfileFile(check.name);
      setBusy(false);
      if (added) {
        pushToast({ kind: "success", text: `“${check.name}” was added. Enter its master password to open it.` });
        onDone();
      }
    } catch (err) {
      setBusy(false);
      setError(err instanceof BackendError || err instanceof Error ? err.message : "That file could not be added.");
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
      <Back onBack={onBack} disabled={busy} />
      <div>
        <h2 className="font-display text-[1.45rem] leading-tight font-semibold">Open a vault file</h2>
        <p className="mt-2 text-mist">Choose a vault file you exported from Passman. It keeps the master password it was created with, and nothing in it is decrypted until you unlock it.</p>
      </div>
      {error && <p role="alert" className="rounded-lg border border-ember/40 bg-danger/10 px-4 py-3 text-[0.95em]">{error}</p>}
      <NameField value={name} onChange={setName} disabled={busy} autoFocus />
      <Button type="submit" variant="key" block busy={busy} disabled={!check.ok} icon={<FolderOpen className="size-4" />} className="min-h-11">Choose the file</Button>
    </form>
  );
}

/** Restores a vault from the master backup into a new account on this device. */
export function RestoreFlow({ onBack, onDone }: { onBack: () => void; onDone: () => void }) {
  const [name, setName] = useState("");
  const check = useNameCheck(name);

  async function restore(input: BackupTargetInput) {
    if (!check.ok) throw new Error(check.message);
    await restoreFromBackup(check.name, { ...input, name: check.name });
    pushToast({ kind: "success", text: `“${check.name}” was restored. Enter its master password to open it.` });
    onDone();
  }

  return (
    <div className="flex flex-col gap-4">
      <Back onBack={onBack} />
      <div>
        <h2 className="font-display text-[1.45rem] leading-tight font-semibold">Restore from a backup</h2>
        <p className="mt-2 text-[0.93em] text-mist">Enter the details of your master backup. Passman downloads the encrypted vault; you then unlock it with its master password. Backups you ticked to keep in the master backup come back with it, already set up.</p>
      </div>
      <NameField value={name} onChange={setName} />
      <TargetForm mode="restore" submitLabel="Restore vault" onSubmit={restore} />
    </div>
  );
}
