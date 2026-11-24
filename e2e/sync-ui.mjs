import { launch, sleep } from "./cdp.mjs";
const SP = process.env.SP; const base = "http://localhost:1431/";
let bad = 0; const check = (n, c) => { console.log(`${c ? "PASS" : "FAIL"} ${n}`); if (!c) bad++; };
const p = await launch({ width: 1200, height: 860, port: 9634, profile: `${SP}/chrome-e2e-sync` });
try {
  await p.goto(base + "?vault=unlocked&open=0");
  await p.waitFor(`!!document.querySelector('article')`); await sleep(500);
  check("no other-device note on a normal item", !(await p.eval(`document.body.innerText`)).includes("Changed on another device"));
  await p.goto(base + "?vault=unlocked&open=0&otherdevice=1");
  await p.waitFor(`document.body.innerText.includes('Changed on another device')`); await sleep(400);
  check("the note appears when another device diverged", true); await p.shot(`${SP}/shots2/s-note.png`);
  await p.click("button", "Show history"); await p.waitFor(`document.body.innerText.includes('From another device')`);
  check("history opens and labels the other device's copy", true);
  const cur = await p.eval(`[...document.querySelectorAll('ol li')].filter(li=>li.innerText.includes('Current')).length`);
  check("exactly one version is marked Current", cur === 1);
  await p.shot(`${SP}/shots2/s-history.png`);
  await p.click("button", "Got it"); await p.waitFor(`!document.body.innerText.includes('Changed on another device')`);
  check("'Got it' clears the note and the copy stays in the history", (await p.eval(`document.body.innerText`)).includes("From another device"));
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/s-FAIL.png`).catch(() => {}); }
finally { if (p.logs.length) console.log("console:", [...new Set(p.logs)].slice(0, 6)); await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
