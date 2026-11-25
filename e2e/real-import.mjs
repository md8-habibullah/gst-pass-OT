// Import and plaintext export against the REAL core, through the real Import and Export pages (native-picker seam).
// With REAL_BW / REAL_ENTE set it also imports real vendor exports (read only, never copied into the project).
import { readFileSync, existsSync, statSync, writeFileSync } from "node:fs";
import { Core, SP, check, finish, freshDir, walkFiles, scanFor, modeOf, launch, sleep, text, has, waitText, waitGone, btn, nav, createAccount, firstStart, waitDemoShell, reprompt, runSteps } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const DATA = freshDir("i-data"), SRC = freshDir("i-src"), OUT = freshDir("i-out");
const core = new Core({ bin: BIN, data: DATA, port: 1447 });
const PW = "correct horse battery staple 7";
const REAL_BW = process.env.REAL_BW, REAL_ENTE = process.env.REAL_ENTE;
const pick = (path) => core.post("/e2e/pick-next", { path });
const steps = []; const step = (name, fn) => steps.push({ name, fn });
const items = async () => (await core.ipc("item_list")).ok;
// What the real files should become, read from the files themselves (nothing about them is written into this script).
const TYPE = { 1: "login", 2: "secureNote", 3: "card", 4: "identity", 5: "sshKey" };
const bw = REAL_BW && existsSync(REAL_BW) ? JSON.parse(readFileSync(REAL_BW, "utf8")) : null;
const ente = REAL_ENTE && existsSync(REAL_ENTE) ? readFileSync(REAL_ENTE, "utf8").split(/\r?\n/).filter((l) => l.startsWith("otpauth://")) : [];
const entePlain = (l) => { try { return decodeURIComponent(l.split("?")[0].replace("otpauth://totp/", "")); } catch { return ""; } };
// Strings that must never appear in the data directory: values from the real files, plus the synthetic ones.
const markers = [
  ...(bw?.items ?? []).flatMap((i) => [i.name, i.notes, i.login?.username, i.login?.password, i.card?.cardholderName, i.card?.number]),
  ...ente.flatMap((l) => [entePlain(l).split(":")[0], (l.match(/secret=([A-Za-z0-9]+)/) ?? [])[1]]),
].filter((m) => typeof m === "string" && m.length >= 6);
const byTitle = async (t) => (await items()).find((i) => i.title === t);

async function chooseAndCheck(p, path, looksLike, ready) {
  await pick(path);
  await btn(p, "Choose a file"); await waitText(p, looksLike);
  await btn(p, "Check the file"); await waitText(p, ready);
}

step("account, then the Import page is empty", async (p) => {
  await p.goto(core.ui); await firstStart(p);
  await createAccount(p, "Marlowe", PW);
  await nav(p, "Import"); await waitText(p, "No file chosen");
  check("no file is chosen at first", true);
});

step("a synthetic Bitwarden file: login with a working code, card and note", async (p) => {
  writeFileSync(`${SRC}/synthetic-bw.json`, JSON.stringify({ encrypted: false, folders: [{ id: "f1", name: "Synthetic folder" }], items: [
    { type: 1, name: "Synthetic site", folderId: "f1", notes: "synthetic login note", login: { username: "synthetic-user", password: "synthetic-pass-4711", uris: [{ uri: "https://synthetic.example" }], totp: "JBSWY3DPEHPK3PXP" } },
    { type: 3, name: "Synthetic card", card: { cardholderName: "S. Thetic", number: "4111111111111111", expMonth: "1", expYear: "2031", code: "123", brand: "Visa" } },
    { type: 2, name: "Synthetic note", notes: "synthetic note body" },
  ] }));
  await chooseAndCheck(p, `${SRC}/synthetic-bw.json`, "Looks like Bitwarden (JSON)", "3 entries ready");
  check("the preview counts one working one-time code", (await text(p)).includes("1 one-time code"));
  await btn(p, "Import 3 items"); await waitText(p, "3 items imported");
  const login = (await items()).find((i) => i.title === "Synthetic site");
  const now = await core.ipc("totp_now", { id: login.id });
  check("the login's code works", /^\d{6}$/.test(String(now.ok?.code ?? "")), JSON.stringify(now).replace(/\d{6}/, "######"));
  check("the folder is kept", login.folder === "Synthetic folder", JSON.stringify(login));
});

if (bw) {
  step("a real Bitwarden web-vault export: card, login and secure note", async (p) => {
    await chooseAndCheck(p, REAL_BW, "Looks like Bitwarden (JSON)", `${bw.items.length} entries ready`);
    const t = await text(p);
    check("the preview counts every item of the file", t.includes(`${bw.items.length} entries ready`), t.slice(0, 600));
    await p.shot(`${SP}/shots2/real-import-bw-preview.png`);
    await btn(p, `Import ${bw.items.length} items`); await waitText(p, `${bw.items.length} items imported`);
    const list = await items();
    const want = bw.items.map((i) => `${i.name}:${TYPE[i.type]}`).sort();
    const got = list.map((i) => `${i.title}:${i.type}`).sort();
    check("every item of the file is in the vault with the right type", want.every((w) => got.includes(w)), got.join(", "));
    const folders = new Map((bw.folders ?? []).map((f) => [f.id, f.name]));
    const withFolder = bw.items.filter((i) => i.folderId && folders.has(i.folderId));
    check("folders are kept", withFolder.every((i) => list.find((x) => x.title === i.name)?.folder === folders.get(i.folderId)), JSON.stringify(list.map((x) => x.folder)));
    await p.shot(`${SP}/shots2/real-import-bw-done.png`);
  });
}

