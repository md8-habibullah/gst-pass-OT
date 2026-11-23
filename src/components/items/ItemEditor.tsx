import { useEffect, useState, type KeyboardEvent } from "react";
import { Eye, EyeOff, Image, KeyRound, Plus, StickyNote, Timer, Trash2, Wand2, X, type LucideIcon } from "lucide-react";
import { BackendError, FIELD_SCHEMA, TYPE_LABEL, type ItemDraft, type ItemType } from "../../backend";
import { be, pushToast, refresh, select } from "../../state/app";
import { useStore } from "../../state/store";
import { closeEditor, ui } from "../../state/ui";
import { StrengthMeter } from "../lock/StrengthMeter";
import { Button, IconButton } from "../ui/Button";
import { Field, Switch, TextArea, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";
import { PasswordGenerator } from "./PasswordGenerator";

const TYPES: { type: ItemType; icon: LucideIcon; hint: string }[] = [
  { type: "login", icon: KeyRound, hint: "A username and password, with an optional one-time code." },
  { type: "totp", icon: Timer, hint: "Just a one-time code, for services where you keep the password elsewhere." },
  { type: "secureNote", icon: StickyNote, hint: "Private text: recovery codes, Wi-Fi details, anything you would not email." },
  { type: "media", icon: Image, hint: "A description for a document or image. File attachments arrive with media support." },
];

const blank = (type: ItemType): ItemDraft => ({ type, title: "", favorite: false, tags: [], folder: null, fields: {}, customFields: [] });

function TagInput({ tags, onChange }: { tags: string[]; onChange: (t: string[]) => void }) {
  const [text, setText] = useState("");
  const add = (raw: string) => {
    const t = raw.trim().replace(/,$/, "").trim();
    if (t && !tags.includes(t) && tags.length < 32) onChange([...tags, t.slice(0, 64)]);
    setText("");
  };
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" || e.key === ",") { e.preventDefault(); add(text); }
    else if (e.key === "Backspace" && !text && tags.length) onChange(tags.slice(0, -1));
  };
  return (
    <div className="flex flex-wrap items-center gap-1.5 rounded-sm border border-hairline bg-raised p-1.5 shadow-e1 focus-within:border-brass/70">
      {tags.map((t) => (
        <span key={t} className="inline-flex items-center gap-1 rounded-full bg-hover py-0.5 pr-1 pl-2.5 text-[0.9em]">
          {t}
          <button type="button" aria-label={`Remove tag ${t}`} className="grid size-5 place-items-center rounded-full text-mist hover:bg-pane hover:text-frost" onClick={() => onChange(tags.filter((x) => x !== t))}>
            <X className="size-3" />
          </button>
        </span>
      ))}
      <input
        aria-label="Add a tag"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKey}
        onBlur={() => add(text)}
        placeholder={tags.length ? "" : "work, finance"}
        className="min-h-8 min-w-24 flex-1 bg-transparent px-2 outline-none placeholder:text-fog pointer-coarse:min-h-10"
      />
    </div>
  );
}

