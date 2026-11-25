// Passkeys in a browser on this Linux computer, the whole chain and nothing pretended: a headless Chromium with the real Hexis
// extension loaded -> the browser starts the native host (the launcher script the core wrote, which starts the shim as
// `--native-host`) -> the private socket -> the REAL core -> the request sheet in the Hexis UI (a second headless browser) ->
// the answer back to a page that calls navigator.credentials.create / .get and checks what it got with WebCrypto.
//
// Everything lives in a throw-away home: the shim's `--bridge-home` is the home folder of the browser (`HOME`) and holds its
// `run/` folder (`XDG_RUNTIME_DIR`), so neither the real Hexis, nor a real browser profile, nor the real socket is ever touched.
// The browser's own passkey flow is a CDP *virtual authenticator*, so "Use another way" and a switched-off bridge can be seen
// handing the page back to the browser.
import { existsSync, mkdirSync, readFileSync, statSync } from "node:fs";
import { startSite, rogueOn, request } from "./bridge-common.mjs";
import { Core, SP, check, finish, freshDir, launch, sleep, text, has, waitText, createAccount, firstStart, modeOf, settingsPage } from "./lib.mjs";

const BIN = process.env.E2E_BIN;
const ROOT = freshDir("bridge-real");
const DATA = `${ROOT}/data`;
const HOME = `${ROOT}/home`;
const RUN = `${HOME}/run`;
const SOCKET = `${RUN}/hexis/bridge.sock`;
const PAGE_PORT = 1481;
const SITE = `http://localhost:${PAGE_PORT}`;
const PW = "correct horse battery staple 8";
const EXTENSION_ID = "pejbejenceojhjhcanlfnibgdihclnhm";
const core = new Core({ bin: BIN, data: DATA, port: 1453, args: ["--bridge-home", HOME] });
const ok = async (cmd, args = {}) => { const r = await core.ipc(cmd, args); if (r.err) throw new Error(`${cmd}: ${r.err.code} ${r.err.message}`); return r.ok; };
const bridgeState = async () => JSON.parse(await core.get("/e2e/bridge")).ok;

// ---- helpers --------------------------------------------------------------------------------------------------------------
let ui; let web; let site; // the Hexis window, the browser that has the extension, and the website
const dialog = () => ui.eval(`document.querySelector('[role=dialog]')?.innerText ?? null`);
const sheetOpen = async (title) => { await ui.waitFor(`(document.querySelector('[role=dialog]')?.innerText ?? '').includes(${JSON.stringify(title)}) && !document.querySelector('[role=dialog]').innerText.includes('Looking in your vault')`, { timeout: 20000, label: `the sheet “${title}”` }); await sleep(900); }; // it ignores the first 700 ms on purpose
const sheetClick = (label) => ui.click("[role=dialog] footer button", label);
const noSheet = () => ui.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 15000, label: "the sheet gone" });
const start = (fn, arg = {}) => web.eval(`window.__settled = false; window.__pending = ${fn}(${JSON.stringify(arg)}); window.__pending.then(() => { window.__settled = true; }); 0`);
const result = () => web.eval(`window.__pending`);
const switchOn = () => ui.eval(`document.querySelector('button[role=switch][aria-label="Use Hexis for passkeys in browsers"]')?.getAttribute('aria-checked')`);
const logins = async () => (await ok("item_list")).filter((i) => i.type === "login");
const passkeysOf = async (id) => (await ok("item_get", { id })).fields.filter((f) => f.passkey).map((f) => f.passkey);
const rogue = rogueOn(SOCKET);

const steps = []; const step = (name, fn, { needsBrowser = false } = {}) => steps.push({ name, fn, needsBrowser });
// Chrome 137 and later (the branded builds, which is what CI has) may ignore --load-extension even with the feature switched off;
// Chromium and Chrome for Testing do not. On CI the chain steps are then skipped, loudly; anywhere else it is a failure.
let noExtension = false;

