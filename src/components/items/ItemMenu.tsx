import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Archive, ArchiveRestore, Copy, ExternalLink, Eye, Files, FolderInput, Pencil, Trash2, type LucideIcon } from "lucide-react";
import { useLayout } from "../../lib/layout";
import { shortUrl } from "../../lib/urls";
import { be } from "../../state/app";
import { createStore, useStore } from "../../state/store";
import { Sheet } from "../ui/Sheet";
import { cloneItem, copyQuick, editItem, findUrls, launch, moveItemToFolder, openLink, QUICK_COPY, setArchived, trashItem, viewItem, type Target } from "./actions";

/** What the menu needs of an item: what the actions take, plus the folder it is in (for "Move to folder"). */
type MenuItem = Target & { folder?: string | null };

interface Open {
  item: MenuItem;
  x: number;
  y: number;
  /** Where the menu was opened from, to put focus back when it closes. */
  from: HTMLElement | null;
  /** Opened from a button: the menu hangs below it, right edges aligned. Otherwise it opens at the pointer. */
  button: boolean;
}

const menu = createStore<Open | null>(null);

/** Opens the item menu at a pointer position or under a button. */
export function openItemMenu(item: MenuItem, at: { x: number; y: number } | HTMLElement) {
  if (at instanceof HTMLElement) {
    const r = at.getBoundingClientRect();
    menu.set({ item, x: r.right, y: r.bottom + 4, from: at, button: true });
  } else {
    menu.set({ item, ...at, from: document.activeElement instanceof HTMLElement ? document.activeElement : null, button: false });
  }
}
const closeMenu = () => menu.set(null);

/** A login with many addresses gets this many "Open ..." entries; the rest are on its screen. */
const MAX_OPEN_ENTRIES = 5;

interface Entry {
  id: string;
  label: string;
  icon: LucideIcon;
  run: () => void | Promise<void>;
  tone?: "danger";
  /** A hairline above this entry. */
  group?: boolean;
}

function useEntries(item: MenuItem | null): Entry[] {
  const [urls, setUrls] = useState<string[]>([]);
  const id = item?.id;
  useEffect(() => {
    setUrls([]);
    if (!id) return;
    let alive = true;
    be().getItem(id).then((v) => alive && setUrls(findUrls(v))).catch(() => undefined);
    return () => { alive = false; };
  }, [id]);
  if (!item) return [];
  const quick = QUICK_COPY[item.type] ?? [];
  const list: Entry[] = [
    { id: "view", label: "View", icon: Eye, run: () => viewItem(item) },
    { id: "edit", label: "Edit", icon: Pencil, run: () => editItem(item) },
    { id: "clone", label: "Clone", icon: Files, run: () => cloneItem(item) },
    item.archived
      ? { id: "archive", label: "Unarchive", icon: ArchiveRestore, run: () => setArchived(item, false) }
      : { id: "archive", label: "Archive", icon: Archive, run: () => setArchived(item, true) },
    { id: "move", label: "Move to folder", icon: FolderInput, run: () => moveItemToFolder({ id: item.id, title: item.title, folder: item.folder ?? null }) },
    { id: "trash", label: "Move to trash", icon: Trash2, run: () => trashItem(item), tone: "danger" },
  ];
  // One address: "Launch". Several: one entry each, so the right one is a single tap.
  if (urls.length === 1) list.push({ id: "launch", label: "Launch", icon: ExternalLink, run: () => launch(item), group: true });
  else urls.slice(0, MAX_OPEN_ENTRIES).forEach((u, i) => list.push({ id: `open-${i}`, label: `Open ${shortUrl(u)}`, icon: ExternalLink, run: () => openLink(u), group: i === 0 }));
  const hasUrl = urls.length > 0;
  quick.forEach((q, i) => list.push({ id: `copy-${q.key}`, label: `Copy ${q.label}`, icon: Copy, run: () => copyQuick(item, q), group: !hasUrl && i === 0 }));
  return list;
}

