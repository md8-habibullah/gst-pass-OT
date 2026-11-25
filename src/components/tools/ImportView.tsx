import { useEffect, useRef, useState } from "react";
import { CircleAlert, FileUp, KeyRound, ShieldAlert, TriangleAlert } from "lucide-react";
import { BackendError } from "../../backend";
import type { CsvInspect, CsvMapping, ImportFormat, ImportMode, ImportOptions, ImportOutcome, ImportPick, ImportPreview } from "../../backend";
import { FORMATS, formatLabel, guessMapping } from "../../lib/importFormats";
import { formatBytes, plural } from "../../lib/format";
import { be, pushToast, refresh, setFilter, syncAfterCoreLock } from "../../state/app";
import { PageBody, PageHeader } from "../shell/PageHeader";
import { Button } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";

const MODES: { id: ImportMode; title: string; text: string }[] = [
  { id: "merge", title: "Merge matching accounts (recommended)", text: "The same account from two apps (a Bitwarden login and its Ente Auth code, say) becomes one item. Notes, websites, tags and codes from both are kept, and nothing is overwritten." },
  { id: "skip_duplicates", title: "Skip what is already here", text: "An entry with the same name, username and website is left alone." },
  { id: "append_version", title: "Add as a new version", text: "Same name, username and website: the imported entry becomes a new version in its history." },
  { id: "import_all", title: "Import everything", text: "Every entry becomes a new item, even if you already have it." },
];

const FIELDS: { key: keyof CsvMapping; label: string; required?: boolean }[] = [
  { key: "title", label: "Name", required: true }, { key: "username", label: "Username" }, { key: "password", label: "Password" },
  { key: "url", label: "Web address" }, { key: "totp", label: "One-time code" }, { key: "notes", label: "Notes" }, { key: "folder", label: "Folder" },
];

const msg = (e: unknown, fallback: string) => (e instanceof BackendError || e instanceof Error ? e.message : fallback);
/** The core forgets a picked file after a while (and when the vault locks): say so in plain words. */
const GONE = "That file is no longer open. Choose it again.";
const isGone = (e: unknown) => e instanceof BackendError && e.code === "not_found";

