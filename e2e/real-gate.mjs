// Stage 1 gate against the REAL core: decoy maths, attempt counter, persistence, Stage 2 separation, idle lock, panic, wipe.
import { Core, SP, check, finish, freshDir, walkFiles, scanFor, launch, sleep, text, has, waitText, waitGone, btn, nav, createAccount, unlockUi, addLogin, runSteps } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const DATA = freshDir("g-data");
const core = new Core({ bin: BIN, data: DATA, port: 1442, args: ["--idle-unit-ms", "400"] });
const MAR = "Marlowe", PW = "correct horse battery staple 7", PIN = "24680";

const equationOf = (p) => p.eval(`(()=>{const e=document.querySelector('[aria-label^="What is "]'); return e? e.getAttribute('aria-label').slice(8,-1):null})()`);
const solve = (eq) => { if (!/^[\d+\-*/() ×÷−]+$/.test(eq)) throw new Error("unexpected equation " + eq); return Function(`"use strict";return (${eq.replace(/×/g, "*").replace(/÷/g, "/").replace(/−/g, "-")})`)(); };
const keypad = async (p, digits) => { for (const d of digits) await p.click(`button[aria-label="Number ${d}"]`); };
const submit = (p) => p.click('button[aria-label="Submit"]');
const status = async () => (await core.ipc("gate_status")).ok;
const errCode = (r) => r.err?.code ?? r.err;

const steps = [];
const step = (name, fn) => steps.push({ name, fn });

step("first run, account, one login", async (p) => {
  await p.goto(core.ui); await waitText(p, "Welcome to");
  await createAccount(p, MAR, PW);
  await addLogin(p, [["Title", "GitHub"], ["Username", "marlowe-dev"], ["Password", "Tr0ub4dor&3-horse"]]);
  await waitText(p, "marlowe-dev");
  const g = await status();
  check("the gate is off by default and nothing is refused", g.enabled === false && g.open === true && g.equation === null);
});

step("palindrome guard and length rule in Settings", async (p) => {
  await nav(p, "Settings"); await waitText(p, "App lock");
  await p.click("section[aria-label='App lock'] button", "Set up"); await waitText(p, "Ask before showing the account list");
  await p.typeLabel("Passcode", "12321"); await p.typeLabel("Enter it again", "12321");
  check("a palindrome is rejected while reverse-panic is on", await has(p, "reads the same backwards") || await has(p, "same backwards") || await has(p, "backwards"));
  await p.typeLabel("Passcode", "123"); 
  check("fewer than 4 digits is rejected", await has(p, "at least 4"));
});

step("turn the gate on with a PIN (master password required)", async (p) => {
  await p.typeLabel("Passcode", PIN); await p.typeLabel("Enter it again", PIN);
  await p.typeLabel("Master password of this vault", "wrong password here");
  await btn(p, "Save app lock"); await waitText(p, "not the master password");
  check("a wrong master password cannot change the app lock", (await status()).enabled === false);
  await p.typeLabel("Master password of this vault", PW); await btn(p, "Save app lock");
  await waitText(p, "App lock is on");
  const g = await status();
  check("the gate is now enabled in the core, still open for this run", g.enabled && g.open && g.mode === "pin");
  const hits = scanFor(walkFiles(DATA), [PIN, "08642", "Tr0ub4dor"]);
  check("no plaintext passcode or reversed passcode anywhere on disk", hits.length === 0, hits.join("; "));
});

step("lock the app: the equation shows and the core refuses everything else", async (p) => {
  await p.click('button[aria-label^="Account:"]'); await p.click("[role=dialog] button", "Lock app");
  await p.waitFor(`!!document.querySelector('[aria-label^="What is "]')`, { timeout: 8000 });
  check("the gate screen shows a maths question", true); await p.shot(`${SP}/shots2/real-gate.png`);
  const r = await core.ipc("item_list"); check("item_list is refused with `locked` while the gate is closed", errCode(r) === "locked", JSON.stringify(r));
  const r2 = await core.ipc("profile_list"); check("profile_list is refused too (no account names leak)", errCode(r2) === "locked", JSON.stringify(r2));
  const g = await status(); check("gate_status never carries the answer or the credential", !JSON.stringify(g).includes(PIN) && g.equation !== null && g.attemptsLeft === null);
});

