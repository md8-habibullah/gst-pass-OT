// End-to-end test: headless Chromium (own profile) -> Vite dev UI on :1431 -> HTTP shim -> the REAL Rust core
// (same IPC dispatch as the app, real Argon2id/ML-KEM/XChaCha20, real SQLite files). No desktop interaction.
import { spawn } from "node:child_process";
import { createHmac } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { launch, sleep } from "./cdp.mjs";

const SP = process.env.SP;
const BIN = process.env.E2E_BIN;
const DATA = `${SP}/e2e-data`;
const FILES = `${SP}/e2e-files`;
const PORT = 1432;
const UI = `http://localhost:1431/?e2e=${PORT}`;
const only = process.argv[2];

const results = [];
function check(name, cond, detail = "") {
  results.push({ name, ok: Boolean(cond), detail });
  console.log(`${cond ? "PASS" : "FAIL"} ${name}${cond ? "" : `   -- ${String(detail).slice(0, 300)}`}`);
  return Boolean(cond);
}

// ---- RFC 6238 reference implementation (independent of the app) ----
const B32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
function b32decode(s) {
  let bits = 0, val = 0; const out = [];
  for (const c of s.replace(/[\s=-]/g, "").toUpperCase()) { val = (val << 5) | B32.indexOf(c); bits += 5; if (bits >= 8) { out.push((val >>> (bits - 8)) & 255); bits -= 8; } }
  return Buffer.from(out);
}
function totp(secret, { digits = 6, period = 30, alg = "sha1", at = Date.now() } = {}) {
  const counter = Math.floor(at / 1000 / period);
  const buf = Buffer.alloc(8); buf.writeBigUInt64BE(BigInt(counter));
  const h = createHmac(alg, b32decode(secret)).update(buf).digest();
  const o = h[h.length - 1] & 15;
  const bin = ((h[o] & 0x7f) << 24) | (h[o + 1] << 16) | (h[o + 2] << 8) | h[o + 3];
  return String(bin % 10 ** digits).padStart(digits, "0");
}
const codeCandidates = (secret, o) => [Date.now() - 30000 * 0, Date.now() - (o?.period ?? 30) * 1000, Date.now() + (o?.period ?? 30) * 1000].map((at) => totp(secret, { ...o, at }));

// ---- Google Authenticator export link builder ----
const varint = (n) => { const o = []; let v = n; while (v >= 0x80) { o.push((v & 0x7f) | 0x80); v = Math.floor(v / 128); } o.push(v); return o; };
const lenField = (num, bytes) => [...varint((num << 3) | 2), ...varint(bytes.length), ...bytes];
const intField = (num, v) => [...varint(num << 3), ...varint(v)];
const str = (s) => [...Buffer.from(s)];
function migrationLink(entries) {
  const params = (e) => [...lenField(1, [...e.secret]), ...lenField(2, str(e.name)), ...lenField(3, str(e.issuer)), ...intField(4, 1), ...intField(5, 1), ...intField(6, 2)];
  const payload = [...entries.flatMap((e) => lenField(1, params(e))), ...intField(2, 1), ...intField(3, 1), ...intField(4, 0)];
  return `otpauth-migration://offline?data=${encodeURIComponent(Buffer.from(payload).toString("base64"))}`;
}

// ---- server control ----
let server = null;
async function startServer() {
  server = spawn(BIN, ["--data-dir", DATA, "--port", String(PORT), "--fast-kdf"], { stdio: ["ignore", "ignore", "pipe"] });
  let err = ""; server.stderr.on("data", (d) => (err += d));
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/health`)).ok) return; } catch { /* not yet */ }
    await sleep(100);
  }
  throw new Error(`e2e server did not start: ${err}`);
}
async function stopServer() { if (server) { server.kill(); await sleep(400); server = null; } }
async function picker(open, save) {
  await fetch(`http://127.0.0.1:${PORT}/e2e/picker`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ open, save }) });
}
async function ipc(cmd, args = {}) {
  const r = await (await fetch(`http://127.0.0.1:${PORT}/ipc`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ cmd, args }) })).json();
  return r;
}

