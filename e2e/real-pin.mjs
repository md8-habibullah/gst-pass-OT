// The account PIN against the REAL core (the HTTP shim over the real Rust backend): set, Lock, the PIN screen, wrong PINs,
// the limited vault, the master password lifting the limit, switching accounts, Log out, and the PIN never reaching a file.
import { Core, SP, check, finish, freshDir, launch, sleep, text, has, waitText, waitGone, createAccount, firstStart, noDialog, runSteps, unlockUi, btn, nav, settingsPage, setSelect, walkFiles, scanFor } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const DATA = freshDir("pin-data");
const core = new Core({ bin: BIN, data: DATA, port: 1451 });
const PW = "correct horse battery staple 8", PW2 = "second long master password 22", PIN = "4829-pin";
const steps = []; const step = (name, fn) => steps.push({ name, fn });
const ok = async (cmd, args = {}) => { const r = await core.ipc(cmd, args); if (r.err) throw new Error(`${cmd}: ${r.err.code} ${r.err.message}`); return r.ok; };
const code = async (cmd, args = {}) => (await core.ipc(cmd, args)).err?.code ?? "ok";
const status = () => ok("vault_status");
const login = (title) => ({ type: "login", title, favorite: false, reprompt: false, tags: [], folder: null, customFields: [], fields: { username: "me@example.com", password: "pw-" + title, url: "", notes: "" } });
const chip = (p, name) => p.click(`button[aria-label^="Account: ${name}"]`);

step("an account with one login", async (p) => {
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, "Marlowe", PW);
  await ok("item_create", { draft: login("Cabin") });
  check("a fresh status has neither PIN flag", (await status()).resumable === undefined && (await status()).limited === undefined);
});

step("Settings > Security > Set PIN, with every refusal explained", async (p) => {
  await settingsPage(p, "Security");
  await waitText(p, "Account PIN");
  await btn(p, "Set PIN");
  await waitText(p, "New PIN");
  await p.typeLabel("Master password", "not the password");
  await p.typeLabel("New PIN", PIN); await p.typeLabel("Repeat the PIN", PIN);
  await p.click("[role=dialog] button[type=submit]");
  await waitText(p, "not your master password");
  check("a wrong master password is refused by the core", (await ok("profile_list")).profiles[0].hasPin === undefined);
  await p.typeLabel("Master password", PW);
  await p.click("[role=dialog] button[type=submit]");
  await noDialog(p);
  check("the PIN is set", (await ok("profile_list")).profiles[0].hasPin === true);
  check("the settings row now offers Change and Remove", (await has(p, "Remove")) && (await has(p, "Change")));
});

step("Lock now parks the vault; the PIN screen opens it limited", async (p) => {
  await btn(p, "Lock now");
  await waitText(p, "Open with PIN");
  const st = await status();
  check("the core says locked but resumable", st.unlocked === false && st.resumable === true, JSON.stringify(st));
  check("every vault command answers locked meanwhile", (await code("item_list")) === "locked" && (await code("backup_list")) === "locked");
  await p.typeLabel("PIN", "000000-no");
  await btn(p, "Open with PIN");
  await waitText(p, "Tries left: 4");
  check("a wrong PIN says how many tries are left", true);
  await p.typeLabel("PIN", PIN);
  await btn(p, "Open with PIN");
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 15000, label: "the shell" });
  const after = await status();
  check("opened with the PIN means limited", after.unlocked === true && after.limited === true, JSON.stringify(after));
  await sleep(300);
  check("the sidebar chip says so", await has(p, "On its PIN"));
});

step("a limited vault: secrets and settings ask for the master password, enforced in the core", async (p) => {
  const it = (await ok("item_list"))[0];
  check("reveal without the password is refused", (await code("item_reveal", { id: it.id, password: null })) === "password_required");
  check("edit without the password is refused", (await code("item_begin_edit", { id: it.id, password: null })) === "password_required");
  check("clone without the password is refused", (await code("item_clone", { id: it.id, password: null })) === "password_required");
  check("history_get without the password is refused", (await code("history_get", { id: it.id, version: 1, password: null })) === "password_required");
  check("a backup target cannot be added", (await code("backup_add", { input: { name: "x", endpoint: null, region: "r", bucket: "b", prefix: "", accessKeyId: "AKIA1", secretAccessKey: "s3cret-1", pathStyle: false, enabled: true, master: false, shareInMaster: false } })) === "master_required");
  check("the lock timers cannot change", (await code("app_settings_set", { input: { lockAfterMinutes: 30, lockAfterLeaveSeconds: 300 } })) === "master_required");
  check("a wrong password in the same call is a wrong password", (await code("item_reveal", { id: it.id, password: "nope nope nope" })) === "wrong_password");
  // through the screen: the Cabin login's password asks for the master password
  await p.click("[data-item-row]", "Cabin");
  await p.waitFor(`!!document.querySelector('button[aria-label^="Reveal"]')`, { timeout: 8000 });
  await p.click('button[aria-label^="Reveal"]');
  await waitText(p, "Confirm it is you");
  check("the sheet explains why", await has(p, "opened with its PIN"));
  await p.typeLabel("Master password", PW); await p.press("Enter");
  await waitGone(p, "Confirm it is you");
  check("the password then shows", await has(p, "pw-Cabin"));
  await p.shot(`${SP}/shots2/real-pin-limited.png`);
});

