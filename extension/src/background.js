// Hexis passkeys: the extension's background. One native host process serves one request: it is started when a page asks,
// answers once, and is closed. When the page goes away first, closing the host's pipe tells the Hexis app to drop its prompt.
"use strict";

const api = typeof browser !== "undefined" ? browser : chrome;
const HOST = "dev.habibullah.hexis.passkeys";
const MAX_REQUEST = 64 * 1024;
const SLACK_MS = 6000; // how long past the request's own timeout the app is given to answer before the page is released

function fallback(reason) {
  return { ok: false, code: "fallback", reason: reason };
}

// The page address comes from the browser (the sender), never from what the content script wrote.
function originOf(sender) {
  try {
    if (sender && sender.url) return new URL(sender.url).origin;
  } catch (e) { /* no address */ }
  return null;
}

function secure(origin) {
  try {
    const u = new URL(origin);
    return u.protocol === "https:" || (u.protocol === "http:" && u.hostname === "localhost");
  } catch (e) {
    return false;
  }
}

api.runtime.onConnect.addListener(function (page) {
  if (page.name !== "hexis-passkeys") return;
  const origin = originOf(page.sender);
  let host = null;
  let timer = null;
  let done = false;

  function finish(answer) {
    if (done) return;
    done = true;
    if (timer) clearTimeout(timer);
    if (host) {
      try { host.disconnect(); } catch (e) { /* already closed */ }
      host = null;
    }
    if (answer) {
      try { page.postMessage(answer); } catch (e) { /* the page is gone */ }
    }
    try { page.disconnect(); } catch (e) { /* already closed */ }
  }

  page.onDisconnect.addListener(function () { finish(null); });

  page.onMessage.addListener(function (message) {
    if (host || done || !message || typeof message.id !== "string") return;
    const op = message.op;
    if (op !== "get" && op !== "create" && op !== "status") return finish(fallback("invalid_request"));
    if (op !== "status" && (!origin || !secure(origin))) return finish(fallback("invalid_request"));
    if (op !== "status" && (typeof message.request !== "string" || message.request.length > MAX_REQUEST)) {
      return finish(fallback("invalid_request"));
    }
    const wanted = typeof message.timeoutMs === "number" ? message.timeoutMs : 60000;
    const timeoutMs = Math.min(Math.max(wanted, 5000), 120000);
    try {
      host = api.runtime.connectNative(HOST);
    } catch (e) {
      return finish(fallback("not_installed"));
    }
    host.onMessage.addListener(function (answer) { finish(answer); });
    host.onDisconnect.addListener(function () {
      // the host went away without answering: it is not set up (no manifest) or the app is not running
      host = null;
      finish(fallback("not_installed"));
    });
    timer = setTimeout(function () { finish(fallback("no_answer")); }, timeoutMs + SLACK_MS);
    host.postMessage({
      v: 1,
      id: message.id,
      op: op,
      origin: op === "status" ? undefined : origin,
      request: op === "status" ? undefined : message.request,
      timeoutMs: timeoutMs,
    });
  });
});
