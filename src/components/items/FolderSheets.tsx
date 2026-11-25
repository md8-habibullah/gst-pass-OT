import { useMemo, useState, type FormEvent } from "react";
import { Check, Folder, FolderPlus } from "lucide-react";
import { allFolders, cleanFolder, folderName, folderProblem, joinFolder, parentFolder, FOLDER_SEP, isInside } from "../../lib/folders";
import { plural } from "../../lib/format";
import { app, createFolder, moveItems, pushToast, removeFolder, renameFolder } from "../../state/app";
import { useStore } from "../../state/store";
import { closeFolderDialog, ui, type FolderDialog } from "../../state/ui";
import { Button } from "../ui/Button";
import { Field, Select, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";

/** Every folder that exists, for a list or a chooser. */
export function useFolderList(): string[] {
  const explicit = useStore(app, (s) => s.folders);
  const items = useStore(app, (s) => s.items);
  return useMemo(() => allFolders(explicit, items), [explicit, items]);
}

/** "Work / Clients" for a folder path, indented by how deep it is, for a plain select list. */
export function folderOptionLabel(path: string): string {
  const depth = path.split(FOLDER_SEP).length - 1;
  return `${"\u00A0\u00A0".repeat(depth)}${folderName(path)}`;
}

const message = (e: unknown, fallback: string): string => (e instanceof Error && e.message ? e.message : fallback);

/**
 * The folder chooser of the item editor: the folders that exist, "No folder", or a new one typed in. Typing
 * `Work/Clients` makes a folder inside Work. The folder itself is made when the item is saved into it.
 */
export function FolderField({ value, onChange }: { value: string; onChange: (path: string) => void }) {
  const folders = useFolderList();
  const [naming, setNaming] = useState(false);
  const known = value === "" || folders.includes(value);
  const problem = value.trim() === "" ? null : folderProblem(value);

  if (naming || !known) {
    return (
      <div className="flex flex-col gap-2">
        <Field label="Folder" hint={`A new folder. Use ${FOLDER_SEP} to put it inside another, like Work${FOLDER_SEP}Clients.`} error={problem}>
          {({ id, describedBy, invalid }) => (
            <TextInput id={id} data-autofocus={naming || undefined} value={value} invalid={invalid} aria-describedby={describedBy} maxLength={128} placeholder="Folder name" onChange={(e) => onChange(e.target.value)} />
          )}
        </Field>
        <div>
          <Button variant="ghost" onClick={() => { setNaming(false); onChange(""); }}>Choose an existing folder</Button>
        </div>
      </div>
    );
  }
  return (
    <Field label="Folder">
      {({ id, describedBy }) => (
        <Select
          id={id}
          aria-describedby={describedBy}
          value={value}
          onChange={(e) => {
            if (e.target.value === "\u0000new") { setNaming(true); onChange(""); } else onChange(e.target.value);
          }}
        >
          <option value="">No folder</option>
          {folders.map((f) => <option key={f} value={f}>{folderOptionLabel(f)}</option>)}
          <option value={"\u0000new"}>New folder…</option>
        </Select>
      )}
    </Field>
  );
}

/** Put one or more items in a folder (or take them out of every folder). */
function MoveSheet({ dialog }: { dialog: Extract<FolderDialog, { kind: "move" }> }) {
  const folders = useFolderList();
  const [busy, setBusy] = useState(false);
  const [naming, setNaming] = useState(false);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function go(target: string | null) {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const n = await moveItems(dialog.ids, target);
      closeFolderDialog();
      const what = dialog.ids.length === 1 ? `“${dialog.title}”` : plural(n, "item");
      pushToast({ kind: "success", text: n === 0 ? `${what} is already there.` : target ? `${what} moved to ${folderName(target)}.` : `${what} taken out of its folder.` });
    } catch (e) {
      setError(message(e, "The items could not be moved."));
      setBusy(false);
    }
  }

  function submitNew(e: FormEvent) {
    e.preventDefault();
    const path = cleanFolder(name);
    const problem = folderProblem(name);
    if (!path || problem) return setError(problem ?? "Give the folder a name.");
    void go(path);
  }

  const row = (on: boolean) => `pressable flex min-h-12 w-full items-center gap-3 rounded-lg px-3 text-left ${on ? "bg-raised" : "hover:bg-raised/60"}`;
  return (
    <Sheet open onClose={closeFolderDialog} title={dialog.ids.length === 1 ? `Move “${dialog.title}”` : `Move ${plural(dialog.ids.length, "item")}`} variant="dialog" size="sm" dismissible={!busy}>
      <div className="flex flex-col gap-3 pb-1">
        <ul className="-mx-1 flex max-h-[min(22rem,50vh)] flex-col gap-0.5 overflow-y-auto overscroll-contain px-1">
          <li>
            <button type="button" className={row(dialog.current === null)} disabled={busy} onClick={() => void go(null)}>
              <Folder className="size-4 shrink-0 text-fog" aria-hidden />
              <span className="min-w-0 flex-1 truncate">No folder</span>
              {dialog.current === null && <Check className="size-4 text-brass-ink" aria-label="Current folder" />}
            </button>
          </li>
          {folders.map((f) => {
            const depth = f.split(FOLDER_SEP).length - 1;
            return (
              <li key={f}>
                <button type="button" className={row(dialog.current === f)} style={{ paddingLeft: `${0.75 + depth * 1.25}rem` }} disabled={busy} onClick={() => void go(f)}>
                  <Folder className="size-4 shrink-0 text-brass-ink" aria-hidden />
                  <span className="min-w-0 flex-1 truncate">{folderName(f)}</span>
                  {dialog.current === f && <Check className="size-4 text-brass-ink" aria-label="Current folder" />}
                </button>
              </li>
            );
          })}
        </ul>
        {naming ? (
          <form onSubmit={submitNew} className="flex flex-col gap-3 border-t border-hairline pt-3">
            <Field label="New folder" hint={`Use ${FOLDER_SEP} to put it inside another, like Work${FOLDER_SEP}Clients.`} error={error}>
              {({ id, describedBy, invalid }) => <TextInput id={id} data-autofocus value={name} invalid={invalid} aria-describedby={describedBy} maxLength={128} disabled={busy} onChange={(e) => { setName(e.target.value); setError(null); }} />}
            </Field>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => { setNaming(false); setError(null); }} disabled={busy}>Back</Button>
              <Button type="submit" variant="key" busy={busy} disabled={!name.trim()}>Make folder and move</Button>
            </div>
          </form>
        ) : (
          <div className="border-t border-hairline pt-3">
            <Button variant="quiet" icon={<FolderPlus className="size-4" />} onClick={() => setNaming(true)} disabled={busy}>New folder</Button>
            {error && <p role="alert" className="mt-2 text-[0.93em] text-ember">{error}</p>}
          </div>
        )}
      </div>
    </Sheet>
  );
}