step("an account, and the Browsers page with the switch off", async () => {
  await core.start();
  ui = await launch({ width: 1200, height: 1000, port: 9670, profile: `${SP}/chrome-e2e-bridge-ui` });
  await ui.goto(core.ui); await firstStart(ui);
  await createAccount(ui, "Marlowe", PW);
  mkdirSync(`${HOME}/.config/chromium`, { recursive: true }); // Chromium has been started once on this computer
  mkdirSync(`${HOME}/.mozilla/firefox`, { recursive: true }); // and Firefox too
  const before = await bridgeState();
  check("the shim started the bridge with a stand-in home, and nothing listens yet", before.listening === false && before.home === HOME, JSON.stringify(before));
  check("no socket file exists while the switch is off", !existsSync(SOCKET));
  await ui.goto(core.ui);
  await settingsPage(ui, "Browsers");
  await waitText(ui, "Use Hexis for passkeys in browsers");
  const t = await text(ui);
  check("the real core says this computer is supported, so the page is listed", t.includes("Use Hexis for passkeys in browsers") && t.includes("Chromium") && t.includes("Firefox"));
  check("a browser that was never started is not listed", !t.includes("Brave") && !t.includes("Microsoft Edge"));
  check("the switch starts off", (await switchOn()) === "false");
});

step("the switch opens a private socket and writes the extension; Connect registers the browser", async () => {
  await ui.click('button[role=switch][aria-label="Use Hexis for passkeys in browsers"]');
  await ui.waitFor(`document.querySelector('button[role=switch][aria-label="Use Hexis for passkeys in browsers"]').getAttribute('aria-checked')==='true'`, { timeout: 10000 });
  const state = await bridgeState();
  check("the core listens on the socket under the browser's runtime folder", state.listening === true && state.socket === SOCKET && existsSync(SOCKET), JSON.stringify(state));
  check("the folder is private to the user and the socket is closed to others", modeOf(`${RUN}/hexis`) === "700" && modeOf(SOCKET) === "600", `${modeOf(`${RUN}/hexis`)} ${modeOf(SOCKET)}`);
  check("both extension folders were written", existsSync(`${DATA}/extension/chromium/manifest.json`) && existsSync(`${DATA}/extension/firefox/manifest.json`));
  const manifest = JSON.parse(readFileSync(`${DATA}/extension/chromium/manifest.json`, "utf8"));
  check("the Chromium copy has the key that fixes its id, no more permission than native messaging, and runs on websites", manifest.key && manifest.permissions.length === 1 && manifest.permissions[0] === "nativeMessaging" && manifest.content_scripts.length >= 1, JSON.stringify(manifest.permissions));
  check("the Firefox copy names its add-on id", JSON.parse(readFileSync(`${DATA}/extension/firefox/manifest.json`, "utf8")).browser_specific_settings?.gecko?.id === "hexis-passkeys@habibullah.dev");
  check("the page names the two folders", (await text(ui)).includes(`${DATA}/extension/chromium`) && (await text(ui)).includes(`${DATA}/extension/firefox`));
  await ui.click("button", "Connect"); // Firefox is listed first
  await waitText(ui, "Firefox is connected");
  const firefox = JSON.parse(readFileSync(`${HOME}/.mozilla/native-messaging-hosts/dev.habibullah.hexis.passkeys.json`, "utf8"));
  check("Firefox was told too, and only the add-on with this id may start it", firefox.allowed_extensions?.length === 1 && firefox.allowed_extensions[0] === "hexis-passkeys@habibullah.dev" && !("allowed_origins" in firefox), JSON.stringify(firefox));
  await ui.click("button", "Connect"); // Firefox's button now says Disconnect, so this is Chromium's
  await waitText(ui, "Chromium is connected");
  const host = `${HOME}/.config/chromium/NativeMessagingHosts/dev.habibullah.hexis.passkeys.json`;
  const registered = JSON.parse(readFileSync(host, "utf8"));
  check("Chromium was told where Hexis is, and only this extension may start it", registered.name === "dev.habibullah.hexis.passkeys" && registered.allowed_origins?.length === 1 && registered.allowed_origins[0] === `chrome-extension://${EXTENSION_ID}/`, JSON.stringify(registered));
  check("the launcher is a private script that starts the program with --native-host", existsSync(registered.path) && modeOf(registered.path) === "700" && readFileSync(registered.path, "utf8").includes("--native-host"));
  check("the extension's key gives the id the manifest names", await (async () => {
    const { createHash } = await import("node:crypto");
    const der = Buffer.from(manifest.key, "base64"); const hexHash = createHash("sha256").update(der).digest("hex").slice(0, 32);
    return [...hexHash].map((c) => String.fromCharCode("a".charCodeAt(0) + parseInt(c, 16))).join("") === EXTENSION_ID;
  })());
});

