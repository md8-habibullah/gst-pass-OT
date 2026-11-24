// Encrypted drive against the REAL core: add (native-picker seam), size limit, ciphertext on disk, preview, rename, save copy, delete, tamper, attachments.
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, truncateSync, openSync, writeSync, closeSync, readdirSync, statSync } from "node:fs";
import { Core, SP, check, finish, freshDir, walkFiles, scanFor, modeOf, launch, sleep, text, has, waitText, waitGone, btn, nav, createAccount, addLogin, reprompt, runSteps } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const DATA = freshDir("f-data"), SRC = freshDir("f-src"), OUT = freshDir("f-out");
const core = new Core({ bin: BIN, data: DATA, port: 1446 });
const PW = "correct horse battery staple 7", LIMIT = 52_428_800;
const sha = (b) => createHash("sha256").update(b).digest("hex");
const pick = (paths) => core.post("/e2e/pick-next", { paths });
const attachments = () => walkFiles(DATA).filter((f) => f.includes("/attachments/"));
const steps = []; const step = (name, fn) => steps.push({ name, fn });

const NOTE = "Spare router login: admin / PLAINTEXT-MARKER-ZX81 (keep this file private)\n";
writeFileSync(`${SRC}/notes.txt`, NOTE);
writeFileSync(`${SRC}/pixel.svg`, `<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><rect width="40" height="40" fill="#c0392b"/></svg>`);
const big = Buffer.alloc(3 * 1024 * 1024 + 7); for (let i = 0; i < big.length; i++) big[i] = (i * 31 + 7) & 255;
writeFileSync(`${SRC}/multi-chunk.bin`, big);
const edge = openSync(`${SRC}/limit-ok.bin`, "w"); closeSync(edge); truncateSync(`${SRC}/limit-ok.bin`, LIMIT);
const over = openSync(`${SRC}/too-big.bin`, "w"); closeSync(over); truncateSync(`${SRC}/too-big.bin`, LIMIT + 1);

step("account and an empty Files section", async (p) => {
  await p.goto(core.ui); await waitText(p, "Welcome to");
  await createAccount(p, "Marlowe", PW);
  await nav(p, "Files"); await waitText(p, "kept encrypted next to your passwords");
  const r = await core.ipc("file_list", { itemId: null }); check("the core lists no files yet", Array.isArray(r.ok) && r.ok.length === 0, JSON.stringify(r));
});

step("add files through the native picker seam: ciphertext only on disk", async (p) => {
  await pick([`${SRC}/notes.txt`, `${SRC}/pixel.svg`, `${SRC}/multi-chunk.bin`]);
  await btn(p, "Add files"); await waitText(p, "notes.txt"); await waitText(p, "multi-chunk.bin");
  const list = (await core.ipc("file_list", { itemId: null })).ok;
  check("three files are listed with their real sizes", list.length === 3 && list.find((f) => f.name === "multi-chunk.bin")?.size === big.length, JSON.stringify(list));
  const blobs = attachments();
  check("three encrypted blobs exist, private (0600), larger than their plaintext", blobs.length === 3 && blobs.every((f) => modeOf(f) === "600") && blobs.some((f) => statSync(f).size > big.length), blobs.map((f) => `${modeOf(f)} ${statSync(f).size}`).join(", "));
  const hits = scanFor(walkFiles(DATA), ["PLAINTEXT-MARKER-ZX81", "notes.txt", "multi-chunk", "Spare router", "<svg"]);
  check("no plaintext content or file name anywhere in the data directory", hits.length === 0, hits.join("; "));
});

step("preview decrypts in memory (text, image) and file_read returns raw bytes", async (p) => {
  await p.click("button", "Preview notes.txt"); await waitText(p, "PLAINTEXT-MARKER-ZX81");
  check("text preview shows the decrypted text", true); await p.shot(`${SP}/shots2/real-drive-text.png`); await p.press("Escape"); await sleep(400);
  await p.click("button", "Preview pixel.svg"); await p.waitFor(`!!document.querySelector('[role=dialog] img')`);
  await sleep(400); check("image preview renders from a blob URL", await p.eval(`(()=>{const i=document.querySelector('[role=dialog] img');return i.complete&&i.naturalWidth>0&&i.src.startsWith('blob:')})()`));
  await p.press("Escape"); await sleep(400);
  const id = (await core.ipc("file_list", { itemId: null })).ok.find((f) => f.name === "multi-chunk.bin").id;
  const raw = await core.ipc("file_read", { id });
  check("file_read returns the exact bytes of a multi-chunk file (raw, not JSON)", Buffer.isBuffer(raw.ok) && sha(raw.ok) === sha(big), JSON.stringify(raw).slice(0, 120));
});

