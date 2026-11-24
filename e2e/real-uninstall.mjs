// "Remove Hexis from this device" against the REAL core: the theme and the clipboard time live in the core's device file (and
// nothing in the web view), removal is refused while the app lock is closed, a wrong sum removes nothing, and the right one
// leaves the data folder with no file at all (the web view's leftovers from an older build included), nothing is made again
// afterwards, and the screen ends on its last page. Also: light, dark and "device setting" really change what is drawn.
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { Core, SP, check, finish, freshDir, walkFiles, launch, sleep, has, waitText, btn, createAccount, firstStart, settingsPage, unlockUi, runSteps } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const DATA = freshDir("u-data");
const core = new Core({ bin: BIN, data: DATA, port: 1450 });
const MAR = "Marlowe", PW = "correct horse battery staple 7", PIN = "24680";
const LIGHT = "rgb(231, 238, 240)", DARK = "rgb(8, 20, 26)";

const bg = (p) => p.eval(`getComputedStyle(document.body).backgroundColor`);
const themeAttr = (p) => p.eval(`document.documentElement.dataset.theme ?? 'none'`);
const deviceFile = () => JSON.parse(readFileSync(`${DATA}/device.json`, "utf8"));
// the shim's own marker file (it refuses to run in a folder without it) is the one thing the core must leave: it is not Hexis's
const left = () => (existsSync(DATA) ? walkFiles(DATA).filter((f) => !f.endsWith("/.hexis-e2e")) : []);
const empty = () => left().length === 0;
const solve = (eq) => { if (!/^[\d+\-*/() ×÷−]+$/.test(eq)) throw new Error("unexpected equation " + eq); return Function(`"use strict";return (${eq.replace(/×/g, "*").replace(/÷/g, "/").replace(/−/g, "-")})`)(); };
const sumOf = (p) => p.eval(`(()=>{const e=document.querySelector('[role=dialog] [aria-label^="What is "]'); return e? e.getAttribute('aria-label').slice(8,-1):null})()`);
const errCode = (r) => r.err?.code ?? r.err;

const steps = [];
const step = (name, fn) => steps.push({ name, fn });

step("first run and an account", async (p) => {
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, MAR, PW);
  check("a new install starts on the device's own colours", (await core.ipc("device_get")).ok.theme === "system");
});

step("the theme is chosen in the app and kept by the core", async (p) => {
  await settingsPage(p, "Theme"); await waitText(p, "Pale panes and dark ink");
  await p.click("button[role=radio]", "Dark"); await p.waitFor(`document.documentElement.dataset.theme === 'dark'`, { timeout: 5000 });
  check("Dark paints the dark palette", (await bg(p)) === DARK, await bg(p));
  await p.click("button[role=radio]", "Pale panes"); await p.waitFor(`document.documentElement.dataset.theme === 'light'`, { timeout: 5000 });
  check("Light paints the light palette", (await bg(p)) === LIGHT, await bg(p));
  check("the core has it: device.json says light", deviceFile().theme === "light" && (await core.ipc("device_get")).ok.theme === "light");
  await settingsPage(p, "Security"); await waitText(p, "Clear copied secrets after");
  await p.click("button[role=radio]", "60 s"); await sleep(400);
  check("the clipboard time is in device.json too", deviceFile().clipboardClear === 60 && (await core.ipc("device_get")).ok.clipboardClear === 60, JSON.stringify(deviceFile()));
  check("the web view stored nothing: no local storage, no cookie", await p.eval(`localStorage.length === 0 && sessionStorage.length === 0 && document.cookie === ''`));
});

step("the choice is there at the next start, before any vault opens", async (p) => {
  await core.stop(); await core.start();
  await p.goto(core.ui);
  await p.waitFor(`document.documentElement.dataset.theme === 'light'`, { timeout: 15000, label: "the saved theme on the lock screen" });
  check("the lock screen is light after a restart of the core", (await bg(p)) === LIGHT);
  await unlockUi(p, PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 15000, label: "the shell" });
  check("the clipboard time survived the restart as well", (await core.ipc("device_get")).ok.clipboardClear === 60);
});

