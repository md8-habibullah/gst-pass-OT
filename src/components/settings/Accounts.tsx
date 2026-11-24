import { useState, type FormEvent } from "react";
import { Pencil, Plus, Trash2, UserRound } from "lucide-react";
import { BackendError, type ProfileInfo } from "../../backend";
import { checkDisplayName, MAX_NAME_CHARS } from "../../lib/profileName";
import { relativeTime } from "../../lib/format";
import { DEMO_PASSWORD } from "../../lib/demo";
import { app, isLabelledDemo, pushToast, removeProfile, renameProfile, showPicker } from "../../state/app";
import { useStore } from "../../state/store";
import { setLockIntent } from "../../state/ui";
import { Avatar } from "../ui/Avatar";
import { Button } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";

function Rename({ profile, onClose }: { profile: ProfileInfo | null; onClose: () => void }) {
  const profiles = useStore(app, (s) => s.profiles);
  const [value, setValue] = useState(profile?.displayName ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const check = checkDisplayName(value, profiles, profile?.id);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!profile || busy) return;
    if (!check.ok) return setError(check.message);
    setBusy(true); setError(null);
    try {
      await renameProfile(profile.id, check.name);
      pushToast({ kind: "success", text: `The account is now called “${check.name}”.` });
      onClose();
    } catch (err) {
      setBusy(false);
      setError(err instanceof BackendError || err instanceof Error ? err.message : "Could not rename the account.");
    }
  }
  return (
    <form onSubmit={submit} className="flex flex-col gap-4 pt-1">
      <Field label="Account name" hint={`Shown on the lock screen. Up to ${MAX_NAME_CHARS} characters, different from your other accounts.`} error={error ?? (value && !check.ok ? check.message : null)}>
        {({ id, describedBy, invalid }) => <TextInput id={id} data-autofocus value={value} invalid={invalid} aria-describedby={describedBy} onChange={(e) => { setValue(e.target.value); setError(null); }} maxLength={MAX_NAME_CHARS * 2} disabled={busy} />}
      </Field>
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose} disabled={busy}>Cancel</Button>
        <Button type="submit" variant="key" busy={busy} disabled={!check.ok || (profile !== null && check.ok && check.name === profile.displayName)}>Rename</Button>
      </div>
    </form>
  );
}

/** The demo vault, introduced as a demo on the first screen: no password to type, it holds nothing real. */
function RemoveDemo({ profile, onClose }: { profile: ProfileInfo; onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function go() {
    setBusy(true); setError(null);
    try {
      await removeProfile(profile.id, DEMO_PASSWORD, "CONFIRM");
      pushToast({ kind: "info", text: "The demo vault was removed." });
      onClose();
    } catch (err) {
      setBusy(false);
      setError(err instanceof Error ? err.message : "Could not remove the demo vault.");
    }
  }
  return (
    <div className="flex flex-col gap-4 pt-1">
      <p>This removes the demo vault and its made-up items from this device. Your own vaults are not touched.</p>
      {error && <p role="alert" className="text-[0.93em] text-ember">{error}</p>}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose} disabled={busy}>Keep it</Button>
        <Button variant="danger" busy={busy} onClick={() => void go()} icon={<Trash2 className="size-4" />}>Remove demo</Button>
      </div>
    </div>
  );
}

