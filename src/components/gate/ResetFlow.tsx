import { useCallback, useEffect, useState, type FormEvent } from "react";
import { TriangleAlert } from "lucide-react";
import { BackendError } from "../../backend";
import { prettyEquation } from "../../lib/gateRules";
import { prepareUninstall, resetAppData, resetChallenge } from "../../state/app";
import { Button } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";

/** "erase": back to a fresh app (the lock screen's way out when the passcode is lost). "remove": the app removes everything it keeps, for uninstalling. */
export type WipeKind = "erase" | "remove";

const ERASE_STEPS = [
  { title: "Reset this app?", body: "This removes every vault on this device: the items, files, backup settings and the app lock. It is meant for when you cannot get in any more. Hexis keeps only its display settings (time zone, theme, clipboard time).", go: "Continue" },
  { title: "This cannot be undone", body: "Changes that were never backed up are lost for good, and nobody can bring them back. Backups already in your S3 buckets are not touched, but this device forgets how to reach them.", go: "I understand, continue" },
  { title: "Erase everything now?", body: "This is the last step. Work out the sum and type the answer. Only the right answer erases this device. The app then returns to how it looked on first launch.", go: "Erase everything" },
] as const;

const REMOVE_STEPS = [
  { title: "Remove Hexis from this device?", body: "Do this before you uninstall Hexis, so that nothing is left behind. It removes every vault on this device (the items, files, backup settings and the app lock) and everything else Hexis keeps here: its settings, and anything its window stored as cache or cookies.", go: "Continue" },
  { title: "This cannot be undone", body: "Changes that were never backed up are lost for good, and nobody can bring them back. Backups in your S3 buckets are not on this device and are not touched: delete them in your storage provider's console if you want them gone. Once this is done, Hexis cannot reach them either.", go: "I understand, continue" },
  { title: "Remove everything now?", body: "This is the last step. Work out the sum and type the answer. Only the right answer removes everything. Nothing is made again afterwards, not even an empty vault.", go: "Remove everything" },
] as const;

/**
 * Three separate warnings, then the same wipe the panic triggers use. The last step is a long sum that the core made up
 * and keeps the answer to, so nobody erases a device by tapping through three screens. A wrong answer erases nothing and
 * brings a new sum. "remove" is the same walk; the app then shows `RemovedScreen`.
 */
export function ResetFlow({ open, onClose, kind = "erase" }: { open: boolean; onClose: () => void; kind?: WipeKind }) {
  const STEPS = kind === "remove" ? REMOVE_STEPS : ERASE_STEPS;
  const [step, setStep] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sum, setSum] = useState<string | null>(null);
  const [answer, setAnswer] = useState("");
  const s = STEPS[step]!;
  const last = step === STEPS.length - 1;

  const loadSum = useCallback(async () => {
    try {
      setSum(await resetChallenge());
    } catch (e) {
      setSum(null);
      setError(e instanceof BackendError || e instanceof Error ? e.message : "The sum could not be made. Close this and try again.");
    }
  }, []);

  // The sum is asked for only when the last step opens, and a new one comes each time the step is left and reached again.
  useEffect(() => {
    if (open && last) void loadSum();
    if (!last) { setSum(null); setAnswer(""); }
  }, [open, last, loadSum]);

  function close() { setStep(0); setError(null); setSum(null); setAnswer(""); onClose(); }

  async function next(e?: FormEvent) {
    e?.preventDefault();
    if (!last) return setStep(step + 1);
    if (busy) return;
    const typed = answer.trim();
    if (!sum) return setError("Wait for the sum to appear, then type the answer.");
    if (!/^\d+$/.test(typed)) return setError("Type the answer as a whole number.");
    setBusy(true);
    setError(null);
    try {
      if (kind === "remove") {
        await prepareUninstall(typed); // the app moves on to its last screen; this sheet goes with the page that hosts it
      } else {
        await resetAppData(typed);
      }
    } catch (err) {
      setBusy(false);
      if (err instanceof BackendError && err.code === "wrong_answer") {
        setError(`That is not the answer. Nothing was ${kind === "remove" ? "removed" : "erased"}. Here is a new sum.`);
        setAnswer("");
        setSum(null);
        void loadSum();
      } else {
        setError(err instanceof BackendError || err instanceof Error ? err.message : kind === "remove" ? "The removal did not finish." : "The reset did not finish.");
      }
    }
  }

  return (
    <Sheet open={open} onClose={close} title={s.title} variant="dialog" size="sm" dismissible={!busy} titleIcon={<TriangleAlert className="size-5 text-ember" aria-hidden />}>
      <form key={step} onSubmit={(e) => void next(e)} className="flex flex-col gap-4" noValidate>
        <p className="text-mist">{s.body}</p>
        {last && (
          <div className="flex flex-col gap-3">
            <p
              aria-live="polite"
              aria-label={sum ? `What is ${prettyEquation(sum)}?` : "Making a sum"}
              className="rounded-lg bg-pane/70 px-4 py-4 text-center font-display text-[1.35rem] leading-snug font-semibold tabular-nums text-balance break-words shadow-e1"
            >
              {sum ? <>{prettyEquation(sum)} <span className="text-fog">= ?</span></> : <span className="text-fog">…</span>}
            </p>
            <Field label="Your answer" hint={`A wrong answer ${kind === "remove" ? "removes" : "erases"} nothing and gives you a new sum.`}>
              {({ id, describedBy, invalid }) => (
                <TextInput
                  id={id} data-autofocus mono inputMode="numeric" pattern="[0-9]*" enterKeyHint="go"
                  value={answer} invalid={invalid} aria-describedby={describedBy} disabled={busy || !sum}
                  onChange={(e) => setAnswer(e.target.value.replace(/\D/g, "").slice(0, 9))}
                />
              )}
            </Field>
          </div>
        )}
        <p className="text-[0.86em] text-fog">Step {step + 1} of {STEPS.length}</p>
        {error && <p role="alert" className="rounded-lg border border-ember/40 bg-danger/10 px-4 py-3 text-[0.95em]">{error}</p>}
        <div className="flex flex-wrap justify-end gap-2">
          <Button data-autofocus={last ? undefined : true} onClick={close} disabled={busy}>Cancel</Button>
          <Button type="submit" variant="danger" busy={busy} disabled={last && (!sum || !answer)}>{s.go}</Button>
        </div>
      </form>
    </Sheet>
  );
}
