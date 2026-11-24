// Backup UI against the REAL core and the SigV4-verifying S3 mock: add/test, replication, failure and recovery, restore into a fresh account.
import { spawn } from "node:child_process";
import { Core, SP, check, finish, freshDir, walkFiles, scanFor, launch, sleep, text, has, waitText, waitGone, btn, nav, createAccount, unlockUi, addLogin, runSteps } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const ROOT = new URL("..", import.meta.url).pathname.replace(/\/$/, "");
const S3_PORT = 9333, S3_DATA = freshDir("b-s3"), DATA = freshDir("b-data"), DATA2 = freshDir("b-data2");
const core = new Core({ bin: BIN, data: DATA, port: 1443, args: ["--fast-backup"] });
const core2 = new Core({ bin: BIN, data: DATA2, port: 1444, args: ["--fast-backup"] });
const MAR = "Marlowe", PW = "correct horse battery staple 7";
const legacy = process.env.LEGACY === "1" ? "&legacy=1" : "";
const mock = async (path, body) => (await fetch(`http://127.0.0.1:${S3_PORT}${path}`, body ? { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) } : {})).text();
const bucketKeys = () => walkFiles(S3_DATA).map((f) => { const n = f.slice(S3_DATA.length + 1); return /^[0-9a-f]+$/.test(n) ? Buffer.from(n, "hex").toString("utf8") : n; });

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
const targetText = (p) => p.eval(`document.querySelector('ul[aria-label="Backup targets"]')?.innerText ?? ''`);

const steps = []; const step = (name, fn) => steps.push({ name, fn });

step("account with two logins", async (p) => {
  await p.goto(core.ui + legacy); await waitText(p, "Welcome to");
  await createAccount(p, MAR, PW);
  await addLogin(p, [["Title", "GitHub"], ["Username", "marlowe-dev"], ["Password", "Tr0ub4dor&3-horse"]]);
  await addLogin(p, [["Title", "Cloudflare"], ["Username", "ops@marlowe.example"], ["Password", "Zk9-plain-secret-77"]]);
  await waitText(p, "Cloudflare");
});

step("add a backup target: test, then save as the master", async (p) => {
  await nav(p, "Backup"); await waitText(p, "Add your first backup");
  await btn(p, "Add your first backup"); await waitText(p, "Add a backup");
  await fillTarget(p, { name: "Home bucket" });
  await btn(p, "Test connection"); await waitText(p, "Connected.", 15000).catch(async () => { check("test reports success", false, (await text(p)).slice(-400)); });
  check("the connection test succeeds against the mock and reports strong conditional writes", await has(p, "enforces safe (conditional) writes"));
  await sleep(500); await p.shot(`${SP}/shots2/real-backup-form.png`);
  await p.click("[role=dialog] button", "Add backup"); await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 15000 });
  await waitText(p, "Home bucket");
  await p.waitFor(`(document.querySelector('ul[aria-label="Backup targets"]')?.innerText??'').includes('Up to date')`, { timeout: 20000 });
  check("the target becomes 'Up to date' after the first upload", true);
  const keys = bucketKeys();
  check("the bucket holds the key bundle and both items", keys.some((k) => k.includes("vault/meta/keybundle.json")) && keys.filter((k) => /vault\/items\/.+\.enc$/.test(k)).length === 2, keys.join(", "));
});

step("the bucket holds ciphertext only", async () => {
  const hits = scanFor(walkFiles(S3_DATA), ["Tr0ub4dor", "Zk9-plain", "marlowe-dev", "ops@marlowe", "GitHub", "Cloudflare", "mocksecret"]);
  check("no plaintext marker, title or secret key in any stored object", hits.length === 0, hits.join("; "));
  const local = scanFor(walkFiles(DATA), ["mocksecret", "AKIAMOCK", "hexis-test", "127.0.0.1"]);
  check("the S3 credentials, bucket and endpoint are not readable on disk (sealed)", local.length === 0, local.join("; "));
});

step("a failing bucket shows an honest state, then recovers on the next trigger", async (p) => {
  await mock("/__mock/fail", { status: 403 });
  await nav(p, "All items"); await sleep(600); await addLogin(p, [["Title", "Fastmail"], ["Username", "marlowe@fastmail.example"], ["Password", "fm-pass-1"]]);
  await nav(p, "Backup");
  await p.waitFor(`/Not reaching|Catching up/.test(document.querySelector('ul[aria-label="Backup targets"]')?.innerText??'')`, { timeout: 20000 });
  const t = await targetText(p);
  check("the failing target says so and counts the waiting change", /Not reaching|Catching up/.test(t) && /waiting/.test(t), t);
  const sentence = t.split("\n").find((l) => /refused|HTTP/.test(l)) ?? "";
  check("the error sentence itself names no bucket, key or secret", sentence !== "" && !/hexis-test|mocksecret|AKIAMOCK|127\.0\.0\.1/.test(sentence), sentence);
  await mock("/__mock/fail", { status: null });
  await btn(p, "Back up now");
  await p.waitFor(`(document.querySelector('ul[aria-label="Backup targets"]')?.innerText??'').includes('Up to date')`, { timeout: 20000 });
  check("after the fault is gone, Back up now (a manual trigger) brings it back to 'Up to date'", true);
  check("the new item reached the bucket", bucketKeys().filter((k) => /vault\/items\/.+\.enc$/.test(k)).length === 3);
});

step("no timers: an idle worker makes no requests", async () => {
  const before = JSON.parse(await mock("/__mock/state")); await sleep(2500);
  const after = JSON.parse(await mock("/__mock/state"));
  check("two and a half idle seconds produce no S3 traffic", JSON.stringify(before.requests ?? before) === JSON.stringify(after.requests ?? after), JSON.stringify([before, after]).slice(0, 300));
});

step("restore into a fresh device: items come back, the backup target reappears", async (p) => {
  await core.stop(); await core2.start();
  await p.goto(core2.ui + legacy); await waitText(p, "Welcome to");
  await btn(p, "Restore from a backup"); await waitText(p, "Restore from a backup");
  await p.typeLabel("Account name", "Restored");
  await fillTarget(p, { name: "" });
  await btn(p, "Restore vault");
  await waitText(p, "Master password", 30000);
  await unlockUi(p, PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 20000 });
  await waitText(p, "GitHub"); await waitText(p, "Cloudflare"); await waitText(p, "Fastmail");
  check("all three items are back after the restore and the master password", true);
  await nav(p, "Backup"); await waitText(p, "Home bucket", 20000);
  check("the shared backup target is back and usable", true);
});

await startS3(); await core.start();
const page = await launch({ width: 1280, height: 900, port: 9742, profile: `${SP}/chrome-e2e-backup-real` });
try { await runSteps(page, steps, only); }
finally { if (page.logs.length) console.log("\nbrowser console:", [...new Set(page.logs)].slice(0, 10)); await page.close(); await core.stop(); await core2.stop(); s3.kill(); }
finish("real-backup");