step("a browser with the extension, a test page, and a browser's own authenticator behind it", async () => {
  site = await startSite(PAGE_PORT);
  web = await launch({ width: 900, height: 700, port: 9671, profile: `${HOME}/.config/chromium`, extension: `${DATA}/extension/chromium`, env: { HOME, XDG_RUNTIME_DIR: RUN } });
  await web.send("WebAuthn.enable");
  await web.send("WebAuthn.addVirtualAuthenticator", { options: { protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true } });
  await web.goto(`${SITE}/`);
  const patched = await web.waitFor(`window.__hexisPasskeysInstalled === true`, { timeout: 15000 }).catch(() => false);
  if (!patched && process.env.CI) {
    noExtension = true;
    console.log(`SKIPPED (CI): this browser did not load the extension (${await web.eval("navigator.userAgent")}). The steps that need it are skipped; run e2e/real-bridge.mjs where Chromium or Chrome for Testing is available.`);
    return;
  }
  check("the extension's script is in the page", Boolean(patched), await web.eval(`String(navigator.credentials.get).slice(0, 80)`));
});

step("a website saves a passkey: a sheet names it, and a new login holds the passkey afterwards", async () => {
  const before = (await logins()).length;
  await start("__create", { user: "bob@example.com" });
  await sheetOpen("Save a passkey");
  const t = await dialog();
  check("the sheet names the website and says it is a page on this computer", t.includes("localhost") && t.includes("A page on this computer, not a website.") && t.includes(`Port ${PAGE_PORT}`), t);
  check("the sheet says who the passkey is for and offers a new login", t.includes("bob@example.com") && t.includes("Create a new login"), t);
  check("nothing is made before a person chooses", (await logins()).length === before);
  await ui.shot(`${SP}/shots2/bridge-real-create.png`);
  await sheetClick("Save passkey");
  await noSheet();
  const got = await result();
  check("the page got a real PublicKeyCredential", got.ok === true && got.isPublicKeyCredential === true && got.type === "public-key", JSON.stringify(got));
  check("with the page's own origin and challenge in the client data", got.clientType === "webauthn.create" && got.origin === SITE && got.challengeOk === true, JSON.stringify(got));
  check("the relying party hash is the page's name, the user was present and not verified", got.rpIdHashOk === true && (got.flags & 0x01) === 0x01 && (got.flags & 0x04) === 0 && (got.flags & 0x40) === 0x40, JSON.stringify(got));
  check("ES256 with a public key the page can read", got.alg === -7 && got.hasPublicKey === true);
  const now = await logins();
  check("one new login holds one passkey for localhost", now.length === before + 1, `${before} -> ${now.length}`);
  const fresh = now.find((i) => i.title.includes("Local test page"));
  const keys = fresh ? await passkeysOf(fresh.id) : [];
  check("the passkey in the vault is the one the page was given", keys.length === 1 && keys[0].rpId === "localhost" && keys[0].userName === "bob@example.com" && keys[0].credentialId === got.id, JSON.stringify(keys));
  check("a toast in Hexis says what was saved", await waitText(ui, "Saved a passkey for localhost.", 5000).then(() => true, () => false));
}, { needsBrowser: true });

step("a website signs in: the sheet lists the passkey, and the page can verify the signature", async () => {
  await start("__get");
  await sheetOpen("Sign in with a passkey");
  const t = await dialog();
  check("the sheet lists the account and the login", t.includes("bob@example.com") && t.includes("Local test page"), t);
  check("no password is asked for a website that did not ask for verification", !t.includes("Master password"), t);
  await sheetClick("Sign in");
  await noSheet();
  const got = await result();
  check("the page got an assertion for this origin with its own challenge", got.ok === true && got.clientType === "webauthn.get" && got.origin === SITE && got.challengeOk === true, JSON.stringify(got));
  check("and the signature is valid for the public key it was given at creation", got.known === true && got.verified === true, JSON.stringify(got));
  check("the user was present and not verified", (got.flags & 0x01) === 0x01 && (got.flags & 0x04) === 0, JSON.stringify(got));
}, { needsBrowser: true });

step("a website that requires verification gets it from the master password, typed in the Hexis window", async () => {
  await start("__get", { uv: "required" });
  await sheetOpen("Sign in with a passkey");
  const t = await dialog();
  check("the sheet asks for the master password and says the website requires it", t.includes("Master password") && t.includes("This website requires it."), t);
  check("the button waits for it", await ui.eval(`[...document.querySelectorAll('[role=dialog] footer button')].find(b=>b.textContent.trim()==='Sign in').disabled`));
  await ui.typeLabel("Master password", "not the password");
  await sheetClick("Sign in");
  await waitText(ui, "That master password is not right");
  check("a wrong one keeps the request open, and the page is still waiting", !!(await dialog()) && (await web.eval(`window.__settled`)) === false);
  await ui.typeLabel("Master password", PW);
  await sheetClick("Sign in");
  await noSheet();
  const got = await result();
  check("the right one answers it, and the answer says the user was verified", got.ok === true && got.verified === true && (got.flags & 0x04) === 0x04, JSON.stringify(got));
}, { needsBrowser: true });

