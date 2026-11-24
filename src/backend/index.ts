import { isTauri } from "@tauri-apps/api/core";
import { announceNativeUi } from "../lib/nativeUi";
import { tauriBackend } from "./tauri";
import type { Backend } from "./types";

export * from "./types";

let cached: Promise<Backend> | null = null;

/**
 * Inside the Tauri shell this is always the real Rust core. The in-memory preview backend and the HTTP
 * end-to-end backend are only reachable in a development browser (import.meta.env.DEV): the dynamic imports
 * are dead-code-eliminated from production builds, so neither can ever ship.
 */
export function getBackend(): Promise<Backend> {
  cached ??= (async () => {
    if (isTauri()) return announceNativeUi(tauriBackend);
    if (import.meta.env.DEV) {
      const port = Number(new URLSearchParams(location.search).get("e2e"));
      if (Number.isInteger(port) && port > 0) return (await import("./http")).httpBackend(port);
      return (await import("./mock")).previewBackend;
    }
    throw new Error("Passman must run inside its desktop or Android shell.");
  })();
  return cached;
}