/** Choose a file, say what it is, preview what is in it, then import. The core reads and parses it; this page only ever sees counts and titles. */
export function ImportView() {
  const [pick, setPick] = useState<ImportPick | null>(null);
  const [format, setFormat] = useState<ImportFormat | null>(null);
  const [password, setPassword] = useState("");
  const [keyfile, setKeyfile] = useState<{ handle: string; fileName: string } | null>(null);
  const [csv, setCsv] = useState<CsvInspect | null>(null);
  const [mapping, setMapping] = useState<CsvMapping>({});
  const [mode, setMode] = useState<ImportMode>("merge");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [outcome, setOutcome] = useState<ImportOutcome | null>(null);
  const [busy, setBusy] = useState<"pick" | "check" | "import" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const live = useRef<string | null>(null);
  live.current = pick?.handle ?? null;
  // Leaving the page drops whatever the core was holding for the open file.
  useEffect(() => () => { if (live.current) void be().importCancel(live.current).catch(() => undefined); }, []);

  const effective: ImportFormat | null = format ?? (pick && pick.format !== "unknown" ? pick.format : null);
  const options = (): ImportOptions | null => (pick && effective ? { handle: pick.handle, format: effective, mode, password: password || undefined, keyfileHandle: keyfile?.handle, mapping: effective === "custom-csv" ? mapping : undefined } : null);
  const reset = (keepOutcome = false) => { setPick(null); setFormat(null); setPassword(""); setKeyfile(null); setCsv(null); setMapping({}); setPreview(null); setError(null); if (!keepOutcome) setOutcome(null); };

  async function choose() {
    if (pick) await be().importCancel(pick.handle).catch(() => undefined);
    reset(); setBusy("pick");
    try { const p = await be().importPick(); if (p) setPick(p); } catch (e) { setError(msg(e, "That file could not be opened.")); } finally { setBusy(null); }
  }
  useEffect(() => {
    if (!pick || effective !== "custom-csv" || csv) return;
    void be().inspectCsvHeaders(pick.handle).then((c) => { setCsv(c); setMapping(guessMapping(c.headers)); }).catch((e: unknown) => fail(e, "That file could not be read as a table."));
  }, [pick, effective, csv]);

  async function check() {
    const o = options(); if (!o) return;
    setBusy("check"); setError(null); setPreview(null);
    try { setPreview(await be().importPreview(o)); } catch (e) { fail(e, "That file could not be read."); } finally { setBusy(null); }
  }
  async function run() {
    const o = options(); if (!o) return;
    setBusy("import"); setError(null);
    try {
      const r = await be().importCommit(o);
      await refresh(); setOutcome(r); reset(true);
      pushToast({ kind: r.failed ? "error" : "success", text: `${plural(r.added + r.appended, "item")} imported${r.skipped ? `, ${r.skipped} skipped` : ""}.` });
    } catch (e) { fail(e, "The import did not finish. Nothing was added."); } finally { setBusy(null); }
  }
  /** An error from the core. A forgotten file or a locked vault also ends the wizard: there is nothing left to continue with. */
  function fail(e: unknown, fallback: string) {
    if (isGone(e)) { reset(true); setError(GONE); return; }
    if (e instanceof BackendError && e.code === "locked") { reset(true); void syncAfterCoreLock().catch(() => undefined); return; }
    setError(msg(e, fallback));
  }
  async function chooseKeyfile() { try { setKeyfile(await be().importPickKeyfile()); } catch (e) { setError(msg(e, "That key file could not be opened.")); } }

  const needsPassword = effective === "keepass-kdbx";
  const mappingOk = effective !== "custom-csv" || (Boolean(mapping.title) && Boolean(mapping.password || mapping.totp));
  const ready = Boolean(effective) && mappingOk && (!needsPassword || password.length > 0 || Boolean(keyfile));
  const total = preview ? preview.entries : 0;

  return (
    <PageBody>
      <PageHeader title="Import" back subtitle="Bring in passwords and codes from another manager, a browser, a KeePass file or an authenticator app." />

      <section className="surface flex flex-col gap-4 rounded-lg p-5" aria-label="Choose a file">
        <div className="flex flex-wrap items-center gap-4">
          <span className="grid size-11 shrink-0 place-items-center rounded-lg bg-pane shadow-e1"><FileUp className="size-5 text-brass-ink" aria-hidden /></span>
          <div className="min-w-0 flex-1 basis-56">
            <h3 className="truncate font-semibold">{pick ? pick.fileName : "No file chosen"}</h3>
            <p className="text-[0.93em] text-mist">{pick ? `${formatBytes(pick.size)}. ${pick.format === "unknown" ? "Not recognised: pick what it is below." : `Looks like ${formatLabel(pick.format)}.`}` : "The file is opened and read on this device and never uploaded."}</p>
          </div>
          <Button variant={pick ? "quiet" : "key"} busy={busy === "pick"} onClick={() => void choose()} disabled={busy === "import"}>{pick ? "Choose another" : "Choose a file"}</Button>
        </div>

        {pick && (
          <Field label="This file is from">
            {({ id }) => (
              <select id={id} value={effective ?? ""} onChange={(e) => { setFormat((e.target.value || null) as ImportFormat | null); setPreview(null); setCsv(null); }} className="min-h-11 w-full rounded-sm border border-hairline bg-raised px-3 outline-none focus-visible:border-brass-ink">
                {!effective && <option value="">Choose…</option>}
                {["Password managers", "KeePass", "Browsers", "Code apps", "Other"].map((g) => (
                  <optgroup key={g} label={g}>{FORMATS.filter((f) => f.group === g).map((f) => <option key={f.id} value={f.id}>{f.label}</option>)}</optgroup>
                ))}
              </select>
            )}
          </Field>
        )}

        {needsPassword && (
          <div className="flex flex-col gap-3 rounded-lg bg-pane/70 p-4">
            <p className="flex items-center gap-2 font-medium"><KeyRound className="size-4 text-brass-ink" aria-hidden /> This KeePass database is locked</p>
            <Field label="Database password">{({ id }) => <TextInput id={id} type="password" autoComplete="off" value={password} onChange={(e) => { setPassword(e.target.value); setPreview(null); }} />}</Field>
            <div className="flex flex-wrap items-center gap-3">
              <Button onClick={() => void chooseKeyfile()}>{keyfile ? "Choose another key file" : "Add a key file (optional)"}</Button>
              {keyfile && <p className="text-[0.93em] text-mist">{keyfile.fileName}</p>}
            </div>
            <p className="text-[0.86em] text-fog">It is opened in memory only. Nothing is written to disk and the password is forgotten afterwards.</p>
          </div>
        )}

        {effective === "custom-csv" && csv && (
          <div className="flex flex-col gap-3 rounded-lg bg-pane/70 p-4">
            <p className="font-medium">Match the columns</p>
            <div className="grid grid-cols-1 gap-3 min-[560px]:grid-cols-2">
              {FIELDS.map((f) => (
                <Field key={f.key} label={f.label + (f.required ? " (needed)" : "")}>
                  {({ id }) => (
                    <select id={id} value={mapping[f.key] ?? ""} onChange={(e) => { setMapping({ ...mapping, [f.key]: e.target.value || undefined }); setPreview(null); }} className="min-h-10 w-full rounded-sm border border-hairline bg-raised px-3 outline-none focus-visible:border-brass-ink">
                      <option value="">None</option>
                      {csv.headers.map((h) => <option key={h} value={h}>{h}</option>)}
                    </select>
                  )}
                </Field>
              ))}
            </div>
            <div className="overflow-x-auto rounded-lg border border-hairline">
              <table className="w-full text-left text-[0.86em]">
                <thead className="bg-raised"><tr>{csv.headers.map((h) => <th key={h} className="px-3 py-2 font-medium whitespace-nowrap">{h}</th>)}</tr></thead>
                <tbody>{csv.sample.map((row, i) => <tr key={i} className="border-t border-hairline">{row.map((c, j) => <td key={j} className="max-w-48 truncate px-3 py-1.5 text-mist">{csv.headers[j]?.toLowerCase().includes("pass") ? "••••••" : c}</td>)}</tr>)}</tbody>
              </table>
            </div>
            {!mappingOk && <p className="text-[0.93em] text-mist">Pick a column for the name and for the password or the one-time code.</p>}
          </div>
        )}

        {pick && (
          <fieldset className="flex flex-col gap-2">
            <legend className="mb-1 font-medium">If something is already in your vault</legend>
            {MODES.map((m) => (
              <label key={m.id} className="flex cursor-pointer items-start gap-3 rounded-lg px-3 py-2 hover:bg-raised/50">
                <input type="radio" name="import-mode" checked={mode === m.id} onChange={() => { setMode(m.id); setPreview(null); }} className="mt-1 size-4 accent-[var(--color-brass-ink)]" />
                <span><span className="block font-medium">{m.title}</span><span className="block text-[0.93em] text-mist">{m.text}</span></span>
              </label>
            ))}
          </fieldset>
        )}

        {error && <p role="alert" className="flex items-start gap-2 rounded-lg border border-ember/40 bg-danger/10 px-4 py-3 text-[0.95em]"><CircleAlert className="mt-0.5 size-4 shrink-0 text-ember" aria-hidden />{error}</p>}

        {pick && !preview && <div><Button variant="key" busy={busy === "check"} disabled={!ready || busy !== null} onClick={() => void check()}>Check the file</Button></div>}
      </section>

      {preview && (
        <section className="surface flex flex-col gap-4 rounded-lg p-5" aria-label="What will be imported">
          <h3 className="font-semibold">{plural(total, "entry", "entries")} ready</h3>
          <ul className="flex flex-wrap gap-2 text-[0.93em]">
            <li className="rounded-full bg-raised px-3 py-1">{plural(preview.totpSeeds, "one-time code")}</li>
            <li className="rounded-full bg-raised px-3 py-1">{plural(preview.attachments, "attachment")}</li>
            <li className="rounded-full bg-raised px-3 py-1">{mode === "merge" ? `${plural(preview.duplicates, "match", "matches")} to merge` : `${plural(preview.duplicates, "duplicate")} found`}</li>
            {preview.skipped > 0 && <li className="rounded-full bg-raised px-3 py-1">{preview.skipped} not importable</li>}
          </ul>
          {preview.warnings.length > 0 && (
            <ul className="flex flex-col gap-1 text-[0.93em] text-mist">{preview.warnings.slice(0, 6).map((w, i) => <li key={i} className="flex gap-2"><TriangleAlert className="mt-0.5 size-4 shrink-0 text-brass-ink" aria-hidden />{w}</li>)}</ul>
          )}
          {preview.titles.length > 0 && (
            <details className="text-[0.93em]"><summary className="cursor-pointer text-mist">Show the names</summary><p className="mt-2 text-fog">{preview.titles.join(", ")}{preview.entries > preview.titles.length ? ` and ${preview.entries - preview.titles.length} more` : ""}</p></details>
          )}
          <div className="flex flex-wrap gap-3">
            <Button variant="key" busy={busy === "import"} disabled={total === 0 || busy !== null} onClick={() => void run()}>{busy === "import" ? "Encrypting and saving" : `Import ${plural(total, "item")}`}</Button>
            <Button variant="ghost" disabled={busy !== null} onClick={() => setPreview(null)}>Change options</Button>
          </div>
        </section>
      )}

      {outcome && (
        <section className="flex flex-col gap-3 rounded-lg bg-pane/70 p-5 shadow-e1" role="status" aria-label="Import finished">
          <p className="font-semibold">{plural(outcome.added + outcome.appended, "item")} imported{outcome.appended ? (mode === "merge" ? `, ${outcome.appended} merged into items you already had` : `, ${outcome.appended} as new versions`) : ""}{outcome.skipped ? `, ${outcome.skipped} skipped` : ""}{outcome.failed ? `, ${outcome.failed} could not be saved` : ""}.</p>
          {outcome.warnings.length > 0 && (
            <ul className="flex flex-col gap-1 text-[0.93em] text-mist">{outcome.warnings.slice(0, 6).map((w, i) => <li key={i} className="flex gap-2"><TriangleAlert className="mt-0.5 size-4 shrink-0 text-brass-ink" aria-hidden />{w}</li>)}</ul>
          )}
          <p className="flex items-start gap-2 text-[0.93em] text-mist"><ShieldAlert className="mt-0.5 size-4 shrink-0 text-brass-ink" aria-hidden />The file you imported from is not encrypted. Delete it from your disk (and empty the trash) now that everything is safe in your vault.</p>
          <div><Button onClick={() => setFilter("all")}>Open your vault</Button></div>
        </section>
      )}
    </PageBody>
  );
}
