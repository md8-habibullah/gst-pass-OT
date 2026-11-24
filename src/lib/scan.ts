// QR scanning for one-time codes, all inside the app (no external scanner app).
//   Android / iOS: the official Tauri barcode-scanner plugin (asks for the camera permission, shows the camera in-app).
//   Everywhere:    decode a QR code from an image (chosen file, dropped file or pasted screenshot) with jsQR.
import { isTauri } from "@tauri-apps/api/core";
import jsQR from "jsqr";
import { brandName } from "../state/brand";
import { createStore } from "../state/store";
import { withNativeUi } from "./nativeUi";

export class ScanError extends Error {
  constructor(message: string, readonly kind: "denied" | "failed" | "cancelled" = "failed") {
    super(message);
  }
}

/** True on the mobile shells, where the camera plugin exists. */
export const canScanWithCamera = (): boolean => isTauri() && /Android|iPhone|iPad/i.test(navigator.userAgent);

export const scanState = createStore<{ active: boolean }>({ active: false });

function setScanning(on: boolean) {
  scanState.set({ active: on });
  if (on) document.documentElement.dataset.scanning = "1";
  else delete document.documentElement.dataset.scanning;
}

/** Opens the in-app camera. Resolves with the QR text, or null if the person cancelled. */
export async function scanWithCamera(): Promise<string | null> {
  const scanner = await import("@tauri-apps/plugin-barcode-scanner");
  let state = await scanner.checkPermissions();
  // The permission prompt is a system screen on top of the app: it must not count as leaving the app.
  if (state !== "granted") state = await withNativeUi(() => scanner.requestPermissions());
  if (state !== "granted") throw new ScanError(`Camera access is turned off for ${brandName()}. Allow it in the app settings to scan QR codes.`, "denied");
  setScanning(true);
  try {
    const result = await withNativeUi(() => scanner.scan({ windowed: true, formats: [scanner.Format.QRCode] }));
    return result.content;
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    if (/cancel/i.test(msg)) return null;
    throw new ScanError("The camera could not read a QR code. Try again, or paste the setup key instead.");
  } finally {
    setScanning(false);
  }
}

export async function cancelCameraScan(): Promise<void> {
  try {
    const scanner = await import("@tauri-apps/plugin-barcode-scanner");
    await scanner.cancel();
  } finally {
    setScanning(false);
  }
}

export async function openCameraSettings(): Promise<void> {
  const scanner = await import("@tauri-apps/plugin-barcode-scanner");
  await scanner.openAppSettings();
}

const MAX_SIDE = 1600;

/** Decodes the first QR code in an image. Tries the full image, a smaller copy and an inverted copy. */
export async function decodeQrFromImage(file: Blob): Promise<string> {
  if (file.size > 25 * 1024 * 1024) throw new ScanError("That image is too large.");
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    throw new ScanError("That file could not be opened as an image.");
  }
  try {
    for (const scale of [1, 0.5, 0.25]) {
      const longest = Math.max(bitmap.width, bitmap.height) * scale;
      const k = longest > MAX_SIDE ? MAX_SIDE / longest : scale;
      const w = Math.max(1, Math.round(bitmap.width * k));
      const h = Math.max(1, Math.round(bitmap.height * k));
      const canvas = document.createElement("canvas");
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx) throw new ScanError("Image decoding is not available here.");
      ctx.drawImage(bitmap, 0, 0, w, h);
      const data = ctx.getImageData(0, 0, w, h);
      const hit = jsQR(data.data, w, h, { inversionAttempts: "attemptBoth" });
      if (hit?.data) return hit.data;
    }
  } finally {
    bitmap.close();
  }
  throw new ScanError("No QR code was found in that image. Crop closer to the code, or paste the setup key instead.");
}
