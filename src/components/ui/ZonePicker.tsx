import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Check, Search } from "lucide-react";
import { filterZones, zoneChoices } from "../../lib/zones";
import { TextInput } from "./Inputs";

/**
 * Pick an IANA time zone from the webview's own list. Searchable by place or offset ("dhaka", "utc+06"), keyboard friendly
 * (Down from the search box enters the list, Up and Down move, Enter picks). Used on the first page and in Settings.
 */
export function ZonePicker({ value, onChange, autoFocus = false, height = "h-64" }: { value: string | null; onChange: (zone: string) => void; autoFocus?: boolean; height?: string }) {
  const all = useMemo(() => zoneChoices(), []);
  const [query, setQuery] = useState("");
  const shown = useMemo(() => filterZones(all, query), [all, query]);
  const list = useRef<HTMLUListElement>(null);
  const listId = useId();

  // Bring the chosen zone into view when the list first appears.
  useEffect(() => {
    list.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ block: "center" });
    // only on mount: later changes come from the person scrolling or typing
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function onSearchKey(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      list.current?.querySelector<HTMLElement>("button")?.focus();
    } else if (e.key === "Enter" && shown[0]) {
      e.preventDefault();
      onChange(shown[0].id);
    }
  }
  function onListKey(e: KeyboardEvent<HTMLUListElement>) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Home" && e.key !== "End") return;
    const buttons = [...(list.current?.querySelectorAll<HTMLElement>("button") ?? [])];
    const at = buttons.indexOf(document.activeElement as HTMLElement);
    if (at < 0) return;
    e.preventDefault();
    const next = e.key === "Home" ? 0 : e.key === "End" ? buttons.length - 1 : at + (e.key === "ArrowDown" ? 1 : -1);
    buttons[Math.max(0, Math.min(buttons.length - 1, next))]?.focus();
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="relative">
        <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-fog" aria-hidden />
        <TextInput
          type="search" value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={onSearchKey} data-autofocus={autoFocus || undefined}
          placeholder="Search by place or offset" aria-label="Search time zones" aria-controls={listId} className="pl-9"
        />
      </div>
      <ul
        ref={list} id={listId} role="listbox" aria-label="Time zones" onKeyDown={onListKey}
        className={`${height} overflow-y-auto overscroll-contain rounded-sm border border-hairline bg-marine/40 p-1`}
      >
        {shown.length === 0 && <li className="px-3 py-6 text-center text-mist">No time zone matches that search.</li>}
        {shown.map((z) => {
          const on = z.id === value;
          return (
            <li key={z.id} role="presentation">
              <button
                type="button" role="option" aria-selected={on} onClick={() => onChange(z.id)}
                className={`flex min-h-10 w-full items-center gap-3 rounded-sm px-3 text-left transition-colors pointer-coarse:min-h-12 ${on ? "bg-raised text-frost" : "text-mist hover:bg-raised/60 hover:text-frost"}`}
              >
                <span className="min-w-0 flex-1 truncate">{z.label}</span>
                <span className="shrink-0 font-mono text-[0.86em] text-fog tabular-nums">{z.offset}</span>
                <Check className={`size-4 shrink-0 text-brass ${on ? "" : "invisible"}`} aria-hidden />
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