if (ente.length) {
  step("a real Ente Auth plain-text export: otpauth links with codeDisplay", async (p) => {
    await btn(p, "Choose another").catch(() => undefined);
    const n = ente.length;
    await chooseAndCheck(p, REAL_ENTE, "Looks like List of otpauth:// links", `${n} entries ready`);
    check("every link is previewed as a one-time code", (await text(p)).includes(`${n} one-time codes`));
    await btn(p, `Import ${n} items`); await waitText(p, `${n} items imported`);
    const list = (await items()).filter((i) => i.type === "totp");
    check("every link became a code item", list.length === n, list.map((i) => i.title).join(" | "));
    for (const it of list) {
      const now = await core.ipc("totp_now", { id: it.id });
      check(`code item ${list.indexOf(it) + 1} produces a six-digit code`, /^\d{6}$/.test(String(now.ok?.code ?? "")), JSON.stringify(now).replace(/\d{6}/, "######"));
    }
    const full = (await core.ipc("item_get", { id: list[0].id })).ok;
    check("the stored value carries no codeDisplay settings", !JSON.stringify(full).includes("codeDisplay"));
  });

  step("importing the same two files again adds nothing", async (p) => {
    await btn(p, "Choose another").catch(() => undefined);
    const before = (await items()).length;
    const n = ente.length;
    await chooseAndCheck(p, REAL_ENTE, "Looks like List of otpauth:// links", `${n} entries ready`);
    check("the preview finds every link as a duplicate", (await text(p)).includes(`${n} duplicates found`));
    await btn(p, `Import ${n} items`); await waitText(p, "0 items imported");
    check("nothing was added", (await items()).length === before, `${before} -> ${(await items()).length}`);
  });
}

step("a synthetic file the first time: a damaged export is refused in plain words", async (p) => {
  writeFileSync(`${SRC}/broken.json`, `{"encrypted": false, "items": [ {"type": 1, "name": `);
  await btn(p, "Choose another").catch(() => undefined);
  await pick(`${SRC}/broken.json`); await btn(p, "Choose a file");
  await waitText(p, "broken.json");
  let t = await text(p);
  if (/Looks like/.test(t)) { await btn(p, "Check the file"); await p.waitFor(`!!document.querySelector('[role=alert]')`, { timeout: 8000, label: "error shown" }); }
  const alert = await p.eval(`document.querySelector('[role=alert]')?.innerText ?? ''`);
  t = await text(p);
  console.log("   page says:", JSON.stringify(alert || (t.match(/Not recognised[^\n]*/) ?? [""])[0]));
  check("a damaged file is refused in plain words, with no file content in the message", /Not recognised|could not be read|damaged|not a valid/i.test(alert + t) && !alert.includes("encrypted\": false"), alert);
});

step("the native picker seam: a cancelled dialog changes nothing, and an expired file is explained", async (p) => {
  await core.post("/e2e/pick-next", { path: null });
  await btn(p, "Choose another").catch(() => undefined);
  await btn(p, "Choose a file");
  await sleep(500);
  check("cancelling the dialog leaves the page usable", await has(p, "Choose a file"));
});

step("export a readable Bitwarden JSON copy, then read it back", async (p) => {
  const before = await items();
  await nav(p, "Export"); await waitText(p, "Readable copy for another app");
  const dest = `${OUT}/vault-plain.json`;
  await pick(dest);
  await btn(p, "Export readable file");
  await reprompt(p, PW);
  await waitText(p, "Saved to");
  check("the file exists and is private (0600)", existsSync(dest) && modeOf(dest) === "600", existsSync(dest) ? modeOf(dest) : "missing");
  const j = JSON.parse(readFileSync(dest, "utf8"));
  check("it is an unencrypted Bitwarden export with every item", j.encrypted === false && j.items.length === before.length, `${j.items?.length} vs ${before.length}`);
  const card = j.items.find((i) => i.type === 3);
  if (card) check("the card number is written out in the file", /^\d{12,19}$/.test(card.card?.number ?? ""), "");
  const totp = j.items.filter((i) => i.login?.totp);
  check("authenticator codes are exported as otpauth links", before.some((i) => i.type === "totp") ? totp.length > 0 && totp.every((i) => i.login.totp.startsWith("otpauth://") || /^[A-Z2-7]+$/.test(i.login.totp)) : true, `${totp.length}`);
  check("no codeDisplay, no vault internals in the export", !readFileSync(dest, "utf8").includes("codeDisplay"));
  await p.shot(`${SP}/shots2/real-import-export.png`);
});

step("the export imports back into Hexis without loss", async (p) => {
  const dest = `${OUT}/vault-plain.json`;
  const before = await items();
  await nav(p, "Import"); await waitText(p, "Choose a file");
  await chooseAndCheck(p, dest, "Looks like Bitwarden (JSON)", `${before.length} entries ready`);
  check("every exported item is recognised as a match with what is already here", (await text(p)).includes(`${before.length} matches to merge`), (await text(p)).slice(0, 400));
});

step("nothing readable of the imported data sits on disk", async () => {
  const hits = scanFor(walkFiles(DATA), markers.length ? markers : ["no-real-files-given"]);
  check(`the data directory holds only ciphertext (${markers.length} values from the real files searched for)`, hits.length === 0, hits.join("; "));
});

await core.start();
const p = await launch({ width: 1200, height: 1000, port: 9660, profile: `${SP}/chrome-e2e-import2` });
try { await runSteps(p, steps, only); } finally { await p.close(); await core.stop(); }
finish("real-import");
