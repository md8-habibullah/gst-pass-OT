import { useEffect, useState } from "react";
import { BackendError } from "../../backend";
import type { DisguiseInput, DisguiseView } from "../../backend";
import { DISGUISES, disguiseName, disguiseTile, ORIGINAL_NAME } from "../../lib/disguise";
import { be } from "../../state/app";
import { setBrand } from "../../state/brand";
import { Button } from "../ui/Button";
import { Field, Switch, TextInput } from "../ui/Inputs";
import { VaultMark } from "../lock/VaultMark";

/** "App icon and name": 11 neutral looks for the launcher, a lower-case option, and a name of your own. */
export function DisguiseGroup() {
  const [view, setView] = useState<DisguiseView | null>(null);
  const [custom, setCustom] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { void be().disguiseGet().then((v) => { setView(v); setCustom(v.customName ?? ""); }).catch(() => undefined); }, []);

  async function save(next: DisguiseInput) {
    setBusy(true); setError(null);
    try {
      const v = await be().disguiseSet(next);
      setView(v); setCustom(v.customName ?? ""); setBrand(v);
    } catch (e) {
      setError(e instanceof BackendError || e instanceof Error ? e.message : "Could not change the icon.");
    } finally {
      setBusy(false);
    }
  }

  if (!view) return null;
  const input: DisguiseInput = { presetId: view.presetId, customName: view.customName, lowercase: view.lowercase };
  const label = (name: string) => (view.lowercase ? name.toLowerCase() : name);
  const customDirty = custom.trim() !== (view.customName ?? "");

  const tile = "pressable flex w-[5.5rem] flex-col items-center gap-2 rounded-lg p-2 text-center";
  return (
    <section aria-label="App icon and name" className="flex flex-col gap-2">
      <h3 className="px-1 font-semibold">App icon and name</h3>
      <div className="glass flex flex-col gap-5 rounded-lg p-4">
        <p className="text-[0.93em] text-mist">Change how the app looks in your launcher and on its own screens, so it does not announce what it is. The name is shown as <span className="font-medium text-frost">{view.appliedName}</span>.</p>

        <div role="radiogroup" aria-label="Icon" className="flex flex-wrap gap-2">
          <button type="button" role="radio" aria-checked={view.presetId === null} disabled={busy} onClick={() => void save({ ...input, presetId: null })} className={`${tile} ${view.presetId === null ? "bg-raised ring-2 ring-brass" : "hover:bg-raised/60"}`}>
            <span className="grid size-14 place-items-center rounded-[22%] bg-pane"><VaultMark size={34} /></span>
            <span className="text-[0.86em]">{label(ORIGINAL_NAME)}</span>
          </button>
          {DISGUISES.map((d) => (
            <button key={d.id} type="button" role="radio" aria-checked={view.presetId === d.id} disabled={busy} onClick={() => void save({ ...input, presetId: d.id })} title={d.description} className={`${tile} ${view.presetId === d.id ? "bg-raised ring-2 ring-brass" : "hover:bg-raised/60"}`}>
              <img src={disguiseTile(d.id)} alt="" width={56} height={56} className="size-14" draggable={false} />
              <span className="text-[0.86em]">{label(d.name)}</span>
            </button>
          ))}
        </div>

        <div className="flex items-center justify-between gap-4">
          <div>
            <p className="font-medium">Show names in lower case</p>
            <p className="text-[0.93em] text-mist">{label("Kerf")}, {label("Kovren")}: the same name, written quietly.</p>
          </div>
          <Switch label="Lower-case names" checked={view.lowercase} onChange={(v) => void save({ ...input, lowercase: v })} />
        </div>

        <form className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); void save({ ...input, customName: custom.trim() || null }); }}>
          <Field label="Your own name (optional)" error={error} hint={`Replaces the name above. Currently ${disguiseName({ ...input, customName: custom.trim() || null })}.`}>
            {({ id, describedBy, invalid }) => <TextInput id={id} aria-describedby={describedBy} invalid={invalid} maxLength={30} value={custom} disabled={busy} onChange={(e) => setCustom(e.target.value)} placeholder="Leave empty to use the icon's name" />}
          </Field>
          <div className="flex justify-end gap-2">
            {view.customName && <Button variant="ghost" disabled={busy} onClick={() => void save({ ...input, customName: null })}>Use the icon's name</Button>}
            <Button type="submit" variant="key" busy={busy} disabled={!customDirty}>Apply name</Button>
          </div>
        </form>
        {view.note && <p className="text-[0.86em] text-fog">{view.note}</p>}
      </div>
    </section>
  );
}