/** Make a folder (inside the open one) or rename one. */
function NameSheet({ dialog }: { dialog: Extract<FolderDialog, { kind: "create" | "rename" }> }) {
  const renaming = dialog.kind === "rename";
  const [name, setName] = useState(renaming ? folderName(dialog.path) : "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const parent = renaming ? parentFolder(dialog.path) : dialog.parent;
  const target = joinFolder(parent, name);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (busy) return;
    const problem = folderProblem(target);
    if (problem) return setError(problem);
    if (renaming && cleanFolder(target) === dialog.path) return closeFolderDialog();
    setBusy(true);
    try {
      if (renaming) {
        const n = await renameFolder(dialog.path, cleanFolder(target) ?? target);
        pushToast({ kind: "success", text: n === 0 ? `Folder renamed to ${name.trim()}.` : `Folder renamed to ${name.trim()}. ${plural(n, "item")} followed it.` });
      } else {
        await createFolder(target);
        pushToast({ kind: "success", text: `Folder ${name.trim()} made.` });
      }
      closeFolderDialog();
    } catch (err) {
      setError(message(err, renaming ? "The folder could not be renamed." : "The folder could not be made."));
      setBusy(false);
    }
  }

  return (
    <Sheet open onClose={closeFolderDialog} title={renaming ? "Rename folder" : "New folder"} variant="dialog" size="sm" dismissible={!busy}>
      <form onSubmit={submit} className="flex flex-col gap-4 pt-1">
        <Field label="Name" hint={parent ? `Inside ${parent.split(FOLDER_SEP).join(" / ")}.` : undefined} error={error}>
          {({ id, describedBy, invalid }) => <TextInput id={id} data-autofocus value={name} invalid={invalid} aria-describedby={describedBy} maxLength={64} disabled={busy} onChange={(e) => { setName(e.target.value); setError(null); }} />}
        </Field>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={closeFolderDialog} disabled={busy}>Cancel</Button>
          <Button type="submit" variant="key" busy={busy} disabled={!name.trim()}>{renaming ? "Rename" : "Make folder"}</Button>
        </div>
      </form>
    </Sheet>
  );
}

/** Removing a folder never deletes what is in it: it moves up one level. */
function RemoveSheet({ dialog }: { dialog: Extract<FolderDialog, { kind: "remove" }> }) {
  const items = useStore(app, (s) => s.items);
  const folders = useFolderList();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inside = items.filter((i) => i.folder && isInside(i.folder, dialog.path)).length;
  const subs = folders.filter((f) => f !== dialog.path && isInside(f, dialog.path)).length;
  const up = parentFolder(dialog.path);

  async function go() {
    setBusy(true);
    try {
      await removeFolder(dialog.path);
      closeFolderDialog();
      pushToast({ kind: "success", text: `Folder ${folderName(dialog.path)} removed. Nothing was deleted.` });
    } catch (e) {
      setError(message(e, "The folder could not be removed."));
      setBusy(false);
    }
  }

  return (
    <Sheet open onClose={closeFolderDialog} title={`Remove “${folderName(dialog.path)}”?`} variant="dialog" size="sm" dismissible={!busy}>
      <div className="flex flex-col gap-4 pt-1">
        <p className="text-mist">
          Only the folder goes away.{" "}
          {inside + subs === 0
            ? "It is empty."
            : `${inside > 0 ? plural(inside, "item") : ""}${inside > 0 && subs > 0 ? " and " : ""}${subs > 0 ? plural(subs, "folder") : ""} inside move${inside + subs === 1 ? "s" : ""} ${up ? `into ${folderName(up)}` : "to the top level"}. Nothing is deleted.`}
        </p>
        {error && <p role="alert" className="text-[0.93em] text-ember">{error}</p>}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={closeFolderDialog} disabled={busy}>Cancel</Button>
          <Button variant="danger" busy={busy} onClick={() => void go()}>Remove folder</Button>
        </div>
      </div>
    </Sheet>
  );
}

/** Mounted once in the app: whichever folder dialog was asked for. */
export function FolderDialogHost() {
  const dialog = useStore(ui, (s) => s.folderDialog);
  if (!dialog) return null;
  switch (dialog.kind) {
    case "move": return <MoveSheet key="move" dialog={dialog} />;
    case "create":
    case "rename": return <NameSheet key={dialog.kind} dialog={dialog} />;
    case "remove": return <RemoveSheet key="remove" dialog={dialog} />;
  }
}
