// Unit tests for the three scripts of the extension, with stand-ins for what a browser gives them (no browser needed):
//   node --test extension/tests/
// The whole chain with a real Chromium and a real Firefox is e2e/real-bridge.mjs and e2e/real-bridge-firefox.mjs.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");
const PAGE = read("page.js");
const CONTENT = read("content.js");
const BACKGROUND = read("background.js");

// ---------------------------------------------------------------- page.js (runs in the page's own world)

/** A window with the pieces page.js uses. The page's own functions record what they were called with. */
function pageWindow({ secure = true } = {}) {
  const listeners = {}; const posted = []; const browserCalls = [];
  class CredentialsContainer {}
  CredentialsContainer.prototype.create = function create(options) { browserCalls.push(["create", options]); return Promise.resolve("the browser's create"); };
  CredentialsContainer.prototype.get = function get(options) { browserCalls.push(["get", options]); return Promise.resolve("the browser's get"); };
  function PublicKeyCredential() {}
  PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable = async () => false;
  class AuthenticatorAttestationResponse {}
  class AuthenticatorAssertionResponse {}
  const win = {
    navigator: { credentials: new CredentialsContainer() }, CredentialsContainer, PublicKeyCredential, AuthenticatorAttestationResponse, AuthenticatorAssertionResponse,
    isSecureContext: secure, ArrayBuffer, btoa, atob, crypto: globalThis.crypto, DOMException, location: { origin: "https://example.com" },
    setTimeout: (f, ms) => { const t = setTimeout(f, ms); t.unref(); return t; }, // the page's 60 s limits must not keep the test process alive
    clearTimeout, Date,
    addEventListener(type, fn) { (listeners[type] ??= []).push(fn); },
    postMessage(message) { posted.push(message); },
  };
  win.window = win;
  return { win, posted, browserCalls, answer: (id, result) => (listeners.message ?? []).forEach((f) => f({ source: win, data: { source: "hexis-content", id, result } })) };
}

function installPage(options) {
  const env = pageWindow(options);
  const saved = globalThis.window;
  globalThis.window = { __HEXIS_TEST__: true };
  try { vm.runInThisContext(PAGE); globalThis.window.__hexisPasskeysInstall(env.win); } finally { globalThis.window = saved; }
  return env;
}

const bytes = (...n) => new Uint8Array(n).buffer;
const creation = (extra = {}) => ({ publicKey: { rp: { id: "example.com", name: "Example" }, user: { id: bytes(1, 2, 3), name: "ana", displayName: "Ana" }, challenge: bytes(9, 8, 7), pubKeyCredParams: [{ type: "public-key", alg: -7 }], excludeCredentials: [{ type: "public-key", id: bytes(4, 5), transports: ["internal"] }], ...extra } });
const request = (extra = {}) => ({ publicKey: { challenge: bytes(1, 1), rpId: "example.com", allowCredentials: [], userVerification: "required", ...extra } });
const tick = () => new Promise((r) => setTimeout(r, 0));

test("a page that is not a secure context is left alone", () => {
  const { win, browserCalls } = installPage({ secure: false });
  assert.equal(win.__hexisPasskeysInstalled, undefined);
  win.navigator.credentials.create(creation());
  assert.equal(browserCalls.length, 1);
});

test("create asks the extension with the request as WebAuthn JSON, base64url, and nothing from the page's address", async () => {
  const { win, posted } = installPage();
  const pending = win.navigator.credentials.create(creation({ timeout: 30000, attestation: "none" }));
  await tick();
  assert.equal(posted.length, 1);
  const [message] = posted;
  assert.equal(message.source, "hexis-page"); assert.equal(message.op, "create"); assert.equal(message.timeoutMs, 30000);
  assert.match(message.id, /^[A-Za-z0-9_-]{8,64}$/);
  assert.equal("origin" in message, false, "the page does not say its own origin: the extension reads it from the browser");
  const json = JSON.parse(message.request);
  assert.deepEqual(json.rp, { name: "Example", id: "example.com" });
  assert.equal(json.user.id, "AQID"); assert.equal(json.challenge, "CQgH");
  assert.deepEqual(json.excludeCredentials, [{ type: "public-key", id: "BAU", transports: ["internal"] }]);
  assert.equal(json.attestation, "none");
  assert.equal(await Promise.race([pending, tick().then(() => "still waiting")]), "still waiting", "the page waits for the person");
});

