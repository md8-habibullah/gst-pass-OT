// Two devices on one bucket against the REAL core: device A (server on :1445) and device B (restored from the bucket, :1446).
// The browser shows device B; device A acts through the same IPC the app uses. Covers the event-driven pull: fast-forward,
// diverged edits (other-device copy, banner, history badge, "Got it", restore -> the other device fast-forwards), deletion,
// and an object that fails authentication (ignored, counted, local data untouched).
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { Core, SP, check, finish, freshDir, walkFiles, launch, sleep, text, has, waitText, waitGone, btn, nav, createAccount, firstStart, waitDemoShell, unlockUi, addLogin, runSteps } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const ROOT = new URL("..", import.meta.url).pathname.replace(/\/$/, "");
const S3_PORT = 9335, S3_DATA = freshDir("s-s3"), DATA_A = freshDir("s-data-a"), DATA_B = freshDir("s-data-b");
const A = new Core({ bin: BIN, data: DATA_A, port: 1445, args: [] });
const B = new Core({ bin: BIN, data: DATA_B, port: 1446, args: [] });
const NAME = "Marlowe", PW = "correct horse battery staple 7";
const legacy = process.env.LEGACY === "1" || process.env.LEGACY === "2" ? "" : ""; // the Core class appends &legacy itself
const mock = async (path) => (await fetch(`http://127.0.0.1:${S3_PORT}${path}`)).text();
const keyOf = (f) => { const n = f.slice(S3_DATA.length + 1); return /^[0-9a-f]+$/.test(n) ? Buffer.from(n, "hex").toString("utf8") : n; };

let s3;
async function startS3() {
  s3 = spawn("python3", ["-I", `${ROOT}/scripts/mock_s3.py`, "--port", String(S3_PORT), "--bucket", "hexis-test", "--access-key", "AKIAMOCK", "--secret-key", "mocksecret", "--region", "us-east-1", "--data-dir", S3_DATA], { stdio: ["ignore", "pipe", "pipe"] });
  for (let i = 0; i < 50; i++) { try { await fetch(`http://127.0.0.1:${S3_PORT}/__mock/state`); return; } catch { await sleep(100); } }
  throw new Error("mock S3 did not start");
}
async function fillTarget(p, { name }) {
  if (name) await p.typeLabel("Name", name);
  await p.click("button[aria-pressed]", "Other S3-compatible");
  await p.typeLabel("Endpoint", `http://127.0.0.1:${S3_PORT}`);
  await p.typeLabel("Bucket", "hexis-test"); await p.typeLabel("Region", "us-east-1");
  await p.typeLabel("Access key ID", "AKIAMOCK"); await p.typeLabel("Secret access key", "mocksecret");
  const pathStyle = await p.eval(`(()=>{const b=[...document.querySelectorAll('[role=switch]')].find(x=>x.getAttribute('aria-label')==='Path-style addressing'); return b? b.getAttribute('aria-checked')==='true': null})()`);
  if (pathStyle === false) await p.click('[role=switch][aria-label="Path-style addressing"]');
}
const ok = async (core, cmd, args = {}) => { const r = await core.ipc(cmd, args); if (r.err) throw new Error(`${cmd}: ${r.err.code} ${r.err.message}`); return r.ok; };
const items = async (core) => await ok(core, "item_list");
const idOf = async (core, title) => (await items(core)).find((i) => i.title === title)?.id;
async function edit(core, id, patch) {
  const b = await ok(core, "item_begin_edit", { id, password: PW });
  const draft = { ...b.draft, ...patch.top, fields: { ...b.draft.fields, ...(patch.fields ?? {}) } };
  return await ok(core, "item_update", { id, token: b.token, expectedVersion: b.version, draft });
}
/** Wait until a device has sent everything it has (its master target is healthy with an empty queue). */
async function settled(core, label, ms = 20000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const l = await ok(core, "backup_list"); const h = l[0]?.health;
    if (h && h.state === "healthy" && h.queued === 0) return true;
    await sleep(250);
  }
  check(`${label}: all changes reached the bucket`, false, JSON.stringify(await ok(core, "backup_list")).slice(0, 300)); return false;
}
async function pullNow(p) {
  // The row's own button (the sidebar status button can quote "Back up now" inside an error sentence).
  await nav(p, "Backup"); await p.click('ul[aria-label="Backup targets"] button', "Back up now");
  await sleep(1500); await nav(p, "All items");   // the run is done by now; the list must follow by itself
}
const bucketItemFiles = () => walkFiles(S3_DATA).filter((f) => /vault\/items\/.+\.enc$/.test(keyOf(f)));