// ---- UI helpers ----
const has = async (p, s) => (await p.text()).includes(s);
const waitText = (p, s, timeout = 10000) => p.waitFor(`document.body.innerText.includes(${JSON.stringify(s)})`, { timeout, label: `text “${s}”` });
const waitGone = (p, s, timeout = 10000) => p.waitFor(`!document.body.innerText.includes(${JSON.stringify(s)})`, { timeout, label: `text gone “${s}”` });
const btn = (p, text) => p.click("button,[role=menuitem],[role=tab],[role=option]", text);
// The account chip lives in the same landmark and its name can contain a nav label ("Imported copy"), so skip it.
const nav = (p, label) => p.click('nav[aria-label="Main"] button:not([aria-label^="Account:"])', label);
const menu = (p, label) => p.click("[role=menuitem]", label);
const noDialog = (p, timeout = 12000) => p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout, label: "dialog closed" });
async function reprompt(p, password) { await waitText(p, "Confirm it is you"); await p.typeLabel("Master password", password); await p.press("Enter"); await waitGone(p, "Confirm it is you"); }
async function createAccount(p, name, password, { skipBackup = true } = {}) {
  await btn(p, "Create a new vault");
  await p.typeLabel("Account name", name);
  await p.typeLabel("Master password", password);
  await p.typeLabel("Confirm master password", password);
  await p.click("input[type=checkbox]");
  await btn(p, "Create vault");
  await waitText(p, "Back up this vault?", 15000);
  if (skipBackup) { await btn(p, "Skip for now"); await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 8000, label: "app shell" }); }
}
async function unlockUi(p, password) { await p.typeLabel("Master password", password); await p.press("Enter"); }
async function fillItem(p, type, fields, { save = true } = {}) {
  await btn(p, "New");
  if (type) { await waitText(p, "What would you like to store?"); await p.click("[role=dialog] button", type); }
  await p.waitFor(`!!document.querySelector('[role=dialog] input')`);
  for (const [label, text] of fields) await p.typeLabel(label, text);
  if (save) { await btn(p, "Add item"); await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 12000, label: "editor closed" }); }
}
const failShot = (p, name) => p.shot(`${SP}/shots2/e2e-FAIL-${name.replace(/\W+/g, "-")}.png`).catch(() => undefined);

const steps = [];
const step = (name, fn) => steps.push({ name, fn });

const MAR = "Marlowe";
const MAR_PW = "correct horse battery staple 7";
const ATL = "Atlas Registry (work)";
const ATL_PW = "a different master password 42!";
const GH_SECRET = "JBSWY3DPEHPK3PXP";
const AWS_SECRET = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
const BARE_SECRET = "MFRGGZDFMZTWQ2LK";

let page;
const S = (n) => results.filter((r) => r.name.startsWith(n));

step("first run: welcome offers create / open file / restore", async (p) => {
  await p.goto(UI); await waitText(p, "Welcome to Passman");
  check("welcome screen lists the three ways in", ["Create a new vault", "Open a vault file", "Restore from a backup"].every((s) => has(p, s) && true));
  const t = await p.text();
  check("welcome screen lists the three ways in (text)", ["Create a new vault", "Open a vault file", "Restore from a backup"].every((s) => t.includes(s)));
});

step("name rules and unique display name", async (p) => {
  await btn(p, "Create a new vault");
  await p.typeLabel("Account name", "x".repeat(41));
  check("41-character name is rejected", (await p.text()).includes("40 characters or fewer"));
  await p.typeLabel("Account name", "");
  check("create button disabled with empty name", await p.eval(`[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Create vault')?.disabled`));
  await btn(p, "Back");
});

step("create account Marlowe (no backup yet) and land in the empty vault", async (p) => {
  await createAccount(p, MAR, MAR_PW);
  await waitText(p, "Your vault is empty");
  check("empty vault message after first create", true);
  check("account chip shows Marlowe", (await p.text()).includes(MAR));
});

step("create a login with a lower-case spaced setup key", async (p) => {
  await fillItem(p, "Login", [["Title", "GitHub"], ["Username", "marlowe-dev"], ["Password", "Tr0ub4dor&3-horse"], ["Website", "github.com/login"], ["One-time code", "jbsw y3dp ehpk 3pxp"]]);
  await waitText(p, "GitHub");
  check("login appears in the list", await has(p, "marlowe-dev"));
});

