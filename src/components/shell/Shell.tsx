import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { AnimatePresence, motion } from "motion/react";
import { ShieldCheck } from "lucide-react";
import { ItemDetail } from "../items/ItemDetail";
import { BackupView } from "../backup/BackupView";
import { CodesView } from "../items/CodesView";
import { ListPane } from "../items/ListPane";
import { SettingsView } from "../settings/SettingsView";
import { ExportView } from "../tools/ExportView";
import { GeneratorView } from "../tools/GeneratorView";
import { ImportView } from "../tools/ImportView";
import { TrashDetail, TrashList } from "../trash/TrashPane";
import { app, consumeUnlockMoment, select } from "../../state/app";
import { useBackHandler } from "../../state/back";
import { useStore } from "../../state/store";
import { useLayout } from "../../lib/layout";
import { FilesView } from "../files/FilesView";
import { BottomNav, Fab, Rail, Sidebar } from "./Navigation";
import { installShortcuts } from "./shortcuts";
import { FILTER_TYPE, ITEM_FILTERS, WIDE_VIEWS } from "./nav";
import type { Filter } from "../../state/app";

/** The full-width views (no list/detail split). */
function WideView({ filter }: { filter: Filter }) {
  switch (filter) {
    case "codes": return <CodesView />;
    case "generator": return <GeneratorView />;
    case "import": return <ImportView />;
    case "export": return <ExportView />;
    case "files": return <FilesView />;
    case "backup": return <BackupView />;
    default: return <SettingsView />;
  }
}

function EmptyDetail({ trash }: { trash?: boolean }) {
  return (
    <div className="grid h-full place-items-center px-8 text-center">
      <div className="max-w-xs">
        <ShieldCheck className="mx-auto size-9 text-fog" aria-hidden strokeWidth={1.4} />
        <p className="mt-4 font-display text-xl">{trash ? "Choose a deleted item" : "Choose an item"}</p>
        <p className="mt-2 text-mist">{trash ? "You can restore it, or delete it forever." : "Its details open here. Passwords stay hidden until you reveal them."}</p>
      </div>
    </div>
  );
}

/** Rises into place once, right after unlocking (the second half of the unlock moment). */
function Rise({ play, order, className = "", children }: { play: boolean; order: number; className?: string; children: ReactNode }) {
  const style: CSSProperties | undefined = play ? { animationDelay: `${order * 40}ms` } : undefined;
  return <div className={`${className} ${play ? "animate-rise" : ""}`} style={style}>{children}</div>;
}

export function Shell() {
  const layout = useLayout();
  const filter = useStore(app, (s) => s.filter);
  const selectedId = useStore(app, (s) => s.selectedId);
  const justUnlocked = useStore(app, (s) => s.justUnlocked);
  const [play, setPlay] = useState(justUnlocked);
  const itemMode = ITEM_FILTERS.has(filter);
  const trashMode = filter === "trash";
  const hasDetailPage = (itemMode || trashMode) && selectedId !== null;

  useEffect(() => {
    if (!justUnlocked) return;
    const t = setTimeout(() => { setPlay(false); consumeUnlockMoment(); }, 700);
    return () => clearTimeout(t);
  }, [justUnlocked]);

  useEffect(() => installShortcuts(), []);

  // Android back: the detail page is a history entry, so the system back gesture returns to the list.
  const pushed = useRef(false);
  useEffect(() => {
    if (layout !== "compact") return;
    if (hasDetailPage && !pushed.current) { history.pushState({ hexis: "detail" }, ""); pushed.current = true; }
    if (!hasDetailPage) pushed.current = false;
  }, [layout, hasDetailPage]);
  useEffect(() => {
    const onPop = () => { if (pushed.current) { pushed.current = false; select(null); } };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);
  const back = () => { if (pushed.current) history.back(); else select(null); };
  useBackHandler(layout === "compact" && hasDetailPage, back);

  if (layout === "compact") {
    return (
      <div className="relative flex h-dvh flex-col overflow-hidden">
        <Rise play={play} order={0} className="flex min-h-0 flex-1 flex-col bg-pane/55">
          {itemMode && <ListPane />}
          {trashMode && <TrashList />}
          {WIDE_VIEWS.has(filter) && <WideView filter={filter} />}
        </Rise>
        <AnimatePresence>
          {hasDetailPage && (
            <motion.div
              key="detail-page"
              data-compact-detail
              className="absolute inset-0 z-20 overflow-y-auto overscroll-contain bg-pane pt-[var(--safe-top)]"
              initial={{ x: "100%" }}
              animate={{ x: 0 }}
              exit={{ x: "100%" }}
              transition={{ duration: 0.16, ease: [0.2, 0.8, 0.2, 1] }}
            >
              {trashMode ? <TrashDetail id={selectedId!} onBack={back} /> : <ItemDetail id={selectedId!} onBack={back} />}
            </motion.div>
          )}
        </AnimatePresence>
        <Fab visible={((ITEM_FILTERS.has(filter) && filter !== "archive") || filter === "codes") && !hasDetailPage} type={FILTER_TYPE[filter] ?? null} />
        {!hasDetailPage && <BottomNav />}
      </div>
    );
  }

  const cols = layout === "wide" ? "grid-cols-[236px_344px_minmax(0,1fr)]" : "grid-cols-[64px_320px_minmax(0,1fr)]";
  const fullBleed = WIDE_VIEWS.has(filter);
  return (
    <div className={`grid h-dvh ${cols}`}>
      <Rise play={play} order={0} className="min-h-0">{layout === "wide" ? <Sidebar /> : <Rail />}</Rise>
      {fullBleed ? (
        <Rise play={play} order={1} className="col-span-2 flex min-h-0 flex-col bg-pane/55">
          <WideView filter={filter} />
        </Rise>
      ) : (
        <>
          <Rise play={play} order={1} className="flex min-h-0 flex-col border-r border-hairline bg-pane/70">
            {trashMode ? <TrashList /> : <ListPane />}
          </Rise>
          <Rise play={play} order={2} className="min-h-0 overflow-y-auto overscroll-contain bg-pane/40">
            {selectedId ? (trashMode ? <TrashDetail id={selectedId} /> : <ItemDetail id={selectedId} />) : <EmptyDetail trash={trashMode} />}
          </Rise>
        </>
      )}
    </div>
  );
}