const steps = []; const step = (name, fn) => steps.push({ name, fn });

step("device A: account with two logins and a master backup", async (p) => {
  await p.goto(A.ui); await firstStart(p);
  await createAccount(p, NAME, PW);
  await addLogin(p, [["Title", "GitHub"], ["Username", "marlowe-dev"], ["Password", "Tr0ub4dor&3-horse"]]);
  await addLogin(p, [["Title", "Cloudflare"], ["Username", "ops@marlowe.example"], ["Password", "Zk9-plain-secret-77"]]);
  await nav(p, "Backup"); await btn(p, "Add your first backup"); await waitText(p, "Add a backup");
  await fillTarget(p, { name: "Shared bucket" });
  await p.click("[role=dialog] button", "Add backup"); await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 15000 });
  await p.waitFor(`(document.querySelector('ul[aria-label="Backup targets"]')?.innerText??'').includes('Up to date')`, { timeout: 20000 });
  check("device A is backed up", true);
});

step("device B: restore from the bucket", async (p) => {
  await B.start();
  await p.goto(B.ui); await firstStart(p);
  await btn(p, "Restore from a backup"); await waitText(p, "Restore from a backup");
  await p.typeLabel("Account name", "Device B");
  await fillTarget(p, { name: "" });
  await btn(p, "Restore vault"); await waitText(p, "Master password", 30000);
  await unlockUi(p, PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 20000 });
  check("the restored vault is the one that is open, not the demo", await p.eval(`!!document.querySelector('button[aria-label^="Account: Device B"]')`));
  await waitText(p, "GitHub"); await waitText(p, "Cloudflare");
  check("device B has both items", true);
  const st = await ok(B, "vault_status");
  check("vault_status carries the sync fields", typeof st.syncRevision === "number" && typeof st.reconciling === "boolean", JSON.stringify(st));
});

/** Waits until a device's own pull-on-unlock has finished (it runs on a worker thread after the unlock returns). */
async function quiet(core, label, ms = 20000) {
  const t0 = Date.now();
  let still = 0;
  while (Date.now() - t0 < ms) {
    still = (await ok(core, "vault_status")).reconciling ? 0 : still + 1;
    if (still >= 3) return;
    await sleep(150);
  }
  check(`${label}: its own pull after the unlock finished`, false, JSON.stringify(await ok(core, "vault_status")));
}

step("fast-forward: A edits, B pulls on 'Back up now' and the list updates by itself", async (p) => {
  // B's own pull-on-unlock must be over before A edits. On a slow machine it can still be running, find A's edit in the
  // bucket and apply it by itself, so the counter has already moved when this step reads it (seen on a CI runner: 1 -> 1).
  await quiet(B, "B");
  const before = (await ok(B, "vault_status")).syncRevision;
  const id = await idOf(A, "GitHub");
  await edit(A, id, { top: { title: "GitHub (work)" }, fields: { password: "new-pass-after-edit-1" } });
  await settled(A, "A");
  await pullNow(p);
  await waitText(p, "GitHub (work)", 25000);
  check("B shows A's new title without a reload", true);
  const after = (await ok(B, "vault_status")).syncRevision;
  check("syncRevision went up on B", after > before, `${before} -> ${after}`);
  const view = await ok(B, "item_get", { id });
  check("a plain fast-forward is not flagged as 'changed elsewhere'", !view.changedElsewhere, JSON.stringify(view).slice(0, 200));
});

