import { useMemo } from "react";
import { ChevronRight, Timer } from "lucide-react";
import type { ItemSummary } from "../../backend";
import { copyText } from "../../lib/clipboard";
import { groupDigits } from "../../lib/format";
import { ringTone, TONE_COLOR } from "../../lib/totp";
import { app, pushToast, select, setFilter } from "../../state/app";
import { useStore } from "../../state/store";
import { Avatar } from "../ui/Avatar";
import { IconButton } from "../ui/Button";
import { CountdownRing } from "../ui/Ring";
import { useTotp } from "./FieldRows";

function CodeTile({ item }: { item: ItemSummary }) {
  const { now, failed, remainingMs, seconds } = useTotp(item.id, true);
  const tone = ringTone(seconds);
  async function copy() {
    if (!now) return;
    try {
      await copyText(now.code);
      pushToast({ kind: "success", text: `Code for ${item.title} copied.` });
    } catch {
      pushToast({ kind: "error", text: "Copying was blocked on this device." });
    }
  }
  return (
    <li className="relative">
      <button type="button" onClick={() => void copy()} disabled={!now} aria-label={now ? `Copy code for ${item.title}` : `${item.title}, loading`} className="pressable glass flex w-full items-center gap-3.5 rounded-lg py-3.5 pr-14 pl-4 text-left hover:bg-raised/70 pointer-coarse:py-4">
        <Avatar title={item.title} size={42} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-semibold">{item.title}</span>
          <span className="block truncate text-[0.86em] text-mist">{item.subtitle ?? "Tap to copy"}</span>
          <span className={`mt-1 block font-mono text-[1.55rem] leading-8 font-medium tracking-[0.08em] tabular-nums ${tone === "critical" ? "text-ember" : "text-frost"}`}>
            {failed ? <span className="text-[0.6em] text-ember">{failed}</span> : now ? groupDigits(now.code) : "••• •••"}
          </span>
        </span>
        {now && (
          <CountdownRing totalMs={now.period * 1000} remainingMs={remainingMs} windowKey={now.validUntilMs} color={TONE_COLOR[tone]} size={36} stroke={3}>
            <span className="relative text-[0.78em] font-semibold tabular-nums">{seconds}</span>
          </CountdownRing>
        )}
      </button>
      <IconButton label={`Open ${item.title}`} className="absolute top-1/2 right-2 -translate-y-1/2" onClick={() => { setFilter("all"); select(item.id); }}>
        <ChevronRight className="size-[18px]" />
      </IconButton>
    </li>
  );
}

/** Every item that has a one-time code, with live rings. Tap a tile to copy its code. */
export function CodesView() {
  const items = useStore(app, (s) => s.items);
  const withCodes = useMemo(() => items.filter((i) => i.hasTotp).sort((a, b) => a.title.localeCompare(b.title)), [items]);
  return (
    <section aria-label="Codes" className="flex min-h-0 flex-1 flex-col">
      <div className="px-5 pt-[calc(1rem+var(--safe-top))] pb-3 min-[720px]:pt-4">
        <h2 className="font-display text-[1.45rem] leading-none font-semibold">Codes</h2>
        <p className="mt-2 text-[0.93em] text-mist">Time-based codes for every item that has one. They are generated on this device.</p>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 pb-28 min-[720px]:pb-6">
        {withCodes.length === 0 ? (
          <div className="px-6 pt-14 text-center">
            <Timer className="mx-auto size-8 text-fog" aria-hidden />
            <p className="mt-3 font-display text-xl">No one-time codes yet</p>
            <p className="mt-2 text-mist">Add a setup key or otpauth link to a login, or create a “Code only” item.</p>
          </div>
        ) : (
          <ul className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,17rem),1fr))] gap-3">
            {withCodes.map((i) => <CodeTile key={i.id} item={i} />)}
          </ul>
        )}
      </div>
    </section>
  );
}