step("a login that already has the account is offered, and a second passkey goes on it", async () => {
  const [bob] = (await logins()).filter((i) => i.title.includes("Local test page"));
  const before = (await logins()).length;
  await ok("item_create", { draft: { type: "login", title: "Ana at the test page", favorite: false, reprompt: false, tags: [], folder: null, customFields: [], fields: { username: "ana@example.com", password: "pw", url: SITE, notes: "" } } });
  await start("__create", { user: "ana@example.com" });
  await sheetOpen("Save a passkey");
  const t = await dialog();
  check("the login with the same account and website is the first choice", /Ana at the test page[\s\S]*Same account/.test(t), t);
  await sheetClick("Save passkey");
  await noSheet();
  const got = await result();
  const ana = (await logins()).find((i) => i.title === "Ana at the test page");
  const keys = await passkeysOf(ana.id);
  check("the passkey went onto that login, and no new login was made", got.ok === true && keys.length === 1 && keys[0].credentialId === got.id && (await logins()).length === before + 1, JSON.stringify({ got, keys }));
  check("the other login still holds only its own passkey", (await passkeysOf(bob.id)).length === 1);
}, { needsBrowser: true });

step("with two passkeys for the website the sheet asks which, and the page gets that one", async () => {
  await start("__get");
  await sheetOpen("Sign in with a passkey");
  const t = await dialog();
  check("both accounts are listed", t.includes("bob@example.com") && t.includes("ana@example.com") && t.includes("Choose the account"), t);
  await ui.click("[role=dialog] [role=radio]", "ana@example.com");
  await sheetClick("Sign in");
  await noSheet();
  const got = await result();
  const ana = (await logins()).find((i) => i.title === "Ana at the test page");
  check("the assertion is for the one chosen, and it verifies", got.ok === true && got.verified === true && got.id === (await passkeysOf(ana.id))[0].credentialId, JSON.stringify(got));
}, { needsBrowser: true });

step("Use another way hands the page to the browser's own passkey screen; Cancel says no", async () => {
  const before = (await logins()).length;
  await start("__create", { user: "carol@example.com" });
  await sheetOpen("Save a passkey");
  await sheetClick("Use another way");
  await noSheet();
  const own = await result();
  check("the browser's own authenticator answered, and Hexis kept nothing", own.ok === true && own.origin === SITE && (await logins()).length === before, JSON.stringify(own));
  await start("__get");
  await sheetOpen("Sign in with a passkey");
  await sheetClick("Use another way");
  await noSheet();
  const ownGet = await result();
  check("signing in the same way: the browser's passkey, not one of Hexis's", ownGet.ok === true && ownGet.id === own.id && ownGet.verified === true, JSON.stringify({ own: own.id, ownGet }));
  await start("__get");
  await sheetOpen("Sign in with a passkey");
  await sheetClick("Cancel");
  await noSheet();
  const refused = await result();
  check("Cancel rejects the page's promise as the browser's own cancel does", refused.ok === false && refused.name === "NotAllowedError", JSON.stringify(refused));
}, { needsBrowser: true });

step("a page cannot use a website name that is not its own; a program on the socket gets no sheet for one either", async () => {
  await start("__create", { rpId: "example.com" });
  const t0 = Date.now();
  const got = await result();
  check("the page's own call for another website's name ends at the browser, with no sheet in Hexis", got.ok === false && !(await dialog()), JSON.stringify(got));
  check("and quickly: Hexis did not wait for a person", Date.now() - t0 < 8000, String(Date.now() - t0));
  const lie = await rogue(request("get", "bank.example", "https://evil.example"));
  check("a program that claims another site's name on a web page is refused at once", lie.includes('"fallback"') && lie.includes("invalid_request") && !(await dialog()), lie);
  const shared = await rogue(request("create", "co.uk", "https://evil.co.uk"));
  check("a name that many websites share is refused too", shared.includes("invalid_request") || shared.includes("fallback"), shared);
  const broken = await rogue("not json at all");
  check("garbage is refused without a sheet", !(await dialog()) && broken.length > 0, broken);
  const state = await bridgeState();
  check("nothing is waiting for the window", state.waiting === false);
}, { needsBrowser: true });

