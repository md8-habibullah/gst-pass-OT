import type { PasskeyProviderStatus } from "../backend";
import { be } from "./app";
import { createStore } from "./store";

const NOT_SUPPORTED: PasskeyProviderStatus = { supported: false, enabled: false };

/**
 * Whether this phone can use Hexis as a passkey provider (Android 14 or later) and whether the provider is on. `null` until
 * the core has answered: Settings shows the Passkeys page only for a phone that supports it.
 */
export const passkeyProvider = createStore<PasskeyProviderStatus | null>(null);

export async function refreshPasskeyProvider(): Promise<PasskeyProviderStatus> {
  try {
    const status = await be().passkeyProviderStatus();
    passkeyProvider.set(status);
    return status;
  } catch {
    // An older core without the command, or a plugin that is not there: the page stays hidden.
    passkeyProvider.set(NOT_SUPPORTED);
    return NOT_SUPPORTED;
  }
}

export async function setPasskeyProvider(on: boolean): Promise<PasskeyProviderStatus> {
  const status = await be().passkeyProviderSet(on);
  passkeyProvider.set(status);
  return status;
}
