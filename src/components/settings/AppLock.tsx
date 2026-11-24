import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import { Fingerprint, ScanFace, ShieldCheck } from "lucide-react";
import { BackendError } from "../../backend";
import type { AppSettings, GateConfigInput, GateMode, GateStatus } from "../../backend";
import { LOCK_AFTER_LEAVE_SECONDS, LOCK_AFTER_MINUTES } from "../../backend/types";
import { idleLabel, leaveLabel } from "../../lib/format";
import { credentialProblem, panicProblems } from "../../lib/gateRules";
import { isMobileShell } from "../../lib/platform";
import { be, pushToast, refreshGate } from "../../state/app";
import { app } from "../../state/app";
import { useStore } from "../../state/store";
import { GridPad } from "../gate/GridPad";
import { Button } from "../ui/Button";
import { Field, Segmented, Select, Switch, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";

const MODE_LABEL: Record<GateMode, string> = { pin: "PIN", password: "Password", grid: "Number grid" };

function Row({ title, description, children }: { title: string; description?: ReactNode; children?: ReactNode }) {
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

/** Entering a credential in the way the chosen mode asks for it: digits, text, or a pattern on the grid. */
function CredentialInput({ mode, label, value, onChange, disabled, error }: { mode: GateMode; label: string; value: string; onChange: (v: string) => void; disabled?: boolean; error?: string | null }) {
  if (mode === "grid") {
    return (
      <Field label={label} error={error} hint="Tap numbers or drag across them. Each number can be used once.">
        {() => (
          <div className="flex flex-col items-center gap-4 rounded-lg bg-pane/70 p-4">
            <p className="min-h-8 font-mono text-[1.3em] tracking-[0.3em]" aria-live="polite">{value || "·"}</p>
            <GridPad
              disabled={disabled}
              onDigit={(d) => { if (!value.includes(d)) onChange(value + d); }}
              onBackspace={() => onChange(value.slice(0, -1))}
              onPattern={(d) => onChange(d)}
              onSubmit={() => undefined}
            />
          </div>
        )}
      </Field>
    );
  }
  return (
    <Field label={label} error={error}>
      {({ id, describedBy, invalid }) => (
        <TextInput
          id={id} aria-describedby={describedBy} invalid={invalid} disabled={disabled} value={value} mono
          type={mode === "pin" ? "tel" : "password"} inputMode={mode === "pin" ? "numeric" : "text"}
          autoComplete="off" onChange={(e) => onChange(mode === "pin" ? e.target.value.replace(/\D/g, "") : e.target.value)}
        />
      )}
    </Field>
  );
}

function AppLockSheet({ open, onClose, gate }: { open: boolean; onClose: () => void; gate: GateStatus }) {
  const [enabled, setEnabled] = useState(true);
  const [mode, setMode] = useState<GateMode>(gate.mode);
  const [credential, setCredential] = useState("");
  const [again, setAgain] = useState("");
  const [reversePanic, setReversePanic] = useState(true);
  const [panicCode, setPanicCode] = useState("");
  const [biometric, setBiometric] = useState(gate.biometric);
  const [panicButton, setPanicButton] = useState(gate.panicButton);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setEnabled(true); setMode(gate.mode); setCredential(""); setAgain(""); setReversePanic(true); setPanicCode("");
    setBiometric(gate.biometric); setPanicButton(gate.panicButton); setPassword(""); setError(null); setBusy(false);
  }, [open, gate.mode, gate.biometric, gate.panicButton]);

  const changed = credential !== "" || !gate.enabled || mode !== gate.mode;
  const credProblem = credentialProblem(mode, credential);
  const panic = panicProblems(mode, credential, reversePanic, panicCode);
  const mismatch = changed && again !== "" && again !== credential ? "These do not match yet." : null;
  const valid = !enabled
    ? password.length > 0
    : password.length > 0 && (!changed || (credential !== "" && !credProblem && again === credential)) && !panic.reverse && !panic.panic;

  function pickMode(m: GateMode) { setMode(m); setCredential(""); setAgain(""); setPanicCode(""); }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!valid || busy) return;
    setBusy(true); setError(null);
    const input: GateConfigInput = {
      enabled, mode, credential: credential || null, reversePanic, panicCode: panicCode === "" ? (gate.enabled ? "" : null) : panicCode,
      biometric: biometric && gate.biometricAvailable, panicButton, masterPassword: password,
    };
    try {
      await be().gateConfigure(input);
      await refreshGate();
      onClose();
      pushToast({ kind: "success", text: enabled ? "App lock is on. It asks for your passcode before the account list." : "App lock is off." });
    } catch (err) {
      setBusy(false);
      setError(err instanceof BackendError && err.code === "wrong_password" ? "That is not the master password of this vault." : err instanceof Error ? err.message : "Could not save the app lock.");
    }
  }

  return (
    <Sheet open={open} onClose={() => !busy && onClose()} title="App lock" variant="form" size="md" dismissible={!busy} titleIcon={<ShieldCheck className="size-5 text-brass" aria-hidden />}>
      <form onSubmit={submit} className="flex flex-col gap-5 pt-1" noValidate>
        <div className="flex items-center justify-between gap-4">
          <div>
            <p className="font-medium">Ask before showing the account list</p>
            <p className="text-[0.93em] text-mist">Looks like a small maths exercise. It only guards the list: every vault still needs its master password, and nothing here can open one.</p>
          </div>
          <Switch label="App lock" checked={enabled} onChange={setEnabled} />
        </div>

        {enabled && (
          <>
            <div className="flex flex-col gap-2">
              <p className="font-medium">How you enter it</p>
              <Segmented<GateMode> label="Passcode type" value={mode} onChange={pickMode} options={(Object.keys(MODE_LABEL) as GateMode[]).map((m) => ({ value: m, label: MODE_LABEL[m] }))} />
            </div>
            <CredentialInput mode={mode} label={gate.enabled && mode === gate.mode ? "New passcode (leave empty to keep the current one)" : "Passcode"} value={credential} onChange={setCredential} disabled={busy} error={credProblem} />
            {credential !== "" && !credProblem && <CredentialInput mode={mode} label="Enter it again" value={again} onChange={setAgain} disabled={busy} error={mismatch} />}
            <p className="-mt-2 text-[0.86em] text-fog">Use at least 4. Anything you type with 1 to 3 characters is treated as an answer to the maths and is never checked as a passcode.</p>

            <div className="flex flex-col gap-4 rounded-lg bg-pane/70 p-4 shadow-e1">
              <div className="flex items-start justify-between gap-4">
                <div>
                  <p className="font-medium">Erase everything if the passcode is typed backwards</p>
                  <p className="text-[0.93em] text-mist">If someone makes you open the app, type your passcode in reverse. All vaults on this device are erased at once and the app looks newly installed. Your S3 backups are not touched.</p>
                  {panic.reverse && <p role="alert" className="mt-2 text-[0.93em] text-ember">{panic.reverse}</p>}
                </div>
                <Switch label="Erase on reversed passcode" checked={reversePanic} onChange={setReversePanic} />
              </div>
              <CredentialInput mode={mode} label="Panic code (optional)" value={panicCode} onChange={setPanicCode} disabled={busy} error={panic.panic} />
              <div className="flex items-start justify-between gap-4">
                <div>
                  <p className="font-medium">Show a discreet panic mark on the lock screen</p>
                  <p className="text-[0.93em] text-mist">A small dot. Press and hold it, or double-tap it, to erase everything.</p>
                </div>
                <Switch label="Panic mark" checked={panicButton} onChange={setPanicButton} />
              </div>
              <p className="text-[0.86em] text-fog">Fifteen wrong passcodes in a row erase everything too. Wrong answers to the maths do not count.</p>
            </div>

            <div className="flex items-start justify-between gap-4">
              <div>
                <p className="flex items-center gap-2 font-medium"><Fingerprint className="size-4 text-brass" aria-hidden /> Fingerprint</p>
                <p className="text-[0.93em] text-mist">{gate.biometricAvailable ? "Also opens the account list. It never opens a vault." : "Not available on this device. Fingerprint unlock is for Android."}</p>
              </div>
              <Switch label="Fingerprint" checked={biometric && gate.biometricAvailable} onChange={setBiometric} />
            </div>
          </>
        )}

        <Field label="Master password of this vault" error={error} hint="Changing the app lock needs your master password.">
          {({ id, describedBy, invalid }) => <TextInput id={id} type="password" autoComplete="current-password" value={password} invalid={invalid} aria-describedby={describedBy} disabled={busy} onChange={(e) => setPassword(e.target.value)} />}
        </Field>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose} disabled={busy}>Cancel</Button>
          <Button type="submit" variant="key" busy={busy} disabled={!valid}>{enabled ? "Save app lock" : "Turn off"}</Button>
        </div>
      </form>
    </Sheet>
  );
}

