// The item list against the REAL core: folders first, search through notes and old versions, filters and order, several
// web addresses, "open in search", and the list's folder dialogs. Items are made over IPC; everything else is the real UI.
import { Core, SP, check, finish, freshDir, launch, sleep, text, has, waitText, waitGone, createAccount, firstStart, noDialog, runSteps, unlockUi } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const DATA = freshDir("l-data");
const core = new Core({ bin: BIN, data: DATA, port: 1449 });
const PW = "correct horse battery staple 8";
const steps = []; const step = (name, fn) => steps.push({ name, fn });
const ok = async (cmd, args = {}) => { const r = await core.ipc(cmd, args); if (r.err) throw new Error(`${cmd}: ${r.err.code} ${r.err.message}`); return r.ok; };
const items = () => ok("item_list");
const login = (title, over = {}, fields = {}) => ({ type: "login", title, favorite: false, reprompt: false, tags: [], folder: null, customFields: [], fields: { username: "me@example.com", password: "pw-" + title, url: "", notes: "", ...fields }, ...over });
async function rename(id, title) {
  const b = await ok("item_begin_edit", { id, password: PW });
  await ok("item_update", { id, token: b.token, expectedVersion: b.version, draft: { ...b.draft, title } });
}
/** Reloads the page: the core stays unlocked, so the list is read again from it. */
async function reload(p) {
  await p.goto(core.ui);
  await p.waitFor(`!!document.querySelector('[data-item-row],[data-folder-row],[aria-label="Items"]')`, { timeout: 15000, label: "the item list" });
}
const rows = (p) => p.eval(`[...document.querySelectorAll('[data-item-row]')].map((r) => r.querySelector('span.block.truncate.font-semibold')?.textContent ?? '')`);
const folderRows = (p) => p.eval(`[...document.querySelectorAll('[data-folder-row]')].map((r) => r.querySelector('span.block.truncate.font-semibold')?.textContent ?? '')`);
const search = async (p, q) => { await p.typeAria("Search items", q); await sleep(500); };
const clearSearch = async (p) => { await p.typeAria("Search items", ""); await sleep(300); };
const setOrder = (p, value) => p.eval(`(()=>{const l=[...document.querySelectorAll('label')].find((x)=>x.textContent.trim()==='Order'); const el=l&&document.getElementById(l.htmlFor); if(!el) return false; Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(el, ${JSON.stringify(value)}); el.dispatchEvent(new Event('change',{bubbles:true})); return true;})()`);
const openTools = (p) => p.click('button[aria-label^="Order and filters"]');

step("account, then items made through the core", async (p) => {
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, "Marlowe", PW);
  await ok("item_create", { draft: login("Cabin", {}, { url: "https://cabin.example\nhttps://alt.cabin.example", notes: "the alarm code is behind the clock" }) });
  await ok("item_create", { draft: login("Bank", { folder: "Money" }) });
  await ok("item_create", { draft: login("Broker", { folder: "Money/Savings", favorite: true }) });
  await ok("item_create", { draft: login("Mail", { tags: ["daily"] }) });
  const postbox = await ok("item_create", { draft: login("Postbox") });
  await rename(postbox.id, "Letters");
  await reload(p);
  check("five items exist", (await items()).length === 5);
  const cabin = (await items()).find((i) => i.title === "Cabin");
  const view = await ok("item_get", { id: cabin.id });
  check("a login keeps several web addresses, a line each", view.fields.find((f) => f.key === "url").value === "https://cabin.example\nhttps://alt.cabin.example", JSON.stringify(view.fields.find((f) => f.key === "url")));
});

step("folders come first, then the items with no folder", async (p) => {
  await waitText(p, "Folders");
  check("the folder list is shown above the items", (await folderRows(p)).join() === "Money", (await folderRows(p)).join());
  const r = await rows(p);
  check("only items with no folder are listed at the top level", r.includes("Cabin") && r.includes("Mail") && r.includes("Letters") && !r.includes("Bank") && !r.includes("Broker"), r.join());
  check("the folder says what is inside it", (await text(p)).includes("2 items, 1 folder"));
  await p.shot(`${SP}/shots2/real-lists-top.png`);
});