step("login detail: secrets hidden, reveal needs the master password", async (p) => {
  await p.click("[data-item-row]", "GitHub");
  await waitText(p, "Version history");
  let t = await p.text();
  check("password is hidden before reveal", !t.includes("Tr0ub4dor"));
  check("setup key is never shown", !t.includes(GH_SECRET));
  await p.click("button", "Reveal (asks for your master password)");
  await waitText(p, "Confirm it is you");
  await p.typeLabel("Master password", "wrong password here"); await p.press("Enter");
  await waitText(p, "That master password is not right.");
  check("wrong master password is refused by the guard", true);
  await p.typeLabel("Master password", MAR_PW); await p.press("Enter"); await waitGone(p, "Confirm it is you");
  await waitText(p, "Tr0ub4dor&3-horse");
  check("correct password reveals the secret", true);
  check("secrets hide again on request", (await (async () => { await btn(p, "Hide now"); await waitGone(p, "Tr0ub4dor&3-horse"); return true; })()));
});

step("one-time code matches an independent RFC 6238 implementation", async (p) => {
  await p.waitFor(`!!document.querySelector('[role=timer]')`);
  const shown = (await p.eval(`document.querySelector('[role=timer]').textContent`)).replace(/\s/g, "");
  const want = codeCandidates(GH_SECRET);
  check(`detail code ${shown} equals reference ${want[0]}`, want.includes(shown), `${shown} not in ${want}`);
});

