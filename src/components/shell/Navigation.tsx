import { useEffect, useMemo } from "react";
import { motion } from "motion/react";
import { Lock, Plus } from "lucide-react";
import { app, counts, lockVault, setFilter, type Filter } from "../../state/app";
import { backups, refreshBackups, summarize } from "../../state/backup";
import { useStore } from "../../state/store";
import type { ItemType } from "../../backend";
import { openNewItem } from "../../state/ui";
import { brand } from "../../state/brand";
import { VaultMark } from "../lock/VaultMark";
import { IconButton } from "../ui/Button";
import { AccountChip } from "./AccountMenu";
import { NAV_GROUPS, RAIL, TABS } from "./nav";

function useCounts() {
  const items = useStore(app, (s) => s.items);
  const trash = useStore(app, (s) => s.trash);
  return useMemo(() => counts(items, trash.length), [items, trash]);
}

const SPRING = { type: "spring", stiffness: 520, damping: 42 } as const;
const NO_COUNT: ReadonlySet<Filter> = new Set<Filter>(["files", "generator", "import", "export", "backup", "settings"]);

/** Keeps the backup summary fresh while the app is open (cheap read; the core does the work). */
function useBackupSummary() {
  const targets = useStore(backups, (s) => s.targets);
  const phase = useStore(app, (s) => s.phase);
  useEffect(() => {
    if (phase !== "ready") return;
    let alive = true;
    const load = () => { if (alive) void refreshBackups().catch(() => undefined); };
    load();
    const t = setInterval(load, 30_000);
    return () => { alive = false; clearInterval(t); };
  }, [phase]);
  return summarize(targets);
}

const TONE_DOT = { none: "bg-fog", ok: "bg-verdigris", busy: "bg-brass-ink", bad: "bg-ember", untested: "bg-brass-ink" } as const;

/** Wide layout: sidebar with the account chip, grouped places with counts, and the real backup state. */
export function Sidebar() {
  const brandName = useStore(brand, (b) => b.name);
  const filter = useStore(app, (s) => s.filter);
  const total = useCounts();
  const summary = useBackupSummary();
  return (
    <nav aria-label="Main" className="surface flex h-full flex-col border-y-0 border-l-0 px-3 pt-4 pb-3">
      <div className="mb-3 flex items-center gap-2.5 px-2.5">
        <VaultMark size={28} />
        <span className="font-display text-[1.3rem] leading-none font-semibold tracking-[-0.01em]">{brandName}</span>
      </div>
      <AccountChip className="mb-3 bg-pane/60 shadow-e1" />
      <div className="no-scrollbar min-h-0 flex-1 overflow-y-auto">
        {NAV_GROUPS.map((group, gi) => (
          <ul key={gi} className={`flex flex-col gap-0.5 ${gi > 0 ? "mt-2 border-t border-hairline pt-2" : ""}`}>
            {group.map((n) => {
              const on = filter === n.filter;
              const Icon = n.icon;
              return (
                <li key={n.filter}>
                  <button
                    type="button"
                    aria-current={on ? "page" : undefined}
                    onClick={() => setFilter(n.filter)}
                    className={`pressable relative flex min-h-9 w-full items-center gap-3 rounded-sm px-2.5 text-left font-medium transition-colors ${on ? "text-frost" : "text-mist hover:bg-raised/60 hover:text-frost"}`}
                  >
                    {on && <motion.span layoutId="nav-active" className="absolute inset-0 rounded-sm bg-raised shadow-e2" transition={SPRING} />}
                    {on && <motion.span layoutId="nav-edge" className="absolute top-2 bottom-2 left-0 w-[3px] rounded-full bg-brass-ink" transition={SPRING} />}
                    <Icon className={`relative size-[18px] ${on ? "text-brass-ink" : ""}`} aria-hidden />
                    <span className="relative flex-1">{n.label}</span>
                    {!NO_COUNT.has(n.filter) && <span className="relative text-[0.86em] text-fog tabular-nums">{total[n.filter]}</span>}
                  </button>
                </li>
              );
            })}
          </ul>
        ))}
      </div>
      <div className="flex flex-col gap-2 pt-3">
        <button type="button" onClick={() => setFilter("backup")} className="pressable mx-1 flex items-start gap-2.5 rounded-lg bg-pane/70 px-3 py-2.5 text-left shadow-e1 hover:bg-pane">
          <span className={`mt-[7px] size-2 shrink-0 rounded-full ${TONE_DOT[summary.tone]}`} aria-hidden />
          <span className="min-w-0">
            <span className="block text-[0.93em] leading-5 font-medium">{summary.title}</span>
            <span className="block text-[0.8em] leading-4 text-fog">{summary.detail}</span>
          </span>
        </button>
        <button type="button" onClick={() => void lockVault()} className="pressable flex min-h-9 items-center gap-3 rounded-sm px-2.5 font-medium text-mist hover:bg-raised/60 hover:text-frost">
          <Lock className="size-[18px]" aria-hidden />
          Lock now
        </button>
      </div>
    </nav>
  );
}

