// Passkeys in Firefox on this Linux computer: a headless Firefox (its own throw-away profile and HOME), the real Hexis extension
// installed as a TEMPORARY add-on over Marionette (a release Firefox runs an unsigned add-on only that way), the native host the
// core registered under `~/.mozilla/native-messaging-hosts`, the private socket, the REAL core and the sheet in the Hexis UI.
// The same test page as real-bridge.mjs asks for a passkey and checks the answer with WebCrypto.
//
// Skipped (and says so) when there is no `firefox` to run. Never touches the real Firefox profile: HOME and the profile are
// folders below the stand-in home of the shim.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { connect } from "node:net";
import { startSite, rogueOn, request } from "./bridge-common.mjs";
import { Core, SP, check, finish, freshDir, launch, sleep, waitText, createAccount, firstStart } from "./lib.mjs";

const BIN = process.env.E2E_BIN;
const FIREFOX = process.env.FIREFOX_BIN || "firefox";
if (spawnSync(FIREFOX, ["--version"], { stdio: "ignore" }).status !== 0) { console.log(`skipped: no runnable ${FIREFOX} (set FIREFOX_BIN)`); process.exit(0); }

const ROOT = freshDir("bridge-firefox");
const DATA = `${ROOT}/data`;
const HOME = `${ROOT}/home`;
const RUN = `${HOME}/run`;
const SOCKET = `${RUN}/hexis/bridge.sock`;
const PROFILE = `${HOME}/ffprofile`;
const PAGE_PORT = 1482;
const MARIONETTE_PORT = 2831;
const SITE = `http://localhost:${PAGE_PORT}`;
const PW = "correct horse battery staple 8";
const core = new Core({ bin: BIN, data: DATA, port: 1454, args: ["--bridge-home", HOME] });
const ok = async (cmd, args = {}) => { const r = await core.ipc(cmd, args); if (r.err) throw new Error(`${cmd}: ${r.err.code} ${r.err.message}`); return r.ok; };
const rogue = rogueOn(SOCKET);

// ---- a Marionette client: `<length>:<json>` packets over TCP, [0, id, command, params] -> [1, id, error, result] ---------------
class Marionette {
  constructor() { this.id = 0; this.pending = new Map(); this.buf = Buffer.alloc(0); }
  async connect() {
    for (let i = 0; i < 200; i++) {
      try { await new Promise((res, rej) => { this.sock = connect(MARIONETTE_PORT, "127.0.0.1", res); this.sock.once("error", rej); }); break; } catch { await sleep(150); }
      if (i === 199) throw new Error("Firefox's Marionette port never opened");
    }
    this.sock.on("data", (d) => this.feed(d));
    await new Promise((res) => { this.hello = res; });
  }
  feed(d) {
    this.buf = Buffer.concat([this.buf, d]);
    for (;;) {
      const colon = this.buf.indexOf(":");
      if (colon < 0) return;
      const n = Number(this.buf.subarray(0, colon).toString());
      if (this.buf.length < colon + 1 + n) return;
      const msg = JSON.parse(this.buf.subarray(colon + 1, colon + 1 + n).toString());
      this.buf = this.buf.subarray(colon + 1 + n);
      if (!Array.isArray(msg)) { this.hello?.(msg); continue; }
      const waiting = this.pending.get(msg[1]); this.pending.delete(msg[1]);
      if (waiting) msg[2] ? waiting.rej(new Error(`${msg[2].error}: ${msg[2].message}`)) : waiting.res(msg[3]);
    }
  }
  send(command, params = {}) {
    const id = ++this.id; const body = JSON.stringify([0, id, command, params]);
    this.sock.write(`${Buffer.byteLength(body)}:${body}`);
    return new Promise((res, rej) => this.pending.set(id, { res, rej }));
  }
  async run(script, args = []) { return (await this.send("WebDriver:ExecuteScript", { script, args })).value; }
}

