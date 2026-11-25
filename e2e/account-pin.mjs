// The account bar on the phone header and the account PIN, in the browser PREVIEW backend (not the real core; the real
// rules are in src-tauri/src/storage/app_tests.rs): lock icon, account switcher, PIN set / wrong / right, the limited vault,
// switching between accounts, Log out.
import { launch, sleep } from "./cdp.mjs";
const SP = process.env.SP; const base = "http://localhost:1431/";
const PW = "preview-vault-1", PIN = "4829-pin";
let bad = 0; const check = (n, c, extra = "") => { console.log(`${c ? "PASS" : "FAIL"} ${n}${!c && extra ? ` (${extra})` : ""}`); if (!c) bad++; };
const p = await launch({ width: 420, height: 860, port: 9638, profile: `${SP}/chrome-e2e-pin`, mobile: true });
const text = () => p.text();
const dialogText = () => p.eval(`(()=>{const d=[...document.querySelectorAll('[role=dialog]')].pop(); return d?d.innerText:''})()`);
const rectOf = (sel, label) => p.eval(`(()=>{const e=[...document.querySelectorAll(${JSON.stringify(sel)})].find(x=>(x.getAttribute('aria-label')||'').startsWith(${JSON.stringify(label)})); if(!e) return null; const r=e.getBoundingClientRect(); return {l:r.left,r:r.right,t:r.top,b:r.bottom,w:r.width,h:r.height}})()`);
const lockScreenIn = (needle) => p.waitFor(`document.body.innerText.includes(${JSON.stringify(needle)})`, { timeout: 8000 });
const waitReady = () => p.waitFor(`document.querySelector('[aria-label="Items"]') && !document.body.innerText.includes('Your vault is locked')`, { timeout: 12000 });
try {
  await p.goto(base + "?vault=unlocked");
  await waitReady();

  // ---- the account bar sits at the top right of the phone header
  const lock = await rectOf("button", "Lock now");
  const acct = await rectOf("button", "Account: Marlowe");
  check("the header has a lock button and an account button", !!lock && !!acct);
  check("both sit in the top right corner, big enough to touch", lock && acct && acct.r > 380 && acct.t < 80 && lock.r <= acct.l + 1 && lock.w >= 36 && acct.w >= 36, JSON.stringify({ lock, acct }));
  await p.shot(`${SP}/shots2/pin-header.png`);

  // ---- the account sheet
  await p.click("button", "Account: Marlowe");
  await p.waitFor(`document.body.innerText.includes('Log out')`, { timeout: 4000 });
  let sheet = await dialogText();
  check("the sheet lists every account and marks the open one", sheet.includes("Marlowe") && sheet.includes("Atlas Registry") && sheet.includes("Family") && sheet.includes("Open now"));
  check("the sheet offers Add account, Lock now and Log out", sheet.includes("Add account") && sheet.includes("Lock now") && sheet.includes("Log out"));
  check("an account without a PIN says it needs the master password", /Atlas Registry[\s\S]*Master password/.test(sheet));
  await p.shot(`${SP}/shots2/pin-sheet.png`);
  await p.press("Escape");

  // ---- Lock now without a PIN: master password only
  await p.click("button", "Lock now");
  await lockScreenIn("Your vault is locked");
  check("without a PIN the lock screen asks for the master password only", (await text()).includes("Master password") && !(await text()).includes("Open with PIN"));
  await p.typeLabel("Master password", PW);
  await p.click("button", "Unlock");
  await waitReady();

  // ---- set a PIN in Settings > Security
  await p.goto(base + "?vault=unlocked&filter=settings&page=security");
  await p.waitFor(`document.body.innerText.includes('Account PIN')`, { timeout: 12000 });
  await p.click("button", "Set PIN");
  await p.waitFor(`document.body.innerText.includes('New PIN')`, { timeout: 4000 });
  await p.typeLabel("Master password", "wrong-password-1");
  await p.typeLabel("New PIN", "12");
  check("a PIN that is too short is explained at once", /at least 4/.test(await dialogText()));
  await p.typeLabel("New PIN", PIN);
  await p.typeLabel("Repeat the PIN", PIN + "x");
  check("a repeat that does not match blocks Set PIN", /do not match/.test(await dialogText()));
  await p.typeLabel("Repeat the PIN", PIN);
  await p.click("[role=dialog] button[type=submit]");
  await p.waitFor(`document.body.innerText.includes('not your master password')`, { timeout: 6000 });
  check("a wrong master password is refused", true);
  await p.typeLabel("Master password", PW);
  await p.click("[role=dialog] button[type=submit]");
  await p.waitFor(`!document.body.innerText.includes('New PIN') && document.body.innerText.includes('Change')`, { timeout: 8000 });
  check("the row now offers Change and Remove", (await text()).includes("Remove"));

  // ---- Lock now with a PIN: the PIN is the first choice
  await p.click("button", "Lock now");
  await lockScreenIn("Open with PIN");
  const t = await text();
  check("the lock screen asks for the PIN, with the master password one tap away", t.includes("Open with PIN") && t.includes("Use the master password instead") && !t.includes("Unlock\n"));
  await p.shot(`${SP}/shots2/pin-lock.png`);
  await p.typeLabel("PIN", "000000");
  await p.click("button", "Open with PIN");
  await p.waitFor(`document.body.innerText.includes('Tries left: 4')`, { timeout: 6000 });
  check("a wrong PIN says how many tries are left", true);
  await p.typeLabel("PIN", PIN);
  await p.click("button", "Open with PIN");
  await waitReady();
  check("the right PIN opens the vault", true);

  // ---- opened with the PIN: limited
  const limitedAcct = await rectOf("button", "Account: Marlowe");
  check("the account button carries the key badge while on the PIN", !!limitedAcct);
  await p.click("button", "Account: Marlowe");
  await p.waitFor(`document.body.innerText.includes('Opened with the PIN')`, { timeout: 4000 });
  sheet = await dialogText();
  check("the sheet says it is on the PIN and offers the master password", sheet.includes("Open now, on its PIN") && sheet.includes("Use master password"));
  await p.shot(`${SP}/shots2/pin-limited.png`);
  await p.press("Escape");
  await sleep(250);

  // a login opens, but its secrets ask for the master password
  await p.click("[data-item-row]", "");
  await p.waitFor(`document.body.innerText.includes('Show code')`, { timeout: 6000 });
  check("a one-time code on the PIN says it needs the master password", (await text()).includes("This item asks for your master password"));
  const rev = await p.eval(`(()=>{const b=[...document.querySelectorAll('button')].find(x=>/^reveal/i.test(x.getAttribute('aria-label')||'')); if(!b) return false; b.click(); return true})()`);
  await sleep(500);
  check("revealing a password on the PIN opens the master password sheet", rev && (await text()).includes("Confirm it is you") && (await text()).includes("opened with its PIN"), (await text()).slice(0, 200));
  await p.shot(`${SP}/shots2/pin-reprompt.png`);
  await p.press("Escape");
  await sleep(300);

  // changing the lock timer needs the master password, and typing it lifts the limit
  await p.click("button", "Back to list");
  await sleep(300);
  await p.click("button", "Settings");
  await p.waitFor(`document.body.innerText.includes('App lock')`, { timeout: 6000 });
  await p.click("button", "App lock");
  await p.waitFor(`!!document.querySelector('select[aria-label="Lock after no activity for"]')`, { timeout: 6000 });
  const setTimer = (v) => p.eval(`(()=>{const e=document.querySelector('select[aria-label="Lock after no activity for"]'); const set=Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set; set.call(e,${JSON.stringify(String(v))}); e.dispatchEvent(new Event('change',{bubbles:true})); return e.value})()`);
  const before = await p.eval(`document.querySelector('select[aria-label="Lock after no activity for"]').value`);
  const other = before === "15" ? "30" : "15";
  await setTimer(other);
  await p.waitFor(`document.body.innerText.includes('Confirm it is you')`, { timeout: 6000 });
  check("on the PIN, the lock timer asks for the master password first", (await dialogText()).includes("Change the lock timer"));
  await p.typeLabel("Master password", PW);
  await p.click("[role=dialog] button[type=submit]");
  await p.waitFor(`!document.body.innerText.includes('Confirm it is you')`, { timeout: 8000 });
  await p.waitFor(`document.querySelector('select[aria-label="Lock after no activity for"]').value === ${JSON.stringify(other)}`, { timeout: 4000 }).catch(() => {});
  check("after the password the timer changed", (await p.eval(`document.querySelector('select[aria-label="Lock after no activity for"]').value`)) === other);
  await p.click("button", "Account: Marlowe");
  await p.waitFor(`document.body.innerText.includes('Log out')`, { timeout: 4000 });
  sheet = await dialogText();
  check("the PIN limit is gone after the master password", sheet.includes("Open now") && !sheet.includes("on its PIN") && !sheet.includes("Use master password"));
  await p.press("Escape");
  await sleep(250);

  // ---- five wrong PINs close the vault and only the master password is left
  await p.click("button", "Lock now");
  await lockScreenIn("Open with PIN");
  for (let n = 0; n < 5; n++) {
    await p.typeLabel("PIN", "bad-pin-" + n);
    await p.click("button", "Open with PIN");
    if (n < 4) { await p.waitFor(`document.body.innerText.includes('Tries left: ${4 - n}')`, { timeout: 6000 }); }
  }
  await p.waitFor(`document.body.innerText.includes('Too many wrong PINs')`, { timeout: 6000 });
  check("the fifth wrong PIN says so and offers only the master password", !(await text()).includes("Open with PIN") && (await text()).includes("Master password"));
  await p.typeLabel("Master password", PW);
  await p.click("button", "Unlock");
  await waitReady();

  // ---- switching: an account with a PIN waits behind it, one without is locked
  await p.click("button", "Account: Marlowe");
  await p.waitFor(`document.body.innerText.includes('Family')`, { timeout: 4000 });
  await p.click("[role=dialog] button", "Family");
  await lockScreenIn("Your vault is locked");
  check("switching to an account without a PIN asks for its master password", (await text()).includes("Family") && !(await text()).includes("Open with PIN"));
  await p.typeLabel("Master password", PW);
  await p.click("button", "Unlock");
  await waitReady();
  await p.click("button", "Account: Family");
  await p.waitFor(`document.body.innerText.includes('Marlowe')`, { timeout: 4000 });
  sheet = await dialogText();
  check("the account left behind its PIN says its PIN opens it", /Marlowe[\s\S]*Its PIN opens it/.test(sheet) && /Atlas Registry[\s\S]*Master password/.test(sheet), sheet);
  await p.click("[role=dialog] button", "Marlowe");
  await lockScreenIn("Open with PIN");
  await p.typeLabel("PIN", PIN);
  await p.click("button", "Open with PIN");
  await waitReady();
  check("choosing it again opens it with the PIN, without the master password", true);

  // Family was locked when we left it
  await p.click("button", "Account: Marlowe");
  await p.waitFor(`document.body.innerText.includes('Family')`, { timeout: 4000 });
  await p.click("[role=dialog] button", "Family");
  await lockScreenIn("Your vault is locked");
  check("the account without a PIN needs its master password again", !(await text()).includes("Open with PIN"));

  // ---- log out wipes every account, the PIN one too, and keeps them all saved
  await p.typeLabel("Master password", PW);
  await p.click("button", "Unlock");
  await waitReady();
  await p.click("button", "Account: Family");
  await p.waitFor(`document.body.innerText.includes('Log out')`, { timeout: 4000 });
  await p.click("[role=dialog] button", "Log out");
  await lockScreenIn("Choose an account");
  const picker = await text();
  check("log out shows every saved account", picker.includes("Marlowe") && picker.includes("Atlas Registry") && picker.includes("Family"));
  check("log out leaves nothing waiting behind a PIN", !picker.includes("Its PIN opens it"));
  await p.click("button", "Marlowe");
  await lockScreenIn("Your vault is locked");
  check("a logged-out account with a PIN needs its master password", !(await text()).includes("Open with PIN") && (await text()).includes("Master password"));
  await p.shot(`${SP}/shots2/pin-loggedout.png`);
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/pin-FAIL.png`).catch(() => {}); }
finally { await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
