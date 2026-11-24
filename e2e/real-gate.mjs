// Stage 1 gate against the REAL core: decoy maths, attempt counter, the waits after wrong passcodes, persistence,
// Stage 2 separation, idle lock, lock after leaving the app, panic, wipe, and the long sum before a manual reset.
import { Core, SP, check, finish, freshDir, walkFiles, scanFor, launch, sleep, text, has, waitText, waitGone, btn, nav, createAccount, firstStart, waitDemoShell, settingsPage, unlockUi, addLogin, runSteps, setSelect } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const DATA = freshDir("g-data");
// One "minute" of the inactivity timer lasts 400 ms (the leave timer's "second" is a sixtieth of that). Waits after wrong
// passcodes are off (--lock-unit-ms 0) except in the steps that look at them, which restart the core with their own unit.
const BASE = ["--idle-unit-ms", "400", "--lock-unit-ms", "0"];
const core = new Core({ bin: BIN, data: DATA, port: 1442, args: BASE });
const restartWith = async (args) => { await core.stop(); core.args = args; await core.start(); };
const MAR = "Marlowe", PW = "correct horse battery staple 7", PIN = "24680";

const equationOf = (p) => p.eval(`(()=>{const e=document.querySelector('[aria-label^="What is "]'); return e? e.getAttribute('aria-label').slice(8,-1):null})()`);
const solve = (eq) => { if (!/^[\d+\-*/() ×÷−]+$/.test(eq)) throw new Error("unexpected equation " + eq); return Function(`"use strict";return (${eq.replace(/×/g, "*").replace(/÷/g, "/").replace(/−/g, "-")})`)(); };
const keypad = async (p, digits) => { for (const d of digits) await p.click(`button[aria-label="Number ${d}"]`); };
const submit = (p) => p.click('button[aria-label="Submit"]');
const status = async () => (await core.ipc("gate_status")).ok;
/** After an erase the only vault is a fresh demo (the screen opens one at once). */
const onlyDemo = async () => { const l = (await core.ipc("profile_list")).ok?.profiles ?? []; return l.length === 1 && l[0].demo === true && l[0].displayName === "Hexis123"; };
const errCode = (r) => r.err?.code ?? r.err;

const steps = [];
const step = (name, fn) => steps.push({ name, fn });

step("first run, account, one login", async (p) => {
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, MAR, PW);
  await addLogin(p, [["Title", "GitHub"], ["Username", "marlowe-dev"], ["Password", "Tr0ub4dor&3-horse"]]);
  await waitText(p, "marlowe-dev");
  const g = await status();
  check("the gate is off by default and nothing is refused", g.enabled === false && g.open === true && g.equation === null);
});

step("palindrome guard and length rule in Settings", async (p) => {
  await settingsPage(p, "App lock"); await waitText(p, "Lock an open vault after");
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
  await waitText(p, "15 attempts left");
  check("a wrong 5-digit entry counts and the banner says 15 attempts left", (await status()).attemptsLeft === 15);
});

step("the counter survives a force-close (write-ahead, persisted)", async (p) => {
  await core.ipc("gate_submit", { input: "11111" }); // 14 left
  await core.stop(); await core.start();
  const g = await status();
  check("after killing and restarting the core the attempts are still counted", g.attemptsLeft === 14 && g.open === false, JSON.stringify(g));
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
  await settingsPage(p, "App lock"); await waitText(p, "Lock an open vault after");
  check("the inactivity choices run from 1 minute to 4 hours", (await p.eval(`[...document.querySelector('select[aria-label="Lock after no activity for"]').options].map(o=>o.textContent.trim())`)).join("|") === "1 minute|5 minutes|15 minutes|30 minutes|1 hour|4 hours");
  check("the leave setting is not offered on a desktop", !(await p.eval(`!!document.querySelector('select[aria-label="Lock after leaving the app for"]')`)));
  await setSelect(p, "Lock after no activity for", 1);
  await sleep(300);
  const before = (await core.ipc("app_settings_get")).ok; check("setting stored", before.lockAfterMinutes === 1);
  await core.ipc("activity_ping"); await sleep(1600);
  const st = (await core.ipc("vault_status")); const g = await status();
  check("after the idle time the vault is locked and the gate is closed again", (st.err ? errCode(st) === "locked" : st.ok.unlocked === false) && g.open === false, JSON.stringify([st, g]));
  await p.waitFor(`!!document.querySelector('[aria-label^="What is "]')`, { timeout: 15000 });
  check("the screen follows the core back to the gate", true);
  await core.ipc("app_settings_set", { input: { lockAfterMinutes: 15 } });
});