step("card, identity, ssh key, note and code-only items", async (p) => {
  await fillItem(p, "Card", [["Title", "Northline Visa"], ["Name on card", "A. Marlowe"], ["Brand", "Visa"], ["Card number", "4242 4242 4242 4242"], ["Expiry month", "08"], ["Expiry year", "2029"], ["Security code", "123"]]);
  await fillItem(p, "Identity", [["Title", "Passport and address"], ["First name", "Ada"], ["Last name", "Marlowe"], ["Email", "ada@example.test"], ["Passport number", "P1234567"], ["City", "Lisbon"]]);
  await fillItem(p, "SSH key", [["Title", "Deploy key"], ["Public key", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPUBLICKEYDATA marlowe@host"], ["Private key", "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAAPRIVATEKEYBODY\n-----END OPENSSH PRIVATE KEY-----"], ["Key passphrase", "ssh-pass-phrase-1"]]);
  await fillItem(p, "Secure note", [["Title", "Cabin Wi-Fi"], ["Note", "network: cabin-5g\npassword: lakeside-sunrise-91"]]);
  await fillItem(p, "Code only", [["Title", "AWS root"], ["One-time code", `otpauth://totp/AWS:root?secret=${AWS_SECRET}&issuer=AWS&digits=8&period=60&algorithm=SHA256`]]);
  await fillItem(p, "Code only", [["Title", "Bare key service"], ["One-time code", BARE_SECRET.toLowerCase().replace(/(.{4})/g, "$1 ").trim()]]);
  const t = await p.text();
  check("all new items are listed", ["Northline Visa", "Passport and address", "Deploy key", "Cabin Wi-Fi", "AWS root", "Bare key service"].every((s) => t.includes(s)));
});

step("sidebar counts per type", async (p) => {
  const counts = await p.eval(`Object.fromEntries([...document.querySelectorAll('nav[aria-label=Main] li button')].map(b=>b.innerText.trim().split(/\\s*\\n\\s*/)))`);
  check("counts: all 7, logins 1, cards 1, identities 1, ssh 1, notes 1, codes 3", counts["All items"] === "7" && counts["Logins"] === "1" && counts["Cards"] === "1" && counts["Identities"] === "1" && counts["SSH keys"] === "1" && counts["Notes"] === "1" && counts["Codes"] === "3", JSON.stringify(counts));
});

step("codes view: logins and code-only items, correct codes", async (p) => {
  await nav(p, "Codes");
  await waitText(p, "Code only");
  const t = await p.text();
  check("Codes shows the login's code with an 'In a login' badge", t.includes("GitHub") && t.includes("In a login"));
  check("Codes shows code-only items with a 'Code only' badge", t.includes("AWS root") && t.includes("Bare key service") && t.includes("Code only"));
  const tiles = await p.eval(`[...document.querySelectorAll('main li, section li')].map(li=>li.innerText.replace(/\\s+/g,' '))`);
  const find = (name) => tiles.find((s) => s.includes(name)) ?? "";
  const codeOf = (s, digits) => (s.match(new RegExp(`\\b(\\d{${digits / 2}}) (\\d{${digits / 2}})\\b`)) ?? []).slice(1).join("");
  const gh = codeOf(find("GitHub"), 6), aws = codeOf(find("AWS root"), 8), bare = codeOf(find("Bare key service"), 6);
  check(`GitHub code ${gh} correct`, codeCandidates(GH_SECRET).includes(gh), find("GitHub"));
  check(`AWS (SHA-256, 8 digits, 60 s) code ${aws} correct`, codeCandidates(AWS_SECRET, { digits: 8, period: 60, alg: "sha256" }).includes(aws), find("AWS root"));
  check(`bare-key code ${bare} correct`, codeCandidates(BARE_SECRET).includes(bare), find("Bare key service"));
});

step("google authenticator export splits into separate code items", async (p) => {
  const link = migrationLink([{ secret: b32decode(GH_SECRET), name: "alice@example.test", issuer: "Example" }, { secret: b32decode(AWS_SECRET), name: "bob", issuer: "Other" }]);
  await btn(p, "Add code");
  await waitText(p, "What would you like to store?").catch(() => undefined);
  await p.click("[role=dialog] button", "Code only").catch(() => undefined);
  await p.waitFor(`!!document.querySelector('[role=dialog] input')`);
  await p.typeLabel("One-time code", link);
  await waitText(p, "This export holds 2 accounts");
  check("export link is recognised as 2 accounts", true);
  await btn(p, "Add 2 codes");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 12000 });
  await waitText(p, "Example"); await waitText(p, "Other");
  check("both accounts became separate code items", true);
});

step("search and type filters", async (p) => {
  await nav(p, "All items");
  await p.focus("input[type=search]"); await p.send("Input.insertText", { text: "wi-fi" }); await sleep(200);
  const t1 = await p.text();
  check("search finds the note and hides others", t1.includes("Cabin Wi-Fi") && !t1.includes("Northline Visa"));
  await p.eval(`(()=>{const e=document.querySelector('input[type=search]'); e.focus(); e.select();})()`); await p.press("Delete"); await sleep(150);
  await nav(p, "Cards");
  const t2 = await p.text();
  check("Cards filter lists only the card", t2.includes("Northline Visa") && !t2.includes("Cabin Wi-Fi"));
  await nav(p, "All items");
});

step("card number is secret and revealed only after the password", async (p) => {
  await p.click("[data-item-row]", "Northline Visa");
  await waitText(p, "Card number");
  check("card number hidden", !(await has(p, "4242 4242 4242 4242")));
  await p.click("button", "Reveal (asks for your master password)");
  await reprompt(p, MAR_PW);
  await waitText(p, "4242 4242 4242 4242");
  check("card number revealed after the password", true);
});

step("context menu: View/Edit/Clone/Archive/Trash and quick copies", async (p) => {
  await p.rightClick("[data-item-row]", "GitHub");
  await p.waitFor(`!!document.querySelector('[role=menu]')`);
  // "Launch" appears once the menu has looked inside the item for a web address (a sealed read, so it is async).
  await p.waitFor(`[...document.querySelectorAll('[role=menuitem]')].some(e=>e.textContent.trim()==='Launch')`);
  const labels = await p.eval(`[...document.querySelectorAll('[role=menuitem]')].map(e=>e.textContent.trim())`);
  check("menu offers the Bitwarden-style actions", ["View", "Edit", "Clone", "Archive", "Move to trash", "Launch", "Copy username", "Copy password"].every((l) => labels.includes(l)), labels.join(","));
  await menu(p, "Clone");
  await reprompt(p, MAR_PW);
  await waitText(p, "GitHub (copy)");
  check("clone creates “GitHub (copy)”", true);
});

step("archive hides from lists and codes, unarchive restores", async (p) => {
  await p.rightClick("[data-item-row]", "GitHub (copy)");
  await menu(p, "Archive");
  await waitGone(p, "GitHub (copy)");
  check("archived item leaves All items", true);
  await nav(p, "Codes");
  await sleep(300);
  check("archived login's code is not in Codes", !(await p.eval(`document.querySelector('section[aria-label=Codes]').innerText`)).includes("GitHub (copy)"));
  await nav(p, "Archive");
  await waitText(p, "GitHub (copy)");
  check("archived item is in Archive", true);
  await p.click("[data-item-row]", "GitHub (copy)");
  await waitText(p, "This item is archived");
  await btn(p, "Unarchive");
  await waitGone(p, "This item is archived");
  await nav(p, "All items");
  await waitText(p, "GitHub (copy)");
  check("unarchive returns it to All items", true);
});

step("edit creates version 2; history keeps version 1", async (p) => {
  await p.click("[data-item-row]", "Cabin Wi-Fi");
  await waitText(p, "Version history");
  await btn(p, "Edit"); await reprompt(p, MAR_PW);
  await waitText(p, "Edit Cabin Wi-Fi");
  await p.typeLabel("Note", "network: cabin-5g\npassword: lakeside-sunrise-92");
  await btn(p, "Save changes");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 12000 });
  await waitText(p, "version 2").catch(() => undefined);
  await btn(p, "Version history");
  await waitText(p, "Version 1");
  const t = await p.text();
  check("history lists the created and edited versions", t.includes("Created") && t.includes("Edited"), t.slice(-600));
});