step("open a folder, go deeper, come back by the path", async (p) => {
  await p.click("[data-folder-row]", "Money");
  await waitText(p, "Savings");
  check("a folder shows its subfolders and its own items", (await folderRows(p)).join() === "Savings" && (await rows(p)).join() === "Bank", `${await folderRows(p)} / ${await rows(p)}`);
  await p.click("[data-folder-row]", "Savings");
  await waitGone(p, "Bank");
  check("the deepest folder holds its own item", (await rows(p)).join() === "Broker", (await rows(p)).join());
  check("a path shows where you are", await p.eval(`!!document.querySelector('nav[aria-label="Folder path"] [aria-current="page"]')`));
  await p.click('nav[aria-label="Folder path"] button', "Money");
  await waitGone(p, "Broker");
  await p.click('nav[aria-label="Folder path"] button', "All items");
  await waitText(p, "Cabin");
  check("the path leads back to the top", (await folderRows(p)).join() === "Money");
});

step("search looks through notes, and says where it found the words", async (p) => {
  await search(p, "alarm clock");
  await waitText(p, "Found in Notes");
  check("a note is found, from any folder", (await rows(p)).join() === "Cabin", (await rows(p)).join());
  await clearSearch(p); await search(p, "bank");
  await waitText(p, "Money");
  check("an item in a folder is found by its title, with its folder shown", (await rows(p)).join() === "Bank");
  await clearSearch(p); await search(p, "pw-Cabin");
  await waitText(p, "Nothing matches");
  check("a password is never searched", true);
  await clearSearch(p);
});

step("earlier versions are only searched when the tick is on", async (p) => {
  await search(p, "postbox");
  await waitText(p, "Nothing matches");
  check("the old title finds nothing at first", (await rows(p)).length === 0);
  await p.click("input[type=checkbox]");
  await waitText(p, "Earlier version 1");
  check("with the tick, the item is found by the title it had before", (await rows(p)).join() === "Letters", (await rows(p)).join());
  await p.click("input[type=checkbox]");
  await clearSearch(p);
});

step("filters and order", async (p) => {
  await openTools(p);
  await waitText(p, "Number of versions");
  await p.typeLabel("At least", "2");
  await sleep(300);
  check("a version count filter keeps only items with that many versions, from every folder", (await rows(p)).join() === "Letters", (await rows(p)).join());
  await p.click("button", "Clear filters");
  await sleep(200);
  await p.click("button[aria-pressed]", "Login");
  check("a type filter is switched on", await p.eval(`!!document.querySelector('button[aria-pressed="true"]')`));
  await p.click("button", "Clear filters");
  await p.click('[role=switch][aria-label="Favorites only"]');
  await sleep(200);
  check("favorites only reaches into folders", (await rows(p)).join() === "Broker", (await rows(p)).join());
  await p.click("button", "Clear filters");
  check("the order can be set", await setOrder(p, "title-desc"));
  await sleep(300);
  check("the list follows the chosen order", (await rows(p)).join() === "Mail,Letters,Cabin", (await rows(p)).join());
  check("the core keeps the order", (await ok("device_get")).listSort === "title-desc");
  await setOrder(p, "title-asc"); await sleep(300);
  check("the other way round", (await rows(p)).join() === "Cabin,Letters,Mail", (await rows(p)).join());
  await p.click("button", "Done");
});

