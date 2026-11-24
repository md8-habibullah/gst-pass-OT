import { useEffect } from "react";
import { X } from "lucide-react";
import { cancelCameraScan, scanState } from "../../lib/scan";
import { useBackHandler } from "../../state/back";
import { useStore } from "../../state/store";
import { Button } from "../ui/Button";

/**
 * Shown while the native camera is running behind a transparent webview (Android and iOS).
 * The rest of the interface is hidden by base.css while `data-scanning` is set on <html>.
 */
export function ScanOverlay() {
  const active = useStore(scanState, (s) => s.active);
  useBackHandler(active, () => void cancelCameraScan());

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") void cancelCameraScan(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active]);

  if (!active) return null;
  const corner = "absolute size-9 border-brass";
  return (
    <div data-scan-overlay role="dialog" aria-modal="true" aria-label="Scan a QR code" className="fixed inset-0 z-[90] flex flex-col items-center justify-between px-6 pt-[calc(1.5rem+var(--safe-top))] pb-[calc(1.5rem+var(--safe-bottom))]">
      <p className="rounded-full bg-marine/80 px-5 py-2.5 text-center">Point the camera at the QR code from the website or app.</p>
      <div aria-hidden className="relative size-[min(68vw,18rem)]">
        <span className={`${corner} top-0 left-0 rounded-tl-lg border-t-[3px] border-l-[3px]`} />
        <span className={`${corner} top-0 right-0 rounded-tr-lg border-t-[3px] border-r-[3px]`} />
        <span className={`${corner} bottom-0 left-0 rounded-bl-lg border-b-[3px] border-l-[3px]`} />
        <span className={`${corner} right-0 bottom-0 rounded-br-lg border-r-[3px] border-b-[3px]`} />
      </div>
      <Button variant="key" className="min-h-12 px-8" icon={<X className="size-4" />} onClick={() => void cancelCameraScan()}>Cancel</Button>
    </div>
  );
}
