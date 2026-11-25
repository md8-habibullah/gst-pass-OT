// Shared by real-bridge.mjs (Chromium) and real-bridge-firefox.mjs: the page a website would serve, the way a test talks to the
// bridge's private socket like any program of the user could, and a one-line way to start the page's passkey calls.
import { createServer } from "node:http";
import { connect } from "node:net";

/** A page that calls navigator.credentials.create / .get and checks the answers with WebCrypto. `window.__run(name, arg)` starts a
 * call and writes its result into `document.body.dataset` (a browser that cannot hand a promise over can poll for it). */
export const PAGE = `<!doctype html><meta charset="utf-8"><title>Passkey test page</title><body><p>Passkey test page</p><script>
const dec = new TextDecoder();
const b64u = (b) => btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\\+/g, "-").replace(/\\//g, "_").replace(/=+$/, "");
const hex = (b) => [...new Uint8Array(b)].map((x) => x.toString(16).padStart(2, "0")).join("");
const bytes = (n) => crypto.getRandomValues(new Uint8Array(n));
const sha = async (data) => new Uint8Array(await crypto.subtle.digest("SHA-256", data));
const concat = (...a) => { const out = new Uint8Array(a.reduce((n, x) => n + x.length, 0)); let i = 0; for (const x of a) { out.set(x, i); i += x.length; } return out; };
const state = { keys: {} };
function derToRaw(der) { // ECDSA-Sig-Value { INTEGER r, INTEGER s } -> r || s, 32 bytes each
  const b = new Uint8Array(der); let i = 2; const part = () => { i++; const len = b[i++]; let v = b.slice(i, i + len); i += len; while (v.length > 32 && v[0] === 0) v = v.slice(1); const o = new Uint8Array(32); o.set(v, 32 - v.length); return o; };
  const r = part(); const s = part(); return concat(r, s);
}
window.__create = async (o = {}) => {
  const challenge = bytes(32);
  try {
    const c = await navigator.credentials.create({ publicKey: { challenge, rp: { id: o.rpId ?? location.hostname, name: "Local test page" }, user: { id: bytes(16), name: o.user ?? "ana@example.com", displayName: "Ana" }, pubKeyCredParams: [{ type: "public-key", alg: -7 }], authenticatorSelection: { residentKey: "required", userVerification: o.uv ?? "preferred" }, attestation: "none", timeout: 60000 } });
    const client = JSON.parse(dec.decode(c.response.clientDataJSON));
    const auth = new Uint8Array(c.response.getAuthenticatorData());
    const pub = c.response.getPublicKey();
    state.keys[c.id] = pub;
    return { ok: true, id: c.id, type: c.type, isPublicKeyCredential: c instanceof PublicKeyCredential, clientType: client.type, origin: client.origin, challengeOk: client.challenge === b64u(challenge),
      flags: auth[32], rpIdHashOk: hex(auth.slice(0, 32)) === hex(await sha(new TextEncoder().encode(o.rpId ?? location.hostname))), alg: c.response.getPublicKeyAlgorithm(), hasPublicKey: !!pub && pub.byteLength > 50 };
  } catch (e) { return { ok: false, name: e.name, message: e.message }; }
};
window.__get = async (o = {}) => {
  const challenge = bytes(32);
  try {
    const c = await navigator.credentials.get({ publicKey: { challenge, rpId: o.rpId ?? location.hostname, userVerification: o.uv ?? "preferred", timeout: 60000 } });
    const client = JSON.parse(dec.decode(c.response.clientDataJSON));
    const auth = new Uint8Array(c.response.authenticatorData);
    let verified = null;
    const pub = state.keys[c.id];
    if (pub) {
      const key = await crypto.subtle.importKey("spki", pub, { name: "ECDSA", namedCurve: "P-256" }, false, ["verify"]);
      verified = await crypto.subtle.verify({ name: "ECDSA", hash: "SHA-256" }, key, derToRaw(c.response.signature), concat(auth, await sha(c.response.clientDataJSON)));
    }
    return { ok: true, id: c.id, clientType: client.type, origin: client.origin, challengeOk: client.challenge === b64u(challenge), flags: auth[32], verified, known: !!pub };
  } catch (e) { return { ok: false, name: e.name, message: e.message }; }
};
window.__available = () => PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable();

window.__run = (fn, arg) => { document.body.dataset.state = "pending"; delete document.body.dataset.result; window[fn](JSON.parse(arg || "{}")).then((r) => { document.body.dataset.result = JSON.stringify(r); document.body.dataset.state = "done"; }); };
</script>`;

export const startSite = (port) => new Promise((resolve) => {
  const site = createServer((req, res) => { res.writeHead(200, { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" }); res.end(PAGE); });
  site.listen(port, "127.0.0.1", () => resolve(site));
});

/** One line to the private socket, as any program of this user could send it; the answer is one line back. */
export const rogueOn = (socket) => (line, ms = 8000) => new Promise((resolve, reject) => {
  const s = connect(socket); let buf = ""; const t = setTimeout(() => { s.destroy(); resolve(buf.trim() || "(no answer)"); }, ms);
  s.on("connect", () => s.write(line + "\n"));
  s.on("data", (d) => { buf += d; if (buf.includes("\n")) { clearTimeout(t); s.end(); resolve(buf.trim()); } });
  s.on("close", () => { clearTimeout(t); resolve(buf.trim() || "(closed without an answer)"); });
  s.on("error", (e) => { clearTimeout(t); reject(e); });
});

export const request = (op, rpId, origin, extra = {}) => JSON.stringify({ v: 1, id: `rogue-${Math.random().toString(16).slice(2)}`, op, origin,
  request: JSON.stringify(op === "create"
    ? { rp: { id: rpId, name: rpId }, user: { id: "dXNlcg", name: "x", displayName: "x" }, challenge: "Y2hhbGxlbmdl", pubKeyCredParams: [{ type: "public-key", alg: -7 }] }
    : { challenge: "Y2hhbGxlbmdl", rpId, allowCredentials: [] }), timeoutMs: 20000, ...extra });
