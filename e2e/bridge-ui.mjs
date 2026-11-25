// Browsers on a Linux computer (Settings > Browsers and the request sheet) in the browser PREVIEW backend, which pretends to be
// such a computer: the page is listed, the switch and the connections work, and a website's request opens a sheet that waits for
// the person. The real core's side (the socket, the checks, the answers) is in the Rust tests and in real-bridge.mjs.
import { launch, sleep } from "./cdp.mjs";
const SP = process.env.SP; const base = "http://localhost:1431/";
let bad = 0; const check = (n, c, extra = "") => { console.log(`${c ? "PASS" : "FAIL"} ${n}${!c && extra ? ` (${extra})` : ""}`); if (!c) bad++; };
const p = await launch({ width: 1100, height: 860, port: 9652, profile: `${SP}/chrome-e2e-bridge` });
const text = () => p.eval("document.body.innerText");
const dialog = () => p.eval(`document.querySelector('[role=dialog]')?.innerText ?? null`);
const log = () => p.eval(`JSON.parse(JSON.stringify(globalThis.__hexisBridgeLog ?? []))`);
const switchOn = () => p.eval(`document.querySelector('button[role=switch][aria-label="Use Hexis for passkeys in browsers"]')?.getAttribute('aria-checked')`);
const openSheet = async (query) => {
  await p.goto(base + `?vault=unlocked&${query}`);
  await p.waitFor(`!!document.querySelector('[role=dialog]') && !document.querySelector('[role=dialog]').innerText.includes('Looking in your vault')`, { timeout: 15000 });
};
const armed = () => sleep(900); // the sheet ignores the first 700 ms on purpose
const primary = () => p.eval(`(()=>{const b=[...document.querySelectorAll('[role=dialog] footer button')].find(x=>/^(Sign in|Save passkey)$/.test(x.textContent.trim())); return b?{disabled:b.disabled}:null})()`);
try {
  // ---- the settings page
  await p.goto(base + "?vault=unlocked&filter=settings&page=browsers");
  await p.waitFor(`document.body.innerText.includes('Use Hexis for passkeys in browsers')`, { timeout: 15000 });
  check("Browsers is listed in the settings categories", await p.eval(`document.querySelector('nav[aria-label="Settings categories"] button[aria-current=page]')?.innerText.includes('Browsers')`));
  check("the switch starts off", (await switchOn()) === "false");
  let t = await text();
  check("only the browsers that were found are listed", t.includes("Firefox") && t.includes("Chromium") && t.includes("Brave") && !t.includes("Microsoft Edge") && !t.includes("Vivaldi"));
  check("a connected browser can be disconnected, an old connection is offered again", t.includes("Disconnect") && t.includes("Connect again"));
  check("a browser in a sandbox is named and explained", t.includes("Chrome (Flatpak) cannot use Hexis"));
  check("no extension folder is named before the switch is on", t.includes("Turn the switch on, or connect a browser"));
  for (const rule of ["opens in Hexis and waits for you", "comes from the browser, not from the page", "private key stays in Hexis", "master password, typed in this window", "app lock is closed", "ES256", "Hexis must be open"]) {
    check(`the page says: …${rule}…`, t.includes(rule));
  }
  await p.shot(`${SP}/shots2/bridge-page-off.png`);

  await p.click('button[role=switch][aria-label="Use Hexis for passkeys in browsers"]');
  await p.waitFor(`document.querySelector('button[role=switch][aria-label="Use Hexis for passkeys in browsers"]').getAttribute('aria-checked')==='true'`, { timeout: 5000 });
  t = await text();
  check("the switch turns on and a toast says so", t.includes("Browsers can now ask Hexis for passkeys"));
  check("the extension's two folders are named, each with a Copy button", t.includes("/extension/chromium") && t.includes("/extension/firefox") && (await p.eval(`[...document.querySelectorAll('button')].filter(b=>b.textContent.trim()==='Copy').length`)) === 2);
  check("the list summary follows", await p.eval(`document.querySelector('nav[aria-label="Settings categories"] button[aria-current=page]')?.innerText.includes('On')`));
  await p.shot(`${SP}/shots2/bridge-page-on.png`);

  await p.click("button", "Connect");
  await p.waitFor(`document.body.innerText.includes('Firefox is connected')`, { timeout: 5000 });
  check("Connect connects a browser", (await p.eval(`document.querySelectorAll('button').length`)) > 0 && (await text()).includes("Firefox is connected"));
  await p.click("button", "Connect again");
  await p.waitFor(`document.body.innerText.includes('Brave is connected')`, { timeout: 5000 });
  check("Connect again repairs an old connection", !(await text()).includes("older copy of Hexis"));
  await p.click("button", "Disconnect");
  await p.waitFor(`document.body.innerText.includes('is disconnected')`, { timeout: 5000 });
  check("Disconnect takes a browser away", (await text()).includes("is disconnected"));

  // ---- a website asks to sign in: one passkey, picked already; a click is ignored for the first moment
  await openSheet("bridge=get");
  const early = await primary();
  check("the sheet asks for what the website asked", (await dialog()).includes("Sign in with a passkey") && (await dialog()).includes("example.com") && (await dialog()).includes("wants to sign you in"));
  check("the passkey is listed with the account and the login it is on", (await dialog()).includes("ana@example.com") && (await dialog()).includes("Hetzner Cloud"));
  check("the sign-in button does not work in the first moment", early?.disabled === true);
  check("the page's own list is not changed by a request", !(await text()).includes("Saved a passkey"));
  await p.shot(`${SP}/shots2/bridge-get.png`);
  await armed();
  check("then it does", (await primary())?.disabled === false);
  await p.click("[role=dialog] footer button", "Sign in");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 8000 });
  let entries = await log();
  check("signing in answered the request with that passkey and no password", entries.length === 1 && entries[0].action === "use" && entries[0].password === false, JSON.stringify(entries));
  check("a toast says it was done", (await text()).includes("Signed in to example.com"));

  // ---- a website that requires verification: the master password, wrong then right
  await openSheet("bridge=get&verify=required");
  check("the password field is there because the website requires it", (await dialog()).includes("Master password") && (await dialog()).includes("This website requires it"));
  await armed();
  check("the button waits for the password", (await primary())?.disabled === true);
  await p.typeLabel("Master password", "not the password");
  await p.click("[role=dialog] footer button", "Sign in");
  await p.waitFor(`document.body.innerText.includes('That master password is not right')`, { timeout: 8000 });
  check("a wrong password keeps the request open", !!(await dialog()) && (await log()).length === 0);
  await p.typeLabel("Master password", "preview-vault-1");
  await p.click("[role=dialog] footer button", "Sign in");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 8000 });
  entries = await log();
  check("the right one answers it, with verification", entries.length === 1 && entries[0].action === "use" && entries[0].password === true, JSON.stringify(entries));

  // ---- Use another way and Cancel are different answers; Esc cancels
  await openSheet("bridge=get");
  await armed();
  await p.click("[role=dialog] footer button", "Use another way");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 8000 });
  check("Use another way hands the page to the browser", (await log()).at(-1)?.action === "decline");
  await openSheet("bridge=get");
  await p.click("[role=dialog] footer button", "Cancel");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 8000 });
  check("Cancel tells the page the person said no", (await log()).at(-1)?.action === "cancel");
  await openSheet("bridge=get");
  await p.press("Escape");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 8000 });
  check("Esc cancels", (await log()).at(-1)?.action === "cancel");

  // ---- save a new passkey: a new login by default for an unknown website; another website's name is shown as is
  await openSheet("bridge=create&site=https://shop.example.org");
  t = await dialog();
  check("the website is the title and the sheet says what it wants", t.includes("Save a passkey") && t.includes("shop.example.org") && t.includes("wants to save a passkey for ana@example.com"));
  check("a new login is offered, and others can be chosen", t.includes("Create a new login") && t.includes("Choose another login"));
  await p.shot(`${SP}/shots2/bridge-create.png`);
  await armed();
  await p.click("[role=dialog] footer button", "Save passkey");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 8000 });
  entries = await log();
  check("saving made a new login", entries.at(-1)?.action === "create" && entries.at(-1)?.target?.new !== undefined, JSON.stringify(entries.at(-1)));
  check("and a toast says where", (await text()).includes("Saved a passkey for shop.example.org"));
  await p.click("nav[aria-label=Main] button", "Logins").catch(() => undefined);
  check("the new login is in the vault", (await text()).includes("shop.example.org"));

  // ---- a page on this computer is named as one
  await openSheet("bridge=get&site=http://localhost:3000");
  t = await dialog();
  check("a page on localhost is said to be on this computer, not a website", t.includes("localhost") && t.includes("A page on this computer, not a website"), t);
  await p.click("[role=dialog] footer button", "Cancel");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 8000 });

  // ---- the lock screen: a note, not a sheet; the lock screen stays usable and Hexis opens there as always
  await p.goto(base + "?vault=locked&bridge=get");
  await p.waitFor(`document.body.innerText.includes('is asking for a passkey') || document.body.innerText.includes('wants to sign you in with a passkey')`, { timeout: 15000 });
  t = await text();
  check("while locked a note names the website and says to unlock", t.includes("example.com") && t.includes("Unlock Hexis to answer"), t);
  check("there is no sheet over the lock screen, so the password can be typed", !(await dialog()));
  await p.shot(`${SP}/shots2/bridge-locked.png`);
  await p.typeLabel("Master password", "preview-vault-1");
  await p.press("Enter");
  await p.waitFor(`!!document.querySelector('[role=dialog]')`, { timeout: 20000 });
  check("after unlocking the request is the sheet", (await dialog()).includes("Sign in with a passkey"));
  await p.click("[role=dialog] footer button", "Cancel");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 8000 });
  await p.goto(base + "?vault=locked&bridge=get");
  await p.waitFor(`document.body.innerText.includes('Unlock Hexis to answer')`, { timeout: 15000 });
  await p.click("button", "Use another way");
  await p.waitFor(`!document.body.innerText.includes('Unlock Hexis to answer')`, { timeout: 5000 });
  check("the note's own buttons answer the request", (await log()).at(-1)?.action === "decline");

  // ---- a phone: the sheet is a bottom sheet and the settings page opens from the list
  await p.viewport(400, 820, true);
  await openSheet("bridge=create&verify=required");
  await sleep(600); // it slides up
  check("the password field has the focus when it appears", await p.eval(`document.activeElement?.closest('[role=dialog]') !== null && document.activeElement?.getAttribute('type') === 'password'`));
  check("on a phone the same sheet is a bottom sheet", await p.eval(`(()=>{const d=document.querySelector('[role=dialog]'); const r=d.getBoundingClientRect(); return Math.abs(r.bottom-innerHeight)<2 && r.width>=380})()`));
  check("and nothing overflows sideways", await p.eval(`document.documentElement.scrollWidth<=innerWidth`));
  await p.shot(`${SP}/shots2/bridge-phone.png`);

  check("no errors in the console", p.logs.length === 0, p.logs.join(" | "));
} catch (e) {
  bad++; console.log("FAIL exception:", e.message); await p.shot(`${SP}/shots2/bridge-FAIL.png`).catch(() => undefined);
} finally { p.proc.kill(); }
console.log(bad ? `\n${bad} check(s) failed` : "\nall bridge UI checks passed");
process.exit(bad ? 1 : 0);
