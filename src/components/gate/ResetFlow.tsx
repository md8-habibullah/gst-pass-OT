import { useState } from "react";
import { TriangleAlert } from "lucide-react";
import { BackendError } from "../../backend";
import { resetAppData } from "../../state/app";
import { Button } from "../ui/Button";
import { Sheet } from "../ui/Sheet";

const STEPS = [
  { title: "Reset this app?", body: "This removes every vault on this device: the items, files, backup settings and the app lock. It is meant for when you cannot get in any more.", go: "Continue" },
  { title: "This cannot be undone", body: "Changes that were never backed up are lost for good, and nobody can bring them back. Backups already in your S3 buckets are not touched, but this device forgets how to reach them.", go: "I understand, continue" },
  { title: "Erase everything now?", body: "This is the last step. The app will return to how it looked on first launch.", go: "Erase everything" },
] as const;

/** Three separate warnings, then the same wipe the panic triggers use. */
export function ResetFlow({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [step, setStep] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const s = STEPS[step]!;
  const last = step === STEPS.length - 1;

  function close() { setStep(0); setError(null); onClose(); }
  async function next() {
    if (!last) return setStep(step + 1);
    setBusy(true);
    try {
      await resetAppData();
    } catch (e) {
      setBusy(false);
      setError(e instanceof BackendError || e instanceof Error ? e.message : "The reset did not finish.");
    }
  }

  return (
    <Sheet open={open} onClose={close} title={s.title} variant="dialog" size="sm" dismissible={!busy} titleIcon={<TriangleAlert className="size-5 text-ember" aria-hidden />}>
      <div key={step} className="flex flex-col gap-4">
        <p className="text-mist">{s.body}</p>
        <p className="text-[0.86em] text-fog">Step {step + 1} of {STEPS.length}</p>
        {error && <p role="alert" className="rounded-lg border border-ember/40 bg-danger/10 px-4 py-3 text-[0.95em]">{error}</p>}
        <div className="flex flex-wrap justify-end gap-2">
          <Button data-autofocus onClick={close} disabled={busy}>Cancel</Button>
          <Button variant="danger" busy={busy} onClick={() => void next()}>{s.go}</Button>
        </div>
      </div>
    </Sheet>
  );
}
