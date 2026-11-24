import { nativeUiOpen } from "../lib/nativeUi";
import { app, be, lockApp, lockVault, syncAfterCoreLock } from "./app";

const PING_EVERY_MS = 10_000;
const WATCH_EVERY_MS = 8_000;
const RECONCILE_WATCH_MS = 2_000;

/**
 * While a vault is open: tell the core when the person is active (so its inactivity timer restarts) and notice when the
 * core has locked the vault by itself. The timer lives in the core; the webview only reports input and reads the result.
 * Returns the stop function.
 */
export function watchActivity(): () => void {
  let lastPing = 0;
  const ping = () => {
    const now = Date.now();
    if (now - lastPing < PING_EVERY_MS) return;
    lastPing = now;
    void be().activityPing().catch(() => undefined);
  };
  const events = ["pointerdown", "keydown", "wheel", "touchstart"] as const;
  for (const e of events) window.addEventListener(e, ping, { passive: true });
  // Reads the core's own status (a local call, no network). Normally every 8 s; every 2 s while a pull from the backup
  // is running, so another device's changes show up as soon as they land; and once whenever the window regains focus.
  let stopped = false;
  let timer = 0;
  const look = async () => {
    window.clearTimeout(timer);
    const busy = await syncAfterCoreLock().catch(() => false);
    if (!stopped) timer = window.setTimeout(() => { void look(); }, busy ? RECONCILE_WATCH_MS : WATCH_EVERY_MS);
  };
  timer = window.setTimeout(() => { void look(); }, RECONCILE_WATCH_MS); // the pull starts at unlock: look soon
  const onFocus = () => { void look(); };
  window.addEventListener("focus", onFocus);
  // Phones: when the app goes to the background, (1) ask the core to send whatever is still queued (one of the three
  // sync triggers; bounded to a few seconds, and the queue is durable, so nothing is lost if the system stops us) and
  // (2) lock: the keys are zeroized and the next visit needs the master password again. A native picker or the camera
  // on top of the app is not "leaving", so it is excluded.
  const onHide = () => {
    if (document.visibilityState !== "hidden" || nativeUiOpen()) return;
    void (async () => {
      await be().syncFlush().catch(() => undefined);
      if (document.visibilityState !== "hidden" || nativeUiOpen()) return;
      const phase = app.get().phase;
      const gateOn = app.get().gate?.enabled === true;
      // Stage 1 closes too, also from the account list or the password prompt (no vault open there).
      if (phase === "ready") await (gateOn ? lockApp() : lockVault()).catch(() => undefined);
      else if (gateOn && (phase === "picker" || phase === "locked" || phase === "welcome")) await lockApp().catch(() => undefined);
    })();
  };
  const mobile = /Android|iPhone|iPad/i.test(navigator.userAgent);
  if (mobile) document.addEventListener("visibilitychange", onHide);
  ping();
  return () => {
    for (const e of events) window.removeEventListener(e, ping);
    if (mobile) document.removeEventListener("visibilitychange", onHide);
    window.removeEventListener("focus", onFocus);
    stopped = true;
    window.clearTimeout(timer);
  };
}
