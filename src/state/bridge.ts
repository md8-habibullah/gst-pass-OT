import type { BridgeRequest, BridgeStatus } from "../backend";
import { app, be, pushToast, refreshDevice } from "./app";
import { createStore } from "./store";

/**
 * Browsers on this Linux computer ask Hexis for passkeys through the Hexis extension (`ai-docs/PASSKEYS.md`, "Browsers on
 * Linux"). The core queues each request and this module shows the one in front. `status` is `null` until the core has
 * answered; Settings shows the Browsers page only where `supported` is true.
 */
export interface BridgeState {
  status: BridgeStatus | null;
  /** The request in front, if a website is waiting. */
  front: BridgeRequest | null;
  /** When `front` reached the screen (its remaining time counts from here). */
  arrivedAt: number;
}

export const bridge = createStore<BridgeState>({ status: null, front: null, arrivedAt: 0 });

/** Requests the person already dealt with in the window, so a request that ends by itself is told apart from one that was answered. */
const handled = new Set<string>();
export const markHandled = (id: string) => { handled.add(id); if (handled.size > 50) handled.delete(handled.values().next().value as string); };

export async function refreshBridge(): Promise<BridgeStatus | null> {
  try {
    const status = await be().bridgeStatus();
    bridge.set((s) => ({ ...s, status }));
    return status;
  } catch {
    // An older core without the command, or the app lock is closed: the page stays hidden for now.
    return null;
  }
}

async function adopt(status: BridgeStatus): Promise<BridgeStatus> {
  bridge.set((s) => ({ ...s, status, front: status.enabled ? s.front : null }));
  await refreshDevice().catch(() => undefined); // the switch is part of the device settings the rest of the app reads
  return status;
}

/** The switch. The core opens its private socket first and keeps the setting only if that worked. */
export const setBridge = async (on: boolean) => adopt(await be().bridgeSet(on));
export const registerBrowsers = async (ids: string[]) => adopt(await be().bridgeRegister(ids));
export const unregisterBrowsers = async (ids: string[]) => adopt(await be().bridgeUnregister(ids));

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/**
 * Listens for requests: one long call at a time, answered the moment something changes, so a request shows at once and
 * nothing runs in between. Returns the function that stops it.
 */
export function listenForRequests(): () => void {
  let stop = false;
  void (async () => {
    let after = 0;
    let failures = 0;
    let lastId: string | null = null;
    while (!stop) {
      try {
        const w = await be().bridgeWait(after, 25_000);
        if (stop) return;
        failures = 0;
        after = w.version;
        const id = w.front?.id ?? null;
        if (id !== lastId) {
          // a request that ended on its own (the tab closed, it timed out) while the person had not answered it
          if (lastId && !handled.has(lastId)) pushToast({ kind: "info", text: "The website stopped waiting for a passkey." });
          lastId = id;
        }
        bridge.set((s) => (s.front?.id === id && id !== null ? s : { ...s, front: w.front, arrivedAt: Date.now() }));
      } catch {
        // the app lock is closed, or the core is busy: look again in a moment
        if (stop) return;
        failures += 1;
        await sleep(Math.min(5000, 800 * failures));
      }
    }
  })();
  return () => {
    stop = true;
    bridge.set((s) => ({ ...s, front: null }));
  };
}

export const bridgeEnabled = () => app.get().device?.browserBridge === true;
