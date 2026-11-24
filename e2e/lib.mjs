// Shared helpers for the real-core end-to-end scripts: headless Chromium -> Vite dev UI (:1431) -> HTTP shim -> the REAL Rust core.
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, readdirSync, rmSync, statSync, existsSync } from "node:fs";
import { join } from "node:path";
import { launch, sleep } from "./cdp.mjs";
export { launch, sleep };

export const SP = process.env.SP;
export const UI_PORT = 1431;
const results = [];
export function check(name, cond, detail = "") {
  results.push({ name, ok: Boolean(cond), detail });
  console.log(`${cond ? "PASS" : "FAIL"} ${name}${cond ? "" : `   -- ${String(detail).slice(0, 400)}`}`);
  return Boolean(cond);
}
export const finish = (label) => {
  const bad = results.filter((r) => !r.ok);
  console.log(`\n[${label}] ${results.length - bad.length}/${results.length} checks passed${bad.length ? `; FAILED: ${bad.map((b) => b.name).join(" | ")}` : ""}`);
  process.exit(bad.length ? 1 : 0);
};

// ---- server control ----
export class Core {
  constructor({ bin, data, port, args = [] }) { Object.assign(this, { bin, data, port, args }); this.proc = null; this.err = ""; }
  get ui() { return `http://localhost:${UI_PORT}/?e2e=${this.port}${process.env.LEGACY ? `&legacy=${process.env.LEGACY}` : ""}`; }
  async start() {
    this.proc = spawn(this.bin, ["--data-dir", this.data, "--port", String(this.port), "--fast-kdf", "--no-time-check", ...this.args], { stdio: ["ignore", "ignore", "pipe"] });
    this.proc.stderr.on("data", (d) => (this.err += d));
    for (let i = 0; i < 150; i++) {
      try { if ((await fetch(`http://127.0.0.1:${this.port}/health`)).ok) return; } catch { /* not yet */ }
      await sleep(100);
    }
    throw new Error(`e2e server did not start: ${this.err.slice(0, 400)}`);
  }
  async stop() { if (this.proc) { this.proc.kill(); await sleep(400); this.proc = null; } }
  async ipc(cmd, args = {}) {
    const r = await fetch(`http://127.0.0.1:${this.port}/ipc`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ cmd, args }) });
    if (r.headers.get("x-hexis-binary") === "1") return { ok: Buffer.from(await r.arrayBuffer()) };
    return await r.json();
  }
  async post(path, body) { return await (await fetch(`http://127.0.0.1:${this.port}${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })).text(); }
  async get(path) { return await (await fetch(`http://127.0.0.1:${this.port}${path}`)).text(); }
}
export function freshDir(name) { const d = `${SP}/${name}`; rmSync(d, { recursive: true, force: true }); mkdirSync(d, { recursive: true }); return d; }
export function walkFiles(dir) {
  const out = [];
  const walk = (d) => { if (!existsSync(d)) return; for (const e of readdirSync(d, { withFileTypes: true })) { const f = join(d, e.name); e.isDirectory() ? walk(f) : out.push(f); } };
  walk(dir); return out;
}
export const modeOf = (f) => (statSync(f).mode & 0o777).toString(8);
export function scanFor(files, markers) {
  const hits = [];
  for (const f of files) { const buf = readFileSync(f); for (const m of markers) if (buf.includes(Buffer.from(m)) || buf.includes(Buffer.from(m, "utf16le"))) hits.push(`${f.split("/").slice(-2).join("/")}: ${m}`); }
  return hits;
}

// ---- UI helpers ----
export const text = (p) => p.eval("document.body.innerText");
export const has = async (p, s) => (await text(p)).includes(s);
export const waitText = (p, s, timeout = 10000) => p.waitFor(`document.body.innerText.includes(${JSON.stringify(s)})`, { timeout, label: `text “${s}”` });
export const waitGone = (p, s, timeout = 10000) => p.waitFor(`!document.body.innerText.includes(${JSON.stringify(s)})`, { timeout, label: `text gone “${s}”` });
export const btn = (p, t) => p.click("button,[role=menuitem],[role=tab],[role=option]", t);
export const nav = (p, label) => p.click('nav[aria-label="Main"] button:not([aria-label^="Account:"])', label);
/** Chooses an option of a native <select> by its aria-label and fires the change React listens for. */
export const setSelect = (p, label, value) => p.eval(`(()=>{const el=document.querySelector('select[aria-label=${JSON.stringify(label)}]'); if(!el) return false; Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(el, ${JSON.stringify(String(value))}); el.dispatchEvent(new Event('change',{bubbles:true})); return true;})()`);
export const noDialog = (p, timeout = 12000) => p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout, label: "dialog closed" });
export async function reprompt(p, password) { await waitText(p, "Confirm it is you"); await p.typeLabel("Master password", password); await p.press("Enter"); await waitGone(p, "Confirm it is you"); }
/** Settings, then one of its pages (these scripts run at desktop width, where the pages sit in a column on the left). */
export const settingsPage = async (p, name) => { await nav(p, "Settings"); await p.click("nav[aria-label='Settings categories'] button", name); };
/**
 * A start with no accounts. A brand-new install shows the time zone page, then the demo vault with its welcome sheet; after
 * an erase the zone is remembered, so the demo vault opens at once with no sheet. Either way the demo vault ends up open.
 */
