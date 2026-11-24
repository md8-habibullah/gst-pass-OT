import { useState, type FormEvent } from "react";
import { motion } from "motion/react";
import { ArrowLeft, LockKeyhole, RotateCcw, TriangleAlert } from "lucide-react";
import { BackendError } from "../../backend";
import { exactTime, relativeTime } from "../../lib/format";
import { app, be, pushToast, refresh, select } from "../../state/app";
import { useStore } from "../../state/store";
import { Avatar } from "../ui/Avatar";
import { Button, IconButton } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";
import { useLayout } from "../../lib/layout";
import { FilterChips } from "../shell/FilterChips";

/** Trash list: only the title and when it was deleted (spec: nothing else is visible or openable). */
export function TrashList() {
  const layout = useLayout();
  const trash = useStore(app, (s) => s.trash);
  const selectedId = useStore(app, (s) => s.selectedId);
  return (
    <section aria-label="Trash" className="flex min-h-0 flex-1 flex-col">
      <div className="px-4 pt-[calc(1rem+var(--safe-top))] pb-3 min-[720px]:pt-4">
        <h2 className="font-display text-[1.45rem] leading-none font-semibold">Trash</h2>
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

/** Step 1: warning. Step 2: master password and the item name (or CONFIRM). Only then does Rust erase it. */
function PurgeFlow({ id, title, open, onClose, onDone }: { id: string; title: string; open: boolean; onClose: () => void; onDone?: () => void }) {
  const [step, setStep] = useState<1 | 2>(1);
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [pwError, setPwError] = useState<string | null>(null);
  const [confError, setConfError] = useState<string | null>(null);

  const close = () => { if (busy) return; setStep(1); setPassword(""); setConfirmation(""); setPwError(null); setConfError(null); onClose(); };
  const matches = confirmation.trim() === "CONFIRM" || confirmation.trim() === title.trim();

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (busy) return;
    setPwError(null); setConfError(null);
    if (!password) return setPwError("Enter your master password.");
    if (!matches) return setConfError(`Type “${title}” or CONFIRM exactly.`);
    setBusy(true);
    try {
      await be().purgeItem(id, password, confirmation.trim());
      setBusy(false);
      setStep(1); setPassword(""); setConfirmation("");
      onClose();
      await refresh();
      select(null);
      onDone?.();
      pushToast({ kind: "success", text: `“${title}” was deleted forever.` });
    } catch (err) {
      setBusy(false);
      if (err instanceof BackendError && err.code === "wrong_password") setPwError("That master password is not right.");
      else if (err instanceof BackendError && err.code === "confirmation_mismatch") setConfError(err.message);
      else setPwError(err instanceof Error ? err.message : "Could not delete the item.");
    }
  }

  return (
    <Sheet
      open={open}
      onClose={close}
      title={step === 1 ? `Delete “${title}” forever?` : "Confirm with your master password"}
      variant="dialog"
      size="sm"
      dismissible={!busy}
      titleIcon={<TriangleAlert className="size-5 shrink-0 text-ember" aria-hidden />}
    >
      {step === 1 ? (
        <div className="flex flex-col gap-4 pt-1">
          <p className="text-mist">This permanently erases the item, every earlier version and any attachments from this device. It cannot be undone, and nothing is kept in a hidden backup.</p>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={close}>Cancel</Button>
            <Button variant="danger" onClick={() => setStep(2)}>Continue</Button>
          </div>
        </div>
      ) : (
        <form onSubmit={submit} className="flex flex-col gap-4 pt-1">
          <Field label="Master password" error={pwError}>
            {({ id: fid, describedBy, invalid }) => <TextInput id={fid} data-autofocus type="password" autoComplete="current-password" value={password} invalid={invalid} aria-describedby={describedBy} disabled={busy} onChange={(e) => setPassword(e.target.value)} />}
          </Field>
          <Field label={`Type “${title}” or CONFIRM`} error={confError}>
            {({ id: fid, describedBy, invalid }) => <TextInput id={fid} value={confirmation} invalid={invalid} aria-describedby={describedBy} disabled={busy} onChange={(e) => setConfirmation(e.target.value)} />}
          </Field>
          <div className="flex justify-end gap-2 pt-1">
            <Button variant="ghost" onClick={() => setStep(1)} disabled={busy}>Back</Button>
            <Button type="submit" variant="danger" busy={busy} disabled={!matches || !password}>Delete forever</Button>
          </div>
        </form>
      )}
    </Sheet>
  );
}