step("diverged edits: B keeps its version, A's copy lands in the history with a banner", async (p) => {
  const idA = await idOf(A, "Cloudflare"), idB = await idOf(B, "Cloudflare");
  check("both devices know the item by one id", idA && idA === idB, `${idA} / ${idB}`);
  await edit(A, idA, { top: { title: "Cloudflare A-side" } }); await settled(A, "A");
  await edit(B, idB, { top: { title: "Cloudflare B-side" } });        // B has not pulled A's edit yet
  await pullNow(p);
  await p.waitFor(`document.body.innerText.includes('Cloudflare B-side')`, { timeout: 25000 });
  await sleep(2500);                                                   // let the status poll pick the change up
  const view = await ok(B, "item_get", { id: idB });
  check("B's own version stays the current one", view.title === "Cloudflare B-side", view.title);
  check("the item is flagged changedElsewhere", view.changedElsewhere === true, JSON.stringify(view).slice(0, 200));
  const hist = await ok(B, "history_list", { id: idB });
  const other = hist.filter((h) => h.otherDevice);
  check("the history holds exactly one other-device copy", other.length === 1, JSON.stringify(hist).slice(0, 300));
  const copy = await ok(B, "history_get", { id: idB, version: other[0].version, password: PW });
  check("that copy is A's edit, intact", copy.title === "Cloudflare A-side", copy.title);
  await nav(p, "All items"); await p.click("[data-item-row]", "Cloudflare B-side");
  await waitText(p, "Changed on another device", 10000);
  check("the banner is shown on the item", true);
  await p.shot(`${SP}/shots2/real-sync-banner.png`);
  await btn(p, "Show history"); await waitText(p, "From another device", 10000);
  check("the history marks the other device's copy", true);
  await p.shot(`${SP}/shots2/real-sync-history.png`);
  await btn(p, "Got it"); await waitGone(p, "Changed on another device", 10000);
  const after = await ok(B, "item_get", { id: idB });
  check("'Got it' clears the flag and keeps the copy", after.changedElsewhere === false && (await ok(B, "history_list", { id: idB })).some((h) => h.otherDevice));
  const sent = await ok(A, "item_get", { id: idA });
  check("A still shows its own edit (nothing was overwritten on A)", sent.title === "Cloudflare A-side", sent.title);
});

step("restoring the other device's copy makes a head both devices accept", async () => {
  const idB = await idOf(B, "Cloudflare B-side");
  const other = (await ok(B, "history_list", { id: idB })).find((h) => h.otherDevice);
  await ok(B, "history_restore", { id: idB, version: other.version, password: PW });
  await settled(B, "B");
  await ok(A, "backup_run");
  const t0 = Date.now(); let title = "";
  while (Date.now() - t0 < 20000) { title = (await ok(A, "item_get", { id: idB })).title; if (title === "Cloudflare A-side") break; await sleep(300); }
  check("A reads the merged head without a conflict", title === "Cloudflare A-side", title);
  const a = await ok(A, "item_get", { id: idB });
  check("the merged head is not flagged on A", !a.changedElsewhere, JSON.stringify(a).slice(0, 200));
  const [titleB] = [(await ok(B, "item_get", { id: idB })).title];
  check("B and A now agree", titleB === title, `${titleB} / ${title}`);
});

step("deletion on A reaches B", async (p) => {
  const id = await idOf(A, "GitHub (work)");
  await ok(A, "item_trash", { id }); await ok(A, "item_purge", { id, password: PW, confirmation: "CONFIRM" });
  await settled(A, "A");
  await pullNow(p);
  await waitGone(p, "GitHub (work)", 25000);
  check("the item disappears on B without a reload", true);
  const r = await B.ipc("item_get", { id });
  check("B answers not_found for it", r.err?.code === "not_found", JSON.stringify(r).slice(0, 200));
});

