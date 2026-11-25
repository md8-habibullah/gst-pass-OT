// The item editor in the browser PREVIEW backend (not the real core): several sign-ins and codes on one login, the Cancel
// confirmation staying in view, a scan or paste that is not a one-time code never staying in the box.
import { launch, sleep } from "./cdp.mjs";
const SP = process.env.SP; const base = "http://localhost:1431/";
let bad = 0; const check = (n, c, extra = "") => { console.log(`${c ? "PASS" : "FAIL"} ${n}${!c && extra ? ` (${extra})` : ""}`); if (!c) bad++; };
const p = await launch({ width: 420, height: 860, port: 9637, profile: `${SP}/chrome-e2e-editor`, mobile: true });
const dialogText = () => p.eval(`(()=>{const d=[...document.querySelectorAll('[role=dialog]')].pop(); return d?d.innerText:''})()`);
/** Is the element with this text fully inside the visible window? */
const inView = (sel, text) => p.eval(`(()=>{const e=[...document.querySelectorAll(${JSON.stringify(sel)})].find(x=>x.textContent.includes(${JSON.stringify(text)})); if(!e) return null; const r=e.getBoundingClientRect(); return r.width>0 && r.top>=0 && r.bottom<=window.innerHeight})()`);
const SEED_A = "JBSWY3DPEHPK3PXP", SEED_B = "ONSWG4TFOQYTEMZU";
try {
  await p.goto(base + "?vault=unlocked&sheet=new&type=login");
  await p.waitFor(`document.body.innerText.includes('New login')`, { timeout: 15000 });

  // ---- Cancel with nothing typed closes at once
  await p.click("button", "Cancel");
  await p.waitFor(`!document.body.innerText.includes('New login')`, { timeout: 4000 });
  check("Cancel on an untouched editor closes it", true);

  await p.goto(base + "?vault=unlocked&sheet=new&type=login");
  await p.waitFor(`document.body.innerText.includes('New login')`, { timeout: 15000 });
  await p.typeLabel("Title", "Amazon");
  await p.typeLabel("Username", "me@example.com");
  await p.typeLabel("Password", "first-pw-123456");
  check("one sign-in is the plain Username and Password boxes (no panel, no Remove)", !(await dialogText()).includes("Sign-in 1") && !(await dialogText()).includes("Remove"));

  // ---- two sign-ins
  await p.click("button", "Add another sign-in");
  await p.waitFor(`document.body.innerText.includes('Sign-in 2')`, { timeout: 4000 });
  check("a second sign-in appears in its own panel next to the first", (await dialogText()).includes("Sign-in 1") && (await dialogText()).includes("Sign-in 2"));
  check("an empty last sign-in does not offer a third", !(await dialogText()).includes("Add another sign-in"));
  await p.type(`[aria-label="Sign-in 2"] input:not([type=password])`, "work@example.com");
  await p.type(`[aria-label="Sign-in 2"] input[type=password]`, "work-pw-123456");
  check("a filled second sign-in offers a third", (await dialogText()).includes("Add another sign-in"));
  await p.click("button", "Add another sign-in");
  await p.waitFor(`document.body.innerText.includes('Sign-in 3')`, { timeout: 4000 });
  await p.click("button", "Remove sign-in 3");
  await p.waitFor(`!document.body.innerText.includes('Sign-in 3')`, { timeout: 4000 });
  await p.click("button", "Remove sign-in 2");
  await p.waitFor(`!document.body.innerText.includes('Sign-in 2')`, { timeout: 4000 });
  check("Remove takes the sign-in out and the first goes back to plain boxes", !(await dialogText()).includes("Sign-in 1"));
  await p.click("button", "Add another sign-in");
  await p.type(`[aria-label="Sign-in 2"] input:not([type=password])`, "work@example.com");
  await p.type(`[aria-label="Sign-in 2"] input[type=password]`, "work-pw-123456");

  // ---- codes
  await p.typeLabel("One-time code", SEED_A);
  await p.waitFor(`document.body.innerText.includes('Recognised')`, { timeout: 4000 });
  check("the Scan and image buttons go away once the box has a code", await p.eval(`(()=>{const b=[...document.querySelectorAll('[role=dialog] button')].find(x=>x.textContent.includes('Choose an image')||x.textContent.includes('Read a QR image')); return !b || b.offsetParent===null || getComputedStyle(b.parentElement).display==='none'})()`));
  await p.click("button", "Add another code");
  await p.waitFor(`document.body.innerText.includes('One-time code 2')`, { timeout: 4000 });
  check("a second code gets its own box, numbered", (await dialogText()).includes("One-time code 1"));

  // ---- something that is not a code never sits in the box silently
  await p.typeLabel("One-time code 2", "https://www.lenovo.com/us/en/");
  await sleep(200);
  check("a web address in a code box is explained", /not|valid|secret|link/i.test(await dialogText()));
  // the Clear button of the second box (the first box has one too)
  const clear = await p.eval(`(()=>{const b=[...document.querySelectorAll('[role=dialog] button[aria-label="Clear this code"]')].pop(); b.scrollIntoView({block:'center'}); const r=b.getBoundingClientRect(); return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);
  await p.mouse(clear.x, clear.y);
  await sleep(150);
  check("the clear button empties the box", (await p.valueOf("One-time code 2")) === "");
  await p.typeLabel("One-time code 2", SEED_B);

  await p.shot(`${SP}/shots2/editor-two.png`);
  // ---- Cancel with changes: the question is on screen, in the footer
  await p.click("button", "Cancel");
  await p.waitFor(`document.body.innerText.includes('Discard your changes?')`, { timeout: 4000 });
  check("Cancel with changes asks, and the question is inside the visible window", (await inView("[role=alertdialog]", "Discard your changes?")) === true);
  await p.click("button", "Keep editing");
  await p.waitFor(`!document.body.innerText.includes('Discard your changes?')`, { timeout: 4000 });
  check("Keep editing goes back to the form with everything still typed", (await p.valueOf("Title")) === "Amazon");

  // ---- save and look
  await p.click("button", "Add item");
  await p.waitFor(`!document.body.innerText.includes('New login') && document.body.innerText.includes('Amazon')`, { timeout: 8000 });
  await p.waitFor(`document.body.innerText.includes('One-time code 2')`, { timeout: 8000 });
  const detail = await p.text();
  check("the saved login shows its code twice, numbered", detail.includes("One-time code 1") && detail.includes("One-time code 2"));
  check("the saved login shows both sign-ins, paired", /Username 1[\s\S]*Password 1[\s\S]*Username 2[\s\S]*Password 2/.test(detail));
  await p.shot(`${SP}/shots2/editor-detail.png`);

  // ---- editing it: Cancel after a change, then Discard really closes
  await p.click("button", "Edit");
  await p.waitFor(`document.body.innerText.includes('Edit Amazon')`, { timeout: 8000 });
  await p.typeLabel("Title", "Amazon EU");
  await p.click("button", "Cancel");
  await p.waitFor(`document.body.innerText.includes('Discard your changes?')`, { timeout: 4000 });
  await p.shot(`${SP}/shots2/editor-discard.png`);
  await p.click("button", "Discard");
  await p.waitFor(`!document.body.innerText.includes('Edit Amazon')`, { timeout: 4000 });
  check("Discard closes the editor and keeps the saved title", (await p.text()).includes("Amazon") && !(await p.text()).includes("Amazon EU"));

  // ---- the Codes page lists both codes of the login
  await p.goto(base + "?vault=unlocked&filter=codes");
  await p.waitFor(`document.body.innerText.includes('Codes')`, { timeout: 8000 });
  await sleep(600);
  await p.shot(`${SP}/shots2/editor-codes.png`);
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/editor-FAIL.png`).catch(() => {}); }
finally { await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
