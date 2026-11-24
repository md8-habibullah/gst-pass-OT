import type { ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { setFilter, type Filter } from "../../state/app";

/** A titled card of rows. `heading={false}` when the page already carries the title. */
export function Group({ title, children, heading = true }: { title: string; children: ReactNode; heading?: boolean }) {
  return (
    <section aria-label={title} className="flex flex-col gap-2">
      {heading && <h3 className="px-1 font-semibold">{title}</h3>}
      <div className="surface overflow-hidden rounded-lg">{children}</div>
    </section>
  );
}

export function Row({ title, description, children }: { title: string; description?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-x-6 gap-y-3 border-b border-hairline px-4 py-3.5 last:border-b-0">
      <div className="min-w-0 flex-1 basis-60">
        <p className="font-medium">{title}</p>
        {description && <p className="mt-0.5 text-[0.93em] text-mist">{description}</p>}
      </div>
      {children}
    </div>
  );
}

export function LinkRow({ title, description, filter, icon, before }: { title: string; description: string; filter: Filter; icon: ReactNode; before?: () => void }) {
  return (
    <button type="button" onClick={() => { before?.(); setFilter(filter); }} className="pressable flex w-full items-center gap-4 border-b border-hairline px-4 py-3.5 text-left last:border-b-0 hover:bg-raised/60">
      <span className="grid size-9 shrink-0 place-items-center rounded-sm bg-raised text-brass-ink">{icon}</span>
      <span className="min-w-0 flex-1">
        <span className="block font-medium">{title}</span>
        <span className="block text-[0.93em] text-mist">{description}</span>
      </span>
      <ChevronRight className="size-4 shrink-0 text-fog" aria-hidden />
    </button>
  );
}

/** The one-line explanation under a page title. */
export function PageNote({ children }: { children: ReactNode }) {
  return <p className="-mt-2 max-w-[34rem] text-[0.93em] text-mist">{children}</p>;
}