// ---- the password generator's last settings: kept in the vault, shown in every place, synced between devices ----
const genLength = (p) => p.eval(`document.querySelector('#gen-length')?.value ?? null`);
const genSwitch = (p, label) => p.eval(`document.querySelector('[role=switch][aria-label=${JSON.stringify(label)}]')?.getAttribute('aria-checked') ?? null`);
const setGenLength = (p, n) => p.eval(`(()=>{const el=document.querySelector('#gen-length'); if(!el) return false; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(el, ${n}); el.dispatchEvent(new Event('input',{bubbles:true})); return true;})()`);
const savedGen = async (core) => (await ok(core, "prefs_get")).generator;
const prefsObjects = () => walkFiles(S3_DATA).filter((f) => keyOf(f).endsWith("vault/meta/prefs.enc"));
async function waitGen(core, want, label, ms = 20000) {
  const t0 = Date.now(); let g = null;
  while (Date.now() - t0 < ms) { g = await savedGen(core); if (g && Object.entries(want).every(([k, v]) => g[k] === v)) return true; await sleep(250); }
  check(label, false, JSON.stringify(g)); return false;
}

step("generator: nothing is saved at first, the page offers 20, and a change is kept in the vault", async (p) => {
  check("the vault holds no generator settings yet", (await savedGen(B)) === null);
  check("nothing was sent for them either", prefsObjects().length === 0);
  await nav(p, "Generator"); await p.waitFor(`!!document.querySelector('#gen-length')`, { timeout: 10000, label: "the generator" });
  check("the length starts at 20", (await genLength(p)) === "20", await genLength(p));
  await setGenLength(p, 30);
  await p.click('[role=switch][aria-label="Symbols"]');
  await waitGen(B, { length: 30, symbols: false, lower: true, upper: true, digits: true, avoidAmbiguous: true }, "the core holds 30 characters without symbols a moment after the change");
  check("the slider shows 30", (await genLength(p)) === "30", await genLength(p));
});

step("generator: the item editor opens with the same settings, and so does the next unlock", async (p) => {
  await nav(p, "All items"); await btn(p, "New");
  await waitText(p, "What would you like to store?"); await p.click("[role=dialog] button", "Login");
  await p.waitFor(`!!document.querySelector('[role=dialog] input')`);
  await p.click("[role=dialog] button[aria-expanded]", "Generate");
  await p.waitFor(`!!document.querySelector('#gen-length')`, { timeout: 5000, label: "the editor's generator" });
  check("the editor's slider starts at 30, not 20", (await genLength(p)) === "30", await genLength(p));
  check("and symbols are still off", (await genSwitch(p, "Symbols")) === "false" && (await genSwitch(p, "Lowercase letters")) === "true");
  const sample = await p.eval(`document.querySelector('[role=dialog] [aria-live=polite]')?.innerText ?? ''`);
  check("the sample password is 30 characters with no symbol", sample.replace(/\s/g, "").length === 30 && /^[A-Za-z0-9]+$/.test(sample.replace(/\s/g, "")), JSON.stringify(sample));
  await p.press("Escape"); await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 8000, label: "editor closed" }).catch(() => undefined);

  await ok(B, "vault_lock");
  await waitText(p, "Master password", 20000);
  await unlockUi(p, PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 20000 });
  await quiet(B, "B");
  await nav(p, "Generator"); await p.waitFor(`!!document.querySelector('#gen-length')`, { timeout: 10000 });
  check("after locking and unlocking the page still shows 30 without symbols", (await genLength(p)) === "30" && (await genSwitch(p, "Symbols")) === "false", `${await genLength(p)} / ${await genSwitch(p, "Symbols")}`);
});

step("generator: settings the core cannot use are refused and change nothing", async () => {
  const base = { length: 30, lower: true, upper: true, digits: true, symbols: false, avoidAmbiguous: true };
  for (const [what, generator] of [["3 characters", { ...base, length: 3 }], ["129 characters", { ...base, length: 129 }], ["no character type", { ...base, lower: false, upper: false, digits: false, symbols: false }]]) {
    const r = await B.ipc("prefs_set_generator", { generator });
    check(`${what}: refused`, r.err?.code === "invalid_input", JSON.stringify(r).slice(0, 200));
  }
  const g = await savedGen(B);
  check("the saved settings are untouched", g?.length === 30 && g.symbols === false, JSON.stringify(g));
});