step("leaving the app: the core's own timer locks the vault and closes the gate; coming back in time does not", async (p) => {
  const open = async () => {
    const r = (await core.ipc("gate_submit", { input: PIN })).ok; if (r?.kind !== "opened") throw new Error("could not open Stage 1: " + JSON.stringify(r));
    const u = await core.ipc("vault_unlock", { password: PW }); if (u.err) throw new Error("could not unlock: " + JSON.stringify(u));
  };
  const state = async () => { const st = await core.ipc("vault_status"); const g = await status(); return { unlocked: st.ok?.unlocked === true, gateOpen: g.open === true }; };
  await open();
  check("the default time away is five minutes", (await core.ipc("app_settings_get")).ok.lockAfterLeaveSeconds === 300);
  const bad = await core.ipc("app_settings_set", { input: { lockAfterMinutes: 15, lockAfterLeaveSeconds: 7 } });
  check("a time away that is not one of the choices is refused", errCode(bad) === "invalid_input", JSON.stringify(bad));
  await core.ipc("app_settings_set", { input: { lockAfterMinutes: 15, lockAfterLeaveSeconds: 60 } }); // 60 s = 400 ms at this scale
  await core.ipc("app_leave"); await sleep(100); await core.ipc("app_return");
  let st = await state(); check("coming back before the time is up leaves the vault open and the gate open", st.unlocked && st.gateOpen, JSON.stringify(st));
  await core.ipc("app_leave"); await sleep(1000);
  st = await state(); check("after the time away the core has locked the vault and closed the gate, with no page involved", !st.unlocked && !st.gateOpen, JSON.stringify(st));
  await core.ipc("app_return");
  st = await state(); check("coming back later reopens nothing", !st.unlocked && !st.gateOpen, JSON.stringify(st));
  await open(); await core.ipc("app_settings_set", { input: { lockAfterMinutes: 15, lockAfterLeaveSeconds: 0 } });
  await core.ipc("app_leave"); await sleep(150);
  st = await state(); check("“Right away” locks the moment the app is left", !st.unlocked && !st.gateOpen, JSON.stringify(st));
  await core.ipc("app_return");
  await open(); await core.ipc("app_settings_set", { input: { lockAfterMinutes: 15, lockAfterLeaveSeconds: 300 } });
  await core.ipc("app_leave"); await sleep(400); await core.ipc("app_return");
  st = await state(); check("leaving counts as no activity but does not lock before the time", st.unlocked && st.gateOpen, JSON.stringify(st));
  await core.ipc("gate_close");
  await p.goto(core.ui); await p.waitFor(`!!document.querySelector('[aria-label^="What is "]')`, { timeout: 8000 });
});

step("the reversed passcode wipes silently and ends in a clean first run", async (p) => {
  await keypad(p, "08642"); await submit(p);
  await waitDemoShell(p);
  const t = (await text(p)).toLowerCase();
  check("no wipe notice anywhere on screen", !["erase", "wipe", "panic", "deleted", "removed"].some((w) => t.includes(w)));
  const files = walkFiles(DATA);
  check("no gate file or attachment is left", !files.some((f) => /gate\.json|attachments/.test(f)), files.join(", "));
  const g = await status();
  const left = (await core.ipc("profile_list")).ok?.profiles ?? [];
  check("the app is back to defaults: gate off, and the only vault is a fresh demo", g.enabled === false && left.length === 1 && left[0].demo === true && left[0].displayName === "Hexis123", JSON.stringify(left));
  check("the earlier account's data is gone", !JSON.stringify(left).includes("Marlowe"));
});

