import { CloudDownload, FileLock2, ShieldPlus } from "lucide-react";
import { app, finishFirstRun } from "../../state/app";
import { useBrandName } from "../../state/brand";
import { useStore } from "../../state/store";
import { Button } from "../ui/Button";
import { Sheet } from "../ui/Sheet";

/**
 * Shown once, over the demo vault, on a genuine first start. It says plainly what the demo is and offers the real
 * choices. After an erase it never appears (the time zone and the "seen" flag survive it), so a reset says nothing.
 */
export function FirstRunSheet() {
  const name = useBrandName();
  const show = useStore(app, (s) => s.phase === "ready" && s.device !== null && !s.device.setupDone && s.profile?.demo === true);
  const explore = () => void finishFirstRun();
  return (
    <Sheet open={show} onClose={explore} title="You are in a demo vault" variant="dialog" size="sm">
      <p className="text-mist">
        It holds made-up accounts so you can look around {name} before adding anything real. Nothing in it belongs to anyone. Passwords and codes are generated fresh on this device.
      </p>
      <div className="mt-5 flex flex-col gap-2">
        <Button variant="key" block icon={<ShieldPlus className="size-4" />} className="min-h-11" onClick={() => void finishFirstRun("create")}>Create my own vault</Button>
        <Button block icon={<FileLock2 className="size-4" />} className="min-h-11" onClick={() => void finishFirstRun("import")}>Open a vault file</Button>
        <Button block icon={<CloudDownload className="size-4" />} className="min-h-11" onClick={() => void finishFirstRun("restore")}>Restore from a backup</Button>
        <Button variant="ghost" block className="min-h-11" onClick={explore}>Explore the demo</Button>
      </div>
      <p className="mt-4 text-[0.86em] text-fog">You can remove the demo vault later in Settings, under Accounts.</p>
    </Sheet>
  );
}
