import { useEffect, useId, useMemo, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Fingerprint, KeyRound, Plus, TriangleAlert } from "lucide-react";
import { BackendError, type BridgeAction, type BridgeLogin, type BridgeMatch, type BridgeOptions, type BridgeRequest, type LoginSuggestion } from "../../backend";
import { originParts } from "../../lib/origin";
import { app, be, pushToast, refresh, syncAfterCoreLock } from "../../state/app";
import { bridge, listenForRequests, markHandled, refreshBridge } from "../../state/bridge";
import { useStore } from "../../state/store";
import { Avatar } from "../ui/Avatar";
import { Button } from "../ui/Button";
import { Field, Select, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";

/**
 * A website in a browser on this computer asks for a passkey (Settings > Browsers). The core queues each request; this
 * component listens while the switch is on and shows the one in front: a sheet over an open vault, or a note over the lock
 * screen. Nothing is signed or saved until the person chooses here.
 */

/** A click right after the sheet appears is ignored: a key the person was pressing for something else must not allow a request. */
const ARM_MS = 700;

const REASON: Record<LoginSuggestion["reasons"][number], string> = {
  account: "Same account",
  site: "Same website",
  name: "Same name",
};

const messageOf = (e: unknown) => (e instanceof Error ? e.message : "That did not work. Try again.");

export function BridgeHost() {
  const phase = useStore(app, (s) => s.phase);
  const on = useStore(app, (s) => s.device?.browserBridge === true);
  const supported = useStore(bridge, (s) => s.status?.supported === true);
  const front = useStore(bridge, (s) => s.front);
  const awake = phase === "ready" || phase === "locked" || phase === "picker";
  const listening = on && supported && awake;

  // Whether this computer can do it at all is asked once the app is past its first screens (the gate refuses before that).
  useEffect(() => {
    if (awake) void refreshBridge();
  }, [awake]);
  useEffect(() => (listening ? listenForRequests() : undefined), [listening]);

  if (!front || !listening) return null;
  return phase === "ready" ? <RequestSheet key={front.id} request={front} /> : <WaitingNote key={front.id} request={front} />;
}

// ------------------------------------------------------------------ answering

/** Ends the request the quick way (Cancel, Use another way): nothing here can fail in a way the person could fix. */
async function end(request: BridgeRequest, action: Extract<BridgeAction, { kind: "decline" | "cancel" }>) {
  markHandled(request.id);
  bridge.set((s) => (s.front?.id === request.id ? { ...s, front: null } : s));
  try {
    await be().bridgeAnswer(request.id, action);
  } catch {
    // the request had ended already: there is nothing left to answer
  }
}

/** A banner over the lock screen: a website is waiting, and the lock screen is where Hexis opens. */
function WaitingNote({ request }: { request: BridgeRequest }) {
  const { host } = originParts(request.origin);
  return (
    <div role="status" className="surface-float fixed inset-x-4 top-[calc(0.75rem+var(--safe-top))] z-[80] mx-auto flex max-w-[36rem] flex-wrap items-center gap-x-4 gap-y-3 rounded-xl px-4 py-3">
      <Fingerprint className="size-5 shrink-0 text-brass-ink" aria-hidden />
      <p className="min-w-0 flex-1 basis-56 text-[0.95em]">
        <span className="font-semibold break-all">{host}</span> {request.kind === "get" ? "wants to sign you in with a passkey." : "wants to save a passkey."} Unlock Hexis to answer.
      </p>
      <div className="flex gap-2">
        <Button variant="ghost" onClick={() => void end(request, { kind: "decline" })}>Use another way</Button>
        <Button onClick={() => void end(request, { kind: "cancel" })}>Cancel</Button>
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ the sheet

/** One choice in a list: who it is, why it fits, and a mark that it is the one picked. */
function ChoiceRow({ selected, onSelect, title, avatarTitle, subtitle, note, locked, icon }: {
  selected: boolean;
  onSelect: () => void;
  title: string;
  /** The login the tile stands for, when the line above it names someone else (the account at the website). */
  avatarTitle?: string;
  subtitle?: string | null;
  note?: string;
  locked?: boolean;
  icon?: ReactNode;
}) {
  return (
    <li role="presentation">
      <button
        type="button"
        role="radio"
        aria-checked={selected}
        onClick={onSelect}
        className={`pressable flex w-full items-center gap-3 rounded-lg p-3 text-left shadow-e1 transition-colors ${selected ? "bg-hover ring-2 ring-brass-ink/70" : "bg-pane/70 ring-1 ring-hairline hover:bg-raised"}`}
      >
        {icon ?? <Avatar title={avatarTitle ?? title} size={38} />}
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-1.5 truncate font-semibold">
            <span className="truncate">{title}</span>
            {locked && <KeyRound className="size-3.5 shrink-0 text-fog" aria-label="Asks for the master password" />}
          </span>
          {subtitle && <span className="block truncate text-[0.93em] text-mist">{subtitle}</span>}
          {note && <span className="mt-0.5 block text-[0.86em] text-fog">{note}</span>}
        </span>
      </button>
    </li>
  );
}

/** The website is the one thing to check, so it is the title of the request. */
function Site({ request, lead }: { request: BridgeRequest; lead: string }) {
  const { host, port, secure, local } = originParts(request.origin);
  return (
    <div className="flex flex-col gap-1.5 pt-1">
      <p className="font-display text-[1.6rem] leading-[1.15] font-medium tracking-[-0.005em] break-all">{host}</p>
      <p className="text-mist">{lead}</p>
      {(port || !secure) && (
        <p className="flex items-center gap-1.5 text-[0.86em] text-fog">
          {!secure && <TriangleAlert className="size-3.5 shrink-0" aria-hidden />}
          {local ? "A page on this computer, not a website." : request.origin}
          {local && port ? ` Port ${port}.` : ""}
        </p>
      )}
    </div>
  );
}

function useSecondsLeft(request: BridgeRequest): number {
  const arrivedAt = useStore(bridge, (s) => s.arrivedAt);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  return Math.max(0, Math.ceil((request.remainingMs - (now - arrivedAt)) / 1000));
}

function RequestSheet({ request }: { request: BridgeRequest }) {
  const formId = useId();
  const [options, setOptions] = useState<BridgeOptions | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [armed, setArmed] = useState(false);
  const [pick, setPick] = useState<string | null>(null); // "<itemId>/<key>" to sign in, an item id or "new" to save
  const [password, setPassword] = useState("");
  const [askPassword, setAskPassword] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shake, setShake] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const seconds = useSecondsLeft(request);

  useEffect(() => {
    const t = setTimeout(() => setArmed(true), ARM_MS);
    return () => clearTimeout(t);
  }, []);

  useEffect(() => {
    let live = true;
    be()
      .bridgeOptions(request.id)
      .then((o) => {
        if (!live) return;
        setOptions(o);
        setPick(firstChoice(o));
      })
      .catch((e: unknown) => {
        if (!live) return;
        if (e instanceof BackendError && e.code === "locked") void syncAfterCoreLock();
        setLoadError(e instanceof BackendError && e.code === "not_found" ? "The website stopped waiting." : messageOf(e));
      });
    return () => {
      live = false;
    };
  }, [request.id]);

  const selected = useMemo(() => {
    if (!options || pick === null) return null;
    if (options.kind === "get") return options.matches.find((m) => `${m.itemId}/${m.key}` === pick) ?? null;
    if (pick === "new") return null;
    return options.suggestions.find((s) => s.id === pick) ?? options.others.find((o) => o.id === pick) ?? null;
  }, [options, pick]);

  const required = options?.verification === "required";
  const itemAsks = selected?.reprompt === true;
  const needsPassword = required || itemAsks || askPassword;
  const get = request.kind === "get";
  // The password field appears after the options load, so the sheet's own first focus has gone to a button by then.
  const showPassword = options !== null && needsPassword;
  useEffect(() => {
    if (showPassword) input.current?.focus();
  }, [showPassword]);
  const ready = options !== null && pick !== null && (!needsPassword || password.length > 0) && armed && !busy;

  async function finish(action: BridgeAction) {
    setBusy(true);
    setError(null);
    try {
      await be().bridgeAnswer(request.id, action);
      markHandled(request.id);
      bridge.set((s) => (s.front?.id === request.id ? { ...s, front: null } : s));
      const { host } = originParts(request.origin);
      if (action.kind === "create") {
        await refresh().catch(() => undefined);
        pushToast({ kind: "success", text: `Saved a passkey for ${host}.` });
      } else {
        pushToast({ kind: "success", text: `Signed in to ${host} with a passkey.` });
      }
    } catch (e) {
      setBusy(false);
      const code = e instanceof BackendError ? e.code : "unknown";
      if (code === "wrong_password") {
        setError("That master password is not right.");
        setPassword("");
        setShake((n) => n + 1);
        input.current?.focus();
      } else if (code === "password_required" || code === "master_required") {
        setAskPassword(true);
        setError("Enter your master password.");
      } else if (code === "not_found") {
        // it ended while the sheet was open: the loop will take the sheet away; say why in the meantime
        setLoadError("The website stopped waiting.");
      } else {
        if (code === "locked") void syncAfterCoreLock();
        setError(messageOf(e));
      }
    }
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!ready || !options || pick === null) return;
    const pw = needsPassword ? password : undefined;
    if (options.kind === "get") {
      const m = options.matches.find((x) => `${x.itemId}/${x.key}` === pick);
      if (m) void finish({ kind: "use", itemId: m.itemId, key: m.key, ...(pw !== undefined ? { password: pw } : {}) });
    } else {
      const target = pick === "new" ? { new: { title: options.rpName || options.rpId } } : { existing: pick };
      void finish({ kind: "create", target, ...(pw !== undefined ? { password: pw } : {}) });
    }
  }

  const title = get ? "Sign in with a passkey" : "Save a passkey";
  const lead = get ? "wants to sign you in." : options?.kind === "create" && options.userName ? `wants to save a passkey for ${options.userName.slice(0, 80)}.` : "wants to save a passkey.";

  return (
    <Sheet
      open
      onClose={() => !busy && void end(request, { kind: "cancel" })}
      title={title}
      variant="dialog"
      size="md"
      level={2}
      dismissible={!busy}
      titleIcon={<Fingerprint className="size-5 text-brass-ink" aria-hidden />}
      footer={
        <div className="flex flex-wrap items-center justify-between gap-2">
          <Button variant="ghost" onClick={() => void end(request, { kind: "cancel" })} disabled={busy}>Cancel</Button>
          <div className="flex flex-wrap justify-end gap-2">
            <Button onClick={() => void end(request, { kind: "decline" })} disabled={busy}>Use another way</Button>
            <Button type="submit" form={formId} variant="key" busy={busy} disabled={!ready && !busy}>{get ? "Sign in" : "Save passkey"}</Button>
          </div>
        </div>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-5">
        <Site request={request} lead={lead} />

        {loadError ? (
          <p role="alert" className="text-[0.93em] text-ember">{loadError}</p>
        ) : !options ? (
          <p className="text-[0.93em] text-mist" role="status">Looking in your vault</p>
        ) : options.kind === "get" ? (
          <SignInChoices matches={options.matches} rpId={options.rpId} pick={pick} onPick={setPick} disabled={busy} />
        ) : (
          <SaveChoices options={options} pick={pick} onPick={setPick} disabled={busy} />
        )}

        {showPassword && (
          <div key={shake} className={shake > 0 ? "animate-shake" : ""}>
            <Field
              label="Master password"
              error={error}
              hint={required ? "This website requires it." : itemAsks ? "This login asks for it every time." : "Needed for this."}
            >
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
        )}
        {error && !needsPassword && <p role="alert" className="text-[0.86em] text-ember">{error}</p>}

        <p className="text-[0.86em] text-fog">
          {busy && needsPassword
            ? "Checking with Argon2id. This takes a moment on purpose."
            : seconds > 0 && seconds <= 30
              ? `The website stops waiting in ${seconds} ${seconds === 1 ? "second" : "seconds"}.`
              : "Use another way hands this to your browser's own passkey screen."}
        </p>
      </form>
    </Sheet>
  );
}

/** What is picked first: the only passkey, or the first; for a new passkey, a login that is clearly the same account or website. */
function firstChoice(o: BridgeOptions): string | null {
  if (o.kind === "get") {
    const m = o.matches[0];
    return m ? `${m.itemId}/${m.key}` : null;
  }
  const best = o.suggestions.find((s) => !s.full && !s.hasSitePasskey && (s.reasons.includes("account") || s.reasons.includes("site")));
  return best ? best.id : "new";
}

function SignInChoices({ matches, rpId, pick, onPick, disabled }: { matches: BridgeMatch[]; rpId: string; pick: string | null; onPick: (v: string) => void; disabled: boolean }) {
  if (matches.length === 0) {
    return <p className="text-[0.93em] text-mist">Hexis holds no passkey for {rpId}. Use another way to continue in your browser.</p>;
  }
  return (
    <div className="flex flex-col gap-2">
      {matches.length > 1 && <p className="text-[0.86em] font-medium text-mist" id="bridge-choices">Choose the account</p>}
      <ul role="radiogroup" aria-labelledby={matches.length > 1 ? "bridge-choices" : undefined} aria-label={matches.length > 1 ? undefined : "Passkey"} aria-disabled={disabled} className="flex flex-col gap-2">
        {matches.map((m) => (
          <ChoiceRow
            key={`${m.itemId}/${m.key}`}
            selected={pick === `${m.itemId}/${m.key}`}
            onSelect={() => onPick(`${m.itemId}/${m.key}`)}
            title={m.userName || m.title}
            avatarTitle={m.title}
            subtitle={m.userName ? m.title : m.subtitle}
            locked={m.reprompt}
          />
        ))}
      </ul>
    </div>
  );
}

function SaveChoices({ options, pick, onPick, disabled }: { options: Extract<BridgeOptions, { kind: "create" }>; pick: string | null; onPick: (v: string) => void; disabled: boolean }) {
  const [more, setMore] = useState(false);
  const taken = options.suggestions.some((s) => s.id === pick) || pick === "new";
  const chosenOther = !taken ? options.others.find((o: BridgeLogin) => o.id === pick) : undefined;
  return (
    <div className="flex flex-col gap-2">
      <p className="text-[0.86em] font-medium text-mist" id="bridge-save-on">Save it on</p>
      <ul role="radiogroup" aria-labelledby="bridge-save-on" aria-disabled={disabled} className="flex flex-col gap-2">
        {options.suggestions.map((s) => (
          <ChoiceRow
            key={s.id}
            selected={pick === s.id}
            onSelect={() => onPick(s.id)}
            title={s.title}
            subtitle={s.subtitle}
            note={s.full ? "No room for another passkey" : s.hasSitePasskey ? "Already has a passkey for this website" : s.reasons.map((r) => REASON[r]).join(", ")}
            locked={s.reprompt}
          />
        ))}
        <ChoiceRow
          selected={pick === "new"}
          onSelect={() => onPick("new")}
          title="Create a new login"
          subtitle={options.rpName || options.rpId}
          icon={<span className="grid size-[38px] shrink-0 place-items-center rounded-lg bg-raised text-brass-ink" aria-hidden><Plus className="size-5" /></span>}
        />
        {chosenOther && <ChoiceRow selected onSelect={() => onPick(chosenOther.id)} title={chosenOther.title} subtitle={chosenOther.subtitle} locked={chosenOther.reprompt} />}
      </ul>
      {options.others.length > 0 &&
        (more ? (
          <Select
            aria-label="Another login"
            value={chosenOther?.id ?? ""}
            disabled={disabled}
            onChange={(e) => e.target.value && onPick(e.target.value)}
          >
            <option value="">Choose another login</option>
            {options.others.map((o: BridgeLogin) => (
              <option key={o.id} value={o.id}>{o.subtitle ? `${o.title} (${o.subtitle})` : o.title}</option>
            ))}
          </Select>
        ) : (
          <button type="button" onClick={() => setMore(true)} className="pressable self-start rounded-sm px-1 py-1 text-[0.93em] text-brass-ink hover:underline">
            Choose another login
          </button>
        ))}
    </div>
  );
}