let ui; let fox; let site; let ff; const mn = new Marionette();
const dialog = () => ui.eval(`document.querySelector('[role=dialog]')?.innerText ?? null`);
const sheetOpen = async (title) => { await ui.waitFor(`(document.querySelector('[role=dialog]')?.innerText ?? '').includes(${JSON.stringify(title)}) && !document.querySelector('[role=dialog]').innerText.includes('Looking in your vault')`, { timeout: 20000, label: `the sheet “${title}”` }); await sleep(900); };
const sheetClick = (label) => ui.click("[role=dialog] footer button", label);
const noSheet = () => ui.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 15000, label: "the sheet gone" });
const call = (fn, arg = {}) => mn.run(`(window.wrappedJSObject || window).__run(arguments[0], arguments[1]); return true;`, [fn, JSON.stringify(arg)]);
const state = () => mn.run(`return document.body.dataset.state || null;`);
const result = async (timeout = 30000) => {
  const t0 = Date.now();
  while ((await state()) !== "done") { if (Date.now() - t0 > timeout) throw new Error("the page's call never finished"); await sleep(150); }
  return JSON.parse(await mn.run(`return document.body.dataset.result;`));
};
const logins = async () => (await ok("item_list")).filter((i) => i.type === "login");
const passkeysOf = async (id) => (await ok("item_get", { id })).fields.filter((f) => f.passkey).map((f) => f.passkey);

const steps = []; const step = (name, fn) => steps.push({ name, fn });

step("the core listens, Firefox is told where Hexis is", async () => {
  await core.start();
  ui = await launch({ width: 1200, height: 1000, port: 9672, profile: `${SP}/chrome-e2e-bridge-firefox-ui` });
  await ui.goto(core.ui); await firstStart(ui);
  await createAccount(ui, "Marlowe", PW);
  mkdirSync(`${HOME}/.mozilla/firefox`, { recursive: true });
  await ok("bridge_set", { on: true });
  await ui.goto(core.ui); // the window learns the setting from the core when it loads
  await ui.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 15000, label: "the vault list" });
  const status = await ok("bridge_register", { ids: ["firefox"] });
  const row = status.browsers.find((b) => b.id === "firefox");
  check("the core lists Firefox as connected", row?.found === true && row.registered === true && row.stale === false, JSON.stringify(row));
  const manifest = JSON.parse(readFileSync(`${HOME}/.mozilla/native-messaging-hosts/dev.habibullah.hexis.passkeys.json`, "utf8"));
  check("only the add-on with Hexis's id may start the host", manifest.allowed_extensions?.[0] === "hexis-passkeys@habibullah.dev" && manifest.type === "stdio" && existsSync(manifest.path), JSON.stringify(manifest));
});

step("Firefox starts with the extension as a temporary add-on", async () => {
  mkdirSync(PROFILE, { recursive: true });
  writeFileSync(`${PROFILE}/user.js`, [
    ["marionette.port", MARIONETTE_PORT], ["browser.shell.checkDefaultBrowser", false], ["datareporting.policy.dataSubmissionEnabled", false],
    ["app.update.enabled", false], ["extensions.autoDisableScopes", 0], ["browser.startup.page", 0], ["toolkit.telemetry.reportingpolicy.firstRun", false],
    ["browser.aboutwelcome.enabled", false], ["network.proxy.type", 0], ["app.normandy.enabled", false], ["browser.newtabpage.enabled", false],
  ].map(([k, v]) => `user_pref(${JSON.stringify(k)}, ${JSON.stringify(v)});`).join("\n") + "\n");
  site = await startSite(PAGE_PORT);
  ff = spawn(FIREFOX, ["--headless", "--marionette", "--no-remote", "--profile", PROFILE, "about:blank"], { stdio: ["ignore", "ignore", "ignore"], env: { ...process.env, HOME, XDG_RUNTIME_DIR: RUN, MOZ_HEADLESS: "1" } });
  await mn.connect();
  await mn.send("WebDriver:NewSession", { capabilities: { alwaysMatch: {} } });
  const added = await mn.send("Addon:Install", { path: `${DATA}/extension/firefox`, temporary: true });
  check("Firefox accepted the extension and named it with the id the host manifest allows", added.value === "hexis-passkeys@habibullah.dev" || added === "hexis-passkeys@habibullah.dev", JSON.stringify(added));
  await mn.send("WebDriver:Navigate", { url: `${SITE}/` });
  let installed = false;
  for (let i = 0; i < 60 && !installed; i++) { installed = await mn.run(`return (window.wrappedJSObject || window).__hexisPasskeysInstalled === true;`); if (!installed) await sleep(250); }
  check("the extension's script runs in the page's own world", installed === true);
});

