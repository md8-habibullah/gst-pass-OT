import { useState } from "react";
import { ChevronUp, Eye, EyeOff, Wand2 } from "lucide-react";
import type { FieldSpec, ItemType } from "../../backend";
import { StrengthMeter } from "../lock/StrengthMeter";
import { IconButton } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { PasswordGenerator } from "./PasswordGenerator";

/** Fields that get the password generator and a strength meter. */
const GENERATE_FOR = new Set(["login:password", "sshKey:passphrase"]);

/** A secret text field (card numbers, passphrases, ID numbers) with its own show/hide toggle. */
export function SecretField({ spec, value, onChange, itemType, label = spec.label }: { spec: FieldSpec; value: string; onChange: (v: string) => void; itemType: ItemType; label?: string }) {
  const [show, setShow] = useState(false);
  const [gen, setGen] = useState(false);
  const generate = GENERATE_FOR.has(`${itemType}:${spec.key}`);
  return (
    <div className="flex flex-col gap-2.5">
      <Field
        label={label}
        trailing={
          generate ? (
            <button type="button" onClick={() => setGen((g) => !g)} aria-expanded={gen} className="pressable inline-flex items-center gap-1.5 rounded-sm px-1.5 py-1 text-[0.86em] font-semibold text-brass-ink hover:text-brass-ink-hi pointer-coarse:min-h-10">
              {gen ? <ChevronUp className="size-3.5" aria-hidden /> : <Wand2 className="size-3.5" aria-hidden />} {gen ? "Hide" : "Generate"}
            </button>
          ) : undefined
        }
      >
        {({ id }) => (
          <div className="relative">
            <TextInput id={id} mono type={show ? "text" : "password"} autoComplete={generate ? "new-password" : "off"} value={value} placeholder={spec.placeholder} inputMode={spec.key === "number" || spec.key === "code" || spec.key === "pin" ? "numeric" : undefined} onChange={(e) => onChange(e.target.value)} className="pr-11" />
            <IconButton label={show ? `Hide ${label.toLowerCase()}` : `Show ${label.toLowerCase()}`} size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => setShow((v) => !v)}>
              {show ? <EyeOff className="size-[17px]" /> : <Eye className="size-[17px]" />}
            </IconButton>
          </div>
        )}
      </Field>
      {generate && value && <StrengthMeter password={value} />}
      {gen && <PasswordGenerator onUse={(p) => { onChange(p); setShow(true); setGen(false); }} onClose={() => setGen(false)} />}
    </div>
  );
}