function Popover({ open, entries }: { open: Open; entries: Entry[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: open.x, top: open.y, ready: false });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    const pad = 8;
    // A button anchors the menu's right edge under itself; a pointer anchors its top-left corner.
    const left = open.button ? open.x - w : open.x;
    const below = open.y + h + pad <= window.innerHeight;
    setPos({ left: Math.max(pad, Math.min(left, window.innerWidth - w - pad)), top: below ? open.y : Math.max(pad, open.y - h - (open.button ? 36 : 0)), ready: true });
  }, [open, entries.length]);

  useEffect(() => {
    const first = ref.current?.querySelector<HTMLElement>('[role="menuitem"]');
    first?.focus({ preventScroll: true });
    const away = (e: PointerEvent) => { if (!ref.current?.contains(e.target as Node)) closeMenu(); };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.stopPropagation(); closeMenu(); return; }
      if (e.key === "Tab") { closeMenu(); return; }
      if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
      e.preventDefault();
      const items = [...(ref.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
      const at = items.indexOf(document.activeElement as HTMLElement);
      const next = e.key === "Home" ? 0 : e.key === "End" ? items.length - 1 : (at + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
      items[next]?.focus();
    };
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("resize", closeMenu);
    window.addEventListener("blur", closeMenu);
    document.addEventListener("scroll", closeMenu, true);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("resize", closeMenu);
      window.removeEventListener("blur", closeMenu);
      document.removeEventListener("scroll", closeMenu, true);
      open.from?.focus?.({ preventScroll: true });
    };
  }, [open]);

  return createPortal(
    <div
      ref={ref}
      role="menu"
      aria-label={`Actions for ${open.item.title}`}
      className="surface-float fixed z-[60] max-w-[min(22rem,calc(100vw-1rem))] min-w-52 rounded-lg p-1.5"
      style={{ left: pos.left, top: pos.top, visibility: pos.ready ? "visible" : "hidden" }}
    >
      {entries.map((e) => (
        <div key={e.id}>
          {e.group && <div role="separator" className="mx-2 my-1 h-px bg-hairline" />}
          <button
            type="button"
            role="menuitem"
            onClick={() => { menu.set(null); void e.run(); }}
            className={`pressable flex min-h-9 w-full items-center gap-3 rounded-sm px-3 text-left hover:bg-raised focus-visible:bg-raised ${e.tone === "danger" ? "text-ember" : "text-frost"}`}
          >
            <e.icon className="size-4 shrink-0 opacity-80" aria-hidden />
            <span className="min-w-0 truncate">{e.label}</span>
          </button>
        </div>
      ))}
    </div>,
    document.body,
  );
}

/** Mounted once in the app. A popover next to the pointer on desktop, a bottom sheet on the compact layout. */
export function ItemMenuHost() {
  const open = useStore(menu, (s) => s);
  const layout = useLayout();
  const entries = useEntries(open?.item ?? null);
  if (!open) return null;
  if (layout === "compact") {
    return (
      <Sheet open onClose={closeMenu} title={open.item.title} variant="dialog" size="sm">
        <ul className="flex flex-col pb-2">
          {entries.map((e) => (
            <li key={e.id} className={e.group ? "mt-1 border-t border-hairline pt-1" : ""}>
              <button type="button" onClick={() => { closeMenu(); void e.run(); }} className={`pressable flex min-h-12 w-full items-center gap-4 rounded-lg px-2 text-left hover:bg-raised ${e.tone === "danger" ? "text-ember" : ""}`}>
                <e.icon className="size-5 shrink-0 opacity-80" aria-hidden />
                <span className="min-w-0 truncate">{e.label}</span>
              </button>
            </li>
          ))}
        </ul>
      </Sheet>
    );
  }
  return <Popover open={open} entries={entries} />;
}
