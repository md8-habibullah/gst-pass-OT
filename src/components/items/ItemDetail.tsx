import { useCallback, useEffect, useRef, useState } from "react";
import { ArchiveRestore, ArrowLeft, EyeOff, ExternalLink, KeyRound, MoreHorizontal, Paperclip, Pencil, Star } from "lucide-react";
import { TYPE_LABEL, type ItemDraft, type ItemView } from "../../backend";
import { copyText } from "../../lib/clipboard";
import { exactTime, relativeTime } from "../../lib/format";
import { groupSignIns } from "../../lib/instances";
import { shortUrl, urlsOf } from "../../lib/urls";
import { be, pushToast, app } from "../../state/app";
import { useStore } from "../../state/store";
import { withItemPassword } from "../reprompt/Reprompt";
import { Avatar } from "../ui/Avatar";
import { Button, IconButton } from "../ui/Button";
import { CountdownRing } from "../ui/Ring";
import { CopyButton, FieldRow, HiddenPill, PasswordText, RevealToggle, TotpValue } from "./FieldRows";
import { editItem, openLink, setArchived, type Target } from "./actions";
import { openItemMenu } from "./ItemMenu";
import { FileList } from "../files/FileList";
import { PasskeyLine } from "./PasskeyRows";
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
  const syncRevision = useStore(app, (s) => s.syncRevision);
  const [showHistory, setShowHistory] = useState(0);

  // Load (and reload after edits). Switching items always wipes revealed secrets.
  useEffect(() => { setRevealed(null); setView(null); setError(null); }, [id]);
  useEffect(() => {
    let alive = true;
    be().getItem(id).then((v) => alive && setView(v)).catch((e: unknown) => alive && setError(e instanceof Error ? e.message : "Could not open this item."));
    return () => { alive = false; };
  }, [id, version, syncRevision]);
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
    const draft = await withItemPassword({ reason: `Show the secrets for “${title}” for ${REVEAL_MS / 1000} seconds.`, confirmLabel: "Reveal", run: (pw) => be().revealItem(id, pw) });
    if (draft) setRevealed({ draft, until: Date.now() + REVEAL_MS, key: Date.now() });
    return draft;
  }, [revealed, id, title]);

  async function copySecret(label: string, value: string) {
    try {
      await copyText(value, clearAfter);
      pushToast({ kind: "success", text: `${label} copied. The clipboard clears in ${clearAfter} seconds.` });
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

  const target: Target = { id, type: view?.type ?? stored?.type ?? "login", title, archived: stored?.archived ?? false, favorite: view?.favorite ?? false };

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
              {view.favorite && <Star className="size-[18px] shrink-0 fill-brass-ink text-brass-ink" aria-label="Favorite" />}
              {view.reprompt && <KeyRound className="size-[18px] shrink-0 text-brass-ink" aria-label="Asks for the master password" />}
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
          <Button icon={<Pencil className="size-4" />} onClick={() => void editItem(target)}>
            Edit
          </Button>
          <IconButton label="More actions" aria-haspopup="menu" onClick={(e) => openItemMenu({ ...target, folder: view.folder }, e.currentTarget)}>
            <MoreHorizontal className="size-[18px]" />
          </IconButton>
        </div>
      </header>

      {target.archived && (
        <div role="status" className="frosted flex flex-wrap items-center gap-3 rounded-lg border border-hairline-strong px-4 py-2.5">
          <p className="min-w-0 flex-1 text-[0.93em]">This item is archived. It stays encrypted and is hidden from your lists and codes.</p>
          <Button className="min-h-8" icon={<ArchiveRestore className="size-3.5" />} onClick={() => void setArchived(target, false)}>Unarchive</Button>
        </div>
      )}

      {view.changedElsewhere && (
        <div role="status" className="frosted flex flex-wrap items-center gap-3 rounded-lg border border-brass-ink/30 px-4 py-2.5">
          <p className="min-w-0 flex-1 text-[0.93em]">Changed on another device. Where you each changed different things, the changes were combined here. The other device’s own version is saved in the history, so nothing was lost.</p>
          <div className="flex gap-1.5">
            <Button className="min-h-8" onClick={() => setShowHistory((n) => n + 1)}>Show history</Button>
            <Button variant="ghost" className="min-h-8" onClick={() => void be().reviewOtherDevice(id).then(() => be().getItem(id)).then(setView).catch(() => undefined)}>Got it</Button>
          </div>
        </div>
      )}

      {secrets && (
        <div role="status" className="frosted flex items-center gap-3 rounded-lg border border-brass-ink/30 px-4 py-2.5">
          <CountdownRing totalMs={REVEAL_MS} remainingMs={remaining} windowKey={revealed!.key} color="var(--color-brass-ink)" size={28} stroke={3} />
          <p className="flex-1 text-[0.93em]">Secrets hide again in {Math.ceil(remaining / 1000)} seconds.</p>
          <Button className="min-h-8" icon={<EyeOff className="size-3.5" />} onClick={() => setRevealed(null)}>Hide now</Button>
        </div>
      )}

      <div className="surface overflow-hidden rounded-lg">
        {groupSignIns(view.fields.filter((f) => f.hasValue)).map((f) => {
          const shown = secrets?.fields[f.key];
          // A passkey is shown by the website and account it belongs to; its key is never shown, copied or revealed.
          if (f.kind === "passkey") return <FieldRow key={f.key} label={f.label}>{f.passkey ? <PasskeyLine info={f.passkey} /> : <span className="text-mist">A passkey</span>}</FieldRow>;
          if (f.kind === "totp") return <TotpValue key={f.key} id={id} title={title} fieldKey={f.key} label={f.label} onCopy={(c) => copyPlain(f.label, c)} />;
          // Several web addresses: a row each, so any one can be opened or copied.
          if (f.kind === "url" && f.value && urlsOf(f.value).length > 1) {
            const urls = urlsOf(f.value);
            return urls.map((u, n) => (
              <FieldRow
                key={`${f.key}-${n}`}
                label={`${f.label} ${n + 1}`}
                actions={
                  <>
                    <IconButton label={`Open ${shortUrl(u)} in your browser`} onClick={() => void openLink(u)}>
                      <ExternalLink className="size-[18px]" />
                    </IconButton>
                    <CopyButton label={`Copy ${f.label.toLowerCase()} ${n + 1}`} onCopy={() => copyPlain(f.label, u)} />
                  </>
                }
              >
                <span className="block break-all text-brass-ink-hi">{u}</span>
              </FieldRow>
            ));
          }
          if (!f.secret) {
            return (
              <FieldRow
                key={f.key}
                label={f.label}
                actions={
                  f.value ? (
                    <>
                      {f.kind === "url" && (
                        <IconButton label="Open in your browser" onClick={() => void openLink(f.value!)}>
                          <ExternalLink className="size-[18px]" />
                        </IconButton>
                      )}
                      <CopyButton label={`Copy ${f.label.toLowerCase()}`} onCopy={() => copyPlain(f.label, f.value!)} />
                    </>
                  ) : null
                }
              >
                {f.kind === "paragraph" ? (
                  <span className="block font-mono text-[0.9em] leading-6 break-all whitespace-pre-wrap">{f.value}</span>
                ) : (
                  <span className={`block break-all ${f.kind === "url" ? "text-brass-ink-hi" : ""}`}>{f.value}</span>
                )}
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
                  <RevealToggle revealed={shown !== undefined} asks={view.reprompt} onReveal={() => void ensureRevealed()} onHide={() => setRevealed(null)} />
                  <CopyButton
                    label={`Copy ${f.label.toLowerCase()}${view.reprompt ? " (asks for your master password)" : ""}`}
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
                  <RevealToggle revealed={shown !== undefined} asks={view.reprompt} onReveal={() => void ensureRevealed()} onHide={() => setRevealed(null)} />
                ) : (
                  <CopyButton label={`Copy ${c.name.toLowerCase()}`} onCopy={() => copyPlain(c.name, c.value ?? "")} />
                )
              }
            >
              {shown !== undefined && shown !== null ? (c.hidden ? <PasswordText value={shown} /> : <span className="break-all">{shown}</span>) : <HiddenPill />}
            </FieldRow>
          );
        })}
        {!view.fields.some((f) => f.hasValue) && view.customFields.length === 0 && <p className="px-4 py-6 text-mist">This item has no fields yet. Choose Edit to add some.</p>}
      </div>

      <section aria-labelledby={`att-${id}`} className="flex flex-col gap-2 rounded-lg">
        <div className="flex items-center gap-3 px-4 pt-2">
          <Paperclip className="size-[18px] text-mist" aria-hidden />
          <h3 id={`att-${id}`} className="flex-1 font-semibold">Attachments</h3>
        </div>
        <div className="px-1"><FileList itemId={id} emptyText="No files attached to this item." /></div>
      </section>

      <VersionHistory key={id} id={id} title={view.title} version={view.version} stamp={syncRevision} openSignal={showHistory} />
    </article>
  );
}
