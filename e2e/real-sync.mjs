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

step("fast-forward: A edits, B pulls on 'Back up now' and the list updates by itself", async (p) => {
  const id = await idOf(A, "GitHub");
  await edit(A, id, { top: { title: "GitHub (work)" }, fields: { password: "new-pass-after-edit-1" } });
  await settled(A, "A");
  const before = (await ok(B, "vault_status")).syncRevision;
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