step("generator: the settings go to the bucket sealed and reach the other device", async () => {
  await settled(B, "B");
  const objs = prefsObjects();
  check("the bucket holds the settings as vault/meta/prefs.enc", objs.length === 1, bucketItemFiles().map(keyOf).join(", "));
  if (objs.length) {
    const raw = readFileSync(objs[0]);
    check("the object is sealed: no setting name or value is readable in it", !["length", "generator", "symbols", "avoidAmbiguous", "lower"].some((w) => raw.includes(Buffer.from(w))), raw.subarray(0, 80).toString("hex"));
  }
  await ok(A, "backup_run");
  check("device A took them over on its next pull", await waitGen(A, { length: 30, symbols: false }, "device A shows 30 characters without symbols"));
});

step("generator: a newer change on the other device reaches this one, even while its page is open", async (p) => {
  await ok(A, "prefs_set_generator", { generator: { length: 45, lower: true, upper: true, digits: true, symbols: true, avoidAmbiguous: false } });
  await settled(A, "A");
  await nav(p, "Generator"); await p.waitFor(`document.querySelector('#gen-length')?.value === '30'`, { timeout: 5000 });
  await ok(B, "backup_run");
  await p.waitFor(`document.querySelector('#gen-length')?.value === '45'`, { timeout: 20000, label: "the open generator page follows the pull" }).catch(() => undefined);
  check("the open page now shows 45", (await genLength(p)) === "45", await genLength(p));
  check("with symbols back on and look-alikes allowed", (await genSwitch(p, "Symbols")) === "true" && (await genSwitch(p, "Avoid look-alike characters (0 O 1 l I)")) === "false");
  const g = await savedGen(B);
  check("device B's vault holds the same", g?.length === 45 && g.symbols === true && g.avoidAmbiguous === false, JSON.stringify(g));
  const hits = (await ok(B, "backup_list"))[0]?.health;
  check("the pull left nothing waiting to send back", hits?.queued === 0, JSON.stringify(hits));
});

step("an object that fails authentication is ignored and counted", async (p) => {
  const idA = await idOf(A, "Cloudflare A-side");
  await edit(A, idA, { top: { title: "Cloudflare TAMPERED" } }); await settled(A, "A");
  const file = bucketItemFiles().find((f) => readFileSync(f).length > 0 && keyOf(f).includes(idA));
  check("the item's object is in the bucket", !!file, bucketItemFiles().map(keyOf).join(", "));
  const buf = readFileSync(file); const at = Math.floor(buf.length * 0.6); buf[at] ^= 0xff; writeFileSync(file, buf);
  await pullNow(p);
  await waitText(p, "failed its integrity check", 15000).catch(() => undefined);
  const st = await ok(B, "vault_status");
  check("remoteDamaged counts the bad object", st.remoteDamaged >= 1, JSON.stringify(st));
  const view = await ok(B, "item_get", { id: idA });
  check("B's data for that item is untouched", view.title === "Cloudflare A-side", view.title);
  const t = await text(p);
  check("the person is told, without any secret in the sentence", /failed its integrity check/.test(t) && !/mocksecret|AKIAMOCK|Zk9-plain/.test(t), t.slice(0, 300));
});

step("idle devices make no requests", async () => {
  const a = await mock("/__mock/state"); await sleep(2500); const b = await mock("/__mock/state");
  check("2.5 idle seconds, no S3 traffic from either device", a === b, `${a.slice(0, 120)} / ${b.slice(0, 120)}`);
});

await startS3(); await A.start();
const page = await launch({ width: 1280, height: 900, port: 9743, profile: `${SP}/chrome-e2e-sync-real` });
try { await runSteps(page, steps, only); }
finally { if (page.logs.length) console.log("\nbrowser console:", [...new Set(page.logs)].slice(0, 10)); await page.close(); await A.stop(); await B.stop(); s3.kill(); }
finish("real-sync");
