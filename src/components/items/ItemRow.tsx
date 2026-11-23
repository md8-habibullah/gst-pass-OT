import { motion } from "motion/react";
import { Star, Timer } from "lucide-react";
import type { ItemSummary } from "../../backend";
import { relativeTime } from "../../lib/format";
import { Avatar } from "../ui/Avatar";

/** One row of the item list. The brass selection edge glides between rows via a shared layout id. */
export function ItemRow({ item, selected, onSelect, roomy }: { item: ItemSummary; selected: boolean; onSelect: () => void; roomy: boolean }) {
  return (
    <li role="presentation">
      <button
        type="button"
        role="option"
        aria-selected={selected}
        data-item-row
        onClick={onSelect}
        className={`pressable group relative flex w-full items-center gap-3 rounded-lg px-3 text-left transition-colors ${roomy ? "min-h-[4.5rem]" : "min-h-[3.75rem]"} ${selected ? "bg-raised shadow-e2" : "hover:bg-raised/60"}`}
      >
        {selected && (
          <motion.span
            layoutId="row-edge"
            aria-hidden
            className="absolute top-3 bottom-3 left-0 w-[3px] rounded-full bg-brass"
            transition={{ type: "spring", stiffness: 560, damping: 42 }}
          />
        )}
        <Avatar title={item.title} size={roomy ? 44 : 38} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold">{item.title}</span>
          <span className="block truncate text-[0.93em] text-mist">{item.subtitle ?? "No username"}</span>
        </span>
        <span className="flex shrink-0 flex-col items-end gap-1.5 self-start pt-3.5 text-[0.8em] text-fog">
          <span className="whitespace-nowrap">{relativeTime(item.updatedAt)}</span>
          <span className="flex items-center gap-1.5">
            {item.hasTotp && <Timer className="size-3.5 text-verdigris" aria-label="Has a one-time code" />}
            {item.favorite && <Star className="size-3.5 fill-brass text-brass" aria-label="Favorite" />}
          </span>
        </span>
      </button>
    </li>
  );
}
