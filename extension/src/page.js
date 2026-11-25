// Hexis passkeys: the part that runs inside the page (the "main world"), where `navigator.credentials` lives.
//
// It replaces `navigator.credentials.create` and `.get` for passkey requests with a version that asks the Hexis app (through
// the extension and the native host) and, whenever Hexis cannot or should not answer, calls the browser's own function with
// the same arguments, so a page never loses a way it already had. It knows no key and decides nothing about security: the
// app checks the page's address and the website name itself, and the page's address is added by the extension's isolated
// script, never taken from here.
//
// A classic script (no imports), because a content script in the main world cannot be a module. For tests it exposes
// `window.__hexisPasskeysInstall` when `window.__HEXIS_TEST__` is set, and does not install itself.
(function () {
  "use strict";

  function install(win) {
    if (win.__hexisPasskeysInstalled) return;
    const credentials = win.navigator && win.navigator.credentials;
    const proto = win.CredentialsContainer && win.CredentialsContainer.prototype;
    if (!credentials || !proto || typeof win.PublicKeyCredential !== "function" || !win.isSecureContext) return;
    win.__hexisPasskeysInstalled = true;

    const original = {
      create: proto.create,
      get: proto.get,
      uvpaa: win.PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable,
    };

    // ---------------------------------------------------------------- bytes and base64url

    function toBytes(source) {
      if (source instanceof win.ArrayBuffer) return new Uint8Array(source);
      if (win.ArrayBuffer.isView(source)) return new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
      throw new TypeError("Expected a buffer.");
    }

    function toBase64Url(source) {
      const bytes = toBytes(source);
      let text = "";
      for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
      return win.btoa(text).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    }

    function fromBase64Url(text) {
      const padded = String(text).replace(/-/g, "+").replace(/_/g, "/");
      const raw = win.atob(padded + "=".repeat((4 - (padded.length % 4)) % 4));
      const bytes = new Uint8Array(raw.length);
      for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
      return bytes.buffer;
    }

    // ---------------------------------------------------------------- the request, as WebAuthn JSON

    function descriptors(list) {
      return (list || []).map(function (c) {
        const d = { type: c.type, id: toBase64Url(c.id) };
        if (c.transports) d.transports = Array.from(c.transports);
        return d;
      });
    }

    function creationJson(pk) {
      const json = {
        rp: { name: pk.rp && pk.rp.name },
        user: { id: toBase64Url(pk.user.id), name: pk.user.name, displayName: pk.user.displayName },
        challenge: toBase64Url(pk.challenge),
        pubKeyCredParams: (pk.pubKeyCredParams || []).map(function (p) {
          return { type: p.type, alg: p.alg };
        }),
        excludeCredentials: descriptors(pk.excludeCredentials),
      };
      if (pk.rp && pk.rp.id) json.rp.id = pk.rp.id;
      if (typeof pk.timeout === "number") json.timeout = pk.timeout;
      if (pk.authenticatorSelection) json.authenticatorSelection = pk.authenticatorSelection;
      if (pk.attestation) json.attestation = pk.attestation;
      return json;
    }

    function requestJson(pk) {
      const json = {
        challenge: toBase64Url(pk.challenge),
        allowCredentials: descriptors(pk.allowCredentials),
      };
      if (pk.rpId) json.rpId = pk.rpId;
      if (typeof pk.timeout === "number") json.timeout = pk.timeout;
      if (pk.userVerification) json.userVerification = pk.userVerification;
      return json;
    }

    // ---------------------------------------------------------------- the answer, as the objects a page expects

    function define(target, values) {
      Object.keys(values).forEach(function (k) {
        Object.defineProperty(target, k, { value: values[k], enumerable: true, configurable: true, writable: false });
      });
      return target;
    }

    function attestationResponse(r) {
      const publicKey = r.publicKey ? fromBase64Url(r.publicKey) : null;
      return define(Object.create(win.AuthenticatorAttestationResponse.prototype), {
        clientDataJSON: fromBase64Url(r.clientDataJSON),
        attestationObject: fromBase64Url(r.attestationObject),
        getTransports: function () { return (r.transports || []).slice(); },
        getAuthenticatorData: function () { return fromBase64Url(r.authenticatorData); },
        getPublicKey: function () { return publicKey; },
        getPublicKeyAlgorithm: function () { return r.publicKeyAlgorithm; },
      });
    }

    function assertionResponse(r) {
      return define(Object.create(win.AuthenticatorAssertionResponse.prototype), {
        clientDataJSON: fromBase64Url(r.clientDataJSON),
        authenticatorData: fromBase64Url(r.authenticatorData),
        signature: fromBase64Url(r.signature),
        userHandle: r.userHandle ? fromBase64Url(r.userHandle) : null,
      });
    }

    function credential(json, kind) {
      const extensions = json.clientExtensionResults || {};
      return define(Object.create(win.PublicKeyCredential.prototype), {
        id: json.id,
        rawId: fromBase64Url(json.rawId),
        type: "public-key",
        authenticatorAttachment: json.authenticatorAttachment || null,
        response: kind === "create" ? attestationResponse(json.response) : assertionResponse(json.response),
        getClientExtensionResults: function () { return JSON.parse(JSON.stringify(extensions)); },
        toJSON: function () { return JSON.parse(JSON.stringify(json)); },
      });
    }

    // ---------------------------------------------------------------- talking to the extension's other script

    const waiting = new Map();

    win.addEventListener("message", function (event) {
      const data = event.data;
      if (event.source !== win || !data || data.source !== "hexis-content" || typeof data.id !== "string") return;
      const done = waiting.get(data.id);
      if (!done) return;
      waiting.delete(data.id);
      done(data.result);
    });

    function randomId() {
      return toBase64Url(win.crypto.getRandomValues(new Uint8Array(12)));
    }

    function post(message) {
      win.postMessage(Object.assign({ source: "hexis-page" }, message), win.location.origin);
    }

    // Ask the extension. Resolves with its answer, or with a "fallback" answer when nothing answers in time, so a page is
    // never left waiting on a broken bridge.
    function ask(op, body, signal, limitMs) {
      return new Promise(function (resolve) {
        const id = randomId();
        let timer = null;
        function finish(result) {
          if (timer) win.clearTimeout(timer);
          if (signal) signal.removeEventListener("abort", onAbort);
          waiting.delete(id);
          resolve(result);
        }
        function onAbort() {
          post({ id: id, op: "cancel" });
          finish({ ok: false, code: "aborted" });
        }
        waiting.set(id, finish);
        if (signal) signal.addEventListener("abort", onAbort, { once: true });
        timer = win.setTimeout(function () {
          post({ id: id, op: "cancel" });
          finish({ ok: false, code: "fallback", reason: "no_answer" });
        }, limitMs);
        post(Object.assign({ id: id, op: op }, body));
      });
    }

    // ---------------------------------------------------------------- when the page's call is ours to answer

    function mine(options, kind) {
      const pk = options && options.publicKey;
      if (!pk || typeof pk !== "object") return false;
      if (options.mediation === "conditional") return false; // the autofill flow is the browser's
      if (kind === "create" && pk.authenticatorSelection && pk.authenticatorSelection.authenticatorAttachment === "cross-platform") return false; // a security key
      if (kind === "get" && Array.isArray(pk.hints) && pk.hints.length > 0 && pk.hints.every(function (h) { return h === "security-key"; })) return false;
      return true;
    }

    function failure(name, message) {
      return new win.DOMException(message, name);
    }

    function reject(result) {
      if (result.code === "error") {
        switch (result.errorCode) {
          case "passkey_excluded":
            return failure("InvalidStateError", "A passkey for this account already exists.");
          case "passkey_unsupported":
            return failure("NotSupportedError", "None of the requested passkey types is supported.");
          default:
            return failure("NotAllowedError", result.message || "The operation was not allowed.");
        }
      }
      return failure("NotAllowedError", "The operation either timed out or was not allowed.");
    }

    async function run(kind, options, self) {
      if (options.signal && options.signal.aborted) return original[kind].call(self, options);
      let json;
      try {
        json = kind === "create" ? creationJson(options.publicKey) : requestJson(options.publicKey);
      } catch (e) {
        return original[kind].call(self, options); // malformed: let the browser say what is wrong
      }
      const wanted = options.publicKey.timeout;
      const timeoutMs = typeof wanted === "number" && wanted > 0 ? Math.min(Math.max(wanted, 5000), 120000) : 60000;
      const result = await ask(kind, { request: JSON.stringify(json), timeoutMs: timeoutMs }, options.signal, timeoutMs + 8000);
      if (result.ok) return credential(JSON.parse(result.responseJson), kind);
      if (result.code === "aborted") throw (options.signal.reason || failure("AbortError", "The operation was aborted."));
      if (result.code === "fallback") return original[kind].call(self, options);
      throw reject(result);
    }

    Object.defineProperty(proto, "create", {
      configurable: true,
      writable: true,
      value: function create(options) {
        if (!mine(options, "create")) return original.create.apply(this, arguments);
        return run("create", options, this);
      },
    });

    Object.defineProperty(proto, "get", {
      configurable: true,
      writable: true,
      value: function get(options) {
        if (!mine(options, "get")) return original.get.apply(this, arguments);
        return run("get", options, this);
      },
    });

    // Sites ask this before they offer to make a passkey. Say yes when Hexis is there to make it.
    let availability = { at: 0, yes: false };
    Object.defineProperty(win.PublicKeyCredential, "isUserVerifyingPlatformAuthenticatorAvailable", {
      configurable: true,
      writable: true,
      value: async function isUserVerifyingPlatformAuthenticatorAvailable() {
        const now = Date.now();
        if (now - availability.at > 20000) {
          const result = await ask("status", {}, null, 3000);
          availability = { at: Date.now(), yes: result.ok === true && result.enabled === true };
        }
        if (availability.yes) return true;
        return original.uvpaa ? original.uvpaa.call(win.PublicKeyCredential) : false;
      },
    });
  }

  if (typeof window !== "undefined") {
    window.__hexisPasskeysInstall = install;
    if (!window.__HEXIS_TEST__) install(window);
  }
})();
