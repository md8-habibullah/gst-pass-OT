import type { BackupTargetView, HealthState, TargetHealth } from "../backend";
import { be } from "./app";
import { createStore } from "./store";

export interface BackupState {
  /** null until the first load for the open vault. */
  targets: BackupTargetView[] | null;
}

export const backups = createStore<BackupState>({ targets: null });

export async function refreshBackups(): Promise<BackupTargetView[]> {
  const targets = await be().listBackups();
  backups.set({ targets });
  return targets;
}

export const setBackups = (targets: BackupTargetView[]) => backups.set({ targets });
export const clearBackups = () => backups.set({ targets: null });

/** What a backup row shows: the worker's state, or "backing up" while a target that looks fine still has uploads waiting. */
export type RowState = HealthState | "backing";

/**
 * The worker records a success after a pull or a key check, before the push queue has drained, so a new target (or one that
 * has just been given a lot to send) can report `healthy` or `untested` while its first copy is still going up. "Up to date"
 * is only true with an empty queue; until then the row says "Backing up", like the sidebar summary does. The states that
 * already explain a problem (failing, lagging, disabled) are left as they are.
 */
export function rowState(h: Pick<TargetHealth, "state" | "queued">): RowState {
  return (h.state === "healthy" || h.state === "untested") && h.queued > 0 ? "backing" : h.state;
}

export interface BackupSummary {
  tone: "none" | "ok" | "busy" | "bad" | "untested";
  title: string;
  detail: string;
}

/** One line for the sidebar: the worst thing about the targets, or reassurance. */
export function summarize(targets: BackupTargetView[] | null): BackupSummary {
  if (!targets || targets.length === 0) return { tone: "none", title: "Local only", detail: "Stored encrypted on this device. No backups yet." };
  const on = targets.filter((t) => t.enabled);
  if (on.length === 0) return { tone: "none", title: "Backups paused", detail: "Every backup target is switched off." };
  const failing = on.filter((t) => t.health.state === "failing");
  if (failing.length) return { tone: "bad", title: failing.length === 1 ? "A backup is failing" : `${failing.length} backups are failing`, detail: `${failing[0]!.name}: ${failing[0]!.health.lastError ?? "see Backup for details"}` };
  const lagging = on.filter((t) => t.health.state === "lagging" || t.health.queued > 0);
  if (lagging.length) return { tone: "busy", title: "Backing up", detail: `${lagging.reduce((n, t) => n + t.health.queued, 0)} changes waiting to upload.` };
  if (on.some((t) => t.health.state === "untested")) return { tone: "untested", title: "Not tested yet", detail: "Run a backup to confirm it works." };
  return { tone: "ok", title: on.length === 1 ? "Backed up" : `Backed up to ${on.length} places`, detail: "Every change is already in your backups." };
}
