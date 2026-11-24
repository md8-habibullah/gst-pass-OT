import { useCallback, useEffect, useState } from "react";
import { Delete, Fingerprint } from "lucide-react";

const LENGTH = 6;
const KEYS = ["1", "2", "3", "4", "5", "6", "7", "8", "9"] as const;

/** Touch-first PIN pad. Auto-submits at six digits; a wrong PIN shakes the dots once and clears them. */
export function PinPad({
  onSubmit, onBiometric, disabled,
}: {
  onSubmit: (pin: string) => Promise<void>;
  onBiometric?: () => void;
  disabled?: boolean;
}) {
  const [pin, setPin] = useState("");
  const [shake, setShake] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = useCallback(async (value: string) => {
    setBusy(true);
    setError(null);
    try {
      await onSubmit(value);
    } catch (e) {
      setError(e instanceof Error ? e.message : "That PIN is not right.");
      setShake((n) => n + 1);
      setPin("");
    } finally {
      setBusy(false);
    }
  }, [onSubmit]);

  const press = useCallback((d: string) => {
    if (busy || disabled) return;
    setError(null);
    setPin((p) => {
      if (p.length >= LENGTH) return p;
      const next = p + d;
      if (next.length === LENGTH) queueMicrotask(() => void submit(next));
      return next;
    });
  }, [busy, disabled, submit]);

  const back = useCallback(() => !busy && setPin((p) => p.slice(0, -1)), [busy]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (/^[0-9]$/.test(e.key)) press(e.key);
      else if (e.key === "Backspace") back();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [press, back]);

  const key = "pressable grid size-[4.25rem] place-items-center rounded-full bg-raised text-[1.5em] font-medium shadow-e2 border border-hairline hover:bg-hover disabled:opacity-40";
  return (
    <div className="flex flex-col items-center gap-6">
      <div key={shake} className={`flex gap-3.5 ${shake > 0 ? "animate-shake" : ""}`} role="img" aria-label={`${pin.length} of ${LENGTH} digits entered`}>
        {Array.from({ length: LENGTH }, (_, i) => (
          <span key={i} className={`size-3 rounded-full border transition-colors duration-100 ${i < pin.length ? "scale-110 border-brass bg-brass" : "border-mist/50"}`} />
        ))}
      </div>
      <p role="alert" className="h-5 text-[0.9em] text-ember">{error}</p>
      <div className="grid grid-cols-3 gap-x-5 gap-y-4">
        {KEYS.map((k) => (
          <button key={k} type="button" className={key} disabled={busy || disabled} onClick={() => press(k)} aria-label={k}>
            {k}
          </button>
        ))}
        {onBiometric ? (
          <button type="button" className="pressable grid size-[4.25rem] place-items-center rounded-full text-brass hover:bg-raised" onClick={onBiometric} aria-label="Use biometrics" disabled={busy || disabled}>
            <Fingerprint className="size-7" />
          </button>
        ) : (
          <span />
        )}
        <button type="button" className={key} disabled={busy || disabled} onClick={() => press("0")} aria-label="0">
          0
        </button>
        <button type="button" className="pressable grid size-[4.25rem] place-items-center rounded-full text-mist hover:bg-raised" onClick={back} aria-label="Delete last digit" disabled={busy || disabled || pin.length === 0}>
          <Delete className="size-6" />
        </button>
      </div>
    </div>
  );
}