test("an answer from the app becomes a PublicKeyCredential with the usual members", async () => {
  const { win, posted, answer } = installPage();
  const pending = win.navigator.credentials.create(creation());
  await tick();
  answer(posted[0].id, { ok: true, responseJson: JSON.stringify({ id: "AQID", rawId: "AQID", type: "public-key", authenticatorAttachment: "platform", clientExtensionResults: {},
    response: { clientDataJSON: "e30", attestationObject: "o2M", authenticatorData: "AAEC", publicKey: "AQEB", publicKeyAlgorithm: -7, transports: ["internal"] } }) });
  const credential = await pending;
  assert.ok(credential instanceof win.PublicKeyCredential);
  assert.equal(credential.id, "AQID"); assert.equal(credential.type, "public-key"); assert.equal(credential.authenticatorAttachment, "platform");
  assert.deepEqual([...new Uint8Array(credential.rawId)], [1, 2, 3]);
  assert.ok(credential.response instanceof win.AuthenticatorAttestationResponse);
  assert.deepEqual([...new Uint8Array(credential.response.getPublicKey())], [1, 1, 1]);
  assert.equal(credential.response.getPublicKeyAlgorithm(), -7);
  assert.deepEqual(credential.response.getTransports(), ["internal"]);
  assert.equal(credential.toJSON().id, "AQID");
});

test("get answers with an assertion", async () => {
  const { win, posted, answer } = installPage();
  const pending = win.navigator.credentials.get(request());
  await tick();
  assert.equal(posted[0].op, "get");
  const json = JSON.parse(posted[0].request);
  assert.equal(json.rpId, "example.com"); assert.equal(json.userVerification, "required"); assert.equal(json.challenge, "AQE");
  answer(posted[0].id, { ok: true, responseJson: JSON.stringify({ id: "AQID", rawId: "AQID", type: "public-key", response: { clientDataJSON: "e30", authenticatorData: "AAEC", signature: "MEUC", userHandle: "dXNlcg" } }) });
  const credential = await pending;
  assert.ok(credential.response instanceof win.AuthenticatorAssertionResponse);
  assert.deepEqual([...new Uint8Array(credential.response.userHandle)], [..."user"].map((c) => c.charCodeAt(0)));
});

test("a fallback hands the very same call to the browser's own function", async () => {
  const { win, posted, answer, browserCalls } = installPage();
  const options = creation();
  const pending = win.navigator.credentials.create(options);
  await tick();
  answer(posted[0].id, { ok: false, code: "fallback", reason: "not_running" });
  assert.equal(await pending, "the browser's create");
  assert.equal(browserCalls[0][1], options);
});

test("the person's no is NotAllowedError, an excluded passkey is InvalidStateError, an unsupported type is NotSupportedError", async () => {
  for (const [result, name] of [
    [{ ok: false, code: "cancelled" }, "NotAllowedError"],
    [{ ok: false, code: "timeout" }, "NotAllowedError"],
    [{ ok: false, code: "error", errorCode: "passkey_excluded", message: "x" }, "InvalidStateError"],
    [{ ok: false, code: "error", errorCode: "passkey_unsupported", message: "x" }, "NotSupportedError"],
    [{ ok: false, code: "error", errorCode: "anything_else", message: "Not so." }, "NotAllowedError"],
  ]) {
    const { win, posted, answer } = installPage();
    const pending = win.navigator.credentials.create(creation());
    await tick();
    answer(posted[0].id, result);
    await assert.rejects(pending, (e) => e.name === name);
  }
});

test("calls Hexis does not take go straight to the browser: autofill, security keys, hints", async () => {
  const { win, posted, browserCalls } = installPage();
  await win.navigator.credentials.get({ ...request(), mediation: "conditional" });
  await win.navigator.credentials.create(creation({ authenticatorSelection: { authenticatorAttachment: "cross-platform" } }));
  await win.navigator.credentials.get(request({ hints: ["security-key"] }));
  await win.navigator.credentials.get({ password: true });
  assert.equal(browserCalls.length, 4);
  assert.equal(posted.length, 0);
});