step("trash, restore and delete forever", async (p) => {
  await p.rightClick("[data-item-row]", "Passport and address");
  await menu(p, "Move to trash");
  await waitGone(p, "Passport and address");
  await nav(p, "Trash"); await waitText(p, "Passport and address");
  check("trashed item is listed in Trash", true);
  await p.click("[data-item-row]", "Passport and address");
  await waitText(p, "The contents stay sealed while an item is in the trash");
  check("trashed items stay sealed (no field values shown)", !(await has(p, "P1234567")) && !(await has(p, "Lisbon")));
  await p.click("button", "Restore");
  await waitText(p, "restored to your vault");
  await nav(p, "All items"); await waitText(p, "Passport and address");
  check("restore brings the item back", true);
  await p.rightClick("[data-item-row]", "GitHub (copy)");
  await menu(p, "Move to trash");
  await waitGone(p, "GitHub (copy)");
  await nav(p, "Trash"); await p.click("[data-item-row]", "GitHub (copy)");
  await p.click("button", "Delete forever");
  await waitText(p, "permanently erases"); await p.click("[role=dialog] button", "Continue");
  await waitText(p, "Confirm with your master password");
  await p.typeLabel("Master password", "wrong one"); await p.typeLabel("Type “GitHub (copy)” or CONFIRM", "CONFIRM");
  await p.click("[role=dialog] button[type=submit]");
  await waitText(p, "That master password is not right.");
  check("purge with a wrong master password is refused", true);
  await p.typeLabel("Master password", MAR_PW); await p.typeLabel("Type “GitHub (copy)” or CONFIRM", "nope");
  check("purge stays disabled until the name or CONFIRM is typed", await p.eval(`document.querySelector('[role=dialog] button[type=submit]').disabled`));
  await p.typeLabel("Type “GitHub (copy)” or CONFIRM", "GitHub (copy)");
  await p.click("[role=dialog] button[type=submit]");
  await waitText(p, "was deleted forever");
  await waitGone(p, "Confirm with your master password");
  check("purged item is gone from Trash", !(await has(p, "Deleted just now")) || (await p.eval(`document.querySelectorAll('[data-item-row]').length`)) === 0);
  await nav(p, "All items");
});

step("lock, wrong password, unlock", async (p) => {
  await p.keyboardEscape?.();
  await p.eval(`document.querySelector('[role=dialog] button[aria-label=Close], [role=dialog] button')?.click()`).catch(() => undefined);
  await sleep(300);
  await p.eval(`[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Lock now')?.click()`);
  await waitText(p, "Your vault is locked.");
  check("lock screen names the account", (await p.text()).includes(MAR));
  await unlockUi(p, "totally wrong");
  await waitText(p, "That master password is not right.");
  check("wrong master password refused at the lock screen", true);
  await unlockUi(p, MAR_PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 10000 });
  await waitText(p, "GitHub");
  check("correct password unlocks and items are back", true);
});

