import { Camera } from "lucide-react";
import { canScanWithCamera } from "../../lib/scan";
import { openScanner } from "../../state/ui";
import { IconButton } from "../ui/Button";

/** Opens the scanner. On a phone the camera starts at once; on a computer the scanner asks for a picture or a pasted key. */
export function ScanButton({ className = "" }: { className?: string }) {
  return (
    <IconButton label="Scan a code or passkey" title="Scan a code or passkey" className={className} onClick={() => openScanner(canScanWithCamera() ? "camera" : "choose")}>
      <Camera className="size-[18px]" />
    </IconButton>
  );
}
