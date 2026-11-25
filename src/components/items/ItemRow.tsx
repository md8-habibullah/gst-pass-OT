import { useRef } from "react";
import { motion } from "motion/react";
import { Archive, KeyRound, MoreHorizontal, Star, Timer } from "lucide-react";
import type { ItemSummary } from "../../backend";
import { relativeTime } from "../../lib/format";
import { Avatar } from "../ui/Avatar";
import { IconButton } from "../ui/Button";
import { openItemMenu } from "./ItemMenu";
import { TYPE_META } from "./typeMeta";

const LONG_PRESS_MS = 480;

/**
 * One row of the item list. The brass selection edge glides between rows via a shared layout id.
 * The row menu opens from the "more" button, a right click, the menu key, or a long press on touch screens.
 */
export function ItemRow({ item, selected, onSelect, roomy, showType, note }: { item: ItemSummary; selected: boolean; onSelect: () => void; roomy: boolean; showType: boolean; note?: string }) {
  const press = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number } | null>(null);
  const suppressClick = useRef(false);
  const TypeIcon = TYPE_META[item.type].icon;

  const cancelPress = () => {
    if (press.current) clearTimeout(press.current.timer);
    press.current = null;
  };

  return (
    <li role="presentation" className="group/row relative">
      <button
        type="button"
        role="option"
        aria-selected={selected}
        data-item-row
        data-item-id={item.id}
        onClick={() => {
          if (suppressClick.current) { suppressClick.current = false; return; }
          onSelect();
        }}
        onContextMenu={(e) => { e.preventDefault(); openItemMenu(item, { x: e.clientX, y: e.clientY }); }}
        onPointerDown={(e) => {
          if (e.pointerType !== "touch") return;
          const { clientX: x, clientY: y } = e;
          press.current = { x, y, timer: setTimeout(() => { suppressClick.current = true; press.current = null; openItemMenu(item, { x, y }); }, LONG_PRESS_MS) };
        }}
        onPointerMove={(e) => { if (press.current && Math.hypot(e.clientX - press.current.x, e.clientY - press.current.y) > 10) cancelPress(); }}
        onPointerUp={cancelPress}
        onPointerCancel={cancelPress}
        className={`pressable relative flex w-full items-center gap-3 rounded-lg py-1 pr-12 pl-3 text-left transition-colors ${roomy ? "min-h-[4.5rem]" : "min-h-[3.75rem]"} ${selected ? "bg-raised shadow-e2" : "hover:bg-raised/60"} ${item.archived ? "opacity-80" : ""}`}
      >
        {selected && (
          <motion.span
            layoutId="row-edge"
            aria-hidden
            className="absolute top-3 bottom-3 left-0 w-[3px] rounded-full bg-brass-ink"
            transition={{ type: "spring", stiffness: 560, damping: 42 }}
          />
        )}
        <Avatar title={item.title} size={roomy ? 44 : 38} dim={item.archived} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold">{item.title}</span>
          <span className="block truncate text-[0.93em] text-mist">{item.subtitle ?? (item.type === "login" ? "No username" : "")}</span>
          {note && <span className="block truncate text-[0.8em] text-fog">{note}</span>}
        </span>
        <span className="flex shrink-0 flex-col items-end gap-1.5 self-start pt-3.5 text-[0.8em] text-fog">
          <span className="whitespace-nowrap">{relativeTime(item.updatedAt)}</span>
          <span className="flex items-center gap-1.5">
            {item.archived && <Archive className="size-3.5" aria-label="Archived" />}
            {item.reprompt && <KeyRound className="size-3.5 text-brass-ink" aria-label="Asks for the master password" />}
            {showType && item.type !== "login" && <TypeIcon className="size-3.5" aria-hidden />}
            {item.hasTotp && <Timer className="size-3.5 text-verdigris" aria-label="Has a one-time code" />}
            {item.favorite && <Star className="size-3.5 fill-brass-ink text-brass-ink" aria-label="Favorite" />}
          </span>
        </span>
      </button>
      <IconButton
        label={`More actions for ${item.title}`}
        size="sm"
        aria-haspopup="menu"
        onClick={(e) => openItemMenu(item, e.currentTarget)}
        className={`absolute top-1/2 right-1.5 -translate-y-1/2 ${roomy ? "" : "opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100"}`}
      >
        <MoreHorizontal className="size-[18px]" />
      </IconButton>
    </li>
  );
}