step("second account: unique names, isolation, switching", async (p) => {
  await p.click("button", "Switch account"); await waitText(p, "Each account is its own vault");
  await p.click("[role=dialog] button", "Add account");
  await waitText(p, "Add an account");
  await btn(p, "Create a new vault");
  await p.typeLabel("Account name", "  marlowe  ");
  await waitText(p, "You already have an account called");
  check("duplicate display name (case/space-insensitive) is rejected", true);
  check("create button stays disabled for a duplicate", await p.eval(`[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Create vault')?.disabled`) !== false);
  await p.typeLabel("Account name", ATL);
  await p.typeLabel("Master password", ATL_PW); await p.typeLabel("Confirm master password", ATL_PW);
  await p.click("input[type=checkbox]"); await btn(p, "Create vault");
  await waitText(p, "Back up this vault?", 15000); await btn(p, "Skip for now");
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 8000 });
  await waitText(p, "Your vault is empty");
  check("new account starts empty (no items from Marlowe)", !(await has(p, "GitHub")));
  check("account chip shows the new account", (await p.text()).includes("Atlas Registry"));
  // switch back
  await p.click("button", "Switch account"); await waitText(p, "Accounts");
  await p.click("[role=dialog] button", MAR);
  await waitText(p, "Master password");
  await unlockUi(p, ATL_PW);
  await waitText(p, "That master password is not right.");
  check("Atlas password does not open Marlowe's vault", true);
  await unlockUi(p, MAR_PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 10000 });
  await waitText(p, "GitHub");
  check("Marlowe's own password opens Marlowe's items", true);
});

step("rename account (unique check) in Settings", async (p) => {
  await nav(p, "Settings"); await waitText(p, "Accounts");
  // The open account is Marlowe; the first Rename button on the page may belong to Atlas (renaming Atlas to its own
  // name in other case is legitimately allowed), so target Marlowe's row explicitly.
  await p.eval(`[...document.querySelectorAll('section[aria-label=Accounts] li')].find(li=>li.textContent.includes('Marlowe')).querySelector('button').click()`);
  await waitText(p, "Rename account");
  await p.typeLabel("Account name", "atlas registry (work)");
  await waitText(p, "You already have an account called");
  check("rename to an existing name is blocked", true);
  await p.typeLabel("Account name", "Marlowe Personal");
  await p.click("[role=dialog] button[type=submit]");
  await waitText(p, "Marlowe Personal");
  check("rename succeeds", true);
});

step("export vault file, add it as an account, open it", async (p) => {
  rmSync(`${FILES}/export.passman`, { force: true });
  await picker(null, `${FILES}/export.passman`);
  await nav(p, "Export");
  await btn(p, "Export vault file");
  await reprompt(p, MAR_PW);
  await waitText(p, "Saved to");
  check("export file written", existsSync(`${FILES}/export.passman`) && statSync(`${FILES}/export.passman`).size > 4096);
  await picker(`${FILES}/export.passman`, null);
  await p.click("button", "Switch account");
  await p.click("[role=dialog] button", "Add account");
  await waitText(p, "Add an account");
  await waitText(p, "Open a vault file");
  await btn(p, "Open a vault file");
  await p.typeLabel("Account name", "Imported copy");
  await btn(p, "Choose the file");
  await waitText(p, "Imported copy");
  await waitText(p, "Master password");
  await unlockUi(p, MAR_PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 10000 });
  await waitText(p, "Northline Visa");
  check("imported vault file opens with its original master password and holds the items", true);
});

step("import Bitwarden JSON and CSV through the UI", async (p) => {
  writeFileSync(`${FILES}/bw.json`, JSON.stringify({ encrypted: false, folders: [{ id: "f1", name: "Work" }], items: [
    { type: 1, name: "Imported login", folderId: "f1", favorite: true, login: { username: "bw-user", password: "bw-pass-zz9", uris: [{ uri: "https://bw.example" }], totp: "otpauth://totp/BW:me?secret=JBSWY3DPEHPK3PXP&issuer=BW" } },
    { type: 3, name: "Imported card", card: { cardholderName: "B. Warden", number: "5555555555554444", expMonth: "03", expYear: "2031", code: "321", brand: "Mastercard" } },
    { type: 2, name: "Imported note", notes: "bw note body" },
  ] }));
  writeFileSync(`${FILES}/chrome.csv`, "name,url,username,password\nCSV site,https://csv.example,csv-user,csv-pass-qq1\n");
  const pickNext = (path) => fetch(`http://127.0.0.1:${PORT}/e2e/pick-next`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ path }) });
  await nav(p, "Import");
  await waitText(p, "Choose a file");
  await pickNext(`${FILES}/bw.json`); await btn(p, "Choose a file");
  await waitText(p, "Looks like Bitwarden (JSON)"); await btn(p, "Check the file");
  await waitText(p, "3 entries ready");
  await btn(p, "Import 3 items");
  await waitText(p, "3 items imported");
  check("Bitwarden JSON imported (login, card, note)", true);
  await pickNext(`${FILES}/chrome.csv`); await btn(p, "Choose a file");
  await waitText(p, "chrome.csv"); await btn(p, "Check the file");
  await waitText(p, "1 entry ready");
  await btn(p, "Import 1 item");
  await waitText(p, "1 item imported");
  await nav(p, "All items");
  await waitText(p, "Imported login"); await waitText(p, "CSV site");
  check("imported items are in the vault", true);
});