step("maths: 1 to 3 characters never count, 4+ do", async (p) => {
  const eq = await equationOf(p); const ans = solve(eq);
  check("the equation result is 1..999", Number.isInteger(ans) && ans >= 1 && ans <= 999, `${eq} = ${ans}`);
  const wrong = String(ans === 999 ? 998 : ans + 1);
  const r1 = (await core.ipc("gate_submit", { input: wrong })).ok;
  check("a wrong answer of 1 to 3 characters is 'math, incorrect'", r1.kind === "math" && r1.correct === false);
  check("it did not touch the attempt counter", (await status()).attemptsLeft === null);
  const eq2 = (await status()).equation; const r2 = (await core.ipc("gate_submit", { input: String(solve(eq2)) })).ok;
  check("the right answer is 'math, correct' with a streak", r2.kind === "math" && r2.correct === true && r2.streak >= 1);
  await p.goto(core.ui); await p.waitFor(`!!document.querySelector('[aria-label^="What is "]')`, { timeout: 8000 });
  await keypad(p, "13579"); await submit(p);
  await waitText(p, "14 attempts left");
  check("a wrong 5-digit entry counts and the banner says 14 attempts left", (await status()).attemptsLeft === 14);
});

step("the counter survives a force-close (write-ahead, persisted)", async (p) => {
  await core.ipc("gate_submit", { input: "11111" }); // 13 left
  await core.stop(); await core.start();
  const g = await status();
  check("after killing and restarting the core the attempts are still counted", g.attemptsLeft === 13 && g.open === false, JSON.stringify(g));
  await p.goto(core.ui); await p.waitFor(`!!document.querySelector('[aria-label^="What is "]')`, { timeout: 8000 });
});

step("the right passcode opens Stage 1 only; the vault still needs the master password", async (p) => {
  await keypad(p, PIN); await submit(p);
  await waitText(p, "Master password", 8000);
  const g = await status(); check("Stage 1 is open and the counter is reset", g.open === true && g.attemptsLeft === null);
  const r = await core.ipc("item_list"); check("no key was cached: listing items says the vault is locked", errCode(r) === "locked", JSON.stringify(r));
  await unlockUi(p, PW); await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 10000 });
  check("the master password opens the vault", await has(p, "marlowe-dev"));
});

step("idle lock is enforced in the core (scaled timer): vault locks and the gate closes", async (p) => {
  await nav(p, "Settings"); await waitText(p, "Lock an open vault after");
  await p.click("[role=radiogroup] button, [role=radio]", "1 min");
  await sleep(300);
  const before = (await core.ipc("app_settings_get")).ok; check("setting stored", before.lockAfterMinutes === 1);
  await core.ipc("activity_ping"); await sleep(1600);
  const st = (await core.ipc("vault_status")); const g = await status();
  check("after the idle time the vault is locked and the gate is closed again", (st.err ? errCode(st) === "locked" : st.ok.unlocked === false) && g.open === false, JSON.stringify([st, g]));
  await p.waitFor(`!!document.querySelector('[aria-label^="What is "]')`, { timeout: 15000 });
  check("the screen follows the core back to the gate", true);
  await core.ipc("app_settings_set", { input: { lockAfterMinutes: 15 } });
});

