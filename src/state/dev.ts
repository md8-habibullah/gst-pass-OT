// DEV-ONLY deep links for design review and screenshots. Active only with the in-memory preview backend
// in a development browser; the whole module is dropped from production builds (see App.tsx).
import { PREVIEW_PASSWORD } from "../backend/mock";
import { app, be, createVault, enterApp, lockVault, select, setFilter, visibleItems, type Filter } from "./app";
import { reprompt } from "../components/reprompt/Reprompt";
import { openEditor, openNewItem } from "./ui";

/** ?vault=unlocked|locked  &filter=trash|codes|settings|…  &open=<n>  &sheet=new|edit|reprompt  &history  &reveal */
export async function devBootstrap(): Promise<void> {
  if (be().kind !== "preview") return;
  const q = new URLSearchParams(location.search);
  const mode = q.get("vault");
  if (!mode) return;
  if (app.get().phase === "setup") await createVault(PREVIEW_PASSWORD);
  if (mode === "locked") {
    await lockVault();
    return;
  }
  enterApp();
  const filter = q.get("filter") as Filter | null;
  if (filter) setFilter(filter);
  const open = q.get("open");
  if (open !== null) {
    const s = app.get();
    const id = filter === "trash" ? s.trash[Number(open)]?.id : visibleItems(s.items, s.filter, "")[Number(open)]?.id;
    if (id) select(id);
  }
  const sheet = q.get("sheet");
  const id = app.get().selectedId;
  if (sheet === "new") openNewItem();
  if (sheet === "edit" && id) openEditor(id, await be().beginEdit(id, PREVIEW_PASSWORD));
  if (sheet === "reprompt") void reprompt({ reason: "Show the secrets for “GitHub” for 20 seconds.", confirmLabel: "Reveal", run: async () => undefined });
}
