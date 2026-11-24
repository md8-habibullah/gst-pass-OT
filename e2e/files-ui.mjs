import { launch, sleep } from "./cdp.mjs";
import { writeFileSync, mkdirSync } from "node:fs";
const SP = process.env.SP; const base = "http://localhost:1431/";
mkdirSync(`${SP}/e2e-files`, { recursive: true });
let bad = 0; const check = (n, c) => { console.log(`${c ? "PASS" : "FAIL"} ${n}`); if (!c) bad++; };
const p = await launch({ width: 1200, height: 860, port: 9633, profile: `${SP}/chrome-e2e-files` });
try {
  await p.goto(base + "?vault=unlocked&filter=files"); await p.waitFor(`document.body.innerText.includes('recovery-notes.txt')`);
  await sleep(500); await p.shot(`${SP}/shots2/f-list.png`);
  check("seeded files are listed with sizes", (await p.eval(`document.body.innerText`)).includes("insurance-receipt.pdf"));

  await p.click("button", "Preview recovery-notes.txt"); await p.waitFor(`document.body.innerText.includes('Spare router login')`);
  check("text preview shows the decrypted text", true); await p.shot(`${SP}/shots2/f-text.png`); await p.press("Escape"); await sleep(400);

  await p.click("button", "Preview lakehouse-floorplan.svg"); await p.waitFor(`!!document.querySelector('[role=dialog] img[alt="lakehouse-floorplan.svg"]')`);
  await sleep(500); check("image preview renders (blob URL)", await p.eval(`(()=>{const i=document.querySelector('[role=dialog] img[alt="lakehouse-floorplan.svg"]');return i.complete&&i.naturalWidth>0&&i.src.startsWith('blob:')})()`));
  await p.shot(`${SP}/shots2/f-image.png`); await p.press("Escape"); await sleep(400);

  await p.click("button", "Preview insurance-receipt.pdf"); await p.waitFor(`!!document.querySelector('[role=dialog] canvas')`, { timeout: 15000 });
  await sleep(800); check("pdf preview draws a page on a canvas", await p.eval(`(()=>{const c=document.querySelector('[role=dialog] canvas');return c.width>100 && [...c.getContext('2d').getImageData(0,0,c.width,c.height).data].some((v,i)=>i%4===0&&v<128)})()`));
  await p.shot(`${SP}/shots2/f-pdf.png`); await p.press("Escape"); await sleep(400);

  await p.click("button", "Open archive-2024.zip"); await p.waitFor(`document.body.innerText.includes('cannot be shown inside the app')`);
  check("unknown types explain themselves and still offer a copy", true); await p.press("Escape"); await sleep(400);

  // adding files, and the 50 MB limit (a sparse 51 MB file: the limit message must name the file)
  writeFileSync(`${SP}/e2e-files/new-note.txt`, "hello drive");
  writeFileSync(`${SP}/e2e-files/huge.bin`, Buffer.alloc(52_428_801));
  await p.chooseFiles([`${SP}/e2e-files/new-note.txt`], () => p.click("button", "Add files")); await p.waitFor(`document.body.innerText.includes('new-note.txt')`);
  check("a small file is added and listed", true);
  await p.chooseFiles([`${SP}/e2e-files/huge.bin`], () => p.click("button", "Add files")); await p.waitFor(`document.body.innerText.includes('huge.bin is larger than 50 MB')`);
  check("a file over 50 MB is refused with its name", true);

  // rename + delete (re-prompt)
  await p.click("button", "Rename new-note.txt"); await p.typeLabel("File name", "renamed-note.txt"); await p.click("[role=dialog] button[type=submit]");
  await p.waitFor(`document.body.innerText.includes('renamed-note.txt')`); check("rename works", true);
  await p.click("button", "Delete renamed-note.txt"); await p.waitFor(`document.body.innerText.includes('Confirm it is you')`);
  await p.typeLabel("Master password", "preview-vault-1"); await p.press("Enter");
  await p.waitFor(`!document.body.innerText.includes('renamed-note.txt')`); check("delete asks for the master password and removes the file", true);

  // attachments inside an item
  await p.goto(base + "?vault=unlocked&open=1"); await p.waitFor(`document.body.innerText.includes('Attachments')`); await sleep(600);
  check("an item shows its own attachments section", (await p.eval(`document.body.innerText`)).includes("No files attached to this item."));
  await p.chooseFiles([`${SP}/e2e-files/new-note.txt`], () => p.click("section[aria-labelledby^=att-] button", "Add files")); await p.waitFor(`!!document.querySelector('section[aria-labelledby^=att-] li')`);
  check("a file can be attached to an item", true); await p.shot(`${SP}/shots2/f-attach.png`);
  await p.goto(base + "?vault=unlocked&filter=files"); await p.waitFor(`document.body.innerText.includes('recovery-notes.txt')`);
  check("attachments stay out of the standalone Files list", !(await p.eval(`document.body.innerText`)).includes("new-note.txt"));
} catch (e) { bad++; console.log("ERROR", e.message); await p.shot(`${SP}/shots2/f-FAIL.png`).catch(() => {}); }
finally { if (p.logs.length) console.log("console:", [...new Set(p.logs)].slice(0, 6)); await p.close(); }
console.log(bad ? `${bad} failed` : "all passed"); process.exit(bad ? 1 : 0);