export async function firstStart(p) {
  await p.waitFor(`document.body.innerText.includes('Choose your time zone') || !!document.querySelector('button[aria-label^="Account: Hexis123"]')`, { timeout: 15000, label: "the zone page or the demo vault" });
  if (await has(p, "Choose your time zone")) {
    await btn(p, "Continue");
    await waitText(p, "You are in a demo vault", 20000);
  }
}
/** The shell is showing the demo vault (an erase, or a first start whose sheet has been answered). */
export const waitDemoShell = (p, timeout = 15000) => p.waitFor(`!!document.querySelector('nav[aria-label="Main"]') && !!document.querySelector('button[aria-label^="Account: Hexis123"]')`, { timeout, label: "the demo vault" });
/**
 * Takes the demo vault away through the core (a start or an erase leaves one behind), then reloads so the page lists only the
 * accounts a script makes. Done over IPC and not the Settings screen because some scripts run the core with a very short idle lock.
 */
export async function removeDemo(p) {
  const port = await p.eval(`new URLSearchParams(location.search).get('e2e')`);
  const call = async (cmd, args = {}) => (await fetch(`http://127.0.0.1:${port}/ipc`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ cmd, args }) })).json();
  const demo = ((await call("profile_list")).ok?.profiles ?? []).find((x) => x.demo);
  if (!demo) return;
  const r = await call("profile_remove", { id: demo.id, password: "Hexis123", confirmation: "CONFIRM" });
  if (r.err) throw new Error(`the demo vault would not go: ${JSON.stringify(r.err)}`);
  await p.goto(await p.eval(`location.href`));
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 8000, label: "app shell after removing the demo" });
}
export async function createAccount(p, name, password, { skipBackup = true } = {}) {
  if (await has(p, "You are in a demo vault")) await btn(p, "Create my own vault");
  else {
    if (await p.eval(`!!document.querySelector('nav[aria-label="Main"]')`)) { await p.click('button[aria-label^="Account:"]'); await btn(p, "Add account"); }
    else if (await has(p, "Choose an account")) await btn(p, "Add account");
    await btn(p, "Create a new vault");
  }
  await p.typeLabel("Account name", name);
  await p.typeLabel("Master password", password);
  await p.typeLabel("Confirm master password", password);
  await p.click("input[type=checkbox]");
  await btn(p, "Create vault");
  await waitText(p, "Back up this vault?", 15000);
  if (!skipBackup) return;
  await btn(p, "Skip for now");
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 8000, label: "app shell" });
  await removeDemo(p); // the demo vault from a first start (or an erase) is still listed
}
export async function unlockUi(p, password) { await p.typeLabel("Master password", password); await p.press("Enter"); }
export async function addLogin(p, fields) {
  await btn(p, "New");
  await waitText(p, "What would you like to store?"); await p.click("[role=dialog] button", "Login");
  await p.waitFor(`!!document.querySelector('[role=dialog] input')`);
  for (const [label, t] of fields) await p.typeLabel(label, t);
  await btn(p, "Add item"); await noDialog(p);
}
export const failShot = (p, name) => p.shot(`${SP}/shots2/real-FAIL-${name.replace(/\W+/g, "-")}.png`).catch(() => undefined);
export async function runSteps(page, steps, only) {
  for (const s of steps) {
    if (only && !s.name.includes(only)) continue;
    console.log(`\n--- ${s.name}`);
    try { await s.fn(page); } catch (e) { check(`STEP ERROR: ${s.name}`, false, e.message); await failShot(page, s.name); }
  }
}
