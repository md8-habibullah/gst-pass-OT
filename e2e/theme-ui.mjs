// Light, dark and "same as the device" in a real browser: the preview backend, the emulated device setting (prefers-color-scheme),
// and the rule that the page keeps nothing of its own (no local storage, no cookie).
import { launch, sleep } from "./cdp.mjs";
const SP = process.env.SP; const base = "http://localhost:1431/";
let bad = 0; const check = (n, c, extra = "") => { console.log(`${c ? "PASS" : "FAIL"} ${n}${c ? "" : `   -- ${extra}`}`); if (!c) bad++; };
const LIGHT = "rgb(231, 238, 240)", DARK = "rgb(8, 20, 26)";
const p = await launch({ width: 1000, height: 700, port: 9751, profile: `${SP}/chrome-e2e-theme` });
const bg = () => p.eval(`getComputedStyle(document.body).backgroundColor`);
const attr = () => p.eval(`document.documentElement.dataset.theme ?? 'none'`);
try {
  for (const os of ["light", "dark"]) {
    await p.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: os }] });
    for (const [q, want] of [["", os], ["&theme=system", os], ["&theme=light", "light"], ["&theme=dark", "dark"]]) {
      await p.goto(`${base}?vault=locked${q}`); await sleep(1200);
      const got = await bg();
      check(`device ${os}, theme '${q || "default"}' paints ${want}`, got === (want === "light" ? LIGHT : DARK), `${got} data-theme=${await attr()}`);
    }
  }
  await p.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "light" }] });
  await p.goto(`${base}?vault=unlocked&filter=settings&page=theme`);
  await p.waitFor(`document.body.innerText.includes('Pale panes and dark ink')`, { timeout: 8000 });
  await p.click("button[role=radio]", "Dark"); await sleep(400);
  check("choosing Dark paints dark and writes data-theme", (await bg()) === DARK && (await attr()) === "dark");
  check("the Dark choice is marked as selected", await p.eval(`[...document.querySelectorAll('[role=radiogroup][aria-label=Theme] [role=radio]')].find(b=>b.textContent.includes('Dark')).getAttribute('aria-checked') === 'true'`));
  await p.click("button[role=radio]", "Device setting"); await sleep(400);
  check("choosing Device setting leaves no attribute and follows the device (light here)", (await attr()) === "none" && (await bg()) === LIGHT);
  check("the page keeps nothing: no local storage, no cookie", await p.eval(`localStorage.length === 0 && document.cookie === ''`), JSON.stringify(await p.eval(`Object.keys(localStorage)`)));
  // text stays readable where it matters most: the primary text and the accent text against their real backgrounds
  const ratio = await p.eval(`(()=>{const rgb=(c)=>c.match(/\\d+/g).slice(0,3).map(Number);const lin=(v)=>{v/=255;return v<=0.04045?v/12.92:((v+0.055)/1.055)**2.4};const L=(c)=>{const [r,g,b]=rgb(c);return 0.2126*lin(r)+0.7152*lin(g)+0.0722*lin(b)};const el=document.querySelector('h2');const a=L(getComputedStyle(el).color),b=L(getComputedStyle(document.body).backgroundColor);return (Math.max(a,b)+0.05)/(Math.min(a,b)+0.05)})()`);
  check("a page title is at least 7:1 against the ground", ratio >= 7, ratio);
  console.log("logs:", JSON.stringify(p.logs.slice(0, 5)));
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/theme-FAIL.png`).catch(() => {}); }
finally { await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