step("custom panic code, grid mode and the 16th failure", async (p) => {
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, "Second", PW);
  const cfg = await core.ipc("gate_configure", { input: { enabled: true, mode: "grid", credential: "12589", reversePanic: false, panicCode: "9876", biometric: false, panicButton: true, masterPassword: PW } });
  check("grid mode with a custom panic code can be configured", cfg.ok?.mode === "grid" && cfg.ok?.panicButton === true, JSON.stringify(cfg));
  const bad = await core.ipc("gate_configure", { input: { enabled: true, mode: "pin", credential: "12a45", reversePanic: false, panicCode: null, biometric: false, panicButton: false, masterPassword: PW } });
  check("a PIN with letters is refused by the core", errCode(bad) === "invalid_input", JSON.stringify(bad));
  await core.ipc("gate_close");
  let left = 16;
  for (let i = 1; i <= 15; i++) { const r = (await core.ipc("gate_submit", { input: "1346" })).ok; left = r.attemptsLeft; }
  check("15 wrong attempts leave 1", left === 1);
  check("and the core says the next one is the last", (await status()).finalAttempt === true);
  const r16 = (await core.ipc("gate_submit", { input: "1346" })).ok;
  check("the 16th wrong attempt wipes", r16.kind === "wiped", JSON.stringify(r16));
  check("and the data directory holds no vault afterwards", !walkFiles(DATA).some((f) => /vault\.db|profiles\.json|gate\.json/.test(f)));
});

step("custom panic code wipes", async (p) => {
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, "Third", PW);
  await core.ipc("gate_configure", { input: { enabled: true, mode: "pin", credential: PIN, reversePanic: true, panicCode: "9999", biometric: false, panicButton: true, masterPassword: PW } });
  await core.ipc("gate_close");
  const ok = (await core.ipc("gate_panic")).ok; check("the discreet panic mark wipes when it is on", ok.kind === "wiped");
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, "Fourth", PW);
  await core.ipc("gate_configure", { input: { enabled: true, mode: "pin", credential: PIN, reversePanic: true, panicCode: "9999", biometric: false, panicButton: false, masterPassword: PW } });
  await core.ipc("gate_close");
  const off = await core.ipc("gate_panic"); check("with the mark switched off gate_panic is refused", errCode(off) === "invalid_input", JSON.stringify(off));
  const r = (await core.ipc("gate_submit", { input: "9999" })).ok; check("the custom panic code wipes", r.kind === "wiped", JSON.stringify(r));
});

