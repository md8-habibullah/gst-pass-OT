import { Plus } from "lucide-react";
import { FIELD_SCHEMA, instanceKey, MAX_INSTANCES, type ItemType } from "../../backend";
import { addSlot, CODE, type Fields, removeSlot, rowsOf, SIGN_IN, slotFilled } from "../../lib/instances";
import { Button } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { SecretField } from "./SecretField";
import { TotpInput } from "./TotpInput";

/** Change the editor's fields. A function, so two quick edits never overwrite each other. */
export type EditFields = (change: (fields: Fields) => Fields) => void;

const spec = (key: string) => FIELD_SCHEMA.login.find((s) => s.key === key)!;

/**
 * A login's sign-ins. One sign-in is the plain Username and Password boxes; with more, each sits in its own small panel with
 * a Remove button. "Add another sign-in" appears once the last one has something in it, the same rule as another website.
 */
export function SignInRows({ fields, edit, itemType }: { fields: Fields; edit: EditFields; itemType: ItemType }) {
  const rows = rowsOf(fields, SIGN_IN);
  const several = rows.length > 1;
  const last = rows[rows.length - 1]!;
  const user = spec("username");
  const pass = spec("password");
  const set = (key: string, value: string) => edit((f) => ({ ...f, [key]: value }));

  return (
    <div role="group" aria-label="Sign-ins" className="flex flex-col gap-3">
      {rows.map((n, i) => {
        const inputs = (
          <>
            <Field label={user.label}>
              {({ id, describedBy }) => (
                <TextInput id={id} aria-describedby={describedBy} value={fields[instanceKey("username", n)] ?? ""} placeholder={user.placeholder} maxLength={65536} onChange={(e) => set(instanceKey("username", n), e.target.value)} />
              )}
            </Field>
            <SecretField spec={pass} itemType={itemType} value={fields[instanceKey("password", n)] ?? ""} onChange={(v) => set(instanceKey("password", n), v)} label={pass.label} />
          </>
        );
        if (!several) return <div key={n} className="flex flex-col gap-4">{inputs}</div>;
        return (
          <div key={n} role="group" aria-label={`Sign-in ${i + 1}`} className="flex flex-col gap-4 rounded-lg bg-pane/70 p-3 shadow-e1">
            <div className="flex items-center justify-between gap-3">
              <p className="font-medium">Sign-in {i + 1}</p>
              <button type="button" aria-label={`Remove sign-in ${i + 1}`} onClick={() => edit((f) => removeSlot(f, SIGN_IN, n))} className="pressable rounded-sm px-1.5 py-1 text-[0.86em] font-medium text-mist hover:text-ember pointer-coarse:min-h-10">
                Remove
              </button>
            </div>
            {inputs}
          </div>
        );
      })}
      {rows.length < MAX_INSTANCES && slotFilled(fields, SIGN_IN, last) && (
        <Button className="self-start" icon={<Plus className="size-4" />} onClick={() => edit((f) => addSlot(f, SIGN_IN) ?? f)}>
          Add another sign-in
        </Button>
      )}
    </div>
  );
}

/** A login's one-time codes: the box, and a button for another once the last one is filled. */
export function CodeRows({ fields, edit }: { fields: Fields; edit: EditFields }) {
  const rows = rowsOf(fields, CODE);
  const several = rows.length > 1;
  const last = rows[rows.length - 1]!;
  const totp = spec("totp");
  return (
    <div role="group" aria-label="One-time codes" className="flex flex-col gap-3">
      {rows.map((n, i) => (
        <TotpInput
          key={n}
          label={several ? `${totp.label} ${i + 1}` : totp.label}
          value={fields[instanceKey("totp", n)] ?? ""}
          onChange={(v) => edit((f) => ({ ...f, [instanceKey("totp", n)]: v }))}
          canSplit={false}
          onRemove={several ? () => edit((f) => removeSlot(f, CODE, n)) : undefined}
        />
      ))}
      {rows.length < MAX_INSTANCES && slotFilled(fields, CODE, last) && (
        <Button className="self-start" icon={<Plus className="size-4" />} onClick={() => edit((f) => addSlot(f, CODE) ?? f)}>
          Add another code
        </Button>
      )}
    </div>
  );
}
