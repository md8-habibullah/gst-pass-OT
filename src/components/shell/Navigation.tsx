import { motion } from "motion/react";
import { Lock, Plus } from "lucide-react";
import { app, counts, lockVault, setFilter } from "../../state/app";
import { useStore } from "../../state/store";
import { openNewItem } from "../../state/ui";
import { VaultMark } from "../lock/VaultMark";
import { IconButton } from "../ui/Button";
import { NAV, TABS } from "./nav";
import { useMemo } from "react";

function useCounts() {
  const items = useStore(app, (s) => s.items);
  const trash = useStore(app, (s) => s.trash);
  return useMemo(() => counts(items, trash.length), [items, trash]);
}

const SPRING = { type: "spring", stiffness: 520, damping: 42 } as const;

/** Wide layout: glass sidebar with counts, a gliding active pill and the real sync state. */
export function Sidebar() {
  const filter = useStore(app, (s) => s.filter);
  const total = useCounts();
  return (
    <nav aria-label="Main" className="glass flex h-full flex-col border-y-0 border-l-0 px-3 pt-5 pb-4">
      <div className="mb-6 flex items-center gap-2.5 px-2.5">
        <VaultMark size={30} />
        <span className="font-display text-[1.35rem] leading-none font-semibold tracking-[-0.01em]">Passman</span>
      </div>
      <ul className="flex flex-col gap-0.5">
        {NAV.map((n) => {
          const on = filter === n.filter;
          const Icon = n.icon;
          return (
            <li key={n.filter} className={n.filter === "trash" ? "mt-3 border-t border-hairline pt-3" : ""}>
              <button
                type="button"
                aria-current={on ? "page" : undefined}
                onClick={() => setFilter(n.filter)}
                className={`pressable relative flex min-h-10 w-full items-center gap-3 rounded-sm px-2.5 text-left font-medium transition-colors ${on ? "text-frost" : "text-mist hover:bg-raised/60 hover:text-frost"}`}
              >
                {on && <motion.span layoutId="nav-active" className="absolute inset-0 rounded-sm bg-raised shadow-e2" transition={SPRING} />}
                {on && <motion.span layoutId="nav-edge" className="absolute top-2.5 bottom-2.5 left-0 w-[3px] rounded-full bg-brass" transition={SPRING} />}
                <Icon className={`relative size-[18px] ${on ? "text-brass" : ""}`} aria-hidden />
                <span className="relative flex-1">{n.label}</span>
                {n.filter !== "settings" && <span className="relative text-[0.86em] text-fog tabular-nums">{total[n.filter]}</span>}
              </button>
            </li>
          );
        })}
      </ul>
      <div className="mt-auto flex flex-col gap-3 pt-4">
        <div className="mx-1 flex items-start gap-2.5 rounded-lg bg-pane/70 px-3 py-2.5 shadow-e1">
          <span className="mt-[7px] size-2 shrink-0 rounded-full bg-verdigris" aria-hidden />
          <div>
            <p className="text-[0.93em] leading-5 font-medium">Local only</p>
            <p className="text-[0.8em] leading-4 text-fog">Stored encrypted on this device. No sync targets yet.</p>
          </div>
        </div>
        <button type="button" onClick={() => void lockVault()} className="pressable flex min-h-10 items-center gap-3 rounded-sm px-2.5 font-medium text-mist hover:bg-raised/60 hover:text-frost">
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
  return (
    <nav aria-label="Main" className="glass flex h-full flex-col items-center gap-1 border-y-0 border-l-0 py-4">
      <VaultMark size={30} />
      <ul className="mt-5 flex flex-col gap-1">
        {NAV.map((n) => {
          const on = filter === n.filter;
          const Icon = n.icon;
          return (
            <li key={n.filter} className={n.filter === "trash" ? "mt-2 border-t border-hairline pt-2" : ""}>
              <button type="button" aria-label={n.label} title={n.label} aria-current={on ? "page" : undefined} onClick={() => setFilter(n.filter)} className={`pressable relative grid size-11 place-items-center rounded-sm ${on ? "text-brass" : "text-mist hover:bg-raised/60 hover:text-frost"}`}>
                {on && <motion.span layoutId="rail-active" className="absolute inset-0 rounded-sm bg-raised shadow-e2" transition={SPRING} />}
                <Icon className="relative size-5" />
              </button>
            </li>
          );
        })}
      </ul>
      <IconButton label="Lock now" className="mt-auto" onClick={() => void lockVault()}>
        <Lock className="size-5" />
      </IconButton>
    </nav>
  );
}

/** Compact layout: Material-style bottom navigation with a pill indicator behind the active icon. */
export function BottomNav() {
  const filter = useStore(app, (s) => s.filter);
  return (
    <nav aria-label="Main" className="glass-strong fixed inset-x-0 bottom-0 z-30 rounded-t-xl border-b-0 pt-1.5 pb-[calc(0.5rem+var(--safe-bottom))]">
      <ul className="mx-auto flex max-w-md justify-around px-2">
        {TABS.map((t) => {
          const on = filter === t.filter || (t.filter === "all" && !TABS.some((x) => x.filter === filter));
          const Icon = t.icon;
          return (
            <li key={t.filter} className="flex-1">
              <button type="button" aria-current={on ? "page" : undefined} onClick={() => setFilter(t.filter)} className="pressable flex min-h-14 w-full flex-col items-center justify-center gap-1">
                <span className="relative grid h-8 w-16 place-items-center">
                  {on && <motion.span layoutId="tab-pill" className="absolute inset-0 rounded-full bg-brass/20" transition={SPRING} />}
                  <Icon className={`relative size-[22px] ${on ? "text-brass-hi" : "text-mist"}`} aria-hidden />
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
export function Fab({ visible }: { visible: boolean }) {
  return (
    <motion.button
      type="button"
      aria-label="New item"
      onClick={() => openNewItem()}
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

