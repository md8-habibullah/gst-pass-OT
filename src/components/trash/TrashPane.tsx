import { useCallback, useEffect, useState, type FormEvent } from "react";
import { motion } from "motion/react";
import { ArrowLeft, LockKeyhole, RotateCcw, TriangleAlert } from "lucide-react";
import { BackendError, type PurgePlan } from "../../backend";
import { exactTime, relativeTime } from "../../lib/format";
import { prettyEquation } from "../../lib/gateRules";
import { app, be, pushToast, refresh, select } from "../../state/app";
import { useStore } from "../../state/store";
import { Avatar } from "../ui/Avatar";
import { Button, IconButton } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";
import { useLayout } from "../../lib/layout";
import { AccountBar } from "../shell/AccountMenu";
import { FilterChips } from "../shell/FilterChips";

/** Trash list: only the title and when it was deleted (spec: nothing else is visible or openable). */
export function TrashList() {
  const layout = useLayout();
  const trash = useStore(app, (s) => s.trash);
  const selectedId = useStore(app, (s) => s.selectedId);
  return (
    <section aria-label="Trash" className="flex min-h-0 flex-1 flex-col">
      <div className="px-4 pt-[calc(1rem+var(--safe-top))] pb-3 min-[720px]:pt-4">
        <div className="flex items-center justify-between gap-3">
          <h2 className="font-display text-[1.45rem] leading-none font-semibold">Trash</h2>
          <AccountBar />
        </div>
        <p className="mt-2 text-[0.93em] text-mist">Deleted items wait here until you restore them or delete them forever. Nothing is removed automatically.</p>
        {layout !== "wide" && <div className="mt-3"><FilterChips /></div>}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-28 min-[720px]:pb-6">
        {trash.length === 0 ? (
          <div className="px-8 pt-14 text-center">
            <p className="font-display text-xl">The trash is empty</p>
            <p className="mt-2 text-mist">Items you delete from your vault will appear here.</p>
          </div>
        ) : (
          <ul role="listbox" aria-label="Deleted items" className="flex flex-col gap-0.5">
            {trash.map((t) => {
              const on = t.id === selectedId;
              return (
                <li key={t.id} role="presentation">
                  <button type="button" role="option" aria-selected={on} data-item-row data-item-id={t.id} onClick={() => select(t.id)} className={`pressable relative flex min-h-[3.75rem] w-full items-center gap-3 rounded-lg px-3 text-left pointer-coarse:min-h-[4.5rem] ${on ? "bg-raised shadow-e2" : "hover:bg-raised/60"}`}>
                    {on && <motion.span layoutId="row-edge" aria-hidden className="absolute top-3 bottom-3 left-0 w-[3px] rounded-full bg-brass-ink" />}
                    <Avatar title={t.title} size={38} dim />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-semibold text-mist">{t.title}</span>
                      <span className="block truncate text-[0.93em] text-fog">Deleted {relativeTime(t.deletedAt)}</span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </section>
  );
}

/** Restricted view of one trashed item with Restore and the double-confirmed Delete forever. */
export function TrashDetail({ id, onBack }: { id: string; onBack?: () => void }) {
  const entry = useStore(app, (s) => s.trash.find((t) => t.id === id));
  const [purge, setPurge] = useState(false);
  const [busy, setBusy] = useState(false);
  if (!entry) return <p className="p-8 text-mist">This item is no longer in the trash.</p>;

  async function restore() {
    setBusy(true);
    try {
      const item = await be().restoreItem(id);
      await refresh();
      select(null);
      onBack?.();
      pushToast({ kind: "success", text: `“${item.title}” restored to your vault.` });
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "Could not restore the item." });
    } finally {
      setBusy(false);
    }
  }

  return (
    <article aria-label={entry.title} className="mx-auto flex w-full max-w-[36rem] flex-col gap-5 px-5 pt-5 pb-28 min-[720px]:pb-10">
      <header className="flex items-start gap-4">
        {onBack && (
          <IconButton label="Back to trash" className="-ml-2 mt-1" onClick={onBack}>
            <ArrowLeft className="size-5" />
          </IconButton>
        )}
        <Avatar title={entry.title} size={56} dim />
        <div className="min-w-0">
          <h2 className="font-display text-[1.65rem] leading-[1.15] font-semibold break-words text-mist">{entry.title}</h2>
          <p className="mt-1 text-[0.93em] text-fog" title={exactTime(entry.deletedAt)}>Deleted {exactTime(entry.deletedAt)}</p>
        </div>
      </header>

      <div className="surface flex items-start gap-3.5 rounded-lg px-4 py-4">
        <LockKeyhole className="mt-0.5 size-5 shrink-0 text-mist" aria-hidden />
        <p className="text-mist">The contents stay sealed while an item is in the trash. Restore it to open, copy or edit it.</p>
      </div>

      <div className="flex flex-wrap gap-2.5">
        <Button variant="key" icon={<RotateCcw className="size-4" />} busy={busy} onClick={() => void restore()}>
          Restore
        </Button>
        <Button variant="quiet" className="text-ember" onClick={() => setPurge(true)} disabled={busy}>
          Delete forever
        </Button>
      </div>

      <PurgeFlow id={id} title={entry.title} open={purge} onClose={() => setPurge(false)} onDone={onBack} />
    </article>
  );
}

/**
 * Deleting forever. Step 1 is the warning and says what it will take: an item that went to the trash less than six months
 * ago needs the master password and the answer to a long sum; an older one needs the master password only. Step 2 asks
 * for those. The core decides which rule applies (on its own checked clock) and keeps the sum's answer; a wrong answer
 * uses the sum up and a new one is shown.
 */
function PurgeFlow({ id, title, open, onClose, onDone }: { id: string; title: string; open: boolean; onClose: () => void; onDone?: () => void }) {
  const [step, setStep] = useState<1 | 2>(1);
  const [plan, setPlan] = useState<PurgePlan | null>(null);
  const [password, setPassword] = useState("");
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  const [planError, setPlanError] = useState<string | null>(null);
  const [pwError, setPwError] = useState<string | null>(null);
  const [answerError, setAnswerError] = useState<string | null>(null);

  const loadPlan = useCallback(async () => {
    setPlanError(null);
    try {
      setPlan(await be().purgePlan(id));
    } catch (e) {
      setPlan(null);
      setPlanError(e instanceof Error ? e.message : "Could not check this item.");
    }
  }, [id]);

  // The warning shows the item's age, so the plan is read when the sheet opens.
  useEffect(() => { if (open) void loadPlan(); }, [open, loadPlan]);

  const reset = () => { setStep(1); setPlan(null); setPassword(""); setAnswer(""); setPwError(null); setAnswerError(null); setPlanError(null); };
  const close = () => { if (busy) return; reset(); onClose(); };

  async function toStep2() {
    // A fresh sum for the step that uses it (it is made by the core and its answer never leaves it).
    await loadPlan();
    setStep(2);
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (busy || !plan) return;
    setPwError(null); setAnswerError(null);
    const typed = answer.trim();
    if (!password) return setPwError("Enter your master password.");
    if (plan.recent && !/^\d+$/.test(typed)) return setAnswerError("Type the answer as a whole number.");
    setBusy(true);
    try {
      await be().purgeItem(id, password, plan.recent ? typed : undefined);
      setBusy(false);
      reset();
      onClose();
      await refresh();
      select(null);
      onDone?.();
      pushToast({ kind: "success", text: `“${title}” was deleted forever.` });
    } catch (err) {
      setBusy(false);
      if (err instanceof BackendError && err.code === "wrong_password") {
        setPwError("That master password is not right.");
      } else if (err instanceof BackendError && err.code === "wrong_answer") {
        // The sum is used up, right or wrong: nothing was deleted and a new one is shown.
        setAnswer("");
        await loadPlan();
        setAnswerError("That is not the answer, so nothing was deleted. Here is a new sum.");
      } else {
        setPwError(err instanceof Error ? err.message : "Could not delete the item.");
      }
    }
  }

  const recent = plan?.recent === true;
  return (
    <Sheet
      open={open}
      onClose={close}
      title={step === 1 ? `Delete “${title}” forever?` : recent ? "Confirm with your password and the sum" : "Confirm with your master password"}
      variant="dialog"
      size="sm"
      dismissible={!busy}
      titleIcon={<TriangleAlert className="size-5 shrink-0 text-ember" aria-hidden />}
    >
      {step === 1 ? (
        <div className="flex flex-col gap-4 pt-1">
          <p className="text-mist">This permanently erases the item, every earlier version and any attachments from this device. It cannot be undone, and nothing is kept in a hidden backup.</p>
          {planError ? (
            <p role="alert" className="text-[0.93em] text-ember">{planError}</p>
          ) : plan ? (
            <p className="rounded-lg bg-pane/70 px-4 py-3 text-[0.93em] shadow-e1">
              {plan.recent
                ? <>Deleted {exactTime(plan.deletedAt)}, less than six months ago. To erase it you need your master password <strong className="font-semibold">and</strong> the answer to a sum. From {exactTime(plan.olderAt)} the master password alone is enough.</>
                : <>Deleted {exactTime(plan.deletedAt)}, over six months ago. Your master password is all it takes.</>}
            </p>
          ) : (
            <p className="text-[0.93em] text-fog" aria-busy>Checking how long it has been in the trash…</p>
          )}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={close}>Cancel</Button>
            <Button variant="danger" disabled={!plan} onClick={() => void toStep2()}>Continue</Button>
          </div>
        </div>
      ) : (
        <form onSubmit={submit} className="flex flex-col gap-4 pt-1" noValidate>
          <Field label="Master password" error={pwError}>
            {({ id: fid, describedBy, invalid }) => <TextInput id={fid} data-autofocus type="password" autoComplete="current-password" value={password} invalid={invalid} aria-describedby={describedBy} disabled={busy} onChange={(e) => setPassword(e.target.value)} />}
          </Field>
          {recent && (
            <div className="flex flex-col gap-3">
              <p
                aria-live="polite"
                aria-label={plan?.sum ? `What is ${prettyEquation(plan.sum)}?` : "Making a sum"}
                className="rounded-lg bg-pane/70 px-4 py-4 text-center font-display text-[1.35rem] leading-snug font-semibold tabular-nums text-balance break-words shadow-e1"
              >
                {plan?.sum ? <>{prettyEquation(plan.sum)} <span className="text-fog">= ?</span></> : <span className="text-fog">…</span>}
              </p>
              <Field label="Your answer" hint="A wrong answer deletes nothing and gives you a new sum." error={answerError}>
                {({ id: fid, describedBy, invalid }) => (
                  <TextInput id={fid} mono inputMode="numeric" pattern="[0-9]*" enterKeyHint="go" value={answer} invalid={invalid} aria-describedby={describedBy} disabled={busy || !plan?.sum} onChange={(e) => { setAnswer(e.target.value); setAnswerError(null); }} />
                )}
              </Field>
            </div>
          )}
          {planError && <p role="alert" className="text-[0.93em] text-ember">{planError}</p>}
          <div className="flex justify-end gap-2 pt-1">
            <Button variant="ghost" onClick={() => setStep(1)} disabled={busy}>Back</Button>
            <Button type="submit" variant="danger" busy={busy} disabled={!plan || !password || (recent && !answer.trim())}>Delete forever</Button>
          </div>
        </form>
      )}
    </Sheet>
  );
}
