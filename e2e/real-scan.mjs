// The all-in-one scanner and passkeys against the REAL core (the HTTP shim over the real Rust backend): the scanner suggests the
// login with the same account, adds a code to it, asks the master password of a login that asks for it, and a passkey made
// through the provider's entry (the one Android's service uses) shows in the UI without its key, survives an edit and can be removed.
import { Core, SP, check, finish, freshDir, launch, text, has, waitText, createAccount, firstStart, noDialog, runSteps, btn, nav, settingsPage } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const DATA = freshDir("scan-data");
const core = new Core({ bin: BIN, data: DATA, port: 1452 });
const PW = "correct horse battery staple 8";
const steps = []; const step = (name, fn) => steps.push({ name, fn });
const ok = async (cmd, args = {}) => { const r = await core.ipc(cmd, args); if (r.err) throw new Error(`${cmd}: ${r.err.code} ${r.err.message}`); return r.ok; };
const code = async (cmd, args = {}) => (await core.ipc(cmd, args)).err?.code ?? "ok";
const login = (title, user, extra = {}) => ({ type: "login", title, favorite: false, reprompt: !!extra.reprompt, tags: [], folder: null, customFields: [], fields: { username: user, password: "pw-" + title, url: extra.url ?? "", notes: "" } });
const uri = (issuer, account, secret) => `otpauth://totp/${encodeURIComponent(issuer)}:${encodeURIComponent(account)}?secret=${secret}&issuer=${encodeURIComponent(issuer)}`;
const dialog = (p) => p.eval(`(()=>{const d=[...document.querySelectorAll('[role=dialog]')].pop(); return d?d.innerText:''})()`);
const idOf = async (title) => (await ok("item_list")).find((i) => i.title === title).id;
const provider = async (op, args) => JSON.parse(await core.post("/e2e/provider", { op, args }));
const browser = (rp) => ({ kind: "browser", origin: `https://${rp}`, clientDataHash: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" });
const createRequest = (rp, user) => JSON.stringify({ rp: { id: rp, name: rp }, user: { id: "dXNlci0x", name: user, displayName: user }, challenge: "Y2hhbGxlbmdl", pubKeyCredParams: [{ type: "public-key", alg: -7 }], authenticatorSelection: { residentKey: "required", userVerification: "required" } });
const scan = async (p, text_) => { await p.typeLabel("Or paste a setup key or link", text_); await btn(p, "Check it"); };
/** The footer button that starts over: "Scan something else" after a result, "Scan another" after a save. */
const again = async (p) => { await p.click("[role=dialog] button", (await dialog(p)).includes("Scan something else") ? "Scan something else" : "Scan another"); await waitText(p, "Or paste a setup key or link"); };
const openScanner = async (p) => { await p.click('button[aria-label="Scan a code or passkey"]'); await waitText(p, "Or paste a setup key or link"); };
let passkeyLoginId = null;

step("an account with three logins, one of them asking for the master password", async (p) => {
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, "Marlowe", PW);
  await ok("item_create", { draft: login("GitHub", "ana@example.com", { url: "https://github.com" }) });
  await ok("item_create", { draft: login("Bank", "ana@bank.example", { url: "https://bank.example", reprompt: true }) });
  await ok("item_create", { draft: login("Other", "someone@else.example") });
  await p.goto(core.ui);
  await p.waitFor(`!!document.querySelector('button[aria-label="Scan a code or passkey"]')`, { timeout: 15000 });
  check("the camera button is in the list header", true);
});

step("a code for an account that exists suggests that login and adds the code to it", async (p) => {
  await openScanner(p);
  await scan(p, uri("GitHub", "ana@example.com", "JBSWY3DPEHPK3PXP"));
  await waitText(p, "One-time code for GitHub");
  const t = await dialog(p);
  check("the login with the same account, website and name is suggested", /GitHub[\s\S]*Same account/.test(t), t);
  check("making a new login is offered next to it", t.includes("Or make a new login"));
  await p.click('button[aria-label="Add the code to GitHub"]');
  await waitText(p, "Added the code to “GitHub”");
  const view = await ok("item_get", { id: await idOf("GitHub") });
  check("the real core now holds the code on that login", view.fields.some((f) => f.key === "totp" && f.hasValue), JSON.stringify(view.fields.map((f) => f.key)));
  check("the secret never travelled back to the page", !JSON.stringify(view).includes("JBSWY3DPEHPK3PXP"));
});

step("a code that is on the login already cannot be added twice", async (p) => {
  await again(p);
  await scan(p, uri("GitHub", "ana@example.com", "JBSWY3DPEHPK3PXP"));
  await waitText(p, "Already has this code");
  check("the add button is off", await p.eval(`document.querySelector('button[aria-label="Add the code to GitHub"]').disabled`));
});

step("a login that asks for the master password asks before it takes a code", async (p) => {
  await again(p);
  await scan(p, uri("Bank", "ana@bank.example", "KRSXG5CTMVRXEZLU"));
  await waitText(p, "One-time code for Bank");
  await p.click('button[aria-label="Add the code to Bank"]');
  await waitText(p, "Confirm it is you");
  const bank = await idOf("Bank");
  check("nothing was added before the password", !(await ok("item_get", { id: bank })).fields.some((f) => f.key === "totp"));
  await p.typeLabel("Master password", PW);
  await p.press("Enter");
  await waitText(p, "Added the code to “Bank”");
  check("with the password the code is on the Bank login", (await ok("item_get", { id: bank })).fields.some((f) => f.key === "totp" && f.hasValue));
});

step("an account that has no login offers a new one named after the issuer", async (p) => {
  await again(p);
  await scan(p, uri("Zorg Labs", "someone@new.example", "NB2HI4DTHIXS6Z3PNUXG4ZZO"));
  await waitText(p, "No login in this vault matches someone@new.example");
  check("the new login's name is the issuer", (await p.valueOf("Make a new login")) === "Zorg Labs");
  await btn(p, "Make login");
  await waitText(p, "Made the login");
  const made = (await ok("item_list")).find((i) => i.title === "Zorg Labs");
  check("the real core made the login with the code", made && (await ok("item_get", { id: made.id })).fields.some((f) => f.key === "totp" && f.hasValue));
});

step("other things are named and saved nowhere", async (p) => {
  const before = (await ok("item_list")).length;
  await again(p);
  await scan(p, "https://example.com/some/page");
  await waitText(p, "example.com");
  check("a web address saves nothing by itself", (await ok("item_list")).length === before);
  await again(p);
  await scan(p, "FIDO:/1234567890");
  await waitText(p, "passkey");
  check("a phone sign-in code is explained, not saved", (await ok("item_list")).length === before);
  await p.press("Escape");
  await noDialog(p);
});

step("a passkey made through the provider shows without its key, survives an edit and can be removed", async (p) => {
  const made = await provider("create", { request: createRequest("example.com", "ana@example.com"), caller: browser("example.com"), target: { new: { title: null } }, userVerified: true });
  check("the provider made a passkey in the open vault", made.ok === true, JSON.stringify(made));
  const item = (await ok("item_list")).find((i) => i.title.toLowerCase().includes("example"));
  passkeyLoginId = item.id;
  const view = await ok("item_get", { id: item.id });
  const row = view.fields.find((f) => f.passkey);
  check("the item view carries the passkey's public facts", row?.passkey?.rpId === "example.com" && row.passkey.userName === "ana@example.com", JSON.stringify(row));
  check("and no key anywhere in it", !/privateKey|"keep"/.test(JSON.stringify(view)), JSON.stringify(view).slice(0, 300));
  await p.goto(core.ui);
  await p.waitFor(`!!document.querySelector('nav[aria-label="Main"]')`, { timeout: 15000 });
  await nav(p, "All items");
  await p.click("button", item.title);
  await waitText(p, "example.com");
  check("the detail shows the passkey's website and account", (await has(p, "ana@example.com")) && (await has(p, "Passkey")));
  await p.shot(`${SP}/shots2/real-scan-passkey.png`);

  // edit: rename the login; the passkey must still be there afterwards
  await btn(p, "Edit");
  await p.waitFor(`!!document.querySelector('button[aria-label^="Remove passkey 1"]')`, { timeout: 8000 });
  await p.typeLabel("Title", "Example account");
  await btn(p, "Save changes");
  await p.waitFor(`!document.querySelector('button[aria-label^="Remove passkey"]')`, { timeout: 8000 });
  const after = await ok("item_get", { id: item.id });
  check("the edit kept the passkey (the core swapped the stored one back in)", after.title === "Example account" && after.fields.some((f) => f.passkey), JSON.stringify(after.fields));

  // the provider still signs with it after the edit
  const options = await provider("getOptions", { request: JSON.stringify({ challenge: "Y2hhbGxlbmdl", rpId: "example.com", userVerification: "required" }), caller: browser("example.com") });
  check("the provider still finds it after the edit", options.ok === true && options.data.matches?.length === 1, JSON.stringify(options));

  // remove it in the editor
  await btn(p, "Edit");
  await p.waitFor(`!!document.querySelector('button[aria-label^="Remove passkey 1"]')`, { timeout: 8000 });
  await p.click('button[aria-label^="Remove passkey 1"]');
  await btn(p, "Save changes");
  await p.waitFor(`!document.querySelector('button[aria-label^="Remove passkey"]')`, { timeout: 8000 });
  const gone = await ok("item_get", { id: item.id });
  check("removing it in the editor removes it in the core", !gone.fields.some((f) => f.passkey));
  const none = await provider("getOptions", { request: JSON.stringify({ challenge: "Y2hhbGxlbmdl", rpId: "example.com", userVerification: "required" }), caller: browser("example.com") });
  check("and the provider no longer offers it", none.ok === true && (none.data.matches ?? []).length === 0, JSON.stringify(none));
});

step("the screen cannot make a passkey", async () => {
  const draft = login("Forged", "x");
  draft.fields.passkey = JSON.stringify({ v: 1, rpId: "evil.example", rpName: "Evil", credentialId: "AAECAwQFBgcICQoLDA0ODw", userHandle: "dXNlcg", userName: "x", displayName: "x", alg: -7, privateKey: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE", createdAt: 1 });
  check("item_create refuses a passkey typed into a new item", (await code("item_create", { draft })) === "invalid_input");
});

step("Settings has no Passkeys page on a computer", async (p) => {
  await settingsPage(p, "Security");
  await waitText(p, "Account PIN");
  const cats = await p.eval(`[...document.querySelectorAll("nav[aria-label='Settings categories'] button")].map(b=>b.innerText.split('\\n')[0].trim())`);
  check("the page is listed only where Android's Credential Manager exists", !cats.includes("Passkeys") && cats.includes("Security"), JSON.stringify(cats));
  const status = await ok("passkey_provider_status");
  check("the core says it is not supported here", status.supported === false && status.enabled === false, JSON.stringify(status));
});

await core.start();
const page = await launch({ width: 1200, height: 1000, port: 9664, profile: `${SP}/chrome-e2e-scan-real` });
try { await runSteps(page, steps, only); } finally { await page.close(); await core.stop(); }
finish("real-scan");