/** The "App lock" group: Stage 1 setup and the inactivity timer for open vaults. */
export function AppLockGroup({ heading = true }: { heading?: boolean }) {
  const gate = useStore(app, (s) => s.gate);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => { void be().appSettingsGet().then(setSettings).catch(() => undefined); }, []);

  async function change(next: Partial<AppSettings>) {
    const prev = settings;
    if (!prev) return;
    const wanted = { ...prev, ...next };
    setSettings(wanted);
    try { setSettings(await be().appSettingsSet(wanted)); } catch { setSettings(prev); pushToast({ kind: "error", text: "Could not change the lock timer." }); }
  }

  return (
    <section aria-label="App lock" className="flex flex-col gap-2">
      {heading && <h3 className="px-1 font-semibold">App lock</h3>}
      <div className="surface overflow-hidden rounded-lg">
        <Row
          title="Passcode before the account list"
          description={gate?.enabled ? `On, with a ${MODE_LABEL[gate.mode].toLowerCase()}${gate.biometric ? " or fingerprint" : ""}. Every vault still asks for its master password.` : "Off. Add a passcode or fingerprint that must be passed before anyone sees which vaults exist."}
        >
          <Button icon={<ScanFace className="size-4" />} onClick={() => setOpen(true)} disabled={!gate}>{gate?.enabled ? "Change" : "Set up"}</Button>
        </Row>
        <Row title="Lock an open vault after" description="No taps or typing for this long and the vault key is wiped from memory. You will need the master password again.">
          {settings && (
            <Select aria-label="Lock after no activity for" value={settings.lockAfterMinutes} onChange={(e) => void change({ lockAfterMinutes: Number(e.target.value) as AppSettings["lockAfterMinutes"] })}>
              {LOCK_AFTER_MINUTES.map((m) => <option key={m} value={m}>{idleLabel(m)}</option>)}
            </Select>
          )}
        </Row>
        {isMobileShell() && (
          <Row title="Lock after you leave the app" description="How long Hexis stays open while you use another app. Coming back after this time asks for the master password. “Right away” locks the moment you leave.">
            {settings && (
              <Select aria-label="Lock after leaving the app for" value={settings.lockAfterLeaveSeconds} onChange={(e) => void change({ lockAfterLeaveSeconds: Number(e.target.value) as AppSettings["lockAfterLeaveSeconds"] })}>
                {LOCK_AFTER_LEAVE_SECONDS.map((s) => <option key={s} value={s}>{leaveLabel(s)}</option>)}
              </Select>
            )}
          </Row>
        )}
      </div>
      {gate && <AppLockSheet open={open} onClose={() => setOpen(false)} gate={gate} />}
    </section>
  );
}
