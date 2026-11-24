import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Check, Copy, Eye, EyeOff, KeyRound } from "lucide-react";
import { be } from "../../state/app";
import { withItemPassword } from "../reprompt/Reprompt";
import { charClass } from "../../lib/password";
import { groupDigits } from "../../lib/format";
import { ringTone, secondsLeft, TONE_COLOR } from "../../lib/totp";
import { BackendError, type TotpNow } from "../../backend";
import { Button, IconButton } from "../ui/Button";
import { CountdownRing } from "../ui/Ring";

/** Passwords are tinted by character class (letters, digits, symbols) so they can be read aloud or retyped. */
export function PasswordText({ value }: { value: string }) {
  const parts = useMemo(() => [...value].map((ch) => ({ ch, cls: charClass(ch) })), [value]);
  return (
    <span className="font-mono text-[1.02em] leading-6 font-medium tracking-[0.02em] break-all">
      {parts.map((p, i) => (
        <span key={i} className={p.cls === "digit" ? "text-brass-hi" : p.cls === "symbol" ? "text-verdigris" : "text-frost"}>
          {p.ch}
        </span>
      ))}
    </span>
  );
}

/** A copy button that confirms with a check for a moment (the "detent"). */
export function CopyButton({ label, onCopy, disabled }: { label: string; onCopy: () => Promise<void> | void; disabled?: boolean }) {
  const [done, setDone] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return (
    <IconButton
      label={label}
      disabled={disabled}
      tone={done ? "key" : "default"}
      onClick={async () => {
        try {
          await onCopy();
          setDone(true);
          clearTimeout(timer.current);
          timer.current = setTimeout(() => setDone(false), 1400);
        } catch {
          /* the caller shows its own error */
        }
      }}
    >
      {done ? <Check className="size-[18px]" /> : <Copy className="size-[18px]" />}
    </IconButton>
  );
}

/** One line in the grouped credentials panel: caption, value, trailing actions. Hairline-separated, not boxed. */
export function FieldRow({ label, children, actions }: { label: string; children: ReactNode; actions?: ReactNode }) {
  return (
    <div className="flex items-center gap-3 border-b border-hairline px-4 py-3 last:border-b-0 pointer-coarse:py-3.5">
      <div className="min-w-0 flex-1">
        <p className="text-[0.86em] font-medium text-fog">{label}</p>
        <div className="mt-0.5 min-w-0">{children}</div>
      </div>
      {actions && <div className="flex shrink-0 items-center gap-0.5">{actions}</div>}
    </div>
  );
}

/** Hidden: a frosted pill with a fixed number of dots (never the real length). */
export function HiddenPill({ note }: { note?: boolean }) {
  return (
    <span className={`frosted inline-flex items-center rounded-sm px-3 text-fog ${note ? "min-h-8" : "h-7"}`} aria-label="Hidden">
      <span aria-hidden className="font-mono tracking-[0.28em]">{note ? "••••••••••••" : "••••••••••"}</span>
    </span>
  );
}

/** `asks` is true for an item set to ask for the master password again: the label says so before the person taps. */
export function RevealToggle({ revealed, onReveal, onHide, asks = false }: { revealed: boolean; onReveal: () => void; onHide: () => void; asks?: boolean }) {
  return revealed ? (
    <IconButton label="Hide" onClick={onHide}>
      <EyeOff className="size-[18px]" />
    </IconButton>
  ) : (
    <IconButton label={asks ? "Reveal (asks for your master password)" : "Reveal"} onClick={onReveal}>
      <Eye className="size-[18px]" />
    </IconButton>
  );
}

/**
 * Live code + ring. The secret never reaches the UI: only the current code and its validity window.
 *
 * An item that is set to ask for the master password again does not show a live code. It reports `locked` instead, and
 * `unlock()` asks for the password and shows the code for its own 30 seconds (or whatever the period is), then locks again.
 */
export function useTotp(id: string, enabled: boolean, title = "this item") {
  const [now, setNow] = useState<TotpNow | null>(null);
  const [tick, setTick] = useState(() => Date.now());
  const [failed, setFailed] = useState<string | null>(null);
  const [locked, setLocked] = useState(false);
  const relock = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setNow(null);
    setLocked(false);
    setFailed(null);
    async function load() {
      try {
        const t = await be().totpNow(id);
        if (!alive) return;
        setNow(t);
        setFailed(null);
        setLocked(false);
        timer = setTimeout(load, Math.max(250, t.validUntilMs - Date.now() + 40));
      } catch (e) {
        if (!alive) return;
        if (e instanceof BackendError && e.code === "password_required") { setNow(null); setLocked(true); return; }
        setFailed(e instanceof Error ? e.message : "Could not read the code.");
      }
    }
    void load();
    return () => { alive = false; clearTimeout(timer); clearTimeout(relock.current); };
  }, [id, enabled]);

  const unlock = useCallback(async () => {
    const t = await withItemPassword({ reason: `Show the one-time code for “${title}”.`, confirmLabel: "Show code", run: (pw) => be().totpNow(id, pw) });
    if (!t) return;
    setNow(t);
    setLocked(false);
    clearTimeout(relock.current);
    relock.current = setTimeout(() => { setNow(null); setLocked(true); }, Math.max(0, t.validUntilMs - Date.now()));
  }, [id, title]);

  useEffect(() => {
    if (!enabled) return;
    const i = setInterval(() => setTick(Date.now()), 500);
    return () => clearInterval(i);
  }, [enabled]);

  const remainingMs = now ? Math.max(0, now.validUntilMs - tick) : 0;
  return { now, failed, locked, unlock, remainingMs, seconds: now ? secondsLeft(now.validUntilMs, tick) : 0 };
}

export function TotpValue({ id, title, onCopy }: { id: string; title: string; onCopy: (code: string) => Promise<void> }) {
  const { now, failed, locked, unlock, remainingMs, seconds } = useTotp(id, true, title);
  if (failed) return <FieldRow label="One-time code">{<span className="text-ember">{failed}</span>}</FieldRow>;
  if (locked) {
    return (
      <FieldRow label="One-time code">
        <div className="flex flex-wrap items-center gap-3">
          <span className="text-mist">Hidden. This item asks for your master password.</span>
          <Button className="min-h-8" icon={<KeyRound className="size-3.5" />} onClick={() => void unlock().catch(() => undefined)}>Show code</Button>
        </div>
      </FieldRow>
    );
  }
  const tone = ringTone(seconds);
  return (
    <FieldRow
      label="One-time code"
      actions={<CopyButton label="Copy one-time code" disabled={!now} onCopy={() => (now ? onCopy(now.code) : undefined)} />}
    >
      <div className="flex items-center gap-4">
        <span
          key={now?.code}
          role="timer"
          aria-label={now ? `Code ${now.code.split("").join(" ")}, ${seconds} seconds left` : "Loading code"}
          className={`font-mono text-[1.9rem] leading-9 font-medium tracking-[0.08em] tabular-nums ${tone === "critical" ? "animate-pulse-once text-ember" : "text-frost"}`}
        >
          {now ? groupDigits(now.code) : "••• •••"}
        </span>
        {now && (
          <CountdownRing totalMs={now.period * 1000} remainingMs={remainingMs} windowKey={now.validUntilMs} color={TONE_COLOR[tone]} size={34} stroke={3}>
            <span className="relative text-[0.78em] font-semibold tabular-nums">{seconds}</span>
          </CountdownRing>
        )}
      </div>
    </FieldRow>
  );
}
