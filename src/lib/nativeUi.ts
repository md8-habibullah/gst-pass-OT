// While a native picker, the camera or a save dialog is on top of the app, the system may report the web view as
// hidden. That is not the person leaving, so the "lock when the app goes to the background" rule must not fire.
let depth = 0;
let until = 0;

/** True while a native screen is open and for a moment after it closes (the web view returns a beat later). */
export const nativeUiOpen = (): boolean => depth > 0 || Date.now() < until;

export async function withNativeUi<T>(run: () => Promise<T>): Promise<T> {
  depth += 1;
  try {
    return await run();
  } finally {
    depth -= 1;
    until = Date.now() + 2500;
  }
}

/** Commands that put a native screen on top of the app. */
export const NATIVE_UI_METHODS = ["fileAdd", "fileSaveCopy", "importPick", "importPickKeyfile", "importProfileFile", "exportVaultFile", "exportPlain"] as const;

/** Wraps a backend so those methods announce their native screen. Everything else is passed through untouched. */
export function announceNativeUi<T extends object>(backend: T): T {
  return new Proxy(backend, {
    get(target, prop, receiver) {
      const value = Reflect.get(target, prop, receiver) as unknown;
      if (typeof value === "function" && (NATIVE_UI_METHODS as readonly string[]).includes(String(prop))) {
        return (...args: unknown[]) => withNativeUi(() => (value as (...a: unknown[]) => Promise<unknown>).apply(target, args));
      }
      return value;
    },
  });
}
