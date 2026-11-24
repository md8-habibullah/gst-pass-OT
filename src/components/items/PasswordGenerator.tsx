import { useCallback, useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { DEFAULT_GEN, generatePassword, type GenOptions } from "../../lib/password";
import { Button, IconButton } from "../ui/Button";
import { Switch } from "../ui/Inputs";
import { PasswordText } from "./FieldRows";

const TOGGLES: { key: keyof Pick<GenOptions, "lower" | "upper" | "digits" | "symbols" | "avoidAmbiguous">; label: string }[] = [
  { key: "lower", label: "Lowercase letters" },
  { key: "upper", label: "Uppercase letters" },
  { key: "digits", label: "Numbers" },
  { key: "symbols", label: "Symbols" },
  { key: "avoidAmbiguous", label: "Avoid look-alike characters (0 O 1 l I)" },
];

/** Inline generator. Randomness: crypto.getRandomValues with rejection sampling (lib/password.ts). */
export function PasswordGenerator({ onUse, actionLabel = "Use this password" }: { onUse: (pw: string) => void; actionLabel?: string }) {
  const [opts, setOpts] = useState<GenOptions>(DEFAULT_GEN);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);

  const roll = useCallback((o: GenOptions) => {
    try {
      setValue(generatePassword(o));
      setError(null);
    } catch (e) {
      setValue("");
      setError(e instanceof Error ? e.message : "Could not generate a password.");
    }
  }, []);
  useEffect(() => roll(opts), [opts, roll]);

  return (
    <div className="flex flex-col gap-4 rounded-lg bg-pane/80 p-4 shadow-e1">
      <div className="flex items-start gap-2">
        <div className="min-h-[3rem] min-w-0 flex-1 rounded-sm bg-raised px-3 py-2.5 shadow-e1" aria-live="polite">
          {error ? <span className="text-ember">{error}</span> : <PasswordText value={value} />}
        </div>
        <IconButton label="Generate another" onClick={() => roll(opts)}>
          <RefreshCw className="size-[18px]" />
        </IconButton>
      </div>
      <div>
        <label htmlFor="gen-length" className="flex items-baseline justify-between text-[0.93em] font-medium text-mist">
          Length
          <span className="font-mono text-frost tabular-nums">{opts.length}</span>
        </label>
        <input
          id="gen-length"
          type="range"
          min={8}
          max={64}
          value={opts.length}
          onChange={(e) => setOpts((o) => ({ ...o, length: Number(e.target.value) }))}
          className="mt-2 h-6 w-full accent-[var(--color-brass)] pointer-coarse:h-9"
        />
      </div>
      <ul className="flex flex-col gap-1">
        {TOGGLES.map((t) => (
          <li key={t.key} className="flex min-h-9 items-center justify-between gap-4 pointer-coarse:min-h-12">
            <span className="text-[0.93em]">{t.label}</span>
            <Switch label={t.label} checked={opts[t.key]} onChange={(v) => setOpts((o) => ({ ...o, [t.key]: v }))} />
          </li>
        ))}
      </ul>
      <Button variant="key" disabled={!value} onClick={() => onUse(value)}>
        {actionLabel}
      </Button>
    </div>
  );
}
