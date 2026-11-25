// DEV-ONLY deep links for design review and screenshots. Active only with the in-memory preview backend
// in a development browser; the whole module is dropped from production builds (see App.tsx).
import { PREVIEW_PASSWORD, gate, seedPreview, simulateOtherDeviceEdit } from "../backend/mock";
import { app, be, enterApp, refreshDevice, refreshGate, refreshProfiles, select, selectProfile, setFilter, setTheme, showPicker, syncAfterCoreLock, unlock, visibleList, type Filter } from "./app";
import { setBrand } from "./brand";
import { reprompt } from "../components/reprompt/Reprompt";
import { openEditor, openNewItem, setSettingsPage, type SettingsPageId } from "./ui";
import type { ItemType, Theme } from "../backend";

/** ?theme=light|dark|system (works with every screen)  &removed=1 (the last screen of "Remove Hexis from this device")
 *  ?vault=unlocked|locked|picker|gate  &gate=pin|password|grid (credential 24680, 2468x / abcd / 12589)  &panic=1  &disguise=<preset id>  &lower=1
 *    &page=<settings page: security|applock|accounts|time|theme|appearance|data|help|about>
 *    &account=<name>  &filter=trash|codes|settings|…  &open=<n>  &sheet=new|edit|reprompt  &type=<item type>  &otherdevice=1 (the open item gets a copy from another device) */
export async function devBootstrap(): Promise<void> {
  if (be().kind !== "preview") return;
  const q = new URLSearchParams(location.search);
  const theme = q.get("theme");
  if (theme === "light" || theme === "dark" || theme === "system") await setTheme(theme as Theme);
  if (q.get("removed") === "1") { app.set((s) => ({ ...s, phase: "removed" })); return; }
  const mode = q.get("vault");
  if (!mode) return;
  await seedPreview();
  await refreshDevice(); // the seed marks the device as set up, which boot() had not seen yet
  await refreshProfiles();
  const disguiseId = q.get("disguise");
  if (disguiseId) setBrand(await be().disguiseSet({ presetId: disguiseId, customName: null, lowercase: q.get("lower") === "1" }));
  if (mode === "picker") { await showPicker(); return; }
  if (mode === "gate") {
    const m = (q.get("gate") ?? "pin") as "pin" | "password" | "grid";
    gate.seed(m, m === "pin" ? "24680" : m === "password" ? "abcd" : "12589", { panicButton: q.get("panic") === "1" });
    app.set((s) => ({ ...s, phase: "gate" }));
    await refreshGate();
    return;
  }
  const wanted = q.get("account");
  const pick = app.get().profiles.find((p) => p.displayName === wanted) ?? app.get().profiles[0];
  if (!pick) return;
  await selectProfile(pick.id);
  if (mode === "locked") return;
  await unlock(PREVIEW_PASSWORD);
  enterApp();
  const filter = q.get("filter") as Filter | null;
  if (filter) setFilter(filter);
  const page = q.get("page");
  // after the first render: in development StrictMode mounts Settings twice, and the first unmount resets the page
  if (page) setTimeout(() => setSettingsPage(page as SettingsPageId), 300);
  const open = q.get("open");
  if (open !== null) {
    const s = app.get();
    const id = filter === "trash" ? s.trash[Number(open)]?.id : visibleList(s).items[Number(open)]?.id;
    if (id) select(id);
  }
  const sheet = q.get("sheet");
  const id = app.get().selectedId;
  if (q.get("otherdevice") === "1" && id) { simulateOtherDeviceEdit(id); await syncAfterCoreLock(); }
  if (sheet === "new") openNewItem((q.get("type") as ItemType | null) ?? null);
  if (sheet === "edit" && id) openEditor(id, await be().beginEdit(id, PREVIEW_PASSWORD));
  if (sheet === "reprompt") void reprompt({ reason: "Show the secrets for “GitHub” for 20 seconds.", confirmLabel: "Reveal", run: async () => undefined });
}
