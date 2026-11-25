import { nativeUiOpen } from "../lib/nativeUi";
import { isMobileShell } from "../lib/platform";
import { be, syncAfterCoreLock } from "./app";

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
  // Offline is a normal state: items are read, made and edited with no connection, and the changes wait in the durable
  // queue. When the device comes back online the core pulls what other devices changed (merging it with this device's
  // own changes) and sends what was waiting. This is the fourth sync trigger, still no timer: the system tells us.
  const onOnline = () => { void be().syncReconnect().catch(() => undefined).then(() => look()); };
  window.addEventListener("online", onOnline);
  // Phones: when the app goes to the background the core starts its own "after leaving the app" timer (Settings, App
  // lock; 5 minutes unless changed). When it runs out the core wipes the keys and closes Stage 1, whether or not this
  // page is still running by then. "Right away" does that at once. The first call goes out before anything else, so
  // nothing depends on the page staying alive. Then whatever is still queued for the backups is sent (one of the sync
  // triggers; bounded to a few seconds, and the queue is durable, so nothing is lost if the system stops us).
  // When the app comes back, the core locks now if the time ran out, and the screen follows. A native picker or the
  // camera on top of the app is not "leaving", so it is excluded (see lib/nativeUi.ts).
  const onVisibility = () => {
    if (document.visibilityState === "hidden") {
      if (nativeUiOpen()) return;
      void be().appLeave().catch(() => undefined).then(() => syncAfterCoreLock()).catch(() => undefined);
      void be().syncFlush().catch(() => undefined);
    } else {
      void be().appReturn().catch(() => undefined).then(() => syncAfterCoreLock()).catch(() => undefined);
      ping();
    }
  };
  const mobile = isMobileShell();
  if (mobile) document.addEventListener("visibilitychange", onVisibility);
  ping();
  return () => {
    for (const e of events) window.removeEventListener(e, ping);
    if (mobile) document.removeEventListener("visibilitychange", onVisibility);
    window.removeEventListener("focus", onFocus);
    window.removeEventListener("online", onOnline);
    stopped = true;
    window.clearTimeout(timer);
  };
}