step("make a folder, move an item into it, remove the folder", async (p) => {
  await p.click('button[aria-label="Add folder"]');
  await waitText(p, "Make folder");
  await p.typeLabel("Name", "Clients");
  await p.click("[role=dialog] button[type=submit]");
  await noDialog(p);
  await waitText(p, "Clients");
  check("an empty folder is kept", (await folderRows(p)).includes("Clients") && (await ok("prefs_get")).folders.includes("Clients"));
  await p.rightClick("[data-item-row]", "Mail");
  await p.click("[role=menuitem]", "Move to folder");
  await waitText(p, "Move “Mail”");
  await p.click("[role=dialog] button", "Clients");
  await noDialog(p);
  await p.waitFor(`![...document.querySelectorAll('[data-item-row]')].some((r) => r.textContent.includes('Mail'))`, { label: "Mail to leave the top level" });
  check("the item left the top level", !(await rows(p)).includes("Mail"));
  check("the core has it in the folder", (await items()).find((i) => i.title === "Mail").folder === "Clients");
  await p.click("[data-folder-row]", "Clients");
  await waitText(p, "Mail");
  await p.click('button[aria-label="Remove this folder"]');
  await waitText(p, "Remove folder");
  check("removing a folder says nothing is deleted", await has(p, "Nothing is deleted"));
  await p.click("[role=dialog] button", "Remove folder");
  await noDialog(p);
  await waitText(p, "Mail");
  check("what was inside moved up and the folder is gone", (await items()).find((i) => i.title === "Mail").folder === null && !(await ok("prefs_get")).folders.includes("Clients"));
});

step("an item's several web addresses: each can be opened, and the editor keeps them", async (p) => {
  await p.click("[data-item-row]", "Cabin");
  await waitText(p, "Website 1");
  check("every address has its own row", (await has(p, "https://cabin.example")) && (await has(p, "https://alt.cabin.example")) && (await has(p, "Website 2")));
  await p.click("button", "Edit");
  await p.waitFor(`[...document.querySelectorAll('[role=dialog] label')].some((l) => l.textContent.trim() === 'Website 2')`, { label: "the second address box" });
  check("the editor has a box for each address and a way to add another", (await has(p, "Add another website")));
  await p.click('[role=dialog] button[aria-label^="Remove website 2"]');
  await p.click("[role=dialog] button", "Save changes");
  await noDialog(p);
  await sleep(400);
  const cabin = (await items()).find((i) => i.title === "Cabin");
  const v = await ok("item_get", { id: cabin.id });
  check("removing an address in the editor keeps the other", v.fields.find((f) => f.key === "url").value === "https://cabin.example", JSON.stringify(v.fields.find((f) => f.key === "url")));
});

step("deleting forever from the trash needs the sum while the item is recent", async (p) => {
  const m = (await items()).find((i) => i.title === "Letters");
  await ok("item_trash", { id: m.id });
  const plan = await ok("item_purge_plan", { id: m.id });
  check("a plan for a recent item carries a sum and no answer", plan.recent === true && typeof plan.sum === "string" && !JSON.stringify(plan).includes("answer"));
  const r1 = await core.ipc("item_purge", { id: m.id, password: PW, answer: null });
  check("no answer is refused", r1.err?.code === "wrong_answer", JSON.stringify(r1));
  const still = (await ok("trash_list")).some((t) => t.id === m.id);
  check("a refused delete erases nothing", still);
});

step("open in search: after unlocking, the cursor is in the search box", async (p) => {
  await ok("device_set_open_in_search", { on: true });
  await ok("vault_lock");
  await p.goto(core.ui);
  await waitText(p, "Master password");
  await unlockUi(p, PW);
  await p.waitFor(`document.activeElement && document.activeElement.matches('[data-search]')`, { timeout: 15000, label: "the search box focused" });
  check("the search box has the cursor right after the vault opens", true);
  await ok("device_set_open_in_search", { on: false });
});

await core.start();
const page = await launch({ width: 1200, height: 1000, port: 9662, profile: `${SP}/chrome-e2e-lists` });
try { await runSteps(page, steps, only); } finally { await page.close(); await core.stop(); }
finish("real-lists");
