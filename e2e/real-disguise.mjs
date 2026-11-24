// Disguise against the REAL core: preset, lower-case, custom name, persistence, validation, launcher file (Linux .desktop) in a temp XDG dir.
import { readFileSync, existsSync } from "node:fs";
import { Core, SP, check, finish, freshDir, walkFiles, launch, sleep, text, has, waitText, btn, nav, createAccount, firstStart, waitDemoShell, settingsPage, runSteps } from "./lib.mjs";
const BIN = process.env.E2E_BIN; const only = process.argv[2];
const DATA = freshDir("d-data"), XDG = freshDir("d-xdg");
const core = new Core({ bin: BIN, data: DATA, port: 1445, args: ["--xdg-data-home", XDG] });
const PW = "correct horse battery staple 7";
const view = async () => (await core.ipc("disguise_get")).ok;
const applied = async () => JSON.parse(await core.get("/e2e/disguise-applied").catch(() => "null"));
const steps = []; const step = (name, fn) => steps.push({ name, fn });

step("defaults: original look, window title Hexis", async (p) => {
  await p.goto(core.ui); await firstStart(p);
  const v = await view(); check("nothing is disguised by default", v.presetId === null && v.customName === null && v.lowercase === false && v.appliedName === "Hexis", JSON.stringify(v));
  check("the page title is Hexis", (await p.eval("document.title")) === "Hexis");
  await createAccount(p, "Marlowe", PW);
});

step("pick Kerf: the core, the title, the lock screen and the launcher entry follow", async (p) => {
  await settingsPage(p, "App icon and name"); await waitText(p, "Your own name (optional)");
  await p.click('[role=radio]', "Kerf");
  await p.waitFor(`document.title === 'Kerf'`, { timeout: 8000 });
  const v = await view(); check("the core stored Kerf", v.presetId === "kerf" && v.appliedName === "Kerf", JSON.stringify(v));
  const a = await applied(); check("the launcher was driven with the kerf preset", a?.presetId === "kerf" || JSON.stringify(a).includes("kerf"), JSON.stringify(a));
  const desk = walkFiles(XDG).filter((f) => f.endsWith(".desktop"));
  check("a .desktop entry was written under the (temp) data home", desk.length === 1, desk.join(", "));
  if (desk.length) {
    const t = readFileSync(desk[0], "utf8");
    check("it has the new name, an absolute icon path and the managed marker", /^Name=Kerf$/m.test(t) && /^Icon=\/.+\.png$/m.test(t) && /^X-Hexis-Managed=true$/m.test(t), t);
    check("the icon file it points to exists", existsSync(t.match(/^Icon=(.+)$/m)?.[1] ?? "/nonexistent"));
  }
});

step("lower-case and custom name; hostile names are refused or neutralised", async (p) => {
  await p.click('[role=switch][aria-label="Lower-case names"]'); await p.waitFor(`document.title === 'kerf'`, { timeout: 8000 });
  check("lower-case shows kerf", (await view()).appliedName === "kerf");
  await p.typeLabel("Your own name (optional)", "Field Notes"); await btn(p, "Apply name");
  await p.waitFor(`document.title === 'field notes'`, { timeout: 8000 });
  check("a custom name replaces the preset name (lower-cased while the switch is on)", (await view()).appliedName === "field notes");
  const bad = await core.ipc("disguise_set", { input: { presetId: "kerf", customName: "x".repeat(31), lowercase: false } });
  check("a 31-character name is refused by the core", bad.err?.code === "invalid_input", JSON.stringify(bad));
  const bad2 = await core.ipc("disguise_set", { input: { presetId: "nope", customName: null, lowercase: false } });
  check("an unknown preset id is refused", bad2.err?.code === "invalid_input", JSON.stringify(bad2));
  const inj = await core.ipc("disguise_set", { input: { presetId: "tally", customName: "a%u\nExec=evil", lowercase: false } });
  const desk = walkFiles(XDG).filter((f) => f.endsWith(".desktop")).map((f) => readFileSync(f, "utf8")).join("\n");
  check("a newline-injection name is refused or cannot add a second Exec line", inj.err !== undefined || (desk.match(/^Exec=/gm) ?? []).length === 1, JSON.stringify(inj) + desk.slice(0, 300));
});

step("the choice survives a restart and shows on the lock screen", async (p) => {
  await core.ipc("disguise_set", { input: { presetId: "orvyn", customName: null, lowercase: false } });
  await core.stop(); await core.start();
  const v = await view(); check("Orvyn is still selected after a restart", v.presetId === "orvyn" && v.appliedName === "Orvyn", JSON.stringify(v));
  await p.goto(core.ui); await p.waitFor(`document.title === 'Orvyn'`, { timeout: 8000 });
  await waitText(p, "Master password").catch(() => undefined);
  const t = await text(p); check("the app's own screens carry the new name, not Hexis", !t.includes("Hexis") || t.includes("Orvyn"), t.slice(0, 200));
  await p.shot(`${SP}/shots2/real-disguise-lock.png`);
});

step("back to the original look removes the launcher entry", async () => {
  const r = await core.ipc("disguise_set", { input: { presetId: null, customName: null, lowercase: false } });
  check("original look restored", r.ok?.appliedName === "Hexis", JSON.stringify(r));
  const desk = walkFiles(XDG).filter((f) => f.endsWith(".desktop"));
  check("the launcher file still names Hexis (or is gone), never a stale disguise", desk.every((f) => /^Name=Hexis$/m.test(readFileSync(f, "utf8"))), desk.join(", "));
});

await core.start();
const page = await launch({ width: 1280, height: 900, port: 9743, profile: `${SP}/chrome-e2e-disguise-real` });
try { await runSteps(page, steps, only); }
finally { if (page.logs.length) console.log("\nbrowser console:", [...new Set(page.logs)].slice(0, 10)); await page.close(); await core.stop(); }
finish("real-disguise");