step("a request nobody answers is not signed: closing the sheet by the page's cancel ends it", async () => {
  const pending = rogue(request("get", "localhost", SITE, { id: "rogue-waiting" }), 15000);
  await sheetOpen("Sign in with a passkey");
  check("a request from a program waits for a person like any other and names the site it claims", (await dialog()).includes("localhost"));
  await sheetClick("Cancel");
  await noSheet();
  const answer = await pending;
  check("Cancel in the window is the answer, never a signature", answer.includes('"cancelled"') && !answer.includes("responseJson"), answer);
}, { needsBrowser: true });

step("switching it off closes the socket at once; pages use the browser's own passkeys again", async () => {
  await ui.click('button[role=switch][aria-label="Use Hexis for passkeys in browsers"]');
  await ui.waitFor(`document.querySelector('button[role=switch][aria-label="Use Hexis for passkeys in browsers"]').getAttribute('aria-checked')==='false'`, { timeout: 10000 });
  const state = await bridgeState();
  check("nothing listens and the socket file is gone", state.listening === false && !existsSync(SOCKET), JSON.stringify(state));
  const before = (await logins()).length;
  const got = await web.eval(`window.__create({ user: "dave@example.com" })`);
  check("the page's create call is answered by the browser, and no sheet opened", got.ok === true && !(await dialog()) && (await logins()).length === before, JSON.stringify(got));
  check("the setting is remembered by the core", (await ok("bridge_status")).enabled === false);
}, { needsBrowser: true });

step("switching it on again works without restarting the browser", async () => {
  await ui.click('button[role=switch][aria-label="Use Hexis for passkeys in browsers"]');
  await ui.waitFor(`document.querySelector('button[role=switch][aria-label="Use Hexis for passkeys in browsers"]').getAttribute('aria-checked')==='true'`, { timeout: 10000 });
  await sleep(500);
  await start("__get");
  await sheetOpen("Sign in with a passkey");
  await sheetClick("Sign in");
  await noSheet();
  const got = await result();
  check("the extension finds the new socket and the page signs in", got.ok === true && got.verified === true, JSON.stringify(got));
}, { needsBrowser: true });

step("Disconnect takes Chromium's registration and the launcher away; the data is untouched", async () => {
  await ui.click("button", "Disconnect"); // Firefox is listed first
  await waitText(ui, "Firefox is disconnected");
  check("Firefox's registration is gone and Chromium's is still there", !existsSync(`${HOME}/.mozilla/native-messaging-hosts/dev.habibullah.hexis.passkeys.json`) && existsSync(`${HOME}/.config/chromium/NativeMessagingHosts/dev.habibullah.hexis.passkeys.json`));
  check("the launcher stays while a browser still uses it", existsSync(`${DATA}/hexis-native-host`));
  await ui.click("button", "Disconnect");
  await waitText(ui, "Chromium is disconnected");
  const host = `${HOME}/.config/chromium/NativeMessagingHosts/dev.habibullah.hexis.passkeys.json`;
  check("the registration is gone", !existsSync(host));
  check("so is the launcher, when no browser needs it", !existsSync(`${DATA}/hexis-native-host`));
  check("the vault still holds its passkeys", noExtension || (await logins()).filter((i) => i.title.includes("Local test page") || i.title === "Ana at the test page").length === 2);
  check("no error was logged in the Hexis window", ui.logs.filter((l) => !l.includes("Failed to load resource")).length === 0, ui.logs.join(" | "));
});

try {
  const only = process.argv[2];
  for (const s of steps) {
    if (only && !s.name.includes(only)) continue;
    if (s.needsBrowser && noExtension) continue;
    console.log(`\n--- ${s.name}`);
    try { await s.fn(); } catch (e) { check(`STEP ERROR: ${s.name}`, false, e.message); await ui?.shot(`${SP}/shots2/real-bridge-FAIL-ui.png`).catch(() => undefined); await web?.shot(`${SP}/shots2/real-bridge-FAIL-web.png`).catch(() => undefined); }
  }
} finally {
  site?.close(); site?.closeAllConnections?.();
  await web?.close().catch(() => undefined);
  await ui?.close().catch(() => undefined);
  await core.stop();
}
finish("real-bridge");
