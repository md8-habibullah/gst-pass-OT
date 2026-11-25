// Settings > Passkeys (Android 14 and later) in the browser PREVIEW backend, which pretends to be such a phone: the page is
// listed, the switch turns the provider on and off, the button that opens Android's own passkey list needs the switch on,
// and the rules that stay true are written down. The real core's side (not Android: unsupported; the master password for
// the switch) is in the Rust tests; what Android does with the service is checked on a phone, not here.
import { launch, sleep } from "./cdp.mjs";
const SP = process.env.SP; const base = "http://localhost:1431/";
let bad = 0; const check = (n, c, extra = "") => { console.log(`${c ? "PASS" : "FAIL"} ${n}${!c && extra ? ` (${extra})` : ""}`); if (!c) bad++; };
const p = await launch({ width: 1100, height: 860, port: 9642, profile: `${SP}/chrome-e2e-provider` });
const switchOn = () => p.eval(`document.querySelector('button[role=switch][aria-label="Offer Hexis for passkeys"]')?.getAttribute('aria-checked')`);
const openButton = () => p.eval(`(()=>{const b=[...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='Open Android settings'); return b?{disabled:b.disabled}:null})()`);
try {
  // ---- a tablet or computer: the category is in the left column and the page opens with the switch off
  await p.goto(base + "?vault=unlocked&filter=settings&page=passkeys");
  await p.waitFor(`document.body.innerText.includes('Offer Hexis for passkeys')`, { timeout: 15000 });
  check("Passkeys is listed in the settings categories", await p.eval(`!!document.querySelector('nav[aria-label="Settings categories"] button[aria-current=page]') && document.querySelector('nav[aria-label="Settings categories"] button[aria-current=page]').innerText.includes('Passkeys')`));
  check("the switch starts off", (await switchOn()) === "false");
  const first = await openButton();
  check("opening Android's list needs the switch on", first?.disabled === true && (await p.text()).includes("Turn the switch on first"));
  const t = await p.text();
  for (const rule of ["fingerprint or your screen lock", "never opens it", "app lock is closed", "asks that website, once", "ES256", "even when its icon and name are disguised"]) {
    check(`the page says: …${rule}…`, t.includes(rule), t);
  }
  await p.shot(`${SP}/shots2/provider-off.png`);

  // ---- the website check for apps is on by default and can be switched off and on
  const appCheck = () => p.eval(`document.querySelector('button[role=switch][aria-label="Check an app with its website"]')?.getAttribute('aria-checked')`);
  check("the website check for apps starts on", (await appCheck()) === "true");
  await p.click('button[role=switch][aria-label="Check an app with its website"]');
  await p.waitFor(`document.querySelector('button[role=switch][aria-label="Check an app with its website"]').getAttribute('aria-checked')==='false'`, { timeout: 5000 });
  check("it switches off", (await appCheck()) === "false");
  await p.click('button[role=switch][aria-label="Check an app with its website"]');
  await p.waitFor(`document.querySelector('button[role=switch][aria-label="Check an app with its website"]').getAttribute('aria-checked')==='true'`, { timeout: 5000 });
  check("and on again", (await appCheck()) === "true");

  // ---- switch on: the button works and says what Android wants
  await p.click('button[role=switch][aria-label="Offer Hexis for passkeys"]');
  await p.waitFor(`document.querySelector('button[role=switch][aria-label="Offer Hexis for passkeys"]').getAttribute('aria-checked')==='true'`, { timeout: 5000 });
  const on = await openButton();
  check("with the switch on the button is enabled", on?.disabled === false);
  check("the page says Android must still be told to use Hexis", (await p.text()).includes("pick Hexis"));
  check("a toast says the same", (await p.text()).includes("Hexis can now offer passkeys"));
  await p.shot(`${SP}/shots2/provider-on.png`);
  await p.click("button", "Open Android settings");
  await sleep(300);
  check("opening Android's settings raises no error", !(await p.text()).includes("Could not open"));

  // ---- the summary in the list follows, and the switch can be turned off again
  await p.click('button[role=switch][aria-label="Offer Hexis for passkeys"]');
  await p.waitFor(`document.querySelector('button[role=switch][aria-label="Offer Hexis for passkeys"]').getAttribute('aria-checked')==='false'`, { timeout: 5000 });
  check("the switch turns off again", (await switchOn()) === "false" && (await p.text()).includes("no longer offers passkeys"));

  // ---- a phone: a list of categories, Passkeys among them with its state
  await p.viewport(400, 820, true);
  await p.goto(base + "?vault=unlocked&filter=settings");
  await p.waitFor(`document.body.innerText.includes('Passkeys')`, { timeout: 15000 }).catch(() => {});
  await p.goto(base + "?vault=unlocked&filter=settings&page=passkeys");
  await p.waitFor(`document.body.innerText.includes('Offer Hexis for passkeys')`, { timeout: 15000 });
  check("on a phone the page opens with a Back button", await p.eval(`!!document.querySelector('button[aria-label="Back to settings"]')`));
  await p.click('button[aria-label="Back to settings"]');
  await p.waitFor(`document.body.innerText.includes('Settings') && !!document.querySelector('ul button')`, { timeout: 5000 });
  const list = await p.text();
  check("the phone's settings list has Passkeys with its state", /Passkeys\s*\n?\s*Off/.test(list), list);
  await p.shot(`${SP}/shots2/provider-phone-list.png`);
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/provider-FAIL.png`).catch(() => {}); }
finally { await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