step("removal is refused while the app lock is closed", async (p) => {
  const set = await core.ipc("gate_configure", { input: { enabled: true, mode: "pin", credential: PIN, reversePanic: true, panicCode: null, biometric: false, panicButton: true, masterPassword: PW } });
  check("the app lock is on", set.ok?.enabled === true, JSON.stringify(set));
  await core.ipc("gate_close");
  await core.ipc("app_reset_challenge");
  const r = await core.ipc("app_prepare_uninstall", { confirmedSteps: 3, answer: "1" });
  check("app_prepare_uninstall is refused with `locked` and removes nothing", errCode(r) === "locked" && existsSync(`${DATA}/device.json`), JSON.stringify(r));
  const open = await core.ipc("gate_submit", { input: PIN });
  check("the app lock opens again with the passcode", open.ok?.kind === "opened", JSON.stringify(open));
});

step("a wrong sum removes nothing", async (p) => {
  // what an older build's web view left in the data folder
  for (const [f, body] of [["localstorage/http_localhost_1420.localstorage", "x"], ["WebKitCache/Version 17/Records/aa/bb", "cached"], ["CacheStorage/salt", "s"], ["mediakeys/v1/salt", "s"], ["hsts-storage.sqlite", "db"]]) {
    const full = `${DATA}/${f}`; mkdirSync(full.slice(0, full.lastIndexOf("/")), { recursive: true }); writeFileSync(full, body);
  }
  const before = walkFiles(DATA).length;
  await p.goto(core.ui);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]') || document.body.innerText.includes('Master password')`, { timeout: 15000 });
  if (!(await p.eval(`!!document.querySelector('nav[aria-label="Main"]')`))) await unlockUi(p, PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 15000 });
  await settingsPage(p, "Security"); await waitText(p, "Remove everything Hexis keeps here");
  await btn(p, "Remove…");
  await waitText(p, "Remove Hexis from this device?");
  check("the first step says what goes, before anything is done", await has(p, "cache or cookies"));
  for (const label of ["Continue", "I understand, continue"]) { await p.click("[role=dialog] button", label); await sleep(450); }
  await waitText(p, "Remove everything now?");
  await p.waitFor(`!!document.querySelector('[role=dialog] [aria-label^="What is "]')`, { timeout: 6000 });
  const s1 = await sumOf(p);
  await p.typeLabel("Your answer", String(solve(s1) + 3)); await p.click("[role=dialog] button", "Remove everything");
  await waitText(p, "That is not the answer. Nothing was removed.", 8000);
  check("a wrong answer removes nothing and a new sum comes", walkFiles(DATA).length >= before && existsSync(`${DATA}/device.json`) && existsSync(`${DATA}/localstorage/http_localhost_1420.localstorage`), `${walkFiles(DATA).length} files, was ${before}`);
});

step("the right sum removes everything, and nothing is made again", async (p) => {
  await sleep(500);
  const s2 = await sumOf(p);
  await p.typeLabel("Your answer", String(solve(s2))); await p.click("[role=dialog] button", "Remove everything");
  await waitText(p, "Everything is removed", 15000);
  await p.shot(`${SP}/shots2/real-removed.png`);
  check("the last screen says what to do next", await has(p, "uninstall it") || await has(p, "Uninstall Hexis"));
  check("no file is left in the data folder: vaults, gate, device settings, web view leftovers", empty(), left().join(", "));
  await sleep(3000);
  check("and nothing came back (no empty vault, no settings file)", empty(), left().join(", "));
  check("the page itself kept nothing", await p.eval(`localStorage.length === 0 && sessionStorage.length === 0 && document.cookie === ''`));
  check("the core answers as an empty app would", ((await core.ipc("profile_list")).ok?.profiles ?? []).length === 0);
});

await core.start();
const page = await launch({ width: 1280, height: 900, port: 9750, profile: `${SP}/chrome-e2e-uninstall-real` });
try { await runSteps(page, steps, only); }
finally { if (page.logs.length) console.log("\nbrowser console:", [...new Set(page.logs)].slice(0, 10)); await page.close(); await core.stop(); }
finish("real-uninstall");