function Remove({ profile, onClose }: { profile: ProfileInfo | null; onClose: () => void }) {
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ready = password.length > 0 && (confirmation === profile?.displayName || confirmation === "CONFIRM");

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!profile || !ready || busy) return;
    setBusy(true); setError(null);
    try {
      await removeProfile(profile.id, password, confirmation);
      pushToast({ kind: "info", text: `“${profile.displayName}” and its vault were deleted from this device.` });
      onClose();
    } catch (err) {
      setBusy(false);
      setError(err instanceof BackendError && err.code === "wrong_password" ? "That is not this account's master password." : err instanceof Error ? err.message : "Could not delete the account.");
    }
  }
  return (
    <form onSubmit={submit} className="flex flex-col gap-4 pt-1">
      <p>This permanently deletes “{profile?.displayName}” and every item in it from this device. Copies in your backups are not touched. It cannot be undone.</p>
      <Field label="That account's master password" error={error}>
        {({ id, describedBy, invalid }) => <TextInput id={id} data-autofocus type="password" autoComplete="current-password" value={password} invalid={invalid} aria-describedby={describedBy} onChange={(e) => setPassword(e.target.value)} disabled={busy} />}
      </Field>
      <Field label={`Type “${profile?.displayName}” or CONFIRM to continue`}>
        {({ id }) => <TextInput id={id} value={confirmation} onChange={(e) => setConfirmation(e.target.value)} disabled={busy} />}
      </Field>
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose} disabled={busy}>Keep it</Button>
        <Button type="submit" variant="danger" busy={busy} disabled={!ready} icon={<Trash2 className="size-4" />}>Delete account</Button>
      </div>
    </form>
  );
}

/** The accounts on this device: rename, delete, add. */
export function AccountsGroup({ heading = true }: { heading?: boolean }) {
  const profiles = useStore(app, (s) => s.profiles);
  const current = useStore(app, (s) => s.profile);
  const device = useStore(app, (s) => s.device);
  const [sheet, setSheet] = useState<{ kind: "rename" | "remove"; p: ProfileInfo } | null>(null);
  const close = () => setSheet(null);

  async function add() {
    setLockIntent("choose");
    await showPicker();
  }

  return (
    <section aria-label="Accounts" className="flex flex-col gap-2">
      {heading && <h3 className="px-1 font-semibold">Accounts</h3>}
      <div className="surface overflow-hidden rounded-lg">
        <p className="border-b border-hairline px-4 py-3 text-[0.93em] text-mist">
          Each account is its own vault with its own master password. Names are only labels, and must be different on this device.
        </p>
        <ul>
          {profiles.map((p) => {
            const on = p.id === current?.id;
            return (
              <li key={p.id} className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-hairline px-4 py-3 last:border-b-0">
                <Avatar title={p.displayName} size={38} />
                <div className="min-w-0 flex-1 basis-40">
                  <p className="truncate font-semibold">{p.displayName}</p>
                  <p className="text-[0.86em] text-mist">{isLabelledDemo(device, p) ? (on ? "Demo vault, open now" : "Demo vault") : on ? "Open now" : p.lastUsedAt ? `Last opened ${relativeTime(p.lastUsedAt)}` : "Not opened yet"}</p>
                </div>
                <div className="flex gap-1.5">
                  <Button className="min-h-8" icon={<Pencil className="size-3.5" />} onClick={() => setSheet({ kind: "rename", p })}>Rename</Button>
                  {!(isLabelledDemo(device, p) && profiles.length === 1) && (
                    <Button className="min-h-8 text-ember" variant="ghost" icon={<Trash2 className="size-3.5" />} onClick={() => setSheet({ kind: "remove", p })}>Delete</Button>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
        <div className="flex flex-wrap gap-2 border-t border-hairline px-4 py-3">
          <Button icon={<Plus className="size-4" />} onClick={() => void add()}>Add account</Button>
          <Button icon={<UserRound className="size-4" />} onClick={() => void showPicker()}>Switch account</Button>
        </div>
      </div>
      <Sheet open={sheet?.kind === "rename"} onClose={close} title="Rename account" variant="dialog" size="sm">
        {sheet?.kind === "rename" && <Rename key={sheet.p.id} profile={sheet.p} onClose={close} />}
      </Sheet>
      <Sheet open={sheet?.kind === "remove"} onClose={close} title={sheet && isLabelledDemo(device, sheet.p) ? "Remove the demo vault?" : "Delete this account?"} variant="dialog" size="sm">
        {sheet?.kind === "remove" && (isLabelledDemo(device, sheet.p) ? <RemoveDemo key={sheet.p.id} profile={sheet.p} onClose={close} /> : <Remove key={sheet.p.id} profile={sheet.p} onClose={close} />)}
      </Sheet>
    </section>
  );
}
