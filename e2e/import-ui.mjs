import { launch, sleep } from "./cdp.mjs";
import { writeFileSync, mkdirSync } from "node:fs";
const SP = process.env.SP; const base = "http://localhost:1431/";
mkdirSync(`${SP}/e2e-files`, { recursive: true });
let bad = 0; const check = (n, c) => { console.log(`${c ? "PASS" : "FAIL"} ${n}`); if (!c) bad++; };
writeFileSync(`${SP}/e2e-files/bw2.json`, JSON.stringify({ encrypted: false, folders: [], items: [
  { type: 1, name: "GitHub", login: { username: "marlowe-dev", password: "kX9#mQ2vTz!8pLd4", uris: [{ uri: "https://github.com" }] } },
  { type: 1, name: "Brand new site", login: { username: "me", password: "zz-pass-91", uris: [] } },
  { type: 2, name: "A note", notes: "body" },
] }));
writeFileSync(`${SP}/e2e-files/odd.csv`, "Site Label,Login ID,Secret,Link\nOdd Bank,odd-user,odd-pass-77,https://odd.example\n");
const p = await launch({ width: 1200, height: 1000, port: 9655, profile: `${SP}/chrome-e2e-import` });
const pickFile = (f, label) => p.chooseFiles([f], () => p.click("button", label));
try {
  await p.goto(base + "?vault=unlocked&filter=import"); await p.waitFor(`document.body.innerText.includes('No file chosen')`);
  await p.shot(`${SP}/shots2/i-empty.png`);
  await pickFile(`${SP}/e2e-files/bw2.json`, "Choose a file"); await p.waitFor(`document.body.innerText.includes('Looks like Bitwarden (JSON)')`);
  check("the format is recognised from the content", true);
  await p.click("button", "Check the file"); await p.waitFor(`document.body.innerText.includes('3 entries ready')`);
  const t = await p.eval(`document.body.innerText`);
  check("preview counts duplicates (GitHub already exists)", /1 duplicate found/.test(t));
  await p.shot(`${SP}/shots2/i-preview.png`);
  await p.click("button", "Import 3 items"); await p.waitFor(`document.body.innerText.includes('Delete it from your disk')`);
  const t2 = await p.eval(`document.body.innerText`);
  check("skip-duplicates leaves the existing item alone and adds the rest", /2 items imported/.test(t2) && /1 skipped/.test(t2));
  check("a reminder to delete the unencrypted source file is shown", true);
  await p.shot(`${SP}/shots2/i-done.png`);

  // unknown CSV -> custom mapper
  await pickFile(`${SP}/e2e-files/odd.csv`, "Choose a file").catch(() => undefined);
  await p.waitFor(`document.body.innerText.includes('odd.csv')`);
  await p.eval(`(()=>{const s=document.querySelector('select');s.value='custom-csv';s.dispatchEvent(new Event('change',{bubbles:true}))})()`);
  await p.waitFor(`document.body.innerText.includes('Match the columns')`);
  const mapped = await p.eval(`[...document.querySelectorAll('section[aria-label="Choose a file"] select')].slice(1).map(s=>s.value)`);
  check("the column mapper guesses what it can", mapped[0] === "Site Label" && mapped[1] === "Login ID");
  await p.shot(`${SP}/shots2/i-mapper.png`);

  await p.goto(base + "?vault=unlocked&filter=export"); await p.waitFor(`document.body.innerText.includes('Readable copy for another app')`); await sleep(400);
  check("export page states plainly that the readable copy is not encrypted", (await p.eval(`document.body.innerText`)).includes("Nothing in it is encrypted"));
  await p.shot(`${SP}/shots2/i-export.png`);
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/i-FAIL.png`).catch(() => {}); }
finally { await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
