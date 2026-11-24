import { forwardRef, useId, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, type TextareaHTMLAttributes } from "react";
import { motion } from "motion/react";

const base =
  "w-full rounded-sm border bg-raised px-3 text-frost shadow-e1 outline-none transition-colors placeholder:text-fog hover:border-hairline-strong focus:border-brass-ink/70 focus:bg-hover/60 min-h-10 pointer-coarse:min-h-12 pointer-coarse:text-base";

interface FieldProps {
  label: string;
  hint?: ReactNode;
  error?: string | null;
  children: (a: { id: string; describedBy: string | undefined; invalid: boolean }) => ReactNode;
  trailing?: ReactNode;
}

/** Label + control + hint/error wiring. Labels are sentence case and always visible. */
export function Field({ label, hint, error, children, trailing }: FieldProps) {
  const id = useId();
  const describedBy = error ? `${id}-err` : hint ? `${id}-hint` : undefined;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline justify-between gap-3">
        <label htmlFor={id} className="text-[0.86em] font-medium text-mist">
          {label}
        </label>
        {trailing}
      </div>
      {children({ id, describedBy, invalid: Boolean(error) })}
      {error ? (
        <p id={`${id}-err`} role="alert" className="text-[0.86em] text-ember">
          {error}
        </p>
      ) : hint ? (
        <p id={`${id}-hint`} className="text-[0.86em] text-fog">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

export const TextInput = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement> & { invalid?: boolean; mono?: boolean }>(
  function TextInput({ className = "", invalid, mono, ...rest }, ref) {
    return (
      <input
        ref={ref}
        aria-invalid={invalid || undefined}
        autoComplete="off"
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        className={`${base} ${mono ? "font-mono text-[0.97em]" : ""} ${invalid ? "border-ember/70" : "border-hairline"} ${className}`}
        {...rest}
      />
    );
  },
);

/** A native drop-down (on a phone it opens the system picker). Same look as the text field, as wide as its longest choice. */
export const Select = forwardRef<HTMLSelectElement, SelectHTMLAttributes<HTMLSelectElement>>(function Select({ className = "", children, ...rest }, ref) {
  return (
    <select ref={ref} className={`${base} w-auto min-w-40 cursor-pointer border-hairline pr-8 ${className}`} {...rest}>
      {children}
    </select>
  );
});

export const TextArea = forwardRef<HTMLTextAreaElement, TextareaHTMLAttributes<HTMLTextAreaElement> & { mono?: boolean }>(
  function TextArea({ className = "", mono, ...rest }, ref) {
    return (
      <textarea
        ref={ref}
        spellCheck={false}
        className={`${base} min-h-24 resize-y py-2.5 leading-6 border-hairline ${mono ? "font-mono text-[0.97em]" : ""} ${className}`}
        {...rest}
      />
    );
  },
);

export function Switch({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={`relative h-6 w-10 shrink-0 rounded-full border transition-colors pointer-coarse:h-7 pointer-coarse:w-12 ${checked ? "border-brass-ink/60 bg-brass/25" : "border-hairline bg-raised"}`}
    >
      <span
        className={`absolute top-1/2 left-0.5 size-4.5 -translate-y-1/2 rounded-full shadow-e2 transition-[translate,background-color] duration-100 ease-out pointer-coarse:size-5.5 ${checked ? "translate-x-4.5 bg-brass-ink pointer-coarse:translate-x-5.5" : "translate-x-0 bg-mist"}`}
      />
    </button>
  );
}

/** Segmented control with a gliding thumb (shared layout animation). */
export function Segmented<T extends string | number>({
  value, onChange, options, label,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
  label: string;
}) {
  const group = useId();
  return (
    <div role="radiogroup" aria-label={label} className="relative inline-flex rounded-sm border border-hairline bg-raised p-0.5 shadow-e1">
      {options.map((o) => {
        const on = o.value === value;
        return (
          <button
            key={String(o.value)}
            type="button"
            role="radio"
            aria-checked={on}
            onClick={() => onChange(o.value)}
            className={`relative z-10 min-h-8 rounded-[6px] px-3.5 text-[0.93em] font-medium transition-colors pointer-coarse:min-h-11 ${on ? "text-frost" : "text-mist hover:text-frost"}`}
          >
            {on && <motion.span layoutId={`seg-${group}`} className="absolute inset-0 -z-10 rounded-[6px] bg-hover" transition={{ duration: 0.12, ease: [0.2, 0.8, 0.2, 1] }} />}
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