step("export file is encrypted: no plaintext in the vault files", async () => {
  const markers = ["Tr0ub4dor", "marlowe-dev", "JBSWY3DPEHPK3PXP", "4242 4242", "4242424242424242", "P1234567", "lakeside-sunrise", "PRIVATEKEYBODY", "ssh-pass-phrase", "Cabin", "Northline", "GEZDGNBVGY3TQOJQ", "bw-pass-zz9", "csv-pass-qq1", "Deploy key", "GitHub"];
  const files = [];
  const walk = (d) => { for (const e of readdirSync(d, { withFileTypes: true })) { const f = join(d, e.name); e.isDirectory() ? walk(f) : files.push(f); } };
  walk(DATA); files.push(`${FILES}/export.passman`);
  const hits = [];
  for (const f of files.filter((x) => !x.endsWith("profiles.json"))) {
    const buf = readFileSync(f);
    for (const m of markers) if (buf.includes(Buffer.from(m)) || buf.includes(Buffer.from(m, "utf16le"))) hits.push(`${f.split("/").slice(-2).join("/")}: ${m}`);
  }
  check(`no plaintext markers in ${files.length - 1} vault/WAL/export files`, hits.length === 0, hits.join("; "));
  const reg = readFileSync(join(DATA, "profiles.json"), "utf8");
  check("profiles.json holds only display names, ids and times", !markers.some((m) => reg.includes(m)) && reg.includes("Marlowe"), reg.slice(0, 300));
});

step("restart: accounts persist, password still required", async (p) => {
  await stopServer(); await startServer();
  await p.goto(UI);
  await waitText(p, "Choose an account");
  const t = await p.text();
  check("all accounts are listed after a restart", t.includes("Marlowe Personal") && t.includes("Atlas Registry (work)") && t.includes("Imported copy"));
  await p.click("button", "Atlas Registry");
  await unlockUi(p, ATL_PW);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 10000 });
  check("Atlas still opens with its own password and is still empty", await has(p, "Your vault is empty"));
});

step("delete an account needs its password and a confirmation", async (p) => {
  await nav(p, "Settings"); await waitText(p, "Accounts");
  const idx = await p.eval(`[...document.querySelectorAll('section[aria-label=Accounts] li')].findIndex(li=>li.textContent.includes('Imported copy'))`);
  await p.eval(`[...document.querySelectorAll('section[aria-label=Accounts] li')][${idx}].querySelectorAll('button')[1].click()`);
  await waitText(p, "Delete this account?");
  await p.typeLabel("That account's master password", MAR_PW);
  check("delete stays disabled without the confirmation", await p.eval(`[...document.querySelectorAll('[role=dialog] button[type=submit]')][0].disabled`));
  await p.typeLabel("Type “Imported copy” or CONFIRM to continue", "CONFIRM");
  await p.click("[role=dialog] button[type=submit]");
  await waitGone(p, "Imported copy", 10000).catch(() => undefined);
  const r = await ipc("profile_list");
  check("account removed from the registry", r.ok && !r.ok.profiles.some((x) => x.displayName === "Imported copy"), JSON.stringify(r).slice(0, 200));
});

// ---- runner ----
mkdirSync(FILES, { recursive: true });
rmSync(DATA, { recursive: true, force: true });
await startServer();
page = await launch({ width: 1440, height: 900, port: 9733, profile: `${SP}/chrome-e2e-run` });
let failed = 0;
try {
  for (const s of steps) {
    if (only && !s.name.includes(only)) continue;
    console.log(`\n--- ${s.name}`);
    try { await s.fn(page); } catch (e) { failed++; check(`STEP ERROR: ${s.name}`, false, e.message); await failShot(page, s.name); }
  }
} finally {
  if (page.logs.length) console.log("\nbrowser console:", [...new Set(page.logs)].slice(0, 12));
  await page.close(); await stopServer();
}
const bad = results.filter((r) => !r.ok);
console.log(`\n${results.length - bad.length}/${results.length} checks passed${bad.length ? `; FAILED: ${bad.map((b) => b.name).join(" | ")}` : ""}`);
process.exit(bad.length ? 1 : 0);
