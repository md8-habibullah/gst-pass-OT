import { GROUPS, SHORTCUTS } from "../../lib/shortcuts";
import { Combo } from "../ui/Keycap";

/** Every shortcut, grouped. Read from the same table the key handler uses, so this list cannot drift from what works. */
export function ShortcutList({ columns = false }: { columns?: boolean }) {
  return (
    <div className={columns ? "grid gap-x-10 gap-y-7 min-[40rem]:grid-cols-2" : "flex flex-col gap-7"}>
      {GROUPS.map((group) => (
        <section key={group} aria-label={group}>
          <h3 className="mb-1 font-semibold">{group}</h3>
          <ul className="divide-y divide-hairline">
            {SHORTCUTS.filter((s) => s.group === group).map((s) => (
              <li key={s.id} className="flex flex-wrap items-center gap-x-4 gap-y-1.5 py-2">
                <span className="min-w-0 flex-1 basis-40">{s.label}</span>
                <span className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
                  {s.combos.map((c, i) => (
                    <span key={c} className="inline-flex items-center gap-3">
                      {i > 0 && <span className="text-[0.8em] text-fog">or</span>}
                      <Combo combo={c} />
                    </span>
                  ))}
                </span>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
