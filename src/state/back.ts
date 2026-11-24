// The Android Back button. The native activity (scripts/android/MainActivity.kt.tmpl) asks the page before it acts:
// it calls `window.__hexisBack()`, and when that returns true the page has dealt with the button (a sheet closed, the
// detail page went back to the list). Only when nothing is open does the app go to the background.
//
// Anything that Back should close registers itself while it is open and removes itself when it closes. The most recently
// registered one is asked first, so a sheet above a detail page closes before the page does.
import { useEffect, useRef } from "react";

interface Entry {
  run: () => void;
}

const stack: Entry[] = [];

/** Registers a handler. Returns the function that removes it. */
export function pushBack(run: () => void): () => void {
  const entry: Entry = { run };
  stack.push(entry);
  return () => {
    const i = stack.lastIndexOf(entry);
    if (i >= 0) stack.splice(i, 1);
  };
}

/** Runs the most recent handler. True when there was one, false when the app itself should go to the background. */
export function handleBack(): boolean {
  const top = stack[stack.length - 1];
  if (!top) return false;
  top.run();
  return true;
}

/** Keeps `run` registered while `active` is true. `run` may change between renders: the latest one is called. */
export function useBackHandler(active: boolean, run: () => void): void {
  const latest = useRef(run);
  latest.current = run;
  // Registered once per time `active` turns on. Re-registering on every render would move the handler to the top of
  // the stack each time and break the order in which things close.
  useEffect(() => {
    if (!active) return;
    return pushBack(() => latest.current());
  }, [active]);
}

declare global {
  interface Window {
    __hexisBack?: () => boolean;
  }
}

/** Makes the handler reachable for the native side. Call once at start-up. */
export function installBackBridge(): void {
  window.__hexisBack = handleBack;
}
