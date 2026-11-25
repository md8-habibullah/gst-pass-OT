import { useCallback, useEffect, useState, type KeyboardEvent } from "react";
import { Plus, Trash2, X } from "lucide-react";
import { BackendError, FIELD_SCHEMA, specOf, TYPE_LABEL, type FieldSpec, type ItemDraft, type ItemType } from "../../backend";
import { CODE, compactSlots, type Fields, PASSKEY, SIGN_IN, withoutBlanks } from "../../lib/instances";
import { parseOtpInput, type OtpAccount } from "../../lib/otp";
import { app, be, pushToast, refresh, select, setFilter } from "../../state/app";
import { useStore } from "../../state/store";
import { closeEditor, ui } from "../../state/ui";
import { Button, IconButton } from "../ui/Button";
import { Field, Switch, TextArea, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";
import { FolderField } from "./FolderSheets";
import { CodeRows, SignInRows } from "./CredentialRows";
import { PasskeyRows } from "./PasskeyRows";
import { SecretField } from "./SecretField";
import { TotpInput } from "./TotpInput";
import { UrlListInput } from "./UrlListInput";
import { brandName } from "../../state/brand";
import { TYPE_META, TYPE_ORDER } from "./typeMeta";

/** What the editor compares to see whether anything changed: blank boxes (an empty extra sign-in) are not a change. */
const canon = (d: ItemDraft | null): string => (d ? JSON.stringify({ ...d, fields: withoutBlanks(d.fields) }) : "");

const blank = (type: ItemType): ItemDraft => ({ type, title: "", favorite: false, reprompt: false, tags: [], folder: null, fields: {}, customFields: [] });

const TITLE_HINT: Record<ItemType, string> = {
  login: "GitHub",
  card: "Personal Visa",
  identity: "Passport and address",
  sshKey: "Work laptop key",
  secureNote: "Wi-Fi at the cabin",
  totp: "GitHub",
  media: "Give it a name",
};

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
    <div className="flex flex-wrap items-center gap-1.5 rounded-sm border border-hairline bg-raised p-1.5 shadow-e1 focus-within:border-brass-ink/70">
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
  const inDemo = useStore(app, (s) => s.profile?.demo === true);
  const [draft, setDraft] = useState<ItemDraft | null>(null);
  const [initial, setInitial] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [discard, setDiscard] = useState(false);

  // (Re)initialise whenever a request arrives.
  useEffect(() => {
    setError(null); setDiscard(false); setBusy(false);
    if (!req) { setDraft(null); return; }
    const d = req.mode === "edit" ? structuredClone(req.session.draft) : req.type ? blank(req.type) : null;
    setInitial(canon(d));
    // A scan can hand over a website or an export to start from; that counts as a change, so closing asks first.
    setDraft(d && req.mode === "new" && req.prefill ? { ...d, fields: { ...d.fields, ...req.prefill } } : d);
  }, [req]);

  // An old message about a box stops being true once the person edits anything.
  useEffect(() => { setError(null); }, [draft]);

  const dirty = draft !== null && canon(draft) !== initial;
  const specs = draft ? FIELD_SCHEMA[draft.type] : [];
  const editing = req?.mode === "edit";
  const title = !req ? "" : editing ? `Edit ${req.session.draft.title}` : draft ? `New ${TYPE_LABEL[draft.type].toLowerCase()}` : "New item";
  const totpRaw = draft?.fields.totp ?? "";
  const exportParse = draft?.type === "totp" && !editing ? parseOtpInput(totpRaw) : null;
  const splitCount = exportParse?.kind === "export" ? exportParse.accounts.length : 0;

  const set = <K extends keyof ItemDraft>(k: K, v: ItemDraft[K]) => setDraft((d) => (d ? { ...d, [k]: v } : d));
  const setField = (key: string, v: string) => setDraft((d) => (d ? { ...d, fields: { ...d.fields, [key]: v } } : d));
  const editFields = (change: (f: Fields) => Fields) => setDraft((d) => (d ? { ...d, fields: change(d.fields) } : d));

  /** A link or QR code that names the service fills the empty service, account and title boxes of a code-only item. */
  const onIdentified = useCallback((a: OtpAccount) => {
    setDraft((d) => {
      if (!d || d.type !== "totp") return d;
      const fields = { ...d.fields };
      if (a.issuer && !fields.issuer?.trim()) fields.issuer = a.issuer;
      if (a.account && !fields.account?.trim()) fields.account = a.account;
      return { ...d, fields, title: d.title.trim() ? d.title : (a.issuer ?? a.account ?? "") };
    });
  }, []);

  const requestClose = () => (dirty && !busy ? setDiscard(true) : closeEditor());

  async function save() {
    if (!draft || !req || busy) return;
    // A login's sign-ins and codes are renumbered 1, 2, 3 without the blank ones, so the first is always the plain key.
    let fields: Fields = { ...draft.fields };
    if (draft.type === "login") fields = compactSlots(compactSlots(compactSlots(fields, SIGN_IN), CODE), PASSKEY);
    let split: OtpAccount[] | null = null;

    // One-time codes: accept any form, store a clean value, explain anything that cannot work.
    if (draft.type === "totp" && !(fields.totp ?? "").trim()) return setError("Add the setup key, an otpauth link, or scan the QR code.");
    const codeKeys = Object.keys(fields).filter((k) => specOf(draft.type, k)?.kind === "totp");
    for (const key of codeKeys) {
      const raw = (fields[key] ?? "").trim();
      if (!raw) { delete fields[key]; continue; }
      const parsed = parseOtpInput(raw);
      const which = codeKeys.length > 1 ? `One-time code ${codeKeys.indexOf(key) + 1}: ` : "";
      if (parsed.kind === "error") return setError(`${which}${parsed.message}`);
      if (parsed.kind === "export") {
        if (draft.type !== "totp" || editing) return setError(`${which}That is a multi-account export. Add it from New, Code only, and ${brandName()} will split it into one item per account.`);
        split = parsed.accounts;
      } else if (parsed.kind === "ok") {
        fields[key] = parsed.account.value;
      }
    }
    if (!split && !draft.title.trim()) return setError("Give the item a title.");

    setBusy(true);
    setError(null);
    try {
      const body: ItemDraft = { ...draft, fields, title: draft.title.trim(), folder: draft.folder?.trim() || null };
      if (split) {
        for (const a of split) {
          await be().createItem({
            ...body,
            title: (a.issuer ?? a.account ?? "Code").slice(0, 256),
            fields: { ...(a.issuer ? { issuer: a.issuer } : {}), ...(a.account ? { account: a.account } : {}), totp: a.value },
          });
        }
        await refresh();
        setFilter("codes");
        closeEditor();
        pushToast({ kind: "success", text: `${split.length} codes added. They are in Codes now.` });
      } else if (req.mode === "edit") {
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

  // The footer never scrolls away, so a message about saving or cancelling is always in view, even on a phone.
  const footer = (
    <div className="flex flex-col gap-3">
      {inDemo && <p role="note" className="text-[0.86em] text-mist">This is the demo vault. Anyone who opens Hexis can read what you save here. Create your own vault to keep real data.</p>}
      {error && <p role="alert" className="rounded-lg border border-ember/40 bg-danger/10 px-4 py-3 text-[0.95em]">{error}</p>}
      {discard ? (
        <div role="alertdialog" aria-label="Discard changes" className="flex flex-wrap items-center justify-end gap-2">
          <p className="min-w-0 flex-1 font-medium">Discard your changes?</p>
          <Button autoFocus onClick={() => setDiscard(false)}>Keep editing</Button>
          <Button variant="danger" onClick={closeEditor}>Discard</Button>
        </div>
      ) : (
        <div className="flex items-center justify-end gap-2">
          <Button variant="ghost" onClick={requestClose} disabled={busy}>Cancel</Button>
          <Button variant="key" busy={busy} disabled={!draft || (editing && !dirty)} onClick={() => void save()}>
            {editing ? "Save changes" : splitCount > 1 ? `Add ${splitCount} codes` : "Add item"}
          </Button>
        </div>
      )}
    </div>
  );

  const renderSpec = (s: FieldSpec) => {
    if (!draft) return null;
    const value = draft.fields[s.key] ?? "";
    if (s.kind === "totp") {
      return <TotpInput key={s.key} label={s.label} value={value} onChange={(v) => setField(s.key, v)} canSplit={draft.type === "totp" && !editing} onIdentified={onIdentified} />;
    }
    if (s.kind === "password") return <SecretField key={s.key} spec={s} value={value} onChange={(v) => setField(s.key, v)} itemType={draft.type} />;
    if (s.kind === "note" || s.kind === "paragraph") {
      const mono = draft.type === "sshKey" || draft.type === "secureNote";
      return (
        <Field key={s.key} label={s.label}>
          {({ id }) => <TextArea id={id} mono={mono} rows={draft.type === "secureNote" ? 9 : s.key === "privateKey" ? 6 : 4} maxLength={65536} value={value} placeholder={s.placeholder} onChange={(e) => setField(s.key, e.target.value)} />}
        </Field>
      );
    }
    // Web addresses: one box each and a button for another (a login can have several).
    if (s.kind === "url") return <UrlListInput key={s.key} label={s.label} value={value} placeholder={s.placeholder} onChange={(v) => setField(s.key, v)} />;
    return (
      <Field key={s.key} label={s.label}>
        {({ id, describedBy }) => <TextInput id={id} aria-describedby={describedBy} value={value} placeholder={s.placeholder} onChange={(e) => setField(s.key, e.target.value)} maxLength={65536} />}
      </Field>
    );
  };

  const body = draft ? specs.flatMap((s, i) => {
    // A login's sign-ins and codes can repeat: one panel each, with "Add another".
    if (draft.type === "login") {
      if (s.key === "username") return [<SignInRows key="signins" fields={draft.fields} edit={editFields} itemType="login" />];
      if (s.key === "password") return [];
      if (s.kind === "totp") return [<CodeRows key="codes" fields={draft.fields} edit={editFields} />];
      if (s.kind === "passkey") return [<PasskeyRows key="passkeys" fields={draft.fields} edit={editFields} />];
    }
    // Expiry month and year sit side by side.
    if (s.key === "expMonth") {
      const year = specs[i + 1];
      return [
        <div key="exp" className="grid grid-cols-2 gap-3">
          {renderSpec(s)}
          {year && renderSpec(year)}
        </div>,
      ];
    }
    if (s.key === "expYear") return [];
    return [renderSpec(s)];
  }) : [];

  return (
    <Sheet open={req !== null} onClose={requestClose} title={title} variant="form" size="md" footer={draft ? footer : undefined} dismissible={!busy}>
      {req && !draft && (
        <div className="flex flex-col gap-2.5 pt-1">
          <p className="text-mist">What would you like to store?</p>
          {TYPE_ORDER.map((type) => {
            const { icon: Icon, hint } = TYPE_META[type];
            return (
              <button key={type} type="button" onClick={() => { const d = blank(type); setDraft(d); setInitial(canon(d)); }} className="pressable group flex items-center gap-4 rounded-lg border border-hairline bg-raised px-4 py-3 text-left shadow-e1 hover:bg-hover pointer-coarse:py-3.5">
                <span className="grid size-11 shrink-0 place-items-center rounded-lg bg-pane shadow-e1 group-hover:text-brass-ink"><Icon className="size-5" aria-hidden /></span>
                <span className="min-w-0">
                  <span className="block font-semibold">{TYPE_LABEL[type]}</span>
                  <span className="block text-[0.93em] text-mist">{hint}</span>
                </span>
              </button>
            );
          })}
        </div>
      )}

      {draft && (
        <form className="flex flex-col gap-4 pt-1" onSubmit={(e) => { e.preventDefault(); void save(); }}>
          {splitCount <= 1 && (
            <Field label="Title">
              {({ id }) => <TextInput id={id} data-autofocus={!editing || undefined} value={draft.title} onChange={(e) => set("title", e.target.value)} maxLength={256} placeholder={TITLE_HINT[draft.type]} />}
            </Field>
          )}

          {splitCount > 1 ? (
            <>
              <TotpInput label="One-time codes" value={totpRaw} onChange={(v) => setField("totp", v)} canSplit />
              <ul className="flex flex-col gap-1 rounded-lg bg-pane/70 p-2 shadow-e1" aria-label="Accounts in this export">
                {exportParse?.kind === "export" && exportParse.accounts.map((a, i) => (
                  <li key={i} className="flex min-h-10 items-center gap-3 rounded-sm px-2">
                    <span className="min-w-0 flex-1 truncate"><span className="font-medium">{a.issuer ?? "Unnamed"}</span>{a.account && <span className="text-mist">, {a.account}</span>}</span>
                    <span className="text-[0.86em] text-fog">{a.digits} digits</span>
                  </li>
                ))}
              </ul>
            </>
          ) : body}

          {draft.type !== "totp" || splitCount <= 1 ? (
            <>
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
            </>
          ) : null}

          <Field label="Tags">{() => <TagInput tags={draft.tags} onChange={(t) => set("tags", t)} />}</Field>
          <FolderField value={draft.folder ?? ""} onChange={(path) => set("folder", path || null)} />
          {splitCount <= 1 && (
            <div className="flex flex-col gap-2">
              <div className="flex min-h-11 items-center justify-between gap-4 rounded-lg bg-pane/70 px-4 shadow-e1">
                <span className="font-medium">Favorite</span>
                <Switch label="Favorite" checked={draft.favorite} onChange={(v) => set("favorite", v)} />
              </div>
              <div className="flex min-h-11 items-center justify-between gap-4 rounded-lg bg-pane/70 px-4 py-2.5 shadow-e1">
                <span className="min-w-0">
                  <span className="block font-medium">Ask for the master password again</span>
                  <span className="block text-[0.86em] text-mist">Needed each time you show, copy, edit or clone this item, even while the vault is open.</span>
                </span>
                <Switch label="Ask for the master password again" checked={draft.reprompt} onChange={(v) => set("reprompt", v)} />
              </div>
            </div>
          )}
          <button type="submit" hidden />
        </form>
      )}
    </Sheet>
  );
}
