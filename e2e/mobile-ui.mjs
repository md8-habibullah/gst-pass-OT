import { launch, sleep } from "./cdp.mjs";
const SP = process.env.SP; const base = "http://localhost:1431/";
let bad = 0; const check = (n, c) => { console.log(`${c ? "PASS" : "FAIL"} ${n}`); if (!c) bad++; };
const hide = (p, on) => p.eval(`(()=>{Object.defineProperty(document,'visibilityState',{configurable:true,get:()=>${on ? "'hidden'" : "'visible'"}});document.dispatchEvent(new Event('visibilitychange'));})()`);
const p = await launch({ width: 420, height: 860, port: 9635, profile: `${SP}/chrome-e2e-mobile`, mobile: true });
try {
  await p.send("Emulation.setUserAgentOverride", { userAgent: "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130 Mobile Safari/537.36" });
  await p.goto(base + "?vault=unlocked");
  await p.waitFor(`!!document.querySelector('[role=list], ul, main')`); await sleep(1200);
  check("the vault is open", !(await p.text()).includes("Unlock"));
  // The default is five minutes away: a short trip to another app must not lock anything, and nothing is asked on return.
  await hide(p, true); await sleep(1500);
  check("a short trip to another app does not lock the vault", !(await p.text()).includes("Unlock"));
  await hide(p, false); await sleep(800);
  check("coming back in time finds the vault still open", !(await p.text()).includes("Unlock"));
  // Settings: choose "Right away", then leaving locks at once.
  await p.goto(base + "?vault=unlocked&filter=settings");
  // On a phone Settings is a list of pages: open "App lock" to reach the setting.
  await p.waitFor(`!document.querySelector('select[aria-label="Lock after leaving the app for"]') && document.body.innerText.includes('App lock')`, { timeout: 8000 });
  check("a phone opens Settings on a list of pages, not one long page", await p.eval(`!document.body.innerText.includes('Lock after leaving')`));
  await p.click("button", "App lock");
  await p.waitFor(`!!document.querySelector('select[aria-label="Lock after leaving the app for"]')`, { timeout: 8000 });
  check("the leave setting is offered on a phone", true);
  const options = await p.eval(`[...document.querySelector('select[aria-label="Lock after leaving the app for"]').options].map(o=>o.textContent.trim())`);
  check("the choices run from right away to hours", options[0] === "Right away" && options.length === 9, options.join(", "));
  await p.eval(`(()=>{const el=document.querySelector('select[aria-label="Lock after leaving the app for"]'); Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(el,'0'); el.dispatchEvent(new Event('change',{bubbles:true}));})()`);
  await sleep(500);
  check("the choice is kept", (await p.eval(`document.querySelector('select[aria-label="Lock after leaving the app for"]').selectedOptions[0].textContent.trim()`)) === "Right away");
  await hide(p, true); await p.waitFor(`document.body.innerText.includes('Unlock')`, { timeout: 8000 });
  check("with Right away, going to the background locks the vault (master password needed again)", true);
  await p.shot(`${SP}/shots2/m-locked.png`);
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/m-FAIL.png`).catch(() => {}); }
finally { await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
