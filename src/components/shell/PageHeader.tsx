import type { ReactNode } from "react";
import { ArrowLeft } from "lucide-react";
import { useLayout } from "../../lib/layout";
import { setFilter } from "../../state/app";
import { useBackHandler } from "../../state/back";
import { IconButton } from "../ui/Button";
import { AccountBar } from "./AccountMenu";

/** Title block for the full-width views. On the compact layout, sub-pages of Settings get a back button. */
export function PageHeader({ title, subtitle, back, actions }: { title: string; subtitle?: string; back?: boolean; actions?: ReactNode }) {
  const compact = useLayout() === "compact";
  useBackHandler(!!back && compact, () => setFilter("settings"));
  return (
    <div className="flex flex-wrap items-start gap-x-3 gap-y-3 pb-1">
      {back && compact && (
        <IconButton label="Back to settings" className="-ml-2 shrink-0" onClick={() => setFilter("settings")}>
          <ArrowLeft className="size-5" />
        </IconButton>
      )}
      <div className={`min-w-0 flex-1 ${compact ? "basis-40" : "basis-60"}`}>
        <h2 className="font-display text-[1.45rem] leading-[1.2] font-semibold tracking-[-0.005em]">{title}</h2>
        {subtitle && <p className="mt-1.5 max-w-[34rem] text-[0.93em] text-mist">{subtitle}</p>}
      </div>
      <AccountBar className="-mr-1" />
      {actions}
    </div>
  );
}

/** Scroll container + centred column shared by Generator, Import, Export, Backup and Settings. */
export function PageBody({ children, wide = false }: { children: ReactNode; wide?: boolean }) {
  return (
    <section className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain">
      <div className={`mx-auto flex w-full flex-col gap-6 px-5 pt-[calc(1rem+var(--safe-top))] pb-28 min-[720px]:pt-5 min-[720px]:pb-10 ${wide ? "max-w-[52rem]" : "max-w-[40rem]"}`}>{children}</div>
    </section>
  );
}