/** Medium layout: 64px icon rail. */
export function Rail() {
  const filter = useStore(app, (s) => s.filter);
  useBackupSummary();
  const items = useStore(app, (s) => s.items);
  const itemFilterOn = !RAIL.flat().some((n) => n.filter === filter);
  void items;
  return (
    <nav aria-label="Main" className="surface flex h-full flex-col items-center gap-1 border-y-0 border-l-0 py-4">
      <VaultMark size={30} />
      <AccountChip compact className="mt-3" />
      <div className="no-scrollbar mt-2 min-h-0 flex-1 overflow-y-auto">
        {RAIL.map((group, gi) => (
          <ul key={gi} className={`flex flex-col items-center gap-1 ${gi > 0 ? "mt-2 border-t border-hairline pt-2" : ""}`}>
            {group.map((n) => {
              const on = filter === n.filter || (n.filter === "all" && itemFilterOn);
              const Icon = n.icon;
              return (
                <li key={n.filter}>
                  <button type="button" aria-label={n.label} title={n.label} aria-current={on ? "page" : undefined} onClick={() => setFilter(n.filter)} className={`pressable relative grid size-11 place-items-center rounded-sm ${on ? "text-brass-ink" : "text-mist hover:bg-raised/60 hover:text-frost"}`}>
                    {on && <motion.span layoutId="rail-active" className="absolute inset-0 rounded-sm bg-raised shadow-e2" transition={SPRING} />}
                    <Icon className="relative size-5" />
                  </button>
                </li>
              );
            })}
          </ul>
        ))}
      </div>
      <IconButton label="Lock now" className="mt-1" onClick={() => void lockVault()}>
        <Lock className="size-5" />
      </IconButton>
    </nav>
  );
}

/** Compact layout: Material-style bottom navigation with a pill indicator behind the active icon. */
export function BottomNav() {
  const filter = useStore(app, (s) => s.filter);
  useBackupSummary();
  return (
    <nav aria-label="Main" className="surface-float fixed inset-x-0 bottom-0 z-30 rounded-t-xl border-b-0 pt-1.5 pb-[calc(0.5rem+var(--safe-bottom))]">
      <ul className="mx-auto flex max-w-md justify-around px-2">
        {TABS.map((t) => {
          const on = t.owns.has(filter);
          const Icon = t.icon;
          return (
            <li key={t.filter} className="flex-1">
              <button type="button" aria-current={on ? "page" : undefined} onClick={() => setFilter(t.filter)} className="pressable flex min-h-14 w-full flex-col items-center justify-center gap-1">
                <span className="relative grid h-8 w-16 place-items-center">
                  {on && <motion.span layoutId="tab-pill" className="absolute inset-0 rounded-full bg-brass/20" transition={SPRING} />}
                  <Icon className={`relative size-[22px] ${on ? "text-brass-ink-hi" : "text-mist"}`} aria-hidden />
                </span>
                <span className={`text-[0.8em] font-medium ${on ? "text-frost" : "text-fog"}`}>{t.label}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}

/** Thumb-reach floating action button (compact layout). */
export function Fab({ visible, type = null }: { visible: boolean; type?: ItemType | null }) {
  return (
    <motion.button
      type="button"
      aria-label="New item"
      onClick={() => openNewItem(type)}
      initial={false}
      animate={visible ? { scale: 1, opacity: 1, y: 0 } : { scale: 0.6, opacity: 0, y: 16 }}
      transition={{ type: "spring", stiffness: 520, damping: 34 }}
      tabIndex={visible ? 0 : -1}
      className="key-surface fixed right-4 bottom-[calc(5.75rem+var(--safe-bottom))] z-20 grid size-14 place-items-center rounded-xl"
      style={{ pointerEvents: visible ? "auto" : "none" }}
    >
      <Plus className="size-7" strokeWidth={2.2} />
    </motion.button>
  );
}

