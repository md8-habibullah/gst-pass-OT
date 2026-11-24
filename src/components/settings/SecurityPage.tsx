import { useState, type FormEvent } from "react";
import { KeyRound, Lock } from "lucide-react";
import { BackendError } from "../../backend";
import { app, be, lockVault, pushToast, setClipboardClear, type ClipboardClear } from "../../state/app";
import { useStore } from "../../state/store";
import { StrengthMeter } from "../lock/StrengthMeter";
import { Button } from "../ui/Button";
import { Field, Segmented, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";
import { Group, Row } from "./parts";

function ChangePassword({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [again, setAgain] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const reset = () => { setCurrent(""); setNext(""); setAgain(""); setError(null); setBusy(false); };
  const close = () => { if (busy) return; reset(); onClose(); };
  const valid = current.length > 0 && next.length >= 8 && next === again && next !== current;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!valid || busy) return;
    setBusy(true); setError(null);
    try {
      await be().changePassword(current, next);
      reset(); onClose();
      pushToast({ kind: "success", text: "Master password changed. Your items were not re-encrypted; only the key that guards them was." });
    } catch (err) {
      setBusy(false);
      setError(err instanceof BackendError && err.code === "wrong_password" ? "That is not your current master password." : err instanceof Error ? err.message : "Could not change the password.");
    }
  }

  return (
    <Sheet open={open} onClose={close} title="Change master password" variant="dialog" size="sm" dismissible={!busy} titleIcon={<KeyRound className="size-5 text-brass" aria-hidden />}>
      <form onSubmit={submit} className="flex flex-col gap-4 pt-1">
        <Field label="Current master password" error={error}>
          {({ id, describedBy, invalid }) => <TextInput id={id} data-autofocus type="password" autoComplete="current-password" value={current} invalid={invalid} aria-describedby={describedBy} disabled={busy} onChange={(e) => setCurrent(e.target.value)} />}
        </Field>
        <Field label="New master password" hint="At least 8 characters. There is no recovery if you forget it.">
          {({ id, describedBy }) => <TextInput id={id} type="password" autoComplete="new-password" value={next} aria-describedby={describedBy} disabled={busy} onChange={(e) => setNext(e.target.value)} />}
        </Field>
        <StrengthMeter password={next} />
        <Field label="Repeat the new password" error={again && again !== next ? "The passwords do not match yet." : null}>
          {({ id, describedBy, invalid }) => <TextInput id={id} type="password" autoComplete="new-password" value={again} invalid={invalid} aria-describedby={describedBy} disabled={busy} onChange={(e) => setAgain(e.target.value)} />}
        </Field>
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={close} disabled={busy}>Cancel</Button>
          <Button type="submit" variant="key" busy={busy} disabled={!valid}>Change password</Button>
        </div>
      </form>
    </Sheet>
  );
}

/** Master password, clipboard and locking the open vault. */
export function SecurityPage() {
  const clipboardClear = useStore(app, (s) => s.clipboardClear);
  const demo = useStore(app, (s) => s.profile?.demo === true);
  const [changing, setChanging] = useState(false);
  return (
    <>
      <Group title="Security" heading={false}>
        <Row title="Clear copied secrets after" description="Passwords and other secrets are removed from the clipboard after this long.">
          <Segmented<ClipboardClear> label="Clipboard clear time" value={clipboardClear} onChange={setClipboardClear} options={[{ value: 15, label: "15 s" }, { value: 30, label: "30 s" }, { value: 60, label: "60 s" }]} />
        </Row>
        <Row title="Master password" description="Re-encrypts only the key that guards your vault, so it is quick.">
          <Button icon={<KeyRound className="size-4" />} onClick={() => setChanging(true)}>Change</Button>
        </Row>
        {!demo && (
          <Row title="Lock the vault" description="Wipes the vault key from memory. You will need your master password to open it again.">
            <Button icon={<Lock className="size-4" />} onClick={() => void lockVault()}>Lock now</Button>
          </Row>
        )}
      </Group>
      <ChangePassword open={changing} onClose={() => setChanging(false)} />
    </>
  );
}
