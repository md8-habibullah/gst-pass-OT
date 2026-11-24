import { useCallback, useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { motion } from "motion/react";
import { DUR, tween } from "../../lib/motion";
import { ArrowLeft, ChevronRight, CloudDownload, Eye, EyeOff, FileLock2, Plus, ShieldPlus, UsersRound } from "lucide-react";
import { BackendError } from "../../backend";
import { relativeTime } from "../../lib/format";
import { app, be, enterApp, isLabelledDemo, openDemo, selectProfile, showPicker, unlock } from "../../state/app";
import { brand } from "../../state/brand";
import { useStore } from "../../state/store";
import { setLockIntent, ui, type LockFlow } from "../../state/ui";
import { Avatar } from "../ui/Avatar";
import { Button, IconButton } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { CreateFlow } from "./CreateFlow";
import { ImportFlow, RestoreFlow } from "./JoinFlows";
import { Door, DoorCard } from "./Door";
import { VaultMark } from "./VaultMark";

type Flow = LockFlow | null;

/** The three ways to get a vault onto this device. Shown on first run and from "Add account". */
function AddChoices({ onChoose, first }: { onChoose: (f: LockFlow) => void; first: boolean }) {
  const brandName = useStore(brand, (b) => b.name);
  const rows: { flow: LockFlow; icon: ReactNode; title: string; text: string }[] = [
    { flow: "create", icon: <ShieldPlus className="size-5" />, title: "Create a new vault", text: "Pick a name and a master password. You can set up backups right after." },
    { flow: "import", icon: <FileLock2 className="size-5" />, title: "Open a vault file", text: `Add a vault you exported from ${brandName} on another device.` },
    { flow: "restore", icon: <CloudDownload className="size-5" />, title: "Restore from a backup", text: "Download your vault from an S3 bucket you backed up to." },
  ];
  return (
    <div className="flex flex-col gap-2.5">
      {rows.map((r, i) => (
        <button key={r.flow} type="button" onClick={() => onChoose(r.flow)} className={`pressable group flex items-center gap-4 rounded-lg border px-4 py-3.5 text-left pointer-coarse:py-4 ${first && i === 0 ? "border-hairline-strong bg-raised hover:bg-hover" : "border-hairline bg-pane hover:bg-raised"}`}>
          <span className="grid size-11 shrink-0 place-items-center rounded-lg bg-marine text-brass-ink">{r.icon}</span>
          <span className="min-w-0 flex-1">
            <span className="block font-semibold">{r.title}</span>
            <span className="block text-[0.93em] text-mist">{r.text}</span>
          </span>
          <ChevronRight className="size-4 shrink-0 text-fog group-hover:text-frost" aria-hidden />
        </button>
      ))}
    </div>
  );
}

function Picker({ onAdd }: { onAdd: () => void }) {
  const profiles = useStore(app, (s) => s.profiles);
  const device = useStore(app, (s) => s.device);
  return (
    <div className="flex flex-col gap-3">
      <ul className="flex flex-col gap-1.5" aria-label="Accounts">
        {profiles.map((p) => (
          <li key={p.id}>
            <button type="button" onClick={() => void selectProfile(p.id)} className="pressable flex min-h-16 w-full items-center gap-3.5 rounded-lg border border-hairline bg-raised px-3.5 text-left hover:bg-hover">
              <Avatar title={p.displayName} size={42} />
              <span className="min-w-0 flex-1">
                <span className="block truncate font-semibold">{p.displayName}</span>
                <span className="block text-[0.86em] text-mist">{isLabelledDemo(device, p) ? "Demo vault. Opens without a password." : p.lastUsedAt ? `Last opened ${relativeTime(p.lastUsedAt)}` : "Not opened yet"}</span>
              </span>
              <ChevronRight className="size-4 shrink-0 text-fog" aria-hidden />
            </button>
          </li>
        ))}
      </ul>
      <Button block icon={<Plus className="size-4" />} onClick={onAdd} className="min-h-11">Add account</Button>
    </div>
  );
}

function UnlockForm({ busyExternal, onUnlocked }: { busyExternal: boolean; onUnlocked: () => void }) {
  const profile = useStore(app, (s) => s.profile);
  const profileCount = useStore(app, (s) => s.profiles.length);
  const [password, setPassword] = useState("");
  const [reveal, setReveal] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shake, setShake] = useState(0);
  const input = useRef<HTMLInputElement>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (busy || busyExternal || !password) return;
    setBusy(true);
    setError(null);
    try {
      await unlock(password);
      setPassword("");
      onUnlocked();
    } catch (err) {
      setBusy(false);
      setShake((n) => n + 1);
      setError(err instanceof BackendError && err.code === "wrong_password" ? "That master password is not right." : err instanceof Error ? err.message : "Something went wrong.");
      setPassword("");
      input.current?.focus();
    }
  }

  return (
    <div key={shake} className={shake > 0 ? "animate-shake" : ""}>
      <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
        <Field label="Master password" error={error}>
          {({ id, describedBy, invalid }) => (
            <div className="relative">
              <TextInput id={id} ref={input} data-autofocus type={reveal ? "text" : "password"} autoComplete="current-password" value={password} disabled={busy || busyExternal} invalid={invalid} aria-describedby={describedBy} onChange={(e) => setPassword(e.target.value)} className="pr-11" mono={reveal} />
              <IconButton label={reveal ? "Hide password" : "Show password"} size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => setReveal((r) => !r)}>
                {reveal ? <EyeOff className="size-[17px]" /> : <Eye className="size-[17px]" />}
              </IconButton>
            </div>
          )}
        </Field>
        <Button type="submit" variant="key" block busy={busy} disabled={busy || busyExternal || !password} className="mt-1 min-h-11">
          {busy ? "Unlocking" : "Unlock"}
        </Button>
        {busy && <p className="-mt-1 text-center text-[0.86em] text-fog">Deriving your key with Argon2id. This takes a moment on purpose.</p>}
      </form>
      <div className="mt-4 flex justify-center border-t border-hairline pt-3">
        <Button variant="ghost" icon={<UsersRound className="size-4" />} onClick={() => void showPicker()}>
          {profileCount > 1 ? `Not ${profile?.displayName ?? "you"}? Choose another account` : "Add or switch account"}
        </Button>
      </div>
    </div>
  );
}

