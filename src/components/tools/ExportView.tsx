import { useState } from "react";
import { FileLock2, ShieldAlert } from "lucide-react";
import { app, be, pushToast } from "../../state/app";
import { useStore } from "../../state/store";
import { reprompt } from "../reprompt/Reprompt";
import { PageBody, PageHeader } from "../shell/PageHeader";
import { Button } from "../ui/Button";
import { Segmented } from "../ui/Inputs";
import type { PlainExportFormat } from "../../backend";

/** Exports the whole vault as an encrypted file (the same SQLite vault the app uses; nothing is decrypted). */
export function ExportView() {
  const profile = useStore(app, (s) => s.profile);
  const [saved, setSaved] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [plainFormat, setPlainFormat] = useState<PlainExportFormat>("bitwarden-json");
  const [plainSaved, setPlainSaved] = useState<string | null>(null);
  const [plainBusy, setPlainBusy] = useState(false);

  async function exportFile() {
    setBusy(true);
    try {
      const r = await reprompt({
        reason: "Save an encrypted copy of this whole vault to a file you choose.",
        confirmLabel: "Choose where to save",
        run: async (pw) => (await be().exportVaultFile(pw)) ?? "cancelled",
      });
      if (r && r !== "cancelled") {
        setSaved(r.path);
        pushToast({ kind: "success", text: "Your encrypted vault file was saved." });
      }
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "The export did not finish." });
    } finally {
      setBusy(false);
    }
  }

  async function exportPlain() {
    setPlainBusy(true);
    try {
      const r = await reprompt({
        reason: "Save a READABLE copy of every password in this vault. Anyone who finds the file can read it.", confirmLabel: "Choose where to save", tone: "danger",
        run: async (pw) => (await be().exportPlain(plainFormat, pw)) ?? "cancelled",
      });
      if (r && r !== "cancelled") {
        setPlainSaved(r.path);
        pushToast({ kind: "info", text: ["A readable copy was saved. Delete it as soon as you are done with it.", ...(r.warnings ?? [])].join(" ") });
      }
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "The export did not finish." });
    } finally {
      setPlainBusy(false);
    }
  }

  return (
    <PageBody>
      <PageHeader title="Export" back subtitle={`Save “${profile?.displayName ?? "this vault"}” as a file you can keep, move to another device, or open with Add account, Open a vault file.`} />

      <section className="surface flex flex-col gap-4 rounded-lg p-5">
        <div className="flex items-start gap-4">
          <span className="grid size-11 shrink-0 place-items-center rounded-lg bg-pane shadow-e1"><FileLock2 className="size-5 text-brass" aria-hidden /></span>
          <div className="min-w-0">
            <h3 className="font-semibold">Encrypted vault file</h3>
            <p className="mt-1 text-[0.93em] text-mist">A complete copy of this vault, every item and every earlier version, still sealed with your master password. Without that password the file is unreadable.</p>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <Button variant="key" busy={busy} onClick={() => void exportFile()}>Export vault file</Button>
          {saved && <p className="min-w-0 text-[0.93em] text-verdigris" role="status">Saved to <span className="font-mono break-all">{saved}</span></p>}
        </div>
      </section>

      <section className="surface flex flex-col gap-4 rounded-lg p-5">
        <div className="flex items-start gap-4">
          <span className="grid size-11 shrink-0 place-items-center rounded-lg bg-pane shadow-e1"><ShieldAlert className="size-5 text-ember" aria-hidden /></span>
          <div className="min-w-0">
            <h3 className="font-semibold">Readable copy for another app</h3>
            <p className="mt-1 text-[0.93em] text-mist">A plain file with every password, username and code written out, in a format other managers can import. <span className="text-frost">Nothing in it is encrypted.</span> Keep it off shared folders and cloud sync, and delete it (and empty the trash) as soon as you have imported it.</p>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <Segmented<PlainExportFormat> label="File format" value={plainFormat} onChange={(f) => { setPlainFormat(f); setPlainSaved(null); }} options={[{ value: "bitwarden-json", label: "Bitwarden JSON" }, { value: "csv", label: "CSV" }]} />
          <Button variant="danger" busy={plainBusy} onClick={() => void exportPlain()}>Export readable file</Button>
        </div>
        {plainSaved && <p className="min-w-0 text-[0.93em] text-ember" role="status">Saved to <span className="font-mono break-all">{plainSaved}</span>. Delete it when you are done.</p>}
      </section>
    </PageBody>
  );
}
