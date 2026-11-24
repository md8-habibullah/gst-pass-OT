import { useCallback, useEffect, useState, type FormEvent } from "react";
import { Download, File as FileIcon, FileText, Image as ImageIcon, Pencil, Plus, Trash2 } from "lucide-react";
import { BackendError } from "../../backend";
import type { FileEntry } from "../../backend";
import { formatBytes, relativeTime } from "../../lib/format";
import { MAX_FILE_LABEL, previewKind } from "../../lib/files";
import { be, pushToast } from "../../state/app";
import { reprompt } from "../reprompt/Reprompt";
import { Button } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";
import { FilePreview } from "./FilePreview";

const KIND_ICON = { image: ImageIcon, pdf: FileText, text: FileText, none: FileIcon } as const;
const msg = (e: unknown, fallback: string) => (e instanceof BackendError || e instanceof Error ? e.message : fallback);

/**
 * Files stored in the vault. `itemId` null is the standalone Files section; an id lists that item's attachments.
 * The core does the picking, the encrypting and the saving: this component only ever holds names, sizes and (for the
 * preview) the decrypted bytes of the one file being looked at.
 */
export function FileList({ itemId, emptyText, onChanged }: { itemId: string | null; emptyText: string; onChanged?: () => void }) {
  const [files, setFiles] = useState<FileEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<FileEntry | null>(null);
  const [renaming, setRenaming] = useState<FileEntry | null>(null);
  const [name, setName] = useState("");

  const load = useCallback(async () => {
    try { setFiles(await be().fileList(itemId)); } catch (e) { pushToast({ kind: "error", text: msg(e, "Could not list the files.") }); setFiles([]); }
  }, [itemId]);
  useEffect(() => { void load(); }, [load]);

  async function add() {
    setBusy(true);
    try {
      const added = await be().fileAdd(itemId);
      if (added) {
        pushToast({ kind: "success", text: added.length === 1 ? `${added[0]!.name} was encrypted and added.` : `${added.length} files were encrypted and added.` });
        await load(); onChanged?.();
      }
    } catch (e) {
      pushToast({ kind: "error", text: msg(e, "Could not add the file.") });
    } finally { setBusy(false); }
  }

  async function saveCopy(f: FileEntry) {
    try {
      const r = await reprompt({
        reason: `Save a readable copy of “${f.name}”. The copy is not encrypted.`, confirmLabel: "Choose where to save",
        run: async (pw) => (await be().fileSaveCopy(f.id, pw)) ?? "cancelled",
      });
      if (r && r !== "cancelled") pushToast({ kind: "success", text: `Saved a decrypted copy to ${r.path}.` });
    } catch (e) { pushToast({ kind: "error", text: msg(e, "Could not save the copy.") }); }
  }

  async function remove(f: FileEntry) {
    try {
      const done = await reprompt({
        reason: `Delete “${f.name}” for good. It is removed from this device and from your backups.`, confirmLabel: "Delete file", tone: "danger",
        run: async (pw) => { await be().fileDelete(f.id, pw); return true; },
      });
      if (done) { await load(); onChanged?.(); pushToast({ kind: "info", text: `${f.name} was deleted.` }); }
    } catch (e) { pushToast({ kind: "error", text: msg(e, "Could not delete the file.") }); }
  }

  async function rename(e: FormEvent) {
    e.preventDefault();
    if (!renaming) return;
    try { await be().fileRename(renaming.id, name); setRenaming(null); await load(); } catch (err) { pushToast({ kind: "error", text: msg(err, "Could not rename the file.") }); }
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="key" icon={<Plus className="size-4" />} busy={busy} onClick={() => void add()}>{busy ? "Encrypting" : "Add files"}</Button>
        <p className="text-[0.86em] text-fog">Any file up to {MAX_FILE_LABEL}. Encrypted before it is stored or backed up.</p>
      </div>
      {files === null ? null : files.length === 0 ? (
        <p className="rounded-lg bg-pane/60 px-4 py-6 text-center text-mist">{emptyText}</p>
      ) : (
        <ul className="surface overflow-hidden rounded-lg">
          {files.map((f) => {
            const Icon = KIND_ICON[previewKind(f)];
            const can = previewKind(f) !== "none";
            return (
              <li key={f.id} className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-hairline px-3 py-2.5 last:border-b-0">
                <button type="button" onClick={() => setPreview(f)} className="pressable flex min-w-0 flex-1 basis-56 items-center gap-3 rounded-lg text-left" aria-label={`${can ? "Preview" : "Open"} ${f.name}`}>
                  <span className="grid size-10 shrink-0 place-items-center rounded-sm bg-raised shadow-e1"><Icon className="size-[18px] text-mist" aria-hidden /></span>
                  <span className="min-w-0">
                    <span className="block truncate font-medium">{f.name}</span>
                    <span className="block text-[0.86em] text-fog">{formatBytes(f.size)} · added {relativeTime(f.addedAt)}</span>
                  </span>
                </button>
                <div className="flex gap-1">
                  <Button variant="ghost" className="min-h-8" icon={<Download className="size-3.5" />} onClick={() => void saveCopy(f)} aria-label={`Save a decrypted copy of ${f.name}`}>Save copy</Button>
                  <Button variant="ghost" className="min-h-8 px-2.5" onClick={() => { setRenaming(f); setName(f.name); }} aria-label={`Rename ${f.name}`}><Pencil className="size-3.5" aria-hidden /></Button>
                  <Button variant="ghost" className="min-h-8 px-2.5 text-ember" onClick={() => void remove(f)} aria-label={`Delete ${f.name}`}><Trash2 className="size-3.5" aria-hidden /></Button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
      <FilePreview file={preview} onClose={() => setPreview(null)} />
      <Sheet open={renaming !== null} onClose={() => setRenaming(null)} title="Rename file" variant="dialog" size="sm">
        <form onSubmit={rename} className="flex flex-col gap-4">
          <Field label="File name">{({ id }) => <TextInput id={id} data-autofocus value={name} maxLength={200} onChange={(e) => setName(e.target.value)} />}</Field>
          <div className="flex justify-end gap-2"><Button variant="ghost" onClick={() => setRenaming(null)}>Cancel</Button><Button type="submit" variant="key" disabled={!name.trim() || name.trim() === renaming?.name}>Rename</Button></div>
        </form>
      </Sheet>
    </div>
  );
}
