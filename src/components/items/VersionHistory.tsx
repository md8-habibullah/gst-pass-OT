import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { ChevronDown, Eye, History, RotateCcw } from "lucide-react";
import { FIELD_SCHEMA, instancesOf, type ItemDraft, type VersionEntry } from "../../backend";
import { groupSignIns } from "../../lib/instances";
import { EVENT_LABEL, exactTime, fieldName, plural, relativeTime } from "../../lib/format";
import { be, pushToast, refresh } from "../../state/app";
import { withItemPassword } from "../reprompt/Reprompt";
import { Button } from "../ui/Button";
import { Sheet } from "../ui/Sheet";
import { PasswordText } from "./FieldRows";

interface Props {
  id: string;
  title: string;
  /** Current stored version; the history reloads when it changes. */
  version: number;
  /** Changes when a pull from the backup altered local data, so the timeline reloads. */
  stamp?: number;
  /** Counts up when something else (the "changed on another device" note) asks for the timeline to open. */
  openSignal?: number;
}

function Snapshot({ draft, version, onClose, onRestore, current }: { draft: ItemDraft | null; version: number; onClose: () => void; onRestore: () => void; current: boolean }) {
  return (
    <Sheet
      open={draft !== null}
      onClose={onClose}
      title={`Version ${version}`}
      variant="dialog"
      size="md"
      footer={
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>Close</Button>
          {!current && <Button variant="key" icon={<RotateCcw className="size-4" />} onClick={onRestore}>Restore this version</Button>}
        </div>
      }
    >
      {draft && (
        <dl className="flex flex-col divide-y divide-hairline rounded-lg bg-raised/50">
          <div className="px-4 py-3">
            <dt className="text-[0.86em] font-medium text-fog">Title</dt>
            <dd className="mt-0.5 font-semibold">{draft.title}</dd>
          </div>
          {groupSignIns(FIELD_SCHEMA[draft.type].flatMap((s) => {
            // a login can hold several sign-ins and codes: "Password 1", "Password 2"
            const stored = instancesOf(draft.fields, s.key).filter((x) => x.value);
            return stored.map((x, i) => ({ key: x.key, label: stored.length > 1 ? `${s.label} ${i + 1}` : s.label, kind: s.kind, value: x.value }));
          })).map((f) => (
            <div key={f.key} className="px-4 py-3">
              <dt className="text-[0.86em] font-medium text-fog">{f.label}</dt>
              <dd className="mt-0.5">
                {f.kind === "password" ? <PasswordText value={f.value} />
                  : f.kind === "totp" ? <span className="text-mist">One-time code setup is saved with this version.</span>
                  : f.kind === "note" ? <span className="font-mono text-[0.97em] leading-6 break-words whitespace-pre-wrap">{f.value}</span>
                  : <span className="break-all">{f.value}</span>}
              </dd>
            </div>
          ))}
          {draft.customFields.map((c, i) => (
            <div key={i} className="px-4 py-3">
              <dt className="text-[0.86em] font-medium text-fog">{c.name}</dt>
              <dd className="mt-0.5 break-all">{c.hidden ? <PasswordText value={c.value} /> : c.value}</dd>
            </div>
          ))}
        </dl>
      )}
    </Sheet>
  );
}