step("the reversed passcode wipes silently and ends in a clean first run", async (p) => {
  await keypad(p, "08642"); await submit(p);
  await p.waitFor(`document.body.innerText.includes('Welcome to')`, { timeout: 15000 });
  const t = (await text(p)).toLowerCase();
  check("no wipe notice anywhere on screen", !["erase", "wipe", "panic", "deleted", "removed"].some((w) => t.includes(w)));
  const files = walkFiles(DATA);
  check("no vault, registry or gate file is left", !files.some((f) => /vault\.db|profiles\.json|gate\.json|attachments/.test(f)), files.join(", "));
  const g = await status(); check("the app is back to defaults (gate off, no accounts)", g.enabled === false && ((await core.ipc("profile_list")).ok?.profiles ?? []).length === 0);
});

step("custom panic code, grid mode and the 15th failure", async (p) => {
  await p.goto(core.ui); await waitText(p, "Welcome to");
  await createAccount(p, "Second", PW);
  const cfg = await core.ipc("gate_configure", { input: { enabled: true, mode: "grid", credential: "12589", reversePanic: false, panicCode: "9876", biometric: false, panicButton: true, masterPassword: PW } });
  check("grid mode with a custom panic code can be configured", cfg.ok?.mode === "grid" && cfg.ok?.panicButton === true, JSON.stringify(cfg));
  const bad = await core.ipc("gate_configure", { input: { enabled: true, mode: "pin", credential: "12a45", reversePanic: false, panicCode: null, biometric: false, panicButton: false, masterPassword: PW } });
  check("a PIN with letters is refused by the core", errCode(bad) === "invalid_input", JSON.stringify(bad));
  await core.ipc("gate_close");
  let left = 15;
  for (let i = 1; i <= 14; i++) { const r = (await core.ipc("gate_submit", { input: "1346" })).ok; left = r.attemptsLeft; }
  check("14 wrong attempts leave 1", left === 1);
  const r15 = (await core.ipc("gate_submit", { input: "1346" })).ok;
  check("the 15th wrong attempt wipes", r15.kind === "wiped", JSON.stringify(r15));
  check("and the data directory holds no vault afterwards", !walkFiles(DATA).some((f) => /vault\.db|profiles\.json|gate\.json/.test(f)));
});

step("custom panic code wipes", async (p) => {
  await p.goto(core.ui); await waitText(p, "Welcome to");
  await createAccount(p, "Third", PW);
  await core.ipc("gate_configure", { input: { enabled: true, mode: "pin", credential: PIN, reversePanic: true, panicCode: "9999", biometric: false, panicButton: true, masterPassword: PW } });
  await core.ipc("gate_close");
  const ok = (await core.ipc("gate_panic")).ok; check("the discreet panic mark wipes when it is on", ok.kind === "wiped");
  await p.goto(core.ui); await waitText(p, "Welcome to");
  await createAccount(p, "Fourth", PW);
  await core.ipc("gate_configure", { input: { enabled: true, mode: "pin", credential: PIN, reversePanic: true, panicCode: "9999", biometric: false, panicButton: false, masterPassword: PW } });
  await core.ipc("gate_close");
  const off = await core.ipc("gate_panic"); check("with the mark switched off gate_panic is refused", errCode(off) === "invalid_input", JSON.stringify(off));
  const r = (await core.ipc("gate_submit", { input: "9999" })).ok; check("the custom panic code wipes", r.kind === "wiped", JSON.stringify(r));
});

step("manual reset needs the explicit 3 steps", async () => {
  const bad = await core.ipc("app_reset", { confirmedSteps: 2 }); check("two confirmations are not enough", errCode(bad) === "invalid_input", JSON.stringify(bad));
  const ok = await core.ipc("app_reset", { confirmedSteps: 3 }); check("three are", ok.err === undefined, JSON.stringify(ok));
});

await core.start();
const page = await launch({ width: 1280, height: 900, port: 9741, profile: `${SP}/chrome-e2e-gate-real` });
try { await runSteps(page, steps, only); }
finally { if (page.logs.length) console.log("\nbrowser console:", [...new Set(page.logs)].slice(0, 10)); await page.close(); await core.stop(); }
finish("real-gate");
