import { useEffect, useState } from "react";
import { Cloud, CloudUpload, Crown, KeyRound, Pencil, Plus, RefreshCw, Trash2 } from "lucide-react";
import { BackendError, type BackupTargetInput, type BackupTargetView, type HealthState } from "../../backend";
import { copyText } from "../../lib/clipboard";
import { relativeTime } from "../../lib/format";
import { app, be, pushToast, syncAfterCoreLock } from "../../state/app";
import { backups, refreshBackups, setBackups, summarize } from "../../state/backup";
import { useStore } from "../../state/store";
import { reprompt, withMaster } from "../reprompt/Reprompt";
import { PageBody, PageHeader } from "../shell/PageHeader";
import { Button } from "../ui/Button";
import { Sheet } from "../ui/Sheet";
import { useBrandName } from "../../state/brand";
import { TargetForm } from "./TargetForm";

const STATE_LABEL: Record<HealthState, string> = {
  untested: "Not tested yet",
  healthy: "Up to date",
  lagging: "Catching up",
  failing: "Not reaching the bucket",
  disabled: "Switched off",
};
const STATE_DOT: Record<HealthState, string> = { untested: "bg-brass-ink", healthy: "bg-verdigris", lagging: "bg-brass-ink", failing: "bg-ember", disabled: "bg-fog" };

const where = (t: BackupTargetView) => `${t.endpoint ? new URL(t.endpoint).host : "Amazon S3"} / ${t.bucket}${t.prefix ? ` / ${t.prefix.replace(/\/$/, "")}` : ""}`;
const safeWhere = (t: BackupTargetView) => { try { return where(t); } catch { return `${t.endpoint ?? "Amazon S3"} / ${t.bucket}`; } };

function Row({ t, onEdit, onRemove, busy, onRun }: { t: BackupTargetView; onEdit: () => void; onRemove: () => void; busy: boolean; onRun: () => void }) {
  const h = t.health;
  async function copySecret() {
    const clear = app.get().clipboardClear;
    try {
      const secret = await reprompt({ reason: `Copy the secret access key of “${t.name}”.`, confirmLabel: "Copy", run: (pw) => be().revealBackupSecret(t.id, pw) });
      if (!secret) return;
      await copyText(secret, clear);
      pushToast({ kind: "success", text: `The secret key was copied. The clipboard clears in ${clear} seconds.` });
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "Could not copy the key." });
    }
  }
  async function makeMaster() {
    try {
      const done = await withMaster("Change which backup is the master.", () => be().setMasterBackup(t.id).then(() => true));
      if (!done) return;
      await refreshBackups(); pushToast({ kind: "success", text: `“${t.name}” is now the master backup.` });
    }
    catch (e) { pushToast({ kind: "error", text: e instanceof Error ? e.message : "Could not change the master backup." }); }
  }
  return (
    <li className="flex flex-col gap-3 border-b border-hairline px-4 py-4 last:border-b-0">
      <div className="flex items-start gap-3">
        <span className={`mt-[9px] size-2.5 shrink-0 rounded-full ${STATE_DOT[h.state]}`} aria-hidden />
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 font-semibold">
            {t.name}
            {t.master && <span className="inline-flex items-center gap-1 rounded-full bg-brass/15 px-2 py-px text-[0.78em] font-medium text-brass-ink-hi"><Crown className="size-3" aria-hidden />Master backup</span>}
            {t.shareInMaster && !t.master && <span className="rounded-full border border-hairline-strong px-2 py-px text-[0.78em] font-medium text-mist">In the master backup</span>}
          </p>
          <p className="mt-0.5 truncate font-mono text-[0.86em] text-mist">{safeWhere(t)}</p>
          <p className="mt-1.5 text-[0.93em]">
            <span className={h.state === "failing" ? "text-ember" : "text-frost"}>{STATE_LABEL[h.state]}</span>
            {h.queued > 0 && <span className="text-mist">, {h.queued} {h.queued === 1 ? "change" : "changes"} waiting</span>}
            {h.lastSuccessAt && <span className="text-mist">, last backup {relativeTime(h.lastSuccessAt)}</span>}
          </p>
          {h.lastError && h.state !== "healthy" && <p className="mt-1 text-[0.86em] text-ember">{h.lastError}</p>}
          {h.conditionalWrites === "weak" && <p className="mt-1 text-[0.86em] text-mist">This provider cannot guarantee safe simultaneous writes. Use one device at a time.</p>}
        </div>
      </div>
      <div className="flex flex-wrap gap-2 pl-[1.375rem]">
        <Button className="min-h-8" icon={<RefreshCw className="size-3.5" />} busy={busy} disabled={!t.enabled} onClick={onRun}>Back up now</Button>
        <Button className="min-h-8" icon={<Pencil className="size-3.5" />} onClick={onEdit}>Edit</Button>
        {!t.master && <Button className="min-h-8" icon={<Crown className="size-3.5" />} onClick={() => void makeMaster()}>Make master</Button>}
        <Button className="min-h-8" icon={<KeyRound className="size-3.5" />} onClick={() => void copySecret()}>Copy secret key</Button>
        <Button className="min-h-8 text-ember" variant="ghost" icon={<Trash2 className="size-3.5" />} onClick={onRemove}>Remove</Button>
      </div>
    </li>
  );
}