step("waits: 3 wrong passcodes lock the passcode entry, nothing is checked meanwhile, and it survives a force-close", async (p) => {
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, "Waiter", PW);
  const cfg = await core.ipc("gate_configure", { input: { enabled: true, mode: "pin", credential: PIN, reversePanic: false, panicCode: null, biometric: false, panicButton: false, masterPassword: PW } });
  check("the gate is switched on", cfg.ok?.enabled === true, JSON.stringify(cfg));
  await core.ipc("gate_close");
  await restartWith(["--idle-unit-ms", "400", "--lock-unit-ms", "4000"]); // 1.5 minutes of waiting last 6 s here
  let r;
  for (let i = 1; i <= 2; i++) { r = (await core.ipc("gate_submit", { input: "1346" })).ok; check(`wrong attempt ${i} starts no wait`, r.kind === "failed" && !r.lockedForMs, JSON.stringify(r)); }
  r = (await core.ipc("gate_submit", { input: "1346" })).ok;
  check("the 3rd wrong attempt starts the 1.5-minute wait (scaled)", r.kind === "failed" && r.attemptsLeft === 13 && r.lockedForMs > 4000 && r.lockedForMs <= 6000, JSON.stringify(r));
  let g = await status();
  check("the status carries the time left, never an end time of the vault", g.lockedForMs > 0 && g.attemptsLeft === 13 && g.finalAttempt === false, JSON.stringify(g));
  const during = (await core.ipc("gate_submit", { input: PIN })).ok;
  check("the right passcode is not even looked at during the wait", during.kind === "locked" && during.retryAfterMs > 0, JSON.stringify(during));
  check("and the refused attempt was not counted", (await status()).attemptsLeft === 13);
  const wrongDuring = (await core.ipc("gate_submit", { input: "98765" })).ok;
  check("a wrong one is refused the same way, not counted", wrongDuring.kind === "locked" && (await status()).attemptsLeft === 13, JSON.stringify(wrongDuring));
  const m = (await core.ipc("gate_submit", { input: "1" })).ok;
  check("the decoy maths still answers during a wait", m.kind === "math", JSON.stringify(m));
  await core.stop(); await core.start();
  g = await status();
  check("force-closing the app does not end the wait", g.lockedForMs > 0 && g.attemptsLeft === 13 && g.open === false, JSON.stringify(g));
  await p.goto(core.ui);
  await waitText(p, "Too many wrong tries", 12000);
  check("the gate screen shows a running countdown", await p.eval(`/\\d{1,2}:\\d{2}/.test([...document.querySelectorAll('[role=status]')].map((e) => e.textContent).join(' '))`));
  check("and the time the wait ends, written in the display zone", await has(p, "Try again at"));
  await p.shot(`${SP}/shots2/real-gate-wait.png`);
  check("it says nothing typed is checked meanwhile", await has(p, "Nothing you type as a passcode is checked until then."));
  await waitGone(p, "Too many wrong tries", 15000);
  check("the countdown ends by itself and the screen follows the core", (await status()).lockedForMs === null);
  await keypad(p, PIN); await submit(p);
  await p.waitFor(`document.body.innerText.includes('Master password')`, { timeout: 8000 });
  g = await status(); check("the right passcode opens Stage 1 once the wait is over", g.open === true && g.attemptsLeft === null && g.lockedForMs === null, JSON.stringify(g));
  await core.ipc("gate_close");
});

step("the last attempt: a warning first, a confirmation next, and a wrong passcode erases everything", async (p) => {
  await restartWith(["--idle-unit-ms", "400", "--lock-unit-ms", "1"]); // the whole ladder in about five seconds
  const wrongOnce = async () => {
    for (let tries = 0; tries < 200; tries++) {
      const r = (await core.ipc("gate_submit", { input: "13579" })).ok;
      if (r.kind !== "locked") return r;
      await sleep(Math.min(r.retryAfterMs + 25, 800));
    }
    throw new Error("a wait never ended");
  };
  let r;
  for (let i = 1; i <= 15; i++) r = await wrongOnce();
  check("15 wrong attempts are counted through every wait", r.kind === "failed" && r.attemptsLeft === 1, JSON.stringify(r));
  let g = await status();
  check("the core flags the last attempt", g.finalAttempt === true && g.attemptsLeft === 1 && g.enabled === true, JSON.stringify(g));
  await p.goto(core.ui);
  await waitText(p, "Last attempt", 10000);
  check("the screen warns that a wrong passcode erases everything", await has(p, "every vault, file and setting on this device is erased for good"));
  await p.shot(`${SP}/shots2/real-gate-last.png`);
  await waitGone(p, "Too many wrong tries", 15000); // the 3-day wait is about four seconds here
  await keypad(p, "13579"); await submit(p);
  await waitText(p, "Send your last attempt?", 6000);
  check("the confirmation shows how many digits were typed", await has(p, "You typed 5 digits"));
  g = await status(); check("nothing was sent to the core yet", g.attemptsLeft === 1 && g.enabled === true, JSON.stringify(g));
  await btn(p, "Go back"); await waitGone(p, "Send your last attempt?");
  check("going back sends nothing and erases nothing", (await status()).attemptsLeft === 1 && walkFiles(DATA).some((f) => /vault\.db/.test(f)));
  await keypad(p, "13579"); await submit(p); await waitText(p, "Send your last attempt?", 6000);
  await btn(p, "Check my passcode");
  await waitDemoShell(p);
  check("a wrong last attempt erases everything and the app is back at first run, in a fresh demo vault", !walkFiles(DATA).some((f) => /gate\.json/.test(f)) && (await status()).enabled === false && await onlyDemo());
});