test("a call that is already aborted, or malformed, is the browser's to refuse", async () => {
  const { win, posted, browserCalls } = installPage();
  const controller = new AbortController(); controller.abort();
  await win.navigator.credentials.get({ ...request(), signal: controller.signal });
  await win.navigator.credentials.create({ publicKey: { rp: {}, user: { id: "not a buffer" }, challenge: "nor this" } });
  assert.equal(browserCalls.length, 2);
  assert.equal(posted.length, 0);
});

test("aborting a waiting call tells the extension to cancel and rejects with the signal's reason", async () => {
  const { win, posted } = installPage();
  const controller = new AbortController();
  const pending = win.navigator.credentials.get({ ...request(), signal: controller.signal });
  await tick();
  controller.abort();
  await assert.rejects(pending, (e) => e.name === "AbortError");
  assert.deepEqual(posted.map((m) => m.op), ["get", "cancel"]);
  assert.equal(posted[1].id, posted[0].id);
});

test("the timeout is kept between 5 and 120 seconds", async () => {
  for (const [wanted, sent] of [[100, 5000], [30000, 30000], [900000, 120000], [undefined, 60000]]) {
    const { win, posted } = installPage();
    void win.navigator.credentials.get(request(wanted === undefined ? {} : { timeout: wanted }));
    await tick();
    assert.equal(posted[0].timeoutMs, sent);
  }
});

test("answers that are not for this call, or not from the extension's script, are ignored", async () => {
  const { win, posted, answer } = installPage();
  const pending = win.navigator.credentials.create(creation());
  await tick();
  answer("some-other-id", { ok: false, code: "cancelled" });
  const settled = await Promise.race([pending.then(() => "settled", () => "settled"), tick().then(() => "waiting")]);
  assert.equal(settled, "waiting");
  answer(posted[0].id, { ok: false, code: "cancelled" });
  await assert.rejects(pending);
});

test("sites are told a platform authenticator exists only when Hexis says it is switched on", async () => {
  const on = installPage();
  const asked = on.win.PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable();
  await tick();
  assert.equal(on.posted[0].op, "status");
  on.answer(on.posted[0].id, { ok: true, enabled: true });
  assert.equal(await asked, true);
  const off = installPage();
  const askedOff = off.win.PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable();
  await tick();
  off.answer(off.posted[0].id, { ok: false, code: "fallback", reason: "not_running" });
  assert.equal(await askedOff, false, "the browser's own answer (false here) is used");
});

// ---------------------------------------------------------------- content.js (the extension's script in each page)

function runContent() {
  const listeners = {}; const posted = []; const ports = [];
  const win = {
    addEventListener(type, fn) { (listeners[type] ??= []).push(fn); },
    postMessage(message, target) { posted.push({ message, target }); },
  };
  const api = { runtime: { connect(info) { const port = { info, sent: [], disconnected: false, msg: [], dis: [],
    onMessage: { addListener: (f) => port.msg.push(f) }, onDisconnect: { addListener: (f) => port.dis.push(f) },
    postMessage: (m) => port.sent.push(m), disconnect: () => { port.disconnected = true; } }; ports.push(port); return port; } } };
  const document = { visibilityState: "visible" };
  const sandbox = { window: win, chrome: api, location: { origin: "https://real.example" }, document };
  vm.runInNewContext(CONTENT, sandbox);
  const fromPage = (data, source = win) => (listeners.message ?? []).forEach((f) => f({ source, data }));
  return { win, posted, ports, fromPage, document, hide: () => (listeners.pagehide ?? []).forEach((f) => f()) };
}

test("content: the page's address is the browser's location, whatever the page wrote", () => {
  const { fromPage, ports, posted } = runContent();
  fromPage({ source: "hexis-page", id: "abcdefgh12", op: "get", request: "{}", origin: "https://evil.example" });
  assert.equal(ports.length, 1);
  assert.equal(ports[0].sent[0].origin, "https://real.example");
  assert.equal(posted.length, 0);
});

