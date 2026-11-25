// Minimal zero-dependency Chrome DevTools Protocol client (Node 22+). Drives ONLY its own headless Chromium
// with its own throwaway profile; it never touches the desktop session or any other browser.
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The first Chrome start on a busy CI runner can take longer than any fixed wait, so a failed start is retried.
export async function launch(opts = {}) {
  let last;
  for (let attempt = 1; attempt <= 3; attempt++) {
    try { return await launchOnce(opts); } catch (e) { last = e; if (e.fatal) break; await sleep(1500); }
  }
  throw last;
}

// `extension`: a folder to load as an unpacked extension (the one browser a script starts for the Hexis extension); every other
// script runs with extensions off. `env`: extra environment for the browser (HOME and XDG_RUNTIME_DIR of a throw-away home,
// so a native messaging host the browser starts reaches the shim and never the real app).
async function launchOnce({ width = 1440, height = 900, port = 9333, profile, mobile = false, extension, env } = {}) {
  mkdirSync(profile, { recursive: true });
  // Chrome 137 and later (the branded builds) ignore --load-extension unless this feature is switched off; Chromium ignores the flag.
  const extensions = extension
    ? [`--disable-extensions-except=${extension}`, `--load-extension=${extension}`, "--disable-features=DisableLoadExtensionCommandLineSwitch"]
    : ["--disable-extensions"];
  // CHROME_BIN lets CI use the runner's google-chrome; on the dev machine the default is the `chromium` on the PATH.
  const proc = spawn(process.env.CHROME_BIN || "chromium", [
    "--headless=new", "--disable-gpu", "--no-first-run", ...extensions, "--hide-scrollbars",
    `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, `--window-size=${width},${height}`,
    "--force-device-scale-factor=1", "--disable-background-networking", "--no-sandbox", "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"], env: env ? { ...process.env, ...env } : process.env });
  let errTail = "", spawnError;
  proc.on("error", (e) => { spawnError = e; }); // for example ENOENT: the browser binary does not exist
  proc.stderr?.on("data", (d) => { if (errTail.length < 2000) errTail += d; });
  let version;
  for (let i = 0; i < 200 && !spawnError; i++) { // up to 20 s
    try { version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json(); break; } catch { await sleep(100); }
  }
  if (spawnError) { const e = new Error(`could not start ${process.env.CHROME_BIN || "chromium"}: ${spawnError.message}`); e.fatal = true; throw e; }
  if (!version) { proc.kill(); throw new Error(`chromium did not start: ${errTail.trim().slice(0, 500)}`); }
  const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const target = list.find((t) => t.type === "page");
  const page = new Page(target.webSocketDebuggerUrl, proc, profile);
  await page.ready;
  await page.send("Page.enable"); await page.send("Runtime.enable"); await page.send("Log.enable");
  await page.viewport(width, height, mobile);
  return page;
}

export class Page {
  constructor(wsUrl, proc, profile) {
    this.proc = proc; this.profile = profile; this.id = 0; this.pending = new Map(); this.logs = []; this.waiters = [];
    this.ws = new WebSocket(wsUrl);
    this.ready = new Promise((res, rej) => { this.ws.onopen = res; this.ws.onerror = rej; });
    this.ws.onmessage = (m) => {
      const d = JSON.parse(m.data);
      if (d.id && this.pending.has(d.id)) {
        const { res, rej } = this.pending.get(d.id); this.pending.delete(d.id);
        d.error ? rej(new Error(`${d.error.message}`)) : res(d.result);
      } else if (d.method) {
        if (d.method === "Runtime.consoleAPICalled" && ["error", "warning"].includes(d.params.type)) this.logs.push(`${d.params.type}: ${d.params.args.map((a) => a.value ?? a.description ?? "").join(" ")}`);
        if (d.method === "Runtime.exceptionThrown") this.logs.push(`exception: ${d.params.exceptionDetails.exception?.description ?? d.params.exceptionDetails.text}`);
        if (d.method === "Log.entryAdded" && d.params.entry.level === "error") this.logs.push(`log: ${d.params.entry.text} ${d.params.entry.url ?? ""}`);
        this.waiters = this.waiters.filter((w) => !(w.method === d.method && (w.res(d.params), true)));
      }
    };
  }
  send(method, params = {}) { const id = ++this.id; this.ws.send(JSON.stringify({ id, method, params })); return new Promise((res, rej) => this.pending.set(id, { res, rej })); }
  once(method) { return new Promise((res) => this.waiters.push({ method, res })); }
  async viewport(width, height, mobile = false) {
    await this.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile });
    await this.send("Emulation.setTouchEmulationEnabled", { enabled: mobile });
    if (mobile) await this.send("Emulation.setEmitTouchEventsForMouse", { enabled: false });
  }
  async goto(url) { const loaded = this.once("Page.loadEventFired"); await this.send("Page.navigate", { url }); await loaded; await sleep(150); }
  async eval(expr) {
    const r = await this.send("Runtime.evaluate", { expression: typeof expr === "function" ? `(${expr})()` : expr, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
    return r.result.value;
  }
  async waitFor(expr, { timeout = 10000, label } = {}) {
    const t0 = Date.now();
    for (;;) {
      let v; try { v = await this.eval(expr); } catch { v = false; }
      if (v) return v;
      if (Date.now() - t0 > timeout) throw new Error(`timeout waiting for ${label ?? expr}`);
      await sleep(60);
    }
  }
  /** Finds a visible element by CSS selector plus optional text, returns its centre. */
  async locate(sel, text) {
    const js = `(() => {
      const els=[...document.querySelectorAll(${JSON.stringify(sel)})].filter(e=>{const r=e.getBoundingClientRect();const cs=getComputedStyle(e);return r.width>0&&r.height>0&&cs.visibility!=='hidden'});
      const want=${JSON.stringify(text ?? null)};
      const el=want===null?els[0]:els.find(e=>(e.getAttribute('aria-label')||'').includes(want)||e.textContent.trim().includes(want)||(e.getAttribute('placeholder')||'').includes(want));
      if(!el) return null; el.scrollIntoView({block:'center',inline:'center'}); const r=el.getBoundingClientRect(); return {x:r.x+r.width/2,y:r.y+r.height/2};
    })()`;
    return this.eval(js);
  }
  /** Waits until the located element is actually the top-most thing at its centre (not under a closing overlay). */
  async settle(sel, text) {
    const expr = `(()=>{const el=${this.locateExpr(sel, text)}; if(!el) return null; el.scrollIntoView({block:'center',inline:'center'}); const r=el.getBoundingClientRect(); const x=r.x+r.width/2,y=r.y+r.height/2; const top=document.elementFromPoint(x,y); return top&&(el===top||el.contains(top))?{x,y}:false;})()`;
    let last = null;
    const t0 = Date.now();
    let prev = null;
    for (;;) {
      last = await this.eval(expr);
      // top-most AND not moving (a sheet that is still sliding in would otherwise be clicked at its old position)
      if (last && prev && Math.abs(last.x - prev.x) < 0.5 && Math.abs(last.y - prev.y) < 0.5) return last;
      prev = last || null;
      if (last) { await sleep(70); continue; }
      if (Date.now() - t0 > 8000) throw new Error(`target ${sel} ${text ?? ""} ${last === null ? "not found" : "is covered by another element"}`);
      await sleep(60);
    }
  }
  async click(sel, text, opts = {}) {
    const p = await this.settle(sel, text);
    await this.mouse(p.x, p.y, opts);
  }
  locateExpr(sel, text) {
    return `(()=>{const els=[...document.querySelectorAll(${JSON.stringify(sel)})].filter(e=>{const r=e.getBoundingClientRect();return r.width>0&&r.height>0&&getComputedStyle(e).visibility!=='hidden'});const want=${JSON.stringify(text ?? null)};return want===null?els[0]:els.find(e=>(e.getAttribute('aria-label')||'').includes(want)||e.textContent.trim().includes(want)||(e.getAttribute('placeholder')||'').includes(want));})()`;
  }
  async mouse(x, y, { button = "left", clickCount = 1 } = {}) {
    await this.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    await this.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button, clickCount, buttons: button === "right" ? 2 : 1 });
    await this.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button, clickCount });
    await sleep(120);
  }
  async rightClick(sel, text) { const p = await this.settle(sel, text); await this.mouse(p.x, p.y, { button: "right" }); }
  async focus(sel, text) { await this.waitFor(`!!(${this.locateExpr(sel, text)})`, { label: `focus target ${sel} ${text ?? ""}` }); await this.eval(`(${this.locateExpr(sel, text)}).focus()`); }
  /** Replaces the value of an input by selecting all, then inserting text (fires real input events). */
  async type(sel, text, { target } = {}) {
    await this.focus(sel, target);
    await this.eval(`(()=>{const e=document.activeElement; if(e&&e.select) e.select();})()`);
    if (text === "") await this.press("Delete"); else await this.send("Input.insertText", { text });
    await sleep(60);
  }
  async press(key) {
    const map = { Enter: [13, "\r"], Escape: [27], Tab: [9], ArrowDown: [40], ArrowUp: [38], Delete: [46], Backspace: [8], Home: [36], End: [35] };
    const [code, text] = map[key] ?? [0];
    const base = { key, code: key, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code };
    await this.send("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", ...base, text });
    await this.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    await sleep(80);
  }
  /** One typed character or key with optional Ctrl and Shift, as the keyboard sends it (for shortcuts). */
  async key(key, { ctrl = false, shift = false } = {}) {
    const modifiers = (ctrl ? 2 : 0) | (shift ? 8 : 0);
    const text = ctrl || key.length !== 1 ? undefined : key;
    await this.send("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", key, modifiers, text });
    await this.send("Input.dispatchKeyEvent", { type: "keyUp", key, modifiers });
    await sleep(120);
  }
  async text() { return this.eval("document.body.innerText"); }
  /** Types into the input/textarea whose <label> text equals `label` (searching the top-most dialog first). */
  async typeLabel(label, text) {
    // A sheet's focus trap moves focus into the dialog once it has opened; wait for that so it cannot steal focus back.
    await this.waitFor(`(()=>{const d=[...document.querySelectorAll('[role=dialog]')].pop(); return !d || d.contains(document.activeElement)})()`, { timeout: 3000 }).catch(() => {});
    const ok = await this.waitFor(`(()=>{
      const dialogs=[...document.querySelectorAll('[role=dialog]')]; const root=dialogs.length?dialogs[dialogs.length-1]:document;
      const l=[...root.querySelectorAll('label')].find(x=>x.textContent.trim()===${JSON.stringify(label)}); if(!l) return false;
      const el=document.getElementById(l.htmlFor); if(!el) return false; el.scrollIntoView({block:'center'}); el.focus(); if(el.select) el.select(); return true; })()`, { label: `field ${label}` });
    if (text === "") await this.press("Delete"); else await this.send("Input.insertText", { text });
    await sleep(50);
    return ok;
  }
  /** Types into the input whose aria-label equals `label` (search boxes have no visible <label>). */
  async typeAria(label, text) {
    const sel = `input[aria-label=${JSON.stringify(label)}]`;
    await this.waitFor(`!!document.querySelector(${JSON.stringify(sel)})`, { label: `field ${label}` });
    await this.eval(`(()=>{const el=document.querySelector(${JSON.stringify(sel)}); el.scrollIntoView({block:'center'}); el.focus(); el.select(); })()`);
    if (text === "") await this.press("Delete"); else await this.send("Input.insertText", { text });
    await sleep(80);
  }
  /** Value of the control behind a label. */
  async valueOf(label) { return this.eval(`(()=>{const dialogs=[...document.querySelectorAll('[role=dialog]')]; const root=dialogs.length?dialogs[dialogs.length-1]:document; const l=[...root.querySelectorAll('label')].find(x=>x.textContent.trim()===${JSON.stringify(label)}); return l?document.getElementById(l.htmlFor)?.value:null})()`); }
  /** Runs `trigger` (a click that opens the file picker) and answers the picker with `files`; works with real user activation. */
  async chooseFiles(files, trigger) {
    await this.send("DOM.enable");
    await this.send("Page.setInterceptFileChooserDialog", { enabled: true });
    const opened = this.once("Page.fileChooserOpened");
    await trigger();
    const ev = await Promise.race([opened, sleep(5000).then(() => null)]);
    if (!ev) throw new Error("the file picker never opened");
    await this.send("DOM.setFileInputFiles", { files, backendNodeId: ev.backendNodeId });
    await sleep(200);
  }
  async setFiles(sel, files) {
    await this.send("DOM.enable");
    const r = await this.send("Runtime.evaluate", { expression: `document.querySelector(${JSON.stringify(sel)})` });
    await this.send("DOM.setFileInputFiles", { objectId: r.result.objectId, files });
    await sleep(150);
  }
  async shot(path) { const r = await this.send("Page.captureScreenshot", { format: "png" }); writeFileSync(path, Buffer.from(r.data, "base64")); }
  async close() { try { this.ws.close(); } catch {} this.proc.kill(); await sleep(200); try { rmSync(this.profile, { recursive: true, force: true }); } catch {} }
}
export { sleep };
