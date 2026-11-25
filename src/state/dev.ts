// DEV-ONLY deep links for design review and screenshots. Active only with the in-memory preview backend
// in a development browser; the whole module is dropped from production builds (see App.tsx).
import { PREVIEW_PASSWORD, gate, seedPreview, simulateBridgeRequest, simulateOtherDeviceEdit, simulatePasskeySaved } from "../backend/mock";
import { app, be, enterApp, refresh, refreshDevice, refreshGate, refreshProfiles, select, selectProfile, setFilter, setTheme, showPicker, syncAfterCoreLock, unlock, visibleList, type Filter } from "./app";
import { setBrand } from "./brand";
import { reprompt } from "../components/reprompt/Reprompt";
import { openEditor, openNewItem, openScanner, setSettingsPage, type SettingsPageId } from "./ui";
import type { BridgeVerification, ItemType, Theme } from "../backend";

/** ?theme=light|dark|system (works with every screen)  &removed=1 (the last screen of "Remove Hexis from this device")
 *  ?vault=unlocked|locked|picker|gate  &gate=pin|password|grid (credential 24680, 2468x / abcd / 12589)  &panic=1  &disguise=<preset id>  &lower=1
 *    &page=<settings page: security|list|applock|passkeys|browsers|accounts|time|theme|appearance|data|help|about>
 *    &account=<name>  &filter=trash|codes|settings|…  &open=<n>  &sheet=new|edit|reprompt|scan  &type=<item type>  &otherdevice=1 (the open item gets a copy from another device)  &passkey=1 (the open item gets two passkeys)
 *    &bridge=get|create (a website in a browser asks for a passkey; switches the browser bridge on)  &verify=required|preferred  &site=<https origin> */
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
  if (mode === "locked") {
    // a request that waits behind the lock screen: open the vault, switch the bridge on and queue it, then lock again
    if (q.get("bridge")) { await unlock(PREVIEW_PASSWORD); await startBridgeDemo(q); await selectProfile(pick.id); }
    return;
  }
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
  if (q.get("passkey") === "1" && id) { simulatePasskeySaved(id); simulatePasskeySaved(id, "github.com", "hexis-dev"); await syncAfterCoreLock(); }
  if (q.get("otherdevice") === "1" && id) { simulateOtherDeviceEdit(id); await syncAfterCoreLock(); }
  if (sheet === "scan") openScanner("choose");
  if (sheet === "new") openNewItem((q.get("type") as ItemType | null) ?? null);
  if (sheet === "edit" && id) openEditor(id, await be().beginEdit(id, PREVIEW_PASSWORD));
  await startBridgeDemo(q);
  if (sheet === "reprompt") void reprompt({ reason: "Show the secrets for “GitHub” for 20 seconds.", confirmLabel: "Reveal", run: async () => undefined });
}

/** &bridge=get|create: switch the browser bridge on and let a website ask for a passkey (needs an open vault). */
async function startBridgeDemo(q: URLSearchParams): Promise<void> {
  const ask = q.get("bridge");
  if (ask !== "get" && ask !== "create") return;
  const site = q.get("site") ?? "https://example.com";
  if (ask === "get") {
    // a sign-in needs a passkey for the website: put one on the first login
    const login = app.get().items.find((i) => i.type === "login" && !i.archived);
    if (login) { simulatePasskeySaved(login.id, new URL(site).hostname); await refresh(); }
  }
  await be().bridgeSet(true);
  await refreshDevice();
  simulateBridgeRequest(ask, { origin: site, verification: (q.get("verify") as BridgeVerification | null) ?? "preferred" });
}
