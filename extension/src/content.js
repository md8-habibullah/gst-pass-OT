// Hexis passkeys: the extension's script in each page. It sits between the page's own script (page.js, which can be reached by
// anything the page runs) and the extension's background. Whatever the page says, this script decides the page's address
// itself (`location.origin`), checks the shape of the request, and passes it on.
(function () {
  "use strict";
  const api = typeof browser !== "undefined" ? browser : chrome;
  const MAX_REQUEST = 64 * 1024;
  const MAX_AT_ONCE = 4;
  const open = new Map(); // request id -> the port to the background

  function reply(id, result) {
    window.postMessage({ source: "hexis-content", id: id, result: result }, location.origin);
  }

  function validId(id) {
    return typeof id === "string" && /^[A-Za-z0-9_-]{8,64}$/.test(id);
  }

  function ask(data) {
    const id = data.id;
    if (open.has(id) || open.size >= MAX_AT_ONCE) {
      reply(id, { ok: false, code: "fallback", reason: "busy" });
      return;
    }
    // A tab nobody is looking at must not raise the Hexis window and pull the person's typing into it: the browser's own flow
    // refuses such a call, so it gets it.
    if (data.op !== "status" && document.visibilityState !== "visible") {
      reply(id, { ok: false, code: "fallback", reason: "not_visible" });
      return;
    }
    const request = data.request;
    if (data.op !== "status" && (typeof request !== "string" || request.length === 0 || request.length > MAX_REQUEST)) {
      reply(id, { ok: false, code: "fallback", reason: "invalid_request" });
      return;
    }
    let port;
    try {
      port = api.runtime.connect({ name: "hexis-passkeys" });
    } catch (e) {
      reply(id, { ok: false, code: "fallback", reason: "no_extension" });
      return;
    }
    open.set(id, port);
    let answered = false;
    port.onMessage.addListener(function (message) {
      answered = true;
      open.delete(id);
      reply(id, message);
      try { port.disconnect(); } catch (e) { /* already closed */ }
    });
    port.onDisconnect.addListener(function () {
      open.delete(id);
      if (!answered) reply(id, { ok: false, code: "fallback", reason: "no_extension" });
    });
    port.postMessage({
      id: id,
      op: data.op,
      origin: location.origin,
      request: data.op === "status" ? undefined : request,
      timeoutMs: typeof data.timeoutMs === "number" ? data.timeoutMs : undefined,
    });
  }

  window.addEventListener("message", function (event) {
    const data = event.data;
    if (event.source !== window || !data || data.source !== "hexis-page" || !validId(data.id)) return;
    if (data.op === "cancel") {
      const port = open.get(data.id);
      open.delete(data.id);
      if (port) {
        try { port.disconnect(); } catch (e) { /* already closed */ }
      }
      return;
    }
    if (data.op === "get" || data.op === "create" || data.op === "status") ask(data);
  });

  // leaving the page (or a reload) ends what it asked, so no prompt is left for a page that is gone
  window.addEventListener("pagehide", function () {
    open.forEach(function (port) {
      try { port.disconnect(); } catch (e) { /* already closed */ }
    });
    open.clear();
  });
})();
