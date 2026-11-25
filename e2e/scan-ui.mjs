// The all-in-one scanner and the passkey rows in the browser PREVIEW backend (not the real core): what a scan is told to do,
// suggestions by account / site / name, "make a new login", any other login, the master-password question of a login that
// asks, and a passkey shown (never its key) and removable in the editor. The real core's own rules are in real-scan.mjs.
import { launch, sleep } from "./cdp.mjs";
const SP = process.env.SP; const base = "http://localhost:1431/";
let bad = 0; const check = (n, c, extra = "") => { console.log(`${c ? "PASS" : "FAIL"} ${n}${!c && extra ? ` (${extra})` : ""}`); if (!c) bad++; };
const p = await launch({ width: 1100, height: 860, port: 9641, profile: `${SP}/chrome-e2e-scan` });
const dialog = () => p.eval(`(()=>{const d=[...document.querySelectorAll('[role=dialog]')].pop(); return d?d.innerText:''})()`);
const KEY = "KRSXG5CTMVRXEZLUKN2XAZLT";
const uri = (issuer, account, secret = KEY) => `otpauth://totp/${encodeURIComponent(issuer)}:${encodeURIComponent(account)}?secret=${secret}&issuer=${encodeURIComponent(issuer)}`;
/** Put text in the scanner's box and press "Check it". */
async function check_(text) {
  await p.typeLabel("Or paste a setup key or link", text);
  await p.click("button", "Check it");
}
let loaded = false;
/** The scanner's first screen. The preview keeps its vault only until the page reloads, so after the first time it goes on in the same page. */
async function fresh() {
  if (!loaded) {
    loaded = true;
    await p.goto(base + "?vault=unlocked&sheet=scan");
  } else if (!(await p.eval(`!!document.querySelector('[role=dialog]')`))) {
    await p.click('button[aria-label="Scan a code or passkey"]');
  } else {
    const label = (await dialog()).includes("Scan something else") ? "Scan something else" : "Scan another";
    await p.click("button", label);
  }
  await p.waitFor(`document.body.innerText.includes('Or paste a setup key or link')`, { timeout: 15000 });
}
try {
  // ---- the camera button is in the list header and opens the scanner
  await p.goto(base + "?vault=unlocked");
  await p.waitFor(`!!document.querySelector('button[aria-label="Scan a code or passkey"]')`, { timeout: 15000 });
  await p.click('button[aria-label="Scan a code or passkey"]');
  await p.waitFor(`document.body.innerText.includes('Or paste a setup key or link')`, { timeout: 5000 });
  check("the camera button opens the scanner", true);
  check("on a computer it offers a picture, not a camera", (await dialog()).includes("Choose a picture of the QR code") && !(await dialog()).includes("Open the camera"));
  await p.shot(`${SP}/shots2/scan-start.png`);

  // ---- a code for an account that exists: the logins that fit, the strongest first
  await fresh();
  await check_(uri("Fastmail", "marlowe@fastmail.example"));
  await p.waitFor(`document.body.innerText.includes('One-time code for Fastmail')`, { timeout: 5000 });
  let t = await dialog();
  check("it says what the code is for", t.includes("marlowe@fastmail.example") && t.includes("6 digits"));
  check("it suggests the login with the same account, website and name first", /Fastmail[\s\S]*Same account, Same website, Same name/.test(t), t);
  check("a login with only the same account is suggested too", /Cloudflare[\s\S]*Same account/.test(t), t);
  check("making a new login is offered next to the suggestions", t.includes("Or make a new login"));
  await p.shot(`${SP}/shots2/scan-suggest.png`);
  await p.click(`button[aria-label="Add the code to Fastmail"]`);
  await p.waitFor(`document.body.innerText.includes('Added the code to “Fastmail”')`, { timeout: 5000 });
  check("choosing a suggestion saves the code on that login", true);
  await p.click("button", "Open Fastmail");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 5000 });
  await p.waitFor(`document.body.innerText.includes('One-time code 2')`, { timeout: 5000 });
  check("the login opens with its second one-time code", true);

  // ---- the same code again: it is on the login already
  await fresh();
  await check_(uri("Fastmail", "marlowe@fastmail.example"));
  await p.waitFor(`document.body.innerText.includes('Already has this code')`, { timeout: 5000 });
  check("a code the login already holds cannot be added twice", await p.eval(`document.querySelector('button[aria-label="Add the code to Fastmail"]').disabled`));

  // ---- an account that does not exist: say so and offer a new login
  await fresh();
  await check_(uri("Zorg Labs", "someone@new.example", "NB2HI4DTHIXS6Z3PNUXG4ZZO"));
  await p.waitFor(`document.body.innerText.includes('No login in this vault matches someone@new.example')`, { timeout: 5000 });
  t = await dialog();
  check("with no match it offers a new login named after the issuer", (await p.valueOf("Make a new login")) === "Zorg Labs", t);
  await p.shot(`${SP}/shots2/scan-new.png`);
  // any other login can be chosen too
  await p.click("button", "Add to another login");
  await p.typeAria("Search logins", "hetz");
  await p.waitFor(`!!document.querySelector('button[aria-label="Add the code to Hetzner Cloud"]')`, { timeout: 5000 });
  check("any other login can be searched and chosen", !(await dialog()).includes("Proxmox"));
  await p.click("button", "Make login");
  await p.waitFor(`document.body.innerText.includes('Made the login “Zorg Labs”')`, { timeout: 5000 });
  check("Make login creates the login for the code", true);
  await p.click("button", "Open Zorg Labs");
  await p.waitFor(`!document.querySelector('[role=dialog]')`, { timeout: 5000 });
  await p.waitFor(`document.body.innerText.includes('someone@new.example')`, { timeout: 5000 });

  // ---- the other kinds
  await fresh();
  await check_("FIDO:/1234567890123456789012345678901234567890");
  await p.waitFor(`document.body.innerText.includes('sign in with a passkey')`, { timeout: 5000 });
  check("a passkey sign-in code from a computer is recognised and explained", (await dialog()).includes("meant for a phone"));
  await fresh();
  await check_("https://www.example.com/page?token=abc");
  await p.waitFor(`document.body.innerText.includes('A link to example.com')`, { timeout: 5000 });
  check("a web link is a link, and only its host is offered for a new login", (await dialog()).includes("Make a login for example.com") && !(await dialog()).includes("token=abc"));
  await fresh();
  await check_("otpauth-migration://offline?data=abc");
  await p.waitFor(`document.body.innerText.includes('Several accounts')`, { timeout: 5000 });
  check("an authenticator export is offered as separate codes", (await dialog()).includes("Add them as separate codes"));
  await fresh();
  await check_("hello world, this is just text!");
  await p.waitFor(`document.body.innerText.includes('Nothing to save from this')`, { timeout: 5000 });
  check("plain text is not saved", true);
  await fresh();
  await check_("otpauth://totp/x?secret=!!!");
  await p.waitFor(`document.body.innerText.includes('cannot be used')`, { timeout: 5000 });
  check("a broken code link is explained", true);
  // a bare setup key works only when it is pasted
  await fresh();
  await check_("ORSXG5DJNZTSA43FNZ2GK4TF");
  await p.waitFor(`document.body.innerText.includes('One-time code')`, { timeout: 5000 });
  check("a pasted setup key without a name is a code too", (await dialog()).includes("No login in this vault matches"));

  // ---- passkeys: shown by website and account, no key, removable in the editor
  await p.goto(base + "?vault=unlocked&filter=logins&open=0&passkey=1");
  await p.waitFor(`document.body.innerText.includes('Passkey')`, { timeout: 15000 });
  await sleep(300);
  const detail = await p.text();
  check("the item shows its passkeys by website and account", detail.includes("example.com") && detail.includes("github.com") && detail.includes("hexis-dev"));
  check("no copy or reveal button belongs to a passkey", await p.eval(`(()=>{const rows=[...document.querySelectorAll('p')].filter(x=>/^Passkey( \\d+)?$/.test(x.textContent.trim())); return rows.length===2 && rows.every(r=>!r.parentElement.parentElement.querySelector('button'))})()`));
  await p.shot(`${SP}/shots2/scan-passkey-detail.png`);
  await p.click("button", "Edit");
  await p.waitFor(`!!document.querySelector('button[aria-label^="Remove passkey 1"]')`, { timeout: 8000 });
  check("the editor lists the passkeys with Remove only", (await dialog()).includes("Passkeys") && !(await dialog()).includes("Add another passkey") && !!(await p.eval(`!!document.querySelector('button[aria-label^="Remove passkey 2"]')`)));
  await p.click(`button[aria-label^="Remove passkey 1"]`);
  await sleep(200);
  check("Remove takes one passkey out", !(await p.eval(`!!document.querySelector('button[aria-label^="Remove passkey 2"]')`)));
  await p.shot(`${SP}/shots2/scan-passkey-edit.png`);
  await p.click("button", "Save changes");
  await p.waitFor(`!document.querySelector('button[aria-label^="Remove passkey"]')`, { timeout: 8000 });
  await sleep(300);
  const after = await p.text();
  check("after saving, one passkey is left", (after.includes("example.com") ? 1 : 0) + (after.includes("github.com") ? 1 : 0) === 1, after);
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/scan-FAIL.png`).catch(() => {}); }
finally { await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