step("the lock timer asks for the master password, then the limit is gone", async (p) => {
  await settingsPage(p, "App lock");
  await p.waitFor(`!!document.querySelector('select[aria-label="Lock after no activity for"]')`, { timeout: 8000 });
  await setSelect(p, "Lock after no activity for", "30");
  await waitText(p, "Confirm it is you");
  check("the sheet names the change", await has(p, "Change the lock timer"));
  await p.typeLabel("Master password", PW); await p.press("Enter");
  await waitGone(p, "Confirm it is you");
  await sleep(300);
  check("the timer was saved by the core", (await ok("app_settings_get")).lockAfterMinutes === 30);
  check("typing the master password lifted the limit", (await status()).limited === undefined);
  await ok("app_settings_set", { input: { lockAfterMinutes: 15, lockAfterLeaveSeconds: 300 } });
});

step("switching: the PIN account waits behind its PIN, the other is locked", async (p) => {
  await createAccount(p, "Atlas", PW2); // Marlowe is parked by the switch
  const list = (await ok("profile_list")).profiles;
  check("Marlowe is waiting behind its PIN", list.find((x) => x.displayName === "Marlowe")?.resumable === true, JSON.stringify(list));
  check("Atlas has no PIN", list.find((x) => x.displayName === "Atlas")?.hasPin === undefined);
  await chip(p, "Atlas");
  await waitText(p, "Its PIN opens it");
  check("the account sheet says Marlowe's PIN opens it", true);
  await p.click("[role=dialog] button", "Marlowe");
  await waitText(p, "Open with PIN");
  check("choosing Marlowe asks for its PIN, not its master password", !(await has(p, "Master password\n")));
  await p.typeLabel("PIN", PIN);
  await btn(p, "Open with PIN");
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 15000 });
  check("its own items show and only its own", (await ok("item_list")).map((i) => i.title).join() === "Cabin");
  // Atlas had no PIN: leaving it wiped it
  await chip(p, "Marlowe");
  await p.click("[role=dialog] button", "Atlas");
  await waitText(p, "Master password");
  check("Atlas needs its master password again", !(await has(p, "Open with PIN")));
  check("Marlowe's PIN does not open Atlas", (await code("vault_unlock_pin", { pin: PIN })) === "locked");
  await unlockUi(p, PW2);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 15000 });
});

step("log out wipes every account and keeps them saved", async (p) => {
  await chip(p, "Atlas");
  await btn(p, "Log out");
  await waitText(p, "Choose an account");
  const list = (await ok("profile_list")).profiles;
  check("both accounts are still saved", list.length === 2);
  check("nothing waits behind a PIN any more", list.every((x) => x.resumable === undefined));
  check("the screen does not offer a PIN", !(await has(p, "Its PIN opens it")));
  await p.click("button", "Marlowe");
  await waitText(p, "Master password");
  check("Marlowe needs its master password now", !(await has(p, "Open with PIN")));
  await unlockUi(p, PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 15000 });
  check("a master-password unlock is not limited", (await status()).limited === undefined);
});

step("five wrong PINs close the vault for good", async (p) => {
  await btn(p, "Lock now");
  await waitText(p, "Open with PIN");
  for (let n = 4; n >= 1; n--) check(`wrong PIN, ${n} left`, (await code("vault_unlock_pin", { pin: "bad-" + n })) === "wrong_pin");
  check("the fifth says the vault is closed", (await code("vault_unlock_pin", { pin: "bad-0" })) === "pin_locked");
  check("even the right PIN is useless now", (await code("vault_unlock_pin", { pin: PIN })) === "locked");
  check("no longer resumable", (await status()).resumable === undefined);
  // the screen still shows the PIN form (the wipe happened behind its back): the next PIN it sends finds the vault gone and the form explains
  await p.typeLabel("PIN", PIN);
  await btn(p, "Open with PIN");
  await waitText(p, "locked completely in the meantime");
  check("the screen falls back to the master password with a reason", await has(p, "Master password") && !(await has(p, "Open with PIN")));
  await unlockUi(p, PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 15000 });
  check("the master password opens it, fully", (await status()).unlocked === true && (await status()).limited === undefined);
});

step("the PIN is in no file, and removing it needs the master password", async (p) => {
  const files = walkFiles(DATA);
  check("no file of the data folder holds the PIN text", scanFor(files, [PIN]).length === 0, scanFor(files, [PIN]).join());
  check("profiles.json holds an Argon2id hash for it", (await import("node:fs")).readFileSync(`${DATA}/profiles.json`, "utf8").includes("$argon2id$"));
  check("removal with a wrong password is refused", (await code("account_pin_remove", { password: "wrong wrong wrong" })) === "wrong_password");
  await ok("account_pin_remove", { password: PW });
  await btn(p, "Lock now");
  await waitText(p, "Master password");
  check("without a PIN, Lock wipes: only the master password", !(await has(p, "Open with PIN")) && (await status()).resumable === undefined);
});

await core.start();
const page = await launch({ width: 1200, height: 1000, port: 9663, profile: `${SP}/chrome-e2e-pin-real` });
try { await runSteps(page, steps, only); } finally { await page.close(); await core.stop(); }
finish("real-pin");
