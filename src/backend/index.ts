import { isTauri } from "@tauri-apps/api/core";
import { tauriBackend } from "./tauri";
import type { Backend } from "./types";

export * from "./types";

let cached: Promise<Backend> | null = null;

/**
 * Inside the Tauri shell this is always the real Rust core. The in-memory preview backend is only
 * reachable in a development browser (import.meta.env.DEV): the dynamic import is dead-code-eliminated
 * from production builds, so the insecure preview can never ship.
 */
export function getBackend(): Promise<Backend> {
  cached ??= (async () => {
    if (isTauri()) return tauriBackend;
    if (import.meta.env.DEV) return (await import("./mock")).previewBackend;
    throw new Error("Passman must run inside its desktop or Android shell.");
  })();
  return cached;
}