test("content: messages from another window or with a bad id are ignored", () => {
  const { fromPage, ports } = runContent();
  fromPage({ source: "hexis-page", id: "abcdefgh12", op: "get", request: "{}" }, {});
  fromPage({ source: "hexis-page", id: "short", op: "get", request: "{}" });
  fromPage({ source: "hexis-page", id: "../../../../etc", op: "get", request: "{}" });
  fromPage({ source: "somebody", id: "abcdefgh12", op: "get", request: "{}" });
  fromPage({ source: "hexis-page", id: "abcdefgh12", op: "delete", request: "{}" });
  assert.equal(ports.length, 0);
});

test("content: a missing or oversized request is answered with a fallback, not passed on", () => {
  const { fromPage, ports, posted } = runContent();
  fromPage({ source: "hexis-page", id: "abcdefgh12", op: "get" });
  fromPage({ source: "hexis-page", id: "abcdefgh13", op: "create", request: "x".repeat(64 * 1024 + 1) });
  assert.equal(ports.length, 0);
  assert.deepEqual(posted.map((p) => p.message.result.reason), ["invalid_request", "invalid_request"]);
  assert.ok(posted.every((p) => p.target === "https://real.example"), "answers go to the page's own origin only");
});

test("content: a tab that is not showing gets the browser's own flow, and may still ask whether Hexis is on", () => {
  const { fromPage, ports, posted, document } = runContent();
  document.visibilityState = "hidden";
  fromPage({ source: "hexis-page", id: "abcdefgh12", op: "get", request: "{}" });
  assert.equal(ports.length, 0);
  assert.equal(posted.at(-1).message.result.reason, "not_visible");
  fromPage({ source: "hexis-page", id: "abcdefgh13", op: "status" });
  assert.equal(ports.length, 1);
});

test("content: at most four at once, and the same id twice is busy", () => {
  const { fromPage, ports, posted } = runContent();
  for (const id of ["aaaaaaaa1", "aaaaaaaa2", "aaaaaaaa3", "aaaaaaaa4"]) fromPage({ source: "hexis-page", id, op: "get", request: "{}" });
  fromPage({ source: "hexis-page", id: "aaaaaaaa5", op: "get", request: "{}" });
  fromPage({ source: "hexis-page", id: "aaaaaaaa1", op: "get", request: "{}" });
  assert.equal(ports.length, 4);
  assert.deepEqual(posted.map((p) => p.message.result.reason), ["busy", "busy"]);
});

test("content: the answer goes back once and the port is closed; a port that drops unanswered is a fallback; cancel and leaving close ports", () => {
  const { fromPage, ports, posted, hide } = runContent();
  fromPage({ source: "hexis-page", id: "aaaaaaaa1", op: "get", request: "{}" });
  ports[0].msg[0]({ ok: true, responseJson: "{}" });
  assert.equal(posted.at(-1).message.result.ok, true); assert.equal(ports[0].disconnected, true);
  fromPage({ source: "hexis-page", id: "aaaaaaaa2", op: "get", request: "{}" });
  ports[1].dis[0]();
  assert.equal(posted.at(-1).message.result.reason, "no_extension");
  fromPage({ source: "hexis-page", id: "aaaaaaaa3", op: "get", request: "{}" });
  fromPage({ source: "hexis-page", id: "aaaaaaaa3", op: "cancel" });
  assert.equal(ports[2].disconnected, true);
  fromPage({ source: "hexis-page", id: "aaaaaaaa4", op: "get", request: "{}" });
  hide();
  assert.equal(ports[3].disconnected, true);
});

// ---------------------------------------------------------------- background.js (the extension's background)

function runBackground({ connectNative } = {}) {
  let onConnect;
  const hosts = [];
  const timers = [];
  const api = { runtime: {
    onConnect: { addListener: (f) => { onConnect = f; } },
    connectNative: connectNative ?? ((name) => { const host = { name, sent: [], disconnected: false, msg: [], dis: [],
      onMessage: { addListener: (f) => host.msg.push(f) }, onDisconnect: { addListener: (f) => host.dis.push(f) },
      postMessage: (m) => host.sent.push(m), disconnect: () => { host.disconnected = true; } }; hosts.push(host); return host; }),
  } };
  vm.runInNewContext(BACKGROUND, { chrome: api, setTimeout: (f, ms) => { timers.push({ f, ms }); return timers.length; }, clearTimeout: () => {}, URL });
  const connect = (url, name = "hexis-passkeys") => {
    const port = { name, sender: { url }, posted: [], disconnected: false, msg: [], dis: [],
      onMessage: { addListener: (f) => port.msg.push(f) }, onDisconnect: { addListener: (f) => port.dis.push(f) },
      postMessage: (m) => port.posted.push(m), disconnect: () => { port.disconnected = true; } };
    onConnect(port);
    return port;
  };
  return { connect, hosts, timers };
}

