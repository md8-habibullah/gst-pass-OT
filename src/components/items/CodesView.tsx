import { useMemo, useState } from "react";
import { ChevronRight, KeyRound, Plus, Search, Timer, X } from "lucide-react";
import type { ItemSummary } from "../../backend";
import { copyText } from "../../lib/clipboard";
import { groupDigits } from "../../lib/format";
import { ringTone, TONE_COLOR } from "../../lib/totp";
import { app, pushToast } from "../../state/app";
import { useStore } from "../../state/store";
import { openNewItem } from "../../state/ui";
import { Avatar } from "../ui/Avatar";
import { Button, IconButton } from "../ui/Button";
import { TextInput } from "../ui/Inputs";
import { CountdownRing } from "../ui/Ring";
import { useLayout } from "../../lib/layout";
import { AccountBar } from "../shell/AccountMenu";
import { ScanButton } from "../scan/ScanButton";
import { useTotp } from "./FieldRows";
import { viewItem } from "./actions";

function CodeTile({ item, codeKey, ordinal }: { item: ItemSummary; codeKey: string; /** 1-based place among the item's codes; 0 when it has just one */ ordinal: number }) {
  const { now, failed, locked, unlock, remainingMs, seconds } = useTotp(item.id, true, item.title, codeKey);
  const tone = ringTone(seconds);
  const codeOnly = item.type === "totp";
  async function copy() {
    if (locked) return void unlock().catch((e: unknown) => pushToast({ kind: "error", text: e instanceof Error ? e.message : "Could not show the code." }));
    if (!now) return;
    try {
      await copyText(now.code);
      pushToast({ kind: "success", text: `Code${ordinal ? ` ${ordinal}` : ""} for ${item.title} copied.` });
    } catch {
      pushToast({ kind: "error", text: "Copying was blocked on this device." });
    }
  }
  return (
    <li className="relative">
      <button type="button" onClick={() => void copy()} disabled={!now && !locked} aria-label={now ? `Copy code${ordinal ? ` ${ordinal}` : ""} for ${item.title}` : locked ? `Show code${ordinal ? ` ${ordinal}` : ""} for ${item.title}. It asks for your master password.` : `${item.title}, loading`} className="pressable surface flex w-full items-center gap-3.5 rounded-lg py-3.5 pr-14 pl-4 text-left hover:bg-raised/70 pointer-coarse:py-4">
        <Avatar title={item.title} size={42} />
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            <span className="min-w-0 truncate font-semibold">{item.title}</span>
            <span className="shrink-0 rounded-full border border-hairline-strong px-2 py-px text-[0.76em] text-mist">{codeOnly ? "Code only" : ordinal ? `In a login, code ${ordinal}` : "In a login"}</span>
          </span>
          <span className="block truncate text-[0.86em] text-mist">{item.subtitle ?? "Tap to copy"}</span>
          <span className={`mt-1 block font-mono text-[1.55rem] leading-8 font-medium tracking-[0.08em] tabular-nums ${tone === "critical" ? "text-ember" : "text-frost"}`}>
            {failed ? <span className="text-[0.6em] text-ember">{failed}</span> : now ? groupDigits(now.code) : locked ? <span className="flex items-center gap-2 text-[0.62em] font-normal tracking-normal text-mist"><KeyRound className="size-4" aria-hidden /> Tap to show with your master password</span> : "••• •••"}
          </span>
          {now?.next && (
            <span className="mt-0.5 flex items-baseline gap-2 text-[0.82em] text-mist">
              Next
              <span className={`font-mono tracking-[0.06em] tabular-nums ${tone === "critical" ? "font-medium text-frost" : ""}`}>{groupDigits(now.next)}</span>
            </span>
          )}
        </span>
        {now && (
          <CountdownRing totalMs={now.period * 1000} remainingMs={remainingMs} windowKey={now.validUntilMs} color={TONE_COLOR[tone]} size={36} stroke={3}>
            <span className="relative text-[0.78em] font-semibold tabular-nums">{seconds}</span>
          </CountdownRing>
        )}
      </button>
      <IconButton label={`Open ${item.title}`} className="absolute top-1/2 right-2 -translate-y-1/2" onClick={() => viewItem(item)}>
        <ChevronRight className="size-[18px]" />
      </IconButton>
    </li>
  );
}

/** Every one-time code in the vault: stand-alone "Code only" items and the codes inside logins. */
export function CodesView() {
  const items = useStore(app, (s) => s.items);
  const compact = useLayout() === "compact";
  const [query, setQuery] = useState("");
  const all = useMemo(() => items.filter((i) => i.hasTotp && !i.archived).sort((a, b) => a.title.localeCompare(b.title)), [items]);
  const q = query.trim().toLowerCase();
  const shown = useMemo(() => all.filter((i) => !q || i.title.toLowerCase().includes(q) || (i.subtitle ?? "").toLowerCase().includes(q)), [all, q]);
  return (
    <section aria-label="Codes" className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-start gap-3 px-5 pt-[calc(1rem+var(--safe-top))] pb-3 min-[720px]:pt-4">
        <div className="min-w-0 flex-1">
          <h2 className="font-display text-[1.45rem] leading-none font-semibold">Codes</h2>
          <p className="mt-2 text-[0.93em] text-mist">Every one-time code in your vault: code-only items and the codes saved inside logins. They are generated on this device.</p>
        </div>
        {!compact && <ScanButton />}
        {!compact && <Button variant="key" icon={<Plus className="size-4" />} onClick={() => openNewItem("totp")}>Add code</Button>}
        <AccountBar className="-mt-1.5 -mr-1" />
      </div>
      {all.length > 4 && (
        <div className="px-5 pb-3">
          <div className="relative">
            <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-fog" aria-hidden />
            <TextInput type="search" aria-label="Search codes" placeholder="Search codes" value={query} onChange={(e) => setQuery(e.target.value)} className="pr-10 pl-9" />
            {query && (
              <IconButton label="Clear search" size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => setQuery("")}>
                <X className="size-4" />
              </IconButton>
            )}
          </div>
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 pb-28 min-[720px]:pb-6">
        {all.length === 0 ? (
          <div className="px-6 pt-14 text-center">
            <Timer className="mx-auto size-8 text-fog" aria-hidden />
            <p className="mt-3 font-display text-xl">No one-time codes yet</p>
            <p className="mt-2 text-mist">Add a code on its own, or put a one-time code inside a login. Both show up here.</p>
            <Button variant="key" className="mt-5" icon={<Plus className="size-4" />} onClick={() => openNewItem("totp")}>Add a code</Button>
          </div>
        ) : shown.length === 0 ? (
          <p className="px-6 pt-14 text-center text-mist">No code matches “{query}”.</p>
        ) : (
          <ul className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,19rem),1fr))] gap-3">
            {shown.flatMap((i) => {
              // A login can hold several codes: a tile each, in the order they are stored.
              const keys = i.codeKeys.length ? i.codeKeys : ["totp"];
              return keys.map((k, n) => <CodeTile key={`${i.id}:${k}`} item={i} codeKey={k} ordinal={keys.length > 1 ? n + 1 : 0} />);
            })}
          </ul>
        )}
      </div>
    </section>
  );
}