/** Add and edit sheet (right drawer on desktop, bottom sheet on Android). */
export function ItemEditor() {
  const req = useStore(ui, (s) => s.editor);
  const [draft, setDraft] = useState<ItemDraft | null>(null);
  const [initial, setInitial] = useState("");
  const [showPw, setShowPw] = useState(false);
  const [gen, setGen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [discard, setDiscard] = useState(false);

  // (Re)initialise whenever a request arrives.
  useEffect(() => {
    setError(null); setDiscard(false); setShowPw(false); setGen(false); setBusy(false);
    if (!req) { setDraft(null); return; }
    const d = req.mode === "edit" ? structuredClone(req.session.draft) : req.type ? blank(req.type) : null;
    setDraft(d);
    setInitial(JSON.stringify(d));
  }, [req]);

  const dirty = draft !== null && JSON.stringify(draft) !== initial;
  const specs = draft ? FIELD_SCHEMA[draft.type] : [];
  const editing = req?.mode === "edit";
  const title = !req ? "" : editing ? `Edit ${req.session.draft.title}` : draft ? `New ${TYPE_LABEL[draft.type].toLowerCase()}` : "New item";

  const set = <K extends keyof ItemDraft>(k: K, v: ItemDraft[K]) => setDraft((d) => (d ? { ...d, [k]: v } : d));
  const setField = (key: string, v: string) => setDraft((d) => (d ? { ...d, fields: { ...d.fields, [key]: v } } : d));

  const requestClose = () => (dirty && !busy ? setDiscard(true) : closeEditor());

  async function save() {
    if (!draft || !req || busy) return;
    if (!draft.title.trim()) return setError("Give the item a title.");
    setBusy(true);
    setError(null);
    try {
      const body: ItemDraft = { ...draft, title: draft.title.trim(), folder: draft.folder?.trim() || null };
      if (req.mode === "edit") {
        const s = await be().updateItem(req.id, req.session.token, req.session.version, body);
        await refresh();
        closeEditor();
        pushToast({ kind: "success", text: `Saved. “${s.title}” is now version ${s.version}; the earlier version stays in the history.` });
      } else {
        const s = await be().createItem(body);
        await refresh();
        select(s.id);
        closeEditor();
        pushToast({ kind: "success", text: `“${s.title}” added to your vault.` });
      }
    } catch (e) {
      setBusy(false);
      if (e instanceof BackendError && (e.code === "token_invalid" || e.code === "version_conflict")) {
        setError(`${e.message} Close this editor and choose Edit again; your changes here were not saved.`);
      } else {
        setError(e instanceof Error ? e.message : "Could not save this item.");
      }
    }
  }

  const footer = (
    <div className="flex items-center justify-end gap-2">
      <Button variant="ghost" onClick={requestClose} disabled={busy}>Cancel</Button>
      <Button variant="key" busy={busy} disabled={!draft || (editing && !dirty)} onClick={() => void save()}>
        {editing ? "Save changes" : "Add item"}
      </Button>
    </div>
  );

  return (
    <Sheet open={req !== null} onClose={requestClose} title={title} variant="form" size="md" footer={draft ? footer : undefined} dismissible={!busy}>
      {req && !draft && (
        <div className="flex flex-col gap-2.5 pt-1">
          <p className="text-mist">What would you like to store?</p>
          {TYPES.map(({ type, icon: Icon, hint }) => (
            <button key={type} type="button" onClick={() => { const d = blank(type); setDraft(d); setInitial(JSON.stringify(d)); }} className="pressable group flex items-center gap-4 rounded-lg border border-hairline bg-raised px-4 py-3.5 text-left shadow-e1 hover:bg-hover pointer-coarse:py-4">
              <span className="grid size-11 shrink-0 place-items-center rounded-lg bg-pane shadow-e1 group-hover:text-brass"><Icon className="size-5" aria-hidden /></span>
              <span>
                <span className="block font-semibold">{TYPE_LABEL[type]}</span>
                <span className="block text-[0.93em] text-mist">{hint}</span>
              </span>
            </button>
          ))}
        </div>
      )}

      {draft && (
        <form className="flex flex-col gap-4 pt-1" onSubmit={(e) => { e.preventDefault(); void save(); }}>
          {discard && (
            <div role="alertdialog" aria-label="Discard changes" className="flex flex-wrap items-center gap-3 rounded-lg border border-ember/40 bg-danger/10 px-4 py-3">
              <p className="min-w-0 flex-1 text-[0.95em]">Discard your changes?</p>
              <Button onClick={() => setDiscard(false)}>Keep editing</Button>
              <Button variant="danger" onClick={closeEditor}>Discard</Button>
            </div>
          )}
          {error && <p role="alert" className="rounded-lg border border-ember/40 bg-danger/10 px-4 py-3 text-[0.95em]">{error}</p>}

          <Field label="Title">
            {({ id }) => <TextInput id={id} data-autofocus={!editing || undefined} value={draft.title} onChange={(e) => set("title", e.target.value)} maxLength={256} placeholder={draft.type === "login" ? "GitHub" : "Give it a name"} />}
          </Field>

          {specs.map((s) => {
            const value = draft.fields[s.key] ?? "";
            if (s.kind === "password") {
              return (
                <div key={s.key} className="flex flex-col gap-2.5">
                  <Field
                    label={s.label}
                    trailing={
                      <button type="button" onClick={() => setGen((g) => !g)} aria-expanded={gen} className="pressable inline-flex items-center gap-1.5 rounded-sm px-1.5 py-1 text-[0.86em] font-semibold text-brass hover:text-brass-hi pointer-coarse:min-h-10">
                        <Wand2 className="size-3.5" aria-hidden /> Generate
                      </button>
                    }
                  >
                    {({ id }) => (
                      <div className="relative">
                        <TextInput id={id} mono type={showPw ? "text" : "password"} autoComplete="new-password" value={value} onChange={(e) => setField(s.key, e.target.value)} className="pr-11" />
                        <IconButton label={showPw ? "Hide password" : "Show password"} size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => setShowPw((v) => !v)}>
                          {showPw ? <EyeOff className="size-[17px]" /> : <Eye className="size-[17px]" />}
                        </IconButton>
                      </div>
                    )}
                  </Field>
                  {value && <StrengthMeter password={value} />}
                  {gen && <PasswordGenerator onUse={(p) => { setField(s.key, p); setShowPw(true); setGen(false); }} />}
                </div>
              );
            }
            if (s.kind === "note") {
              return (
                <Field key={s.key} label={s.label}>
                  {({ id }) => <TextArea id={id} mono rows={draft.type === "secureNote" ? 9 : 4} maxLength={65536} value={value} onChange={(e) => setField(s.key, e.target.value)} />}
                </Field>
              );
            }
            return (
              <Field key={s.key} label={s.label} hint={s.kind === "totp" ? "The code is generated on this device. The setup key is never shown again after saving." : undefined}>
                {({ id, describedBy }) => <TextInput id={id} aria-describedby={describedBy} mono={s.kind === "totp"} type={s.kind === "url" ? "url" : "text"} inputMode={s.kind === "url" ? "url" : undefined} value={value} placeholder={s.placeholder} onChange={(e) => setField(s.key, e.target.value)} maxLength={65536} />}
              </Field>
            );
          })}

          <fieldset className="flex flex-col gap-3">
            <legend className="mb-1 text-[0.86em] font-medium text-mist">Custom fields</legend>
            {draft.customFields.map((c, i) => (
              <div key={i} className="grid grid-cols-[1fr_auto] gap-2 rounded-lg bg-pane/70 p-3 shadow-e1">
                <TextInput aria-label={`Custom field ${i + 1} name`} placeholder="Name" value={c.name} onChange={(e) => set("customFields", draft.customFields.map((x, j) => (j === i ? { ...x, name: e.target.value } : x)))} maxLength={64} />
                <IconButton label={`Remove custom field ${c.name || i + 1}`} tone="danger" onClick={() => set("customFields", draft.customFields.filter((_, j) => j !== i))}>
                  <Trash2 className="size-[18px]" />
                </IconButton>
                <TextInput aria-label={`Custom field ${i + 1} value`} placeholder="Value" mono type={c.hidden ? "password" : "text"} value={c.value} onChange={(e) => set("customFields", draft.customFields.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))} maxLength={65536} />
                <div className="flex items-center gap-2 text-[0.86em] text-mist">
                  <Switch label={`Hide value of ${c.name || "custom field"}`} checked={c.hidden} onChange={(v) => set("customFields", draft.customFields.map((x, j) => (j === i ? { ...x, hidden: v } : x)))} />
                  Hide value
                </div>
              </div>
            ))}
            {draft.customFields.length < 50 && (
              <Button className="self-start" icon={<Plus className="size-4" />} onClick={() => set("customFields", [...draft.customFields, { name: "", value: "", hidden: false }])}>
                Add custom field
              </Button>
            )}
          </fieldset>

          <Field label="Tags">{() => <TagInput tags={draft.tags} onChange={(t) => set("tags", t)} />}</Field>
          <Field label="Folder">
            {({ id }) => <TextInput id={id} value={draft.folder ?? ""} onChange={(e) => set("folder", e.target.value || null)} maxLength={64} placeholder="Optional" />}
          </Field>
          <div className="flex min-h-11 items-center justify-between gap-4 rounded-lg bg-pane/70 px-4 shadow-e1">
            <span className="font-medium">Favorite</span>
            <Switch label="Favorite" checked={draft.favorite} onChange={(v) => set("favorite", v)} />
          </div>
          <button type="submit" hidden />
        </form>
      )}
    </Sheet>
  );
}
