import { useEffect, useRef, useState, type FormEvent } from "react";
import { KeyRound, ShieldAlert } from "lucide-react";
import { BackendError } from "../../backend";
import { createStore, useStore } from "../../state/store";
import { Button } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";

/**
 * The Master-Password Re-Prompt Guard (spec §2.3). Every gated action calls `reprompt()`:
 * the user types the master password, `run(password)` executes the gated Rust command (which verifies it with
 * Argon2id and never caches it), and the promise resolves with the command's result. A wrong password keeps
 * the sheet open, shakes it once and lets the user retry. The password lives only in this component's state
 * and is cleared as soon as the sheet closes.
 */
export interface RepromptOptions<T> {
  /** What the person is about to do, in plain words: "Reveal the password for GitHub". */
  reason: string;
  /** Why this item asks at all, shown quietly under the reason. */
  because?: string;
  confirmLabel?: string;
  tone?: "key" | "danger";
  run: (password: string) => Promise<T>;
}

interface Pending {
  opts: RepromptOptions<unknown>;
  resolve: (value: unknown) => void;
}

const store = createStore<Pending | null>(null);

/**
 * Runs a command on ONE item the way the core wants it. Most items work without the master password while the vault is
 * open, so the command is sent without one first. An item that is set to "ask for the master password again" answers
 * `password_required` (no Argon2id run, nothing changed), and only then does the sheet open and the command run again
 * with the password. Resolves with the command's result, or `undefined` when the person cancels the sheet.
 */
export async function withItemPassword<T>(opts: Omit<RepromptOptions<T>, "run"> & { run: (password?: string) => Promise<T> }): Promise<T | undefined> {
  try {
    return await opts.run(undefined);
  } catch (e) {
    if (!(e instanceof BackendError && e.code === "password_required")) throw e;
  }
  return reprompt({ because: "This item is set to ask for the master password every time.", ...opts, run: (pw) => opts.run(pw) });
}

/** Opens the guard. Resolves with the result of `run`, or `undefined` when the person cancels. */
export function reprompt<T>(opts: RepromptOptions<T>): Promise<T | undefined> {
  return new Promise((resolve) => {
    store.get()?.resolve(undefined); // a second request supersedes a stale one
    store.set({ opts: opts as RepromptOptions<unknown>, resolve: resolve as (v: unknown) => void });
  });
}

export function RepromptHost() {
  const pending = useStore(store, (s) => s);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shake, setShake] = useState(0);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setPassword("");
    setError(null);
    setBusy(false);
  }, [pending]);

  const close = (value?: unknown) => {
    const p = store.get();
    setPassword("");
    store.set(null);
    p?.resolve(value);
  };

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!pending || busy) return;
    if (!password) return setError("Enter your master password.");
    setBusy(true);
    setError(null);
    try {
      const result = await pending.opts.run(password);
      close(result);
    } catch (err) {
      setBusy(false);
      if (err instanceof BackendError && err.code === "wrong_password") {
        setError("That master password is not right.");
        setPassword("");
        setShake((n) => n + 1);
        input.current?.focus();
      } else {
        setError(err instanceof Error ? err.message : "Something went wrong.");
      }
    }
  }

  const tone = pending?.opts.tone ?? "key";
  return (
    <Sheet
      open={pending !== null}
      onClose={() => !busy && close(undefined)}
      title="Confirm it is you"
      variant="dialog"
      size="sm"
      level={2}
      dismissible={!busy}
      titleIcon={tone === "danger" ? <ShieldAlert className="size-5 text-ember" aria-hidden /> : <KeyRound className="size-5 text-brass-ink" aria-hidden />}
    >
      <form onSubmit={submit} className="flex flex-col gap-4 pt-1">
        <div className="flex flex-col gap-1.5">
          <p className="text-mist">{pending?.opts.reason}</p>
          {pending?.opts.because && <p className="text-[0.86em] text-fog">{pending.opts.because}</p>}
        </div>
        <div key={shake} className={shake > 0 ? "animate-shake" : ""}>
          <Field label="Master password" error={error}>
            {({ id, describedBy, invalid }) => (
              <TextInput
                id={id}
                ref={input}
                data-autofocus
                type="password"
                autoComplete="current-password"
                value={password}
                disabled={busy}
                invalid={invalid}
                aria-describedby={describedBy}
                onChange={(e) => setPassword(e.target.value)}
              />
            )}
          </Field>
        </div>
        <p className="text-[0.86em] text-fog">
          {busy ? "Checking with Argon2id. This takes a moment on purpose." : "Your password is checked on this device and is not stored."}
        </p>
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={() => close(undefined)} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant={tone === "danger" ? "danger" : "key"} busy={busy}>
            {pending?.opts.confirmLabel ?? "Continue"}
          </Button>
        </div>
      </form>
    </Sheet>
  );
}
