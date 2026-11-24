// DEV-ONLY: the real Rust core over HTTP, for end-to-end tests in a headless browser.
// `cargo run --example e2e_server` hosts the same command layer the app uses (via Tauri's mock runtime).
// Loaded only behind import.meta.env.DEV and an explicit ?e2e=<port> in the URL, never in production builds.
import { makeBackend, normalizeError, type Call } from "./tauri";
import type { Backend } from "./types";

// DEV test aid: `&legacy=1` lets the UI run against an older shim binary that predates the app-level commands;
// `&legacy=2` does the same but leaves `disguise_get` to the shim (for a binary that already has the disguise commands).
const LEGACY: Record<string, unknown> = {
  gate_status: { enabled: false, open: true, mode: "pin", biometricAvailable: false, biometric: false, panicButton: false, equation: null, attemptsLeft: null, streak: 0 },
  disguise_get: { presetId: null, customName: null, lowercase: false, appliedName: "Passman", note: null },
  app_settings_get: { lockAfterMinutes: 15 },
  activity_ping: null,
  sync_flush: { complete: true },
  screen_secure: null,
};

export function httpBackend(port: number): Backend {
  const mode = new URLSearchParams(location.search).get("legacy");
  const call: Call = async <T,>(cmd: string, args?: Record<string, unknown>) => {
    if ((mode === "1" || (mode === "2" && cmd !== "disguise_get")) && cmd in LEGACY) return LEGACY[cmd] as T;
    let res: Response;
    try {
      res = await fetch(`http://127.0.0.1:${port}/ipc`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ cmd, args: args ?? {} }) });
    } catch {
      throw normalizeError("The end-to-end server is not reachable.");
    }
    // `file_read` answers with raw bytes (like tauri::ipc::Response in the app) and says so in a header.
    if (res.headers.get("x-passman-binary") === "1") return (await res.arrayBuffer()) as T;
    const body = (await res.json()) as { ok?: T; err?: unknown };
    if (body.err !== undefined) throw normalizeError(body.err);
    return body.ok as T;
  };
  return makeBackend(call, "tauri");
}
