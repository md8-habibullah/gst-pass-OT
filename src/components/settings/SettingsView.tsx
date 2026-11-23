import { useState, type FormEvent, type ReactNode } from "react";
import { KeyRound, Lock } from "lucide-react";
import { BackendError } from "../../backend";
import { app, be, lockVault, pushToast, setClipboardClear, type ClipboardClear } from "../../state/app";
import { useStore } from "../../state/store";
import { StrengthMeter } from "../lock/StrengthMeter";
import { Button } from "../ui/Button";
import { Field, Segmented, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section aria-label={title} className="flex flex-col gap-2">
      <h3 className="px-1 font-semibold">{title}</h3>
      <div className="glass overflow-hidden rounded-lg">{children}</div>
    </section>
  );
}

function Row({ title, description, children }: { title: string; description?: string; children?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-x-6 gap-y-3 border-b border-hairline px-4 py-3.5 last:border-b-0">
      <div className="min-w-0 flex-1 basis-60">
        <p className="font-medium">{title}</p>
        {description && <p className="mt-0.5 text-[0.93em] text-mist">{description}</p>}
      </div>
      {children}
    </div>
  );
}

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

export function SettingsView() {
  const clipboardClear = useStore(app, (s) => s.clipboardClear);
  const suite = useStore(app, (s) => s.suite);
  const [changing, setChanging] = useState(false);
  const preview = import.meta.env.DEV && be().kind === "preview";

  return (
    <section aria-label="Settings" className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain">
      <div className="mx-auto flex w-full max-w-[40rem] flex-col gap-6 px-5 pt-[calc(1rem+var(--safe-top))] pb-28 min-[720px]:pt-5 min-[720px]:pb-10">
        <h2 className="font-display text-[1.45rem] leading-none font-semibold">Settings</h2>

        <Group title="Security">
          <Row title="Clear copied secrets after" description="Passwords and other secrets are removed from the clipboard after this long.">
            <Segmented<ClipboardClear> label="Clipboard clear time" value={clipboardClear} onChange={setClipboardClear} options={[{ value: 15, label: "15 s" }, { value: 30, label: "30 s" }, { value: 60, label: "60 s" }]} />
          </Row>
          <Row title="Master password" description="Re-encrypts only the key that guards your vault, so it is quick.">
            <Button icon={<KeyRound className="size-4" />} onClick={() => setChanging(true)}>Change</Button>
          </Row>
          <Row title="Lock the vault" description="Wipes the vault key from memory. You will need your master password to open it again.">
            <Button icon={<Lock className="size-4" />} onClick={() => void lockVault()}>Lock now</Button>
          </Row>
        </Group>

        <Group title="Storage">
          <Row title="Local only" description="Your vault is stored encrypted on this device. Sync to your own S3 buckets arrives in a later update.">
            <span className="inline-flex items-center gap-2 text-[0.93em] text-verdigris"><span className="size-2 rounded-full bg-verdigris" aria-hidden />Healthy</span>
          </Row>
        </Group>

        <Group title="About this vault">
          {suite && (
            <>
              <Row title="Key protection" description={`${suite.kem}, so a future quantum computer cannot unwrap your keys from stored data.`} />
              <Row title="Password hashing" description={`${suite.kdf}, ${suite.kdfDefaults.mCostKiB / 1024} MiB of memory, ${suite.kdfDefaults.tCost} passes, ${suite.kdfDefaults.pCost} lanes.`} />
              <Row title="Encryption" description={`${suite.aead} with ${suite.keyBytes * 8}-bit keys and a fresh ${suite.nonceBytes * 8}-bit random nonce for every object.`} />
              <Row title="Format" description={`Version ${suite.version}. Every item and every version is a separate authenticated envelope.`} />
            </>
          )}
          {preview && <Row title="Preview backend" description="This build is talking to an in-memory demo, not the encrypted Rust core. Nothing here is secure." />}
        </Group>
      </div>
      <ChangePassword open={changing} onClose={() => setChanging(false)} />
    </section>
  );
}
