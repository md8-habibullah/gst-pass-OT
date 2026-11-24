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
  await hide(p, true); await p.waitFor(`document.body.innerText.includes('Unlock')`, { timeout: 8000 });
  check("going to the background locks the vault (master password needed again)", true);
  await p.shot(`${SP}/shots2/m-locked.png`);
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/m-FAIL.png`).catch(() => {}); }
finally { await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