step("manual reset: three warnings, then a long sum; only the right answer erases", async (p) => {
  await restartWith(BASE);
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, "Fifth", PW);
  await core.ipc("gate_configure", { input: { enabled: true, mode: "pin", credential: PIN, reversePanic: false, panicCode: null, biometric: false, panicButton: false, masterPassword: PW } });
  await core.ipc("gate_close");
  const vaultFiles = () => walkFiles(DATA).filter((f) => /vault\.db|profiles\.json|gate\.json/.test(f)).length;
  const before = vaultFiles(); check("there is something to erase", before >= 3, String(before));
  const bad2 = await core.ipc("app_reset", { confirmedSteps: 2, answer: "1" }); check("two confirmations are not enough", errCode(bad2) === "invalid_input", JSON.stringify(bad2));
  const none = await core.ipc("app_reset", { confirmedSteps: 3, answer: "1" }); check("without a sum having been asked for, nothing erases", errCode(none) === "wrong_answer" && vaultFiles() === before, JSON.stringify(none));
  const sum = (await core.ipc("app_reset_challenge")).ok; const ans = solve(sum);
  check("the sum is long: the answer has 3 to 5 digits", Number.isInteger(ans) && ans >= 100 && ans <= 99999, `${sum} = ${ans}`);
  const wrong = await core.ipc("app_reset", { confirmedSteps: 3, answer: String(ans + 1) });
  check("a wrong answer erases nothing", errCode(wrong) === "wrong_answer" && vaultFiles() === before, JSON.stringify(wrong));
  const stale = await core.ipc("app_reset", { confirmedSteps: 3, answer: String(ans) });
  check("the sum is used up by the wrong answer: even its right answer erases nothing now", errCode(stale) === "wrong_answer" && vaultFiles() === before, JSON.stringify(stale));
  check("the gate is still there", (await status()).enabled === true);
  // through the screen, from the locked gate (no login needed)
  await p.goto(core.ui); await p.waitFor(`!!document.querySelector('[aria-label="Reset app data"]')`, { timeout: 8000 });
  await p.click('button[aria-label="Reset app data"]');
  for (const label of ["Continue", "I understand, continue"]) { await p.click("[role=dialog] button", label); await sleep(450); }
  await waitText(p, "Erase everything now?");
  const sumOf = () => p.eval(`(()=>{const e=document.querySelector('[role=dialog] [aria-label^="What is "]'); return e? e.getAttribute('aria-label').slice(8,-1):null})()`);
  await p.waitFor(`!!document.querySelector('[role=dialog] [aria-label^="What is "]')`, { timeout: 6000 });
  const s1 = await sumOf();
  await p.typeLabel("Your answer", String(solve(s1) + 3)); await p.click("[role=dialog] button", "Erase everything");
  await waitText(p, "That is not the answer. Nothing was erased.", 8000);
  check("the screen: a wrong answer erases nothing and a new sum comes", vaultFiles() === before && (await status()).enabled === true);
  await sleep(500);
  const s2 = await sumOf();
  await p.typeLabel("Your answer", String(solve(s2))); await p.click("[role=dialog] button", "Erase everything");
  await waitDemoShell(p);
  check("the right answer erases everything and the app is back at first run, in a fresh demo vault", !walkFiles(DATA).some((f) => /gate\.json/.test(f)) && (await status()).enabled === false && await onlyDemo(), String(vaultFiles()));
});

await core.start();
const page = await launch({ width: 1280, height: 900, port: 9741, profile: `${SP}/chrome-e2e-gate-real` });
try { await runSteps(page, steps, only); }
finally { if (page.logs.length) console.log("\nbrowser console:", [...new Set(page.logs)].slice(0, 10)); await page.close(); await core.stop(); }
finish("real-gate");