step("the 50 MB limit is enforced in the core, to the byte", async (p) => {
  const before = attachments().length;
  await pick([`${SRC}/too-big.bin`]); await btn(p, "Add files");
  await waitText(p, "too-big.bin is larger than 50 MB");
  check("52,428,801 bytes is refused with the file's name and nothing is written", attachments().length === before);
  await pick([`${SRC}/limit-ok.bin`]); await btn(p, "Add files");
  await waitText(p, "limit-ok.bin", 120000);
  check("exactly 52,428,800 bytes is accepted", (await core.ipc("file_list", { itemId: null })).ok.some((f) => f.name === "limit-ok.bin" && f.size === LIMIT));
  const blob = attachments().map((f) => ({ f, s: statSync(f).size })).sort((a, b) => b.s - a.s)[0];
  check("its blob is chunked ciphertext (a bit over 50 MiB), not a copy", blob.s > LIMIT && blob.s < LIMIT + 4096, String(blob.s));
});

step("rename, then Save decrypted copy needs the master password and writes 0600", async (p) => {
  await p.click("button", "Rename notes.txt"); await p.typeLabel("File name", "router-notes.txt"); await p.click("[role=dialog] button[type=submit]");
  await waitText(p, "router-notes.txt");
  check("rename shows in the list and nothing readable leaked to disk", scanFor(walkFiles(DATA), ["router-notes"]).length === 0);
  const dest = `${OUT}/router-notes-copy.txt`; await pick([dest]);
  await p.click("button", "Save a decrypted copy of router-notes.txt");
  await waitText(p, "Confirm it is you");
  await p.typeLabel("Master password", "not the password"); await p.press("Enter"); await waitText(p, "master password").catch(() => undefined);
  check("a wrong master password does not save anything", !(await core.get("/e2e/last-saved")).includes(dest));
  await p.typeLabel("Master password", PW); await p.press("Enter"); await waitGone(p, "Confirm it is you");
  await sleep(600);
  check("the copy is byte-identical and private (0600)", readFileSync(dest, "utf8") === NOTE && modeOf(dest) === "600");
});

step("tampering with a blob is detected, with a fixed message and no crash", async (p) => {
  const small = attachments().find((f) => statSync(f).size < 2000 && readFileSync(f).length > 100);
  const buf = readFileSync(small); buf[buf.length - 20] ^= 0x01; writeFileSync(small, buf);
  const id = (await core.ipc("file_list", { itemId: null })).ok.find((f) => f.name === "pixel.svg" || f.name === "router-notes.txt");
  const r = await core.ipc("file_read", { id: id.id }); const r2 = await core.ipc("file_read", { id: (await core.ipc("file_list", { itemId: null })).ok.find((f) => f.name === "router-notes.txt").id });
  const bad = [r, r2].find((x) => x.err);
  check("a flipped bit makes decryption fail with an error (never wrong data)", !!bad, JSON.stringify([r.err, r2.err]));
  check("the error sentence leaks no path or key material", bad && !/\/|key|0x/.test(String(bad.err.message)), bad && bad.err.message);
});

step("delete needs the master password and removes the blob from disk", async (p) => {
  const n = attachments().length;
  await p.click("button", "Delete pixel.svg"); await waitText(p, "Confirm it is you");
  await p.typeLabel("Master password", PW); await p.press("Enter"); await waitGone(p, "pixel.svg");
  check("the blob file is gone from the data directory", attachments().length === n - 1);
});

step("attachments inside an item; locking refuses file access", async (p) => {
  await nav(p, "All items"); await addLogin(p, [["Title", "Router"], ["Username", "admin"], ["Password", "pw-router-1"]]);
  await p.click("[data-item-row]", "Router"); await waitText(p, "Attachments");
  await pick([`${SRC}/pixel.svg`]); await p.click("section[aria-labelledby^=att-] button", "Add files");
  await p.waitFor(`!!document.querySelector('section[aria-labelledby^=att-] li')`);
  check("a file can be attached to an item and stays out of the Files list", !(await core.ipc("file_list", { itemId: null })).ok.some((f) => f.itemId));
  await core.ipc("vault_lock");
  const r = await core.ipc("file_list", { itemId: null }); check("with the vault locked the core refuses file commands", r.err?.code === "locked", JSON.stringify(r));
});

await core.start();
const page = await launch({ width: 1280, height: 900, port: 9744, profile: `${SP}/chrome-e2e-drive-real` });
try { await runSteps(page, steps, only); }
finally { if (page.logs.length) console.log("\nbrowser console:", [...new Set(page.logs)].slice(0, 10)); await page.close(); await core.stop(); }
finish("real-drive");
