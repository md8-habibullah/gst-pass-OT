import { useMemo } from "react";
import { TYPE_LABEL, type ItemType, type ListSort } from "../../backend";
import { activeFilterCount, CHANGED_LABEL, LIST_SORTS, NO_FILTERS, SORT_LABEL, tagsIn, type ChangedWithin, type ListFilters } from "../../lib/listView";
import { app, setFilters, setIncludeOld, setListSort } from "../../state/app";
import { useStore } from "../../state/store";
import { Button } from "../ui/Button";
import { Field, Select, Switch, TextInput } from "../ui/Inputs";
import { TYPE_META, TYPE_ORDER } from "./typeMeta";

/** The tick under the search box: look in earlier versions of every item too. */
export function OldVersionsTick() {
  const on = useStore(app, (s) => s.includeOld);
  return (
    <label className="flex min-h-8 cursor-pointer items-center gap-2.5 text-[0.93em] text-mist pointer-coarse:min-h-11">
      <input type="checkbox" checked={on} onChange={(e) => setIncludeOld(e.target.checked)} className="size-4.5 shrink-0 accent-[var(--color-brass-ink)] pointer-coarse:size-5.5" />
      Include earlier versions
    </label>
  );
}

const CHANGED: ChangedWithin[] = ["any", "day", "week", "month", "year"];

/** A version bound is a whole number from 1 up; anything else (empty, 0, text) means "no bound". */
const bound = (raw: string): number | null => {
  const n = Math.floor(Number(raw));
  return raw.trim() !== "" && Number.isFinite(n) && n >= 1 ? Math.min(n, 99_999) : null;
};

function ToggleRow({ label, hint, on, onChange }: { label: string; hint?: string; on: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0">
        <p className="text-[0.95em]">{label}</p>
        {hint && <p className="text-[0.86em] text-fog">{hint}</p>}
      </div>
      <Switch checked={on} onChange={onChange} label={label} />
    </div>
  );
}

/**
 * Order and filters for the list, in one panel under the search box. Everything here works on what the list already
 * knows about an item (title, type, dates, tags, how many versions it has): no secret is read to filter.
 */
export function ListTools({ onClose }: { onClose: () => void }) {
  const items = useStore(app, (s) => s.items);
  const filters = useStore(app, (s) => s.filters);
  const sort = useStore(app, (s) => s.device?.listSort ?? "updated-desc");
  const tags = useMemo(() => tagsIn(items), [items]);
  const set = (p: Partial<ListFilters>) => setFilters({ ...filters, ...p });
  const active = activeFilterCount(filters);
  const toggleType = (t: ItemType) => set({ types: filters.types.includes(t) ? filters.types.filter((x) => x !== t) : [...filters.types, t] });
  const badRange = filters.minVersion !== null && filters.maxVersion !== null && filters.minVersion > filters.maxVersion;

  return (
    <section aria-label="Order and filters" className="surface mt-3 flex flex-col gap-4 rounded-lg p-4">
      <Field label="Order">
        {({ id, describedBy }) => (
          <Select id={id} aria-describedby={describedBy} value={sort} onChange={(e) => void setListSort(e.target.value as ListSort)}>
            {LIST_SORTS.map((s) => <option key={s} value={s}>{SORT_LABEL[s]}</option>)}
          </Select>
        )}
      </Field>

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1.5 text-[0.86em] font-medium text-mist">Type</legend>
        <div className="flex flex-wrap gap-2">
          {TYPE_ORDER.map((t) => {
            const on = filters.types.includes(t);
            const Icon = TYPE_META[t].icon;
            return (
              <button key={t} type="button" aria-pressed={on} onClick={() => toggleType(t)} className={`pressable inline-flex min-h-9 items-center gap-1.5 rounded-full border px-3 text-[0.93em] font-medium pointer-coarse:min-h-11 ${on ? "border-brass-ink/60 bg-brass/15 text-brass-ink-hi" : "border-hairline bg-raised text-mist"}`}>
                <Icon className="size-3.5" aria-hidden />
                {TYPE_LABEL[t]}
              </button>
            );
          })}
        </div>
      </fieldset>

      <div className="flex flex-col gap-3">
        <ToggleRow label="Favorites only" on={filters.favoritesOnly} onChange={(v) => set({ favoritesOnly: v })} />
        <ToggleRow label="Has a one-time code" on={filters.withCode} onChange={(v) => set({ withCode: v })} />
        <ToggleRow label="Asks for the master password" on={filters.protectedOnly} onChange={(v) => set({ protectedOnly: v })} />
      </div>

      <div className="grid grid-cols-1 gap-4 min-[480px]:grid-cols-2">
        <Field label="Tag">
          {({ id, describedBy }) => (
            <Select id={id} aria-describedby={describedBy} value={filters.tag ?? ""} onChange={(e) => set({ tag: e.target.value || null })} disabled={tags.length === 0 && !filters.tag}>
              <option value="">{tags.length === 0 ? "No tags yet" : "Any tag"}</option>
              {tags.map((t) => <option key={t} value={t}>{t}</option>)}
            </Select>
          )}
        </Field>
        <Field label="Changed">
          {({ id, describedBy }) => (
            <Select id={id} aria-describedby={describedBy} value={filters.changedWithin} onChange={(e) => set({ changedWithin: e.target.value as ChangedWithin })}>
              {CHANGED.map((c) => <option key={c} value={c}>{CHANGED_LABEL[c]}</option>)}
            </Select>
          )}
        </Field>
      </div>

      <fieldset>
        <legend className="mb-1.5 text-[0.86em] font-medium text-mist">Number of versions</legend>
        <div className="grid grid-cols-2 gap-3">
          <Field label="At least">
            {({ id, describedBy, invalid }) => (
              <TextInput id={id} aria-describedby={describedBy} invalid={invalid || badRange} inputMode="numeric" placeholder="Any" value={filters.minVersion ?? ""} onChange={(e) => set({ minVersion: bound(e.target.value) })} />
            )}
          </Field>
          <Field label="At most">
            {({ id, describedBy, invalid }) => (
              <TextInput id={id} aria-describedby={describedBy} invalid={invalid || badRange} inputMode="numeric" placeholder="Any" value={filters.maxVersion ?? ""} onChange={(e) => set({ maxVersion: bound(e.target.value) })} />
            )}
          </Field>
        </div>
        {badRange && <p role="alert" className="mt-1.5 text-[0.86em] text-ember">“At least” is higher than “At most”, so nothing can match.</p>}
      </fieldset>

      <div className="flex items-center justify-between gap-3 border-t border-hairline pt-3">
        <Button variant="ghost" disabled={active === 0} onClick={() => setFilters(NO_FILTERS)}>Clear filters</Button>
        <Button variant="quiet" onClick={onClose}>Done</Button>
      </div>
    </section>
  );
}
