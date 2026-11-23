import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowLeft, EyeOff, Paperclip, Pencil, Plus, Star, Trash2 } from "lucide-react";
import { BackendError, TYPE_LABEL, type ItemDraft, type ItemView } from "../../backend";
import { copyText } from "../../lib/clipboard";
import { exactTime, formatBytes, relativeTime } from "../../lib/format";
import { be, pushToast, refresh, select, app } from "../../state/app";
import { useStore } from "../../state/store";
import { openEditor } from "../../state/ui";
import { reprompt } from "../reprompt/Reprompt";
import { Avatar } from "../ui/Avatar";
import { Button, IconButton } from "../ui/Button";
import { CountdownRing } from "../ui/Ring";
import { CopyButton, FieldRow, HiddenPill, PasswordText, RevealToggle, TotpValue } from "./FieldRows";
import { VersionHistory } from "./VersionHistory";

/** How long revealed secrets stay in the UI before they are wiped (spec: nothing lingers). */
const REVEAL_MS = 20_000;

interface Revealed {
  draft: ItemDraft;
  until: number;
  key: number;
}

/** Item detail: header, one grouped credentials panel, attachments and the version history. */
export function ItemDetail({ id, onBack }: { id: string; onBack?: () => void }) {
  const stored = useStore(app, (s) => s.items.find((i) => i.id === id));
  const clearAfter = useStore(app, (s) => s.clipboardClear);
  const [view, setView] = useState<ItemView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState<Revealed | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const version = stored?.version;

  // Load (and reload after edits). Switching items always wipes revealed secrets.
  useEffect(() => { setRevealed(null); setView(null); setError(null); }, [id]);
  useEffect(() => {
    let alive = true;
    be().getItem(id).then((v) => alive && setView(v)).catch((e: unknown) => alive && setError(e instanceof Error ? e.message : "Could not open this item."));
    return () => { alive = false; };
  }, [id, version]);
  useEffect(() => { if (version !== undefined) setRevealed(null); }, [version]);

  // Auto-hide: one timer for the whole reveal window.
  const hideTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    clearTimeout(hideTimer.current);
    if (!revealed) return;
    hideTimer.current = setTimeout(() => setRevealed(null), Math.max(0, revealed.until - Date.now()));
    const tick = setInterval(() => setNow(Date.now()), 500);
    return () => { clearTimeout(hideTimer.current); clearInterval(tick); };
  }, [revealed]);

  const title = view?.title ?? stored?.title ?? "";

  const ensureRevealed = useCallback(async (): Promise<ItemDraft | undefined> => {
    if (revealed && revealed.until > Date.now()) return revealed.draft;
    const draft = await reprompt({ reason: `Show the secrets for “${title}” for ${REVEAL_MS / 1000} seconds.`, confirmLabel: "Reveal", run: (pw) => be().revealItem(id, pw) });
    if (draft) setRevealed({ draft, until: Date.now() + REVEAL_MS, key: Date.now() });
    return draft;
  }, [revealed, id, title]);

  async function copySecret(label: string, value: string) {
    try {
      await copyText(value, clearAfter);
      pushToast({ kind: "success", text: `${label} copied. The clipboard clears in ${clearAfter} seconds.`, countdownMs: clearAfter * 1000 });
    } catch {
      pushToast({ kind: "error", text: "Copying was blocked. Reveal the value and copy it by hand." });
    }
  }
  async function copyPlain(label: string, value: string) {
    try {
      await copyText(value);
      pushToast({ kind: "success", text: `${label} copied.` });
    } catch {
      pushToast({ kind: "error", text: "Copying was blocked on this device." });
    }
  }

  async function edit() {
    const session = await reprompt({ reason: `Edit “${title}”. Previous versions stay in the history.`, confirmLabel: "Edit item", run: (pw) => be().beginEdit(id, pw) });
    if (session) openEditor(id, session);
  }

  async function trash() {
    try {
      await be().trashItem(id);
      await refresh();
      select(null);
      onBack?.();
      pushToast({
        kind: "info",
        text: `“${title}” moved to the trash.`,
        ttlMs: 6000,
        action: { label: "Undo", run: () => void be().restoreItem(id).then(refresh).then(() => pushToast({ kind: "success", text: `“${title}” is back in your vault.` })) },
      });
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof BackendError ? e.message : "Could not move the item to the trash." });
    }
  }

  if (error) return <p role="alert" className="p-8 text-ember">{error}</p>;
  if (!view) return <div className="p-8 text-mist" aria-busy>Opening.</div>;

  const secrets = revealed?.draft;
  const remaining = revealed ? Math.max(0, revealed.until - now) : 0;

  return (
    <article aria-label={view.title} className="detail-root mx-auto flex w-full max-w-[40rem] flex-col gap-5 px-5 pt-5 pb-28 min-[720px]:pb-10">
      <header className="detail-head" data-back={onBack ? "" : undefined}>
        {onBack && (
          <IconButton data-slot="back" label="Back to list" onClick={onBack}>
            <ArrowLeft className="size-5" />
          </IconButton>
        )}
        <div data-slot="ident" className="flex min-w-0 items-start gap-4">
          <Avatar title={view.title} size={56} />
          <div className="min-w-0 flex-1">
            <h2 className="flex items-center gap-2 font-display text-[1.65rem] leading-[1.15] font-semibold tracking-[-0.01em] break-words">
              {view.title}
              {view.favorite && <Star className="size-[18px] shrink-0 fill-brass text-brass" aria-label="Favorite" />}
            </h2>
            <p className="mt-1 text-[0.93em] text-mist" title={exactTime(view.updatedAt)}>
              {TYPE_LABEL[view.type]}, edited {relativeTime(view.updatedAt)}
            </p>
            {(view.tags.length > 0 || view.folder) && (
              <p className="mt-2 flex flex-wrap gap-1.5">
                {view.folder && <span className="rounded-full border border-hairline-strong px-2.5 py-0.5 text-[0.8em] text-mist">{view.folder}</span>}
                {view.tags.map((t) => <span key={t} className="rounded-full bg-raised px-2.5 py-0.5 text-[0.8em] text-mist shadow-e1">{t}</span>)}
              </p>
            )}
          </div>
        </div>
        <div data-slot="actions" className="flex shrink-0 gap-1.5">
          <Button icon={<Pencil className="size-4" />} onClick={() => void edit()}>
            Edit
          </Button>
          <IconButton label="Move to trash" tone="danger" onClick={() => void trash()}>
            <Trash2 className="size-[18px]" />
          </IconButton>
        </div>
      </header>

      {secrets && (
        <div role="status" className="frosted flex items-center gap-3 rounded-lg border border-brass/30 px-4 py-2.5">
          <CountdownRing totalMs={REVEAL_MS} remainingMs={remaining} windowKey={revealed!.key} color="var(--color-brass)" size={28} stroke={3} />
          <p className="flex-1 text-[0.93em]">Secrets hide again in {Math.ceil(remaining / 1000)} seconds.</p>
          <Button className="min-h-8" icon={<EyeOff className="size-3.5" />} onClick={() => setRevealed(null)}>Hide now</Button>
        </div>
      )}

      <div className="glass overflow-hidden rounded-lg">
        {view.fields.map((f) => {
          const shown = secrets?.fields[f.key];
          if (f.kind === "totp") return <TotpValue key={f.key} id={id} onCopy={(c) => copyPlain("One-time code", c)} />;
          if (!f.secret) {
            return (
              <FieldRow key={f.key} label={f.label} actions={f.value ? <CopyButton label={`Copy ${f.label.toLowerCase()}`} onCopy={() => copyPlain(f.label, f.value!)} /> : null}>
                <span className={`block break-all ${f.kind === "url" ? "text-brass-hi" : ""}`}>{f.value}</span>
              </FieldRow>
            );
          }
          const isNote = f.kind === "note";
          return (
            <FieldRow
              key={f.key}
              label={f.label}
              actions={
                <>
                  <RevealToggle revealed={shown !== undefined} onReveal={() => void ensureRevealed()} onHide={() => setRevealed(null)} />
                  <CopyButton
                    label={`Copy ${f.label.toLowerCase()} (asks for your master password)`}
                    onCopy={async () => {
                      const d = await ensureRevealed();
                      const value = d?.fields[f.key];
                      if (!value) throw new Error("cancelled");
                      await copySecret(f.label, value);
                    }}
                  />
                </>
              }
            >
              {shown !== undefined ? (
                isNote ? <span className="block font-mono text-[0.97em] leading-6 break-words whitespace-pre-wrap">{shown}</span> : <PasswordText value={shown} />
              ) : (
                <HiddenPill note={isNote} />
              )}
            </FieldRow>
          );
        })}
        {view.customFields.map((c, i) => {
          const shown = c.hidden ? secrets?.customFields.find((x) => x.name === c.name)?.value : c.value;
          return (
            <FieldRow
              key={`${c.name}-${i}`}
              label={c.name}
              actions={
                c.hidden ? (
                  <RevealToggle revealed={shown !== undefined} onReveal={() => void ensureRevealed()} onHide={() => setRevealed(null)} />
                ) : (
                  <CopyButton label={`Copy ${c.name.toLowerCase()}`} onCopy={() => copyPlain(c.name, c.value ?? "")} />
                )
              }
            >
              {shown !== undefined && shown !== null ? (c.hidden ? <PasswordText value={shown} /> : <span className="break-all">{shown}</span>) : <HiddenPill />}
            </FieldRow>
          );
        })}
        {view.fields.length === 0 && view.customFields.length === 0 && <p className="px-4 py-6 text-mist">This item has no fields yet. Choose Edit to add some.</p>}
      </div>

      <section aria-labelledby={`att-${id}`} className="rounded-lg">
        <div className="flex items-center gap-3 px-4 py-2">
          <Paperclip className="size-[18px] text-mist" aria-hidden />
          <h3 id={`att-${id}`} className="flex-1 font-semibold">Attachments</h3>
          <Button className="min-h-8" icon={<Plus className="size-3.5" />} disabled title="Encrypted attachments arrive with media support">
            Add file
          </Button>
        </div>
        {view.attachments.length === 0 ? (
          <p className="px-4 pb-2 text-[0.93em] text-fog">No files attached. Encrypted attachments arrive with media support.</p>
        ) : (
          <ul className="px-2">
            {view.attachments.map((a) => (
              <li key={a.id} className="flex items-center gap-3 rounded-lg px-2 py-2">
                <span className="grid size-9 place-items-center rounded-sm bg-raised"><Paperclip className="size-4 text-mist" aria-hidden /></span>
                <span className="min-w-0 flex-1 truncate">{a.name}</span>
                <span className="text-[0.86em] text-fog">{formatBytes(a.size)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <VersionHistory key={id} id={id} title={view.title} version={view.version} />
    </article>
  );
}