/** Opens the built-in vault: no password is asked, because the vault holds nothing real. A failure shows the normal form. */
function DemoOpening({ onOpened, onFail }: { onOpened: () => void; onFail: () => void }) {
  const profile = useStore(app, (s) => s.profile);
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    openDemo().then(onOpened, onFail);
  }, [onOpened, onFail]);
  return <p className="py-4 text-center text-mist" role="status">Opening {profile?.displayName ?? "your vault"}</p>;
}

/** The lock screen: accounts, unlock, and the flows that add an account. Plays the unlock moment. */
export function LockScreen() {
  const brandName = useStore(brand, (b) => b.name);
  const phase = useStore(app, (s) => s.phase);
  const profile = useStore(app, (s) => s.profile);
  const intent = useStore(ui, (s) => s.lockIntent);
  const [flow, setFlow] = useState<Flow>(() => { const i = ui.get().lockIntent; return i === "choose" ? null : i; });
  const [adding, setAdding] = useState(() => ui.get().lockIntent === "choose");
  const [opening, setOpening] = useState(false);
  const [demoFailed, setDemoFailed] = useState(false);

  // "Add account" elsewhere in the app asks the lock screen to start a flow.
  useEffect(() => {
    if (!intent) return;
    if (intent === "choose") { setFlow(null); setAdding(true); } else setFlow(intent);
    setLockIntent(null);
  }, [intent]);
  // Selecting an account (or finishing an import) always ends any flow.
  useEffect(() => {
    if (phase === "locked") { setFlow(null); setAdding(false); }
  }, [phase, profile?.id]);
  useEffect(() => setDemoFailed(false), [profile?.id]);

  /** The unlock moment: the mark closes, the card fades, then the app is there. */
  const finish = useCallback(() => {
    setOpening(true);
    window.setTimeout(enterApp, 200);
  }, []);
  const failDemo = useCallback(() => setDemoFailed(true), []);

  const demoUnlock = phase === "locked" && !flow && profile?.demo === true && !demoFailed;
  const heading =
    flow === "create" ? { title: "Create your vault", text: "Your master password protects everything. It never leaves this device and cannot be recovered." }
    : flow === "import" || flow === "restore" ? null
    : phase === "welcome" ? { title: "Add a vault", text: `Start with a new vault, or bring one you already have into ${brandName}.` }
    : phase === "picker" && adding ? { title: "Add an account", text: "Each account is its own vault with its own master password." }
    : phase === "picker" ? { title: "Choose an account", text: "Each account is its own vault with its own master password." }
    : demoUnlock ? { title: profile?.displayName ?? brandName, text: "" }
    : { title: profile?.displayName ?? brandName, text: "Your vault is locked." };
  const wide = flow === "restore";
  const unlockView = phase === "locked" && !flow;

  return (
    <Door wide={wide}>
      <motion.div
        initial={{ opacity: 0, y: 6 }}
        animate={opening ? { opacity: 0, transition: tween(DUR.base) } : { opacity: 1, y: 0, transition: tween(DUR.slow) }}
      >
        <DoorCard>
          <div className="mb-6 flex flex-col items-center gap-4 text-center">
            {unlockView && profile ? (
              <div className="relative">
                <Avatar title={profile.displayName} size={68} />
                <span className="absolute -right-2 -bottom-2 rounded-full bg-marine p-1"><VaultMark size={26} open={opening} /></span>
              </div>
            ) : (
              <VaultMark size={64} open={opening} />
            )}
            {heading && (
              <div>
                <h1 className="font-display text-[1.9rem] leading-[1.15] font-medium tracking-[-0.01em] break-words">{heading.title}</h1>
                {heading.text && <p className="mx-auto mt-2 max-w-[22rem] text-mist">{heading.text}</p>}
              </div>
            )}
          </div>

          {flow === "create" ? (
            <CreateFlow onBack={() => setFlow(null)} onCreated={() => undefined} onFinish={finish} />
          ) : flow === "import" ? (
            <ImportFlow onBack={() => setFlow(null)} onDone={() => setFlow(null)} />
          ) : flow === "restore" ? (
            <RestoreFlow onBack={() => setFlow(null)} onDone={() => setFlow(null)} />
          ) : phase === "welcome" ? (
            <AddChoices first onChoose={setFlow} />
          ) : phase === "picker" && adding ? (
            <div className="flex flex-col gap-3">
              <AddChoices first={false} onChoose={setFlow} />
              <Button variant="ghost" icon={<ArrowLeft className="size-4" />} onClick={() => setAdding(false)}>Back to accounts</Button>
            </div>
          ) : phase === "picker" ? (
            <Picker onAdd={() => setAdding(true)} />
          ) : demoUnlock ? (
            <DemoOpening onOpened={finish} onFail={failDemo} />
          ) : (
            <UnlockForm busyExternal={opening} onUnlocked={finish} />
          )}
        </DoorCard>
        {import.meta.env.DEV && be().kind === "preview" && (
          <p className="mt-4 text-center text-[0.8em] text-fog">Preview backend: in-memory demo data, not secure. Password: preview-vault-1.</p>
        )}
      </motion.div>
    </Door>
  );
}