/** Backup management: any number of S3-compatible targets, one of which can be the master. */
export function BackupView() {
  const brandName = useBrandName();
  const targets = useStore(backups, (s) => s.targets);
  const [sheet, setSheet] = useState<{ kind: "add" } | { kind: "edit"; t: BackupTargetView } | { kind: "remove"; t: BackupTargetView } | null>(null);
  const [running, setRunning] = useState<string | "all" | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const summary = summarize(targets);

  useEffect(() => {
    let alive = true;
    const load = () => refreshBackups().then(() => alive && setLoadError(null)).catch((e: unknown) => alive && setLoadError(e instanceof Error ? e.message : "Could not read the backup list."));
    void load();
    const t = setInterval(load, 8000);
    return () => { alive = false; clearInterval(t); };
  }, []);

  async function run(id: string | null) {
    setRunning(id ?? "all");
    try {
      setBackups(await be().runBackup(id));
      // The run compared the bucket with this device first: show what it pulled (and any objects it refused) at once.
      void syncAfterCoreLock().catch(() => undefined);
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "The backup could not run." });
      void refreshBackups().catch(() => undefined);
    } finally {
      setRunning(null);
    }
  }

  async function save(input: BackupTargetInput, existing?: BackupTargetView) {
    const t = await withMaster(existing ? "Save the changes to this backup." : "Add a backup.", () => (existing ? be().updateBackup(existing.id, input) : be().addBackup(input)));
    if (!t) return; // the password sheet was cancelled: the form stays as it is
    await refreshBackups();
    setSheet(null);
    pushToast({ kind: "success", text: existing ? `“${t.name}” saved.` : `“${t.name}” added. Backing up now.` });
    if (!existing) void run(t.id);
  }

  async function remove(t: BackupTargetView) {
    try {
      const done = await withMaster("Remove this backup.", () => be().removeBackup(t.id).then(() => true));
      if (!done) return;
      await refreshBackups();
      setSheet(null);
      pushToast({ kind: "info", text: `“${t.name}” was removed. Files already in the bucket are left untouched.` });
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof BackendError ? e.message : "Could not remove the backup." });
    }
  }

  return (
    <PageBody>
      <PageHeader
        title="Backup"
        back
        subtitle="Your vault is encrypted before anything leaves this device. Backups store only that encrypted data, in buckets you control."
        actions={<Button variant="key" icon={<Plus className="size-4" />} onClick={() => setSheet({ kind: "add" })}>Add backup</Button>}
      />

      {loadError && <p role="alert" className="rounded-lg border border-ember/40 bg-danger/10 px-4 py-3 text-[0.95em]">{loadError}</p>}

      {targets === null ? (
        <p className="text-mist" aria-busy>Loading.</p>
      ) : targets.length === 0 ? (
        <div className="surface rounded-lg px-6 py-10 text-center">
          <Cloud className="mx-auto size-9 text-fog" aria-hidden strokeWidth={1.4} />
          <p className="mt-3 font-display text-xl">No backups yet</p>
          <p className="mx-auto mt-2 max-w-md text-mist">Your vault lives on this device only. Add an S3 bucket (Amazon, Cloudflare R2, Backblaze B2, MinIO and others) and every change is copied there, already encrypted.</p>
          <Button variant="key" className="mt-5" icon={<Plus className="size-4" />} onClick={() => setSheet({ kind: "add" })}>Add your first backup</Button>
        </div>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-3 rounded-lg bg-pane/70 px-4 py-3 shadow-e1">
            <CloudUpload className="size-5 shrink-0 text-brass-ink" aria-hidden />
            <div className="min-w-0 flex-1 basis-48">
              <p className="font-semibold">{summary.title}</p>
              <p className="text-[0.93em] text-mist">{summary.detail}</p>
            </div>
            <Button icon={<RefreshCw className="size-4" />} busy={running === "all"} onClick={() => void run(null)}>Back up everything now</Button>
          </div>
          <ul className="surface overflow-hidden rounded-lg" aria-label="Backup targets">
            {targets.map((t) => (
              <Row key={t.id} t={t} busy={running === t.id || running === "all"} onRun={() => void run(t.id)} onEdit={() => setSheet({ kind: "edit", t })} onRemove={() => setSheet({ kind: "remove", t })} />
            ))}
          </ul>
          <div className="rounded-lg bg-pane/70 px-4 py-3.5 text-[0.93em] text-mist shadow-e1">
            <p className="font-medium text-frost">How the master backup works</p>
            <p className="mt-1">On a new device, choose Restore from a backup and enter the keys of the master backup only. {brandName} downloads your vault, you unlock it with your master password, and every other backup that was ticked “Also keep these credentials in the master backup” is already set up inside it.</p>
          </div>
        </>
      )}

      <Sheet open={sheet?.kind === "add"} onClose={() => setSheet(null)} title="Add a backup" variant="form" size="md">
        {sheet?.kind === "add" && <TargetForm mode="add" submitLabel="Add backup" defaultMaster={(targets?.length ?? 0) === 0} onSubmit={(i) => save(i)} onCancel={() => setSheet(null)} />}
      </Sheet>
      <Sheet open={sheet?.kind === "edit"} onClose={() => setSheet(null)} title={sheet?.kind === "edit" ? `Edit ${sheet.t.name}` : "Edit backup"} variant="form" size="md">
        {sheet?.kind === "edit" && <TargetForm mode="edit" initial={sheet.t} submitLabel="Save changes" onSubmit={(i) => save(i, sheet.t)} onCancel={() => setSheet(null)} />}
      </Sheet>
      <Sheet open={sheet?.kind === "remove"} onClose={() => setSheet(null)} title="Remove this backup?" variant="dialog" size="sm">
        {sheet?.kind === "remove" && (
          <div className="flex flex-col gap-4 pt-1">
            <p>“{sheet.t.name}” will stop receiving changes and its credentials are deleted from this vault. Files already in the bucket stay there; delete them with your provider if you no longer want them.</p>
            {sheet.t.master && <p className="text-[0.93em] text-brass-ink-hi">This is your master backup. Choose another one afterwards so a fresh install knows where to restore from.</p>}
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setSheet(null)}>Keep it</Button>
              <Button variant="danger" icon={<Trash2 className="size-4" />} onClick={() => void remove(sheet.t)}>Remove backup</Button>
            </div>
          </div>
        )}
      </Sheet>
    </PageBody>
  );
}
