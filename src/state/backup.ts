import type { BackupTargetView } from "../backend";
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