test("background: the origin comes from the browser's sender, never from the message", () => {
  const { connect, hosts } = runBackground();
  const port = connect("https://real.example/page?x=1");
  port.msg[0]({ id: "abcdefgh12", op: "get", request: "{}", origin: "https://evil.example" });
  assert.equal(hosts.length, 1); assert.equal(hosts[0].name, "dev.habibullah.hexis.passkeys");
  assert.equal(hosts[0].sent[0].origin, "https://real.example");
  assert.equal(hosts[0].sent[0].v, 1);
});

test("background: only secure pages, only known operations, only requests of a sane size", () => {
  const { connect, hosts } = runBackground();
  for (const [url, message] of [
    ["http://plain.example/", { id: "abcdefgh12", op: "get", request: "{}" }],
    ["https://real.example/", { id: "abcdefgh12", op: "delete", request: "{}" }],
    ["https://real.example/", { id: "abcdefgh12", op: "get" }],
    ["https://real.example/", { id: "abcdefgh12", op: "get", request: "x".repeat(64 * 1024 + 1) }],
    [undefined, { id: "abcdefgh12", op: "get", request: "{}" }],
  ]) {
    const port = connect(url);
    port.msg[0](message);
    assert.equal(port.posted[0]?.reason, "invalid_request", JSON.stringify([url, message.op]));
    assert.equal(port.disconnected, true);
  }
  assert.equal(hosts.length, 0, "the host is not even started for any of them");
  const local = connect("http://localhost:3000/");
  local.msg[0]({ id: "abcdefgh12", op: "get", request: "{}" });
  assert.equal(hosts.length, 1, "localhost is allowed");
});

test("background: a port that is not ours is ignored; status needs no origin", () => {
  const { connect, hosts } = runBackground();
  const other = connect("https://real.example/", "something-else");
  assert.equal(other.msg.length, 0);
  const port = connect(undefined);
  port.msg[0]({ id: "abcdefgh12", op: "status" });
  assert.equal(hosts.length, 1);
  assert.equal(hosts[0].sent[0].op, "status"); assert.equal(hosts[0].sent[0].request, undefined);
});

test("background: the answer is passed on once, the host is closed, and a request is handled only once per port", () => {
  const { connect, hosts } = runBackground();
  const port = connect("https://real.example/");
  port.msg[0]({ id: "abcdefgh12", op: "get", request: "{}" });
  port.msg[0]({ id: "abcdefgh13", op: "get", request: "{}" });
  assert.equal(hosts.length, 1);
  hosts[0].msg[0]({ ok: true, responseJson: "{}" });
  assert.deepEqual(port.posted, [{ ok: true, responseJson: "{}" }]);
  assert.equal(hosts[0].disconnected, true); assert.equal(port.disconnected, true);
});

test("background: no host (not registered, or Hexis closed) is a fallback; a page that goes away closes the host", () => {
  const missing = runBackground({ connectNative: () => { throw new Error("no such host"); } });
  const a = missing.connect("https://real.example/");
  a.msg[0]({ id: "abcdefgh12", op: "get", request: "{}" });
  assert.equal(a.posted[0].reason, "not_installed");
  const { connect, hosts, timers } = runBackground();
  const b = connect("https://real.example/");
  b.msg[0]({ id: "abcdefgh12", op: "get", request: "{}" });
  hosts[0].dis[0]();
  assert.equal(b.posted[0].reason, "not_installed");
  const c = connect("https://real.example/");
  c.msg[0]({ id: "abcdefgh12", op: "get", request: "{}", timeoutMs: 30000 });
  assert.equal(timers.at(-1).ms, 36000, "the app is given the request's own timeout plus a little");
  c.dis[0]();
  assert.equal(hosts[1].disconnected, true, "closing the pipe is how the app learns the page went away");
});
