import { useMemo } from "react";
import { checkDisplayName, MAX_NAME_CHARS } from "../../lib/profileName";
import { app } from "../../state/app";
import { useStore } from "../../state/store";
import { Field, TextInput } from "../ui/Inputs";

/** Live check of a new account name against the accounts already on this device. */
export function useNameCheck(raw: string) {
  const profiles = useStore(app, (s) => s.profiles);
  return useMemo(() => checkDisplayName(raw, profiles), [raw, profiles]);
}

export function NameField({ value, onChange, disabled, autoFocus }: { value: string; onChange: (v: string) => void; disabled?: boolean; autoFocus?: boolean }) {
  const check = useNameCheck(value);
  return (
    <Field
      label="Account name"
      hint={`Shown on the lock screen so you can tell your vaults apart. Up to ${MAX_NAME_CHARS} characters, and different from your other accounts.`}
      error={value.trim() && !check.ok ? check.message : null}
    >
      {({ id, describedBy, invalid }) => (
        <TextInput id={id} data-autofocus={autoFocus || undefined} value={value} invalid={invalid} aria-describedby={describedBy} disabled={disabled} onChange={(e) => onChange(e.target.value)} maxLength={MAX_NAME_CHARS * 2} placeholder="Personal" autoComplete="off" />
      )}
    </Field>
  );
}
