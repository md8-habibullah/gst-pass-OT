import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { LoaderCircle } from "lucide-react";

export function Spinner({ className = "size-4" }: { className?: string }) {
  return <LoaderCircle aria-hidden className={`${className} animate-spin`} />;
}

type Variant = "key" | "quiet" | "ghost" | "danger";

const VARIANT: Record<Variant, string> = {
  key: "key-surface font-semibold", // owns its disabled look (see base.css); busy stays vivid
  quiet: "bg-raised text-frost shadow-e1 hover:bg-hover border border-hairline",
  ghost: "text-mist hover:bg-raised hover:text-frost",
  danger: "bg-danger text-white shadow-e1 hover:brightness-110 active:brightness-95",
};

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  busy?: boolean;
  icon?: ReactNode;
  block?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "quiet", busy, icon, block, className = "", children, disabled, type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      className={`pressable inline-flex min-h-9 items-center justify-center gap-2 rounded-sm px-3.5 text-[0.95em] leading-none font-medium whitespace-nowrap pointer-coarse:min-h-12 pointer-coarse:px-5 ${variant === "key" ? "" : "disabled:opacity-50"} ${block ? "w-full" : ""} ${VARIANT[variant]} ${className}`}
      {...rest}
    >
      {busy ? <Spinner /> : icon}
      {children}
    </button>
  );
});

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  tone?: "default" | "key" | "danger";
  size?: "sm" | "md";
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, tone = "default", size = "md", className = "", children, type = "button", ...rest },
  ref,
) {
  const tones = {
    default: "text-mist hover:bg-hover hover:text-frost",
    key: "text-brass hover:bg-hover hover:text-brass-hi",
    danger: "text-ember hover:bg-hover",
  } as const;
  const dims = size === "sm" ? "size-8 pointer-coarse:size-11" : "size-9 pointer-coarse:size-12";
  return (
    <button
      ref={ref}
      type={type}
      aria-label={label}
      title={label}
      className={`pressable inline-flex shrink-0 items-center justify-center rounded-sm disabled:opacity-40 ${dims} ${tones[tone]} ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
});