/** Collapsible, append-only timeline. Viewing a snapshot's secrets and restoring both ask for the master password. */
export function VersionHistory({ id, title, version, stamp = 0, openSignal = 0 }: Props) {
  // DEV deep link (?history) opens the timeline for design review; dead-code-eliminated from production builds.
  const [open, setOpen] = useState(() => import.meta.env.DEV && new URLSearchParams(location.search).has("history"));
  const [entries, setEntries] = useState<VersionEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [snap, setSnap] = useState<{ draft: ItemDraft; version: number } | null>(null);

  const load = useCallback(async () => {
    try {
      setEntries(await be().listHistory(id));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not load the history.");
    }
  }, [id]);

  useEffect(() => { if (open) void load(); }, [open, load, version, stamp]);
  const section = useRef<HTMLElement>(null);
  useEffect(() => {
    if (openSignal === 0) return;
    setOpen(true);
    // The timeline is below the fold on most screens; bring it to the person who asked for it.
    requestAnimationFrame(() => section.current?.scrollIntoView({ block: "start" }));
  }, [openSignal]);

  async function view(v: number) {
    const draft = await withItemPassword({ reason: `Show version ${v} of “${title}”, including its passwords.`, confirmLabel: "Show version", run: (pw) => be().getVersion(id, v, pw) });
    if (draft) setSnap({ draft, version: v });
  }

  async function restore(v: number) {
    const res = await withItemPassword({ reason: `Make version ${v} of “${title}” the newest version. Nothing is deleted; the timeline keeps every version.`, confirmLabel: "Restore version", run: (pw) => be().restoreVersion(id, v, pw) });
    if (res) {
      setSnap(null);
      await refresh();
      pushToast({ kind: "success", text: `Version ${v} restored as version ${res.version}.` });
    }
  }

  // Copies that came from another device are never the current version, whatever their number.
  const latest = entries?.find((e) => !e.otherDevice)?.version;
  return (
    <section ref={section} aria-labelledby={`hist-${id}`}>
      <button
        type="button"
        id={`hist-${id}`}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="pressable flex min-h-11 w-full items-center gap-3 rounded-lg px-4 text-left hover:bg-raised/60"
      >
        <History className="size-[18px] text-mist" aria-hidden />
        <span className="flex-1 font-semibold">Version history</span>
        <span className="text-[0.86em] text-fog">{entries ? plural(entries.length, "version") : `Version ${version}`}</span>
        <ChevronDown className={`size-4 text-mist transition-transform duration-100 ${open ? "rotate-180" : ""}`} aria-hidden />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: "auto", opacity: 1 }} exit={{ height: 0, opacity: 0 }} transition={{ duration: 0.14, ease: [0.2, 0.8, 0.2, 1] }} className="overflow-hidden">
            {error && <p role="alert" className="px-4 py-3 text-ember">{error}</p>}
            {!entries && !error && <p className="px-4 py-3 text-mist">Loading the timeline.</p>}
            {entries && (
              <ol className="relative mx-4 mt-2 mb-1 flex flex-col gap-1 border-l border-hairline-strong pl-5">
                {entries.map((e) => {
                  const isLatest = !e.otherDevice && e.version === latest;
                  const lifecycle = e.event === "trashed" || e.event === "restored";
                  return (
                    <li key={`${e.version}-${e.timestamp}-${e.otherDevice ? "o" : "l"}`} className="relative py-2.5">
                      <span aria-hidden className={`absolute top-[18px] -left-[25px] size-2.5 rounded-full border-2 ${isLatest ? "border-brass-ink bg-brass-ink" : "border-mist/60 bg-pane"}`} />
                      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                        <span className="font-semibold">Version {e.version}</span>
                        {isLatest && <span className="rounded-full bg-brass/15 px-2 py-0.5 text-[0.8em] font-medium text-brass-ink-hi">Current</span>}
                        {e.otherDevice && <span className="rounded-full bg-raised px-2 py-0.5 text-[0.8em] font-medium text-mist shadow-e1">From another device</span>}
                        <span className="text-mist">{e.event === "edited" && e.restoredFrom ? `Combined with the other device’s version ${e.restoredFrom}` : `${EVENT_LABEL[e.event]}${e.restoredFrom ? ` (version ${e.restoredFrom})` : ""}`}</span>
                      </div>
                      <p className="mt-0.5 text-[0.86em] text-fog" title={exactTime(e.timestamp)}>
                        {exactTime(e.timestamp)}, {relativeTime(e.timestamp)}
                      </p>
                      {e.changedFields.length > 0 && (
                        <p className="mt-1.5 flex flex-wrap gap-1.5">
                          {e.changedFields.map((f) => (
                            <span key={f} className="rounded-full bg-raised px-2.5 py-0.5 text-[0.8em] text-mist shadow-e1">{fieldName(f)}</span>
                          ))}
                        </p>
                      )}
                      {!lifecycle && (
                        <p className="mt-2 flex gap-1.5">
                          <Button className="min-h-8" icon={<Eye className="size-3.5" />} onClick={() => void view(e.version)}>View</Button>
                          {!isLatest && <Button variant="ghost" className="min-h-8 text-brass-ink hover:text-brass-ink-hi" icon={<RotateCcw className="size-3.5" />} onClick={() => void restore(e.version)}>Restore this version</Button>}
                        </p>
                      )}
                    </li>
                  );
                })}
              </ol>
            )}
          </motion.div>
        )}
      </AnimatePresence>
      <Snapshot draft={snap?.draft ?? null} version={snap?.version ?? 0} current={snap?.version === latest} onClose={() => setSnap(null)} onRestore={() => snap && void restore(snap.version)} />
    </section>
  );
}