step("a website saves a passkey in Firefox: the sheet, then a login with the passkey, and the page checks the answer", async () => {
  const before = (await logins()).length;
  await call("__create", { user: "bob@example.com" });
  await sheetOpen("Save a passkey");
  const t = await dialog();
  check("the sheet names the page and who the passkey is for", t.includes("localhost") && t.includes(`Port ${PAGE_PORT}`) && t.includes("bob@example.com"), t);
  await sheetClick("Save passkey");
  await noSheet();
  const got = await result();
  check("the page got a PublicKeyCredential for its own origin and challenge", got.ok === true && got.isPublicKeyCredential === true && got.origin === SITE && got.challengeOk === true && got.clientType === "webauthn.create", JSON.stringify(got));
  check("with the right website hash, ES256 and a public key", got.rpIdHashOk === true && got.alg === -7 && got.hasPublicKey === true && (got.flags & 0x41) === 0x41, JSON.stringify(got));
  const now = await logins();
  const fresh = now.find((i) => i.title.includes("Local test page"));
  const keys = fresh ? await passkeysOf(fresh.id) : [];
  check("the vault holds that passkey on one new login", now.length === before + 1 && keys.length === 1 && keys[0].credentialId === got.id && keys[0].rpId === "localhost", JSON.stringify(keys));
});

step("signing in in Firefox: the signature verifies, and a website that requires verification gets the master password", async () => {
  await call("__get");
  await sheetOpen("Sign in with a passkey");
  await sheetClick("Sign in");
  await noSheet();
  const got = await result();
  check("the assertion verifies with the key the page was given at creation", got.ok === true && got.known === true && got.verified === true && got.origin === SITE && got.challengeOk === true, JSON.stringify(got));
  await call("__get", { uv: "required" });
  await sheetOpen("Sign in with a passkey");
  check("the sheet asks for the master password", (await dialog()).includes("Master password"));
  await ui.typeLabel("Master password", PW);
  await sheetClick("Sign in");
  await noSheet();
  const verified = await result();
  check("the answer says the user was verified, and it verifies", verified.ok === true && verified.verified === true && (verified.flags & 0x04) === 0x04, JSON.stringify(verified));
});

step("Cancel and another website's name end in Firefox the way the browser's own flow would", async () => {
  await call("__get");
  await sheetOpen("Sign in with a passkey");
  await sheetClick("Cancel");
  await noSheet();
  const refused = await result();
  check("Cancel rejects with NotAllowedError", refused.ok === false && refused.name === "NotAllowedError", JSON.stringify(refused));
  await call("__create", { rpId: "example.com" });
  const wrong = await result();
  check("a page cannot create a passkey for a name that is not its own, and no sheet opened", wrong.ok === false && !(await dialog()), JSON.stringify(wrong));
  const lie = await rogue(request("get", "bank.example", "https://evil.example"));
  check("a program on the socket claiming another website's name is refused at once", lie.includes("fallback") && lie.includes("invalid_request"), lie);
});

step("switching the bridge off closes the socket", async () => {
  await ok("bridge_set", { on: false });
  check("no socket is left", !existsSync(SOCKET));
});

try {
  const only = process.argv[2];
  for (const s of steps) {
    if (only && !s.name.includes(only)) continue;
    console.log(`\n--- ${s.name}`);
    try { await s.fn(); } catch (e) { check(`STEP ERROR: ${s.name}`, false, e.message); await ui?.shot(`${SP}/shots2/real-bridge-firefox-FAIL-ui.png`).catch(() => undefined); }
  }
} finally {
  try { mn.sock?.destroy(); } catch { /* closed */ }
  ff?.kill();
  site?.close(); site?.closeAllConnections?.();
  await ui?.close().catch(() => undefined);
  await core.stop();
  await sleep(500);
  void fox;
}
finish("real-bridge-firefox");
