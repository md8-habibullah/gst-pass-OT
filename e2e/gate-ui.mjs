// Gate interactions against the DEV preview backend (same rules as the core): drag pattern, taps, long-press reset, panic mark.
import { launch, sleep } from "./cdp.mjs";
const SP = process.env.SP; const base = "http://localhost:1431/";
let bad = 0; const check = (n, c) => { console.log(`${c ? "PASS" : "FAIL"} ${n}`); if (!c) bad++; };
const p = await launch({ width: 420, height: 880, mobile: true, port: 9611, profile: `${SP}/chrome-e2e-gate` });
const centre = (digit) => p.eval(`(()=>{const r=document.querySelector('[data-node="${digit}"]').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);
// An erase ends in a fresh demo vault (its list shows the Hexis persona's items), not on a create screen. The phone layout has no account chip.
const DEMO = `document.body.innerText.includes('hexis@gmail.com') && !document.querySelector('[data-node]')`;
const mouse = (type, x, y) => p.send("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons: type === "mouseReleased" ? 0 : 1, clickCount: 1 });
try {
  await p.goto(base + "?vault=gate&gate=grid&panic=1"); await p.waitFor(`!!document.querySelector('[data-node="1"]')`);
  // 1. a drag across 1,2,5,8,9 shows the live line and, on release, opens the app
  const pts = [];
  for (const d of "12589") pts.push(await centre(d));
  await mouse("mousePressed", pts[0].x, pts[0].y);
  for (let i = 1; i < pts.length; i++) {
    const a = pts[i - 1], b = pts[i];
    for (let s = 1; s <= 6; s++) { await mouse("mouseMoved", a.x + ((b.x - a.x) * s) / 6, a.y + ((b.y - a.y) * s) / 6); await sleep(15); }
  }
  check("live line is drawn while dragging", await p.eval(`document.querySelectorAll('svg polyline').length > 0`));
  await p.shot(`${SP}/shots2/g-grid-drag.png`);
  await mouse("mouseReleased", pts[4].x, pts[4].y);
  await p.waitFor(`document.body.innerText.includes('Master password') || document.body.innerText.includes('Choose an account')`, { timeout: 6000 });
  check("a pattern of 5 numbers opens Stage 1, and the master password is still asked", await p.eval(`document.body.innerText.includes('Master password')`));

  // 2. taps type digits; a 2-digit tap sequence is only a math answer (no attempts counted)
  await p.goto(base + "?vault=gate&gate=grid"); await p.waitFor(`!!document.querySelector('[data-node="1"]')`);
  for (const d of "12") { const c = await centre(d); await mouse("mousePressed", c.x, c.y); await mouse("mouseReleased", c.x, c.y); }
  check("tapping a node types its digit", (await p.eval(`document.querySelector('[role=status][aria-label="Your answer"]').textContent`)).includes("12"));
  await p.click("button", "Submit"); await sleep(500);
  check("1 to 3 characters never show the attempts banner", !(await p.eval(`document.body.innerText.includes('attempts left')`)));

  // 3. a wrong 5-digit grid entry counts
  for (const d of "11111") { const c = await centre(d === "1" ? "1" : d); await mouse("mousePressed", c.x, c.y); await mouse("mouseReleased", c.x, c.y); }
  await p.click("button", "Submit"); await p.waitFor(`document.body.innerText.includes('15 attempts left')`);
  check("a wrong 4+ character entry counts and shows the remaining attempts", true);

  // 4. the reversed pattern wipes silently and lands on first run
  await p.goto(base + "?vault=gate&gate=grid"); await p.waitFor(`!!document.querySelector('[data-node="1"]')`);
  const rev = []; for (const d of "98521") rev.push(await centre(d));
  await mouse("mousePressed", rev[0].x, rev[0].y);
  for (let i = 1; i < rev.length; i++) { const a = rev[i - 1], b = rev[i]; for (let s = 1; s <= 6; s++) { await mouse("mouseMoved", a.x + ((b.x - a.x) * s) / 6, a.y + ((b.y - a.y) * s) / 6); await sleep(15); } }
  await mouse("mouseReleased", rev[4].x, rev[4].y);
  await p.waitFor(DEMO, { timeout: 8000 });
  const t = await p.eval(`document.body.innerText.toLowerCase()`);
  check("reversed pattern lands in a fresh demo vault with no wipe notice", !["erase", "wipe", "panic", "deleted"].some((w) => t.includes(w)));

  // 5. panic mark: a double tap wipes
  await p.goto(base + "?vault=gate&gate=pin&panic=1"); await p.waitFor(`!!document.querySelector('[aria-label="Hold or double-tap to clear"]')`);
  const m = await p.eval(`(()=>{const r=document.querySelector('[aria-label="Hold or double-tap to clear"]').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);
  for (let i = 0; i < 2; i++) { await mouse("mousePressed", m.x, m.y); await mouse("mouseReleased", m.x, m.y); await sleep(120); }
  await p.waitFor(DEMO, { timeout: 8000 });
  check("double-tap on the panic mark lands in a fresh demo vault", true);

  // 6. a single tap does nothing
  await p.goto(base + "?vault=gate&gate=pin&panic=1"); await p.waitFor(`!!document.querySelector('[aria-label="Hold or double-tap to clear"]')`);
  await mouse("mousePressed", m.x, m.y); await mouse("mouseReleased", m.x, m.y); await sleep(700);
  check("a single tap on the panic mark does nothing", await p.eval(`!(${DEMO})`));

  // 7. three-step reset
  await p.goto(base + "?vault=gate&gate=pin"); await p.waitFor(`!!document.querySelector('[aria-label="Reset app data"]')`);
  await p.click("button", "Reset app data");
  for (const label of ["Continue", "I understand, continue"]) { await p.waitFor(`[...document.querySelectorAll('[role=dialog] button')].some(b=>b.textContent.trim()===${JSON.stringify(label)})`); await p.click("[role=dialog] button", label); await sleep(450); }
  check("the third warning is a separate step", await p.eval(`document.body.innerText.includes('Erase everything now?')`));
  const sumOf = () => p.eval(`(()=>{const e=document.querySelector('[role=dialog] [aria-label^="What is "]'); return e? e.getAttribute('aria-label').slice(8,-1):null})()`);
  const solve = (eq) => { if (!/^[\d+\-*/() ×÷−]+$/.test(eq)) throw new Error("unexpected sum " + eq); return Function(`"use strict";return (${eq.replace(/×/g, "*").replace(/÷/g, "/").replace(/−/g, "-")})`)(); };
  await p.waitFor(`!!document.querySelector('[role=dialog] [aria-label^="What is "]')`, { timeout: 6000 });
  const sum = await sumOf(); const answer = solve(sum);
  check("the last step shows a long sum (the answer has three to five digits)", answer >= 100 && answer <= 99999, `${sum} = ${answer}`);
  check("the erase button is off until an answer is typed", await p.eval(`[...document.querySelectorAll('[role=dialog] button')].find(b=>b.textContent.trim()==='Erase everything').disabled`));
  await p.typeLabel("Your answer", String(answer + 7)); await p.click("[role=dialog] button", "Erase everything");
  await p.waitFor(`document.body.innerText.includes('That is not the answer')`, { timeout: 6000 });
  check("a wrong answer erases nothing and says so", await p.eval(`!(${DEMO}) && !!document.querySelector('[aria-label="Reset app data"]')`));
  await sleep(400);
  const sum2 = await sumOf(); const answer2 = solve(sum2);
  await p.typeLabel("Your answer", String(answer2)); await p.click("[role=dialog] button", "Erase everything"); await p.waitFor(DEMO, { timeout: 8000 });
  check("the right answer to the new sum brings the app back to a fresh demo vault", true);
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/g-ui-FAIL.png`).catch(() => {}); }
finally { await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
