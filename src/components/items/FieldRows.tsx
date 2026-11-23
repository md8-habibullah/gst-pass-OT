import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Check, Copy, Eye, EyeOff } from "lucide-react";
import { be } from "../../state/app";
import { charClass } from "../../lib/password";
import { groupDigits } from "../../lib/format";
import { ringTone, secondsLeft, TONE_COLOR } from "../../lib/totp";
import type { TotpNow } from "../../backend";
import { IconButton } from "../ui/Button";
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

export function RevealToggle({ revealed, onReveal, onHide }: { revealed: boolean; onReveal: () => void; onHide: () => void }) {
  return revealed ? (
    <IconButton label="Hide" onClick={onHide}>
      <EyeOff className="size-[18px]" />
    </IconButton>
  ) : (
    <IconButton label="Reveal (asks for your master password)" onClick={onReveal}>
      <Eye className="size-[18px]" />
    </IconButton>
  );
}

/** Live code + ring. The secret never reaches the UI: only the current code and its validity window. */
export function useTotp(id: string, enabled: boolean) {
  const [now, setNow] = useState<TotpNow | null>(null);
  const [tick, setTick] = useState(() => Date.now());
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function load() {
      try {
        const t = await be().totpNow(id);
        if (!alive) return;
        setNow(t);
        setFailed(null);
        timer = setTimeout(load, Math.max(250, t.validUntilMs - Date.now() + 40));
      } catch (e) {
        if (alive) setFailed(e instanceof Error ? e.message : "Could not read the code.");
      }
    }
    void load();
    return () => { alive = false; clearTimeout(timer); };
  }, [id, enabled]);

  useEffect(() => {
    if (!enabled) return;
    const i = setInterval(() => setTick(Date.now()), 500);
    return () => clearInterval(i);
  }, [enabled]);

  const remainingMs = now ? Math.max(0, now.validUntilMs - tick) : 0;
  return { now, failed, remainingMs, seconds: now ? secondsLeft(now.validUntilMs, tick) : 0 };
}

export function TotpValue({ id, onCopy }: { id: string; onCopy: (code: string) => Promise<void> }) {
  const { now, failed, remainingMs, seconds } = useTotp(id, true);
  if (failed) return <FieldRow label="One-time code">{<span className="text-ember">{failed}</span>}</FieldRow>;
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
