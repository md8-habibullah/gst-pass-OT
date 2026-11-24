import { CircleCheck } from "lucide-react";
import { isMobileShell } from "../../lib/platform";

/**
 * The last screen after "Remove Hexis from this device". The core has already removed everything and, on a computer,
 * closes the app a moment later. Nothing here talks to the core: there is nothing left for it to answer with.
 */
export function RemovedScreen() {
  const phone = isMobileShell();
  return (
    <main className="grid min-h-dvh place-items-center px-6 py-10">
      <div className="surface-float flex max-w-md flex-col gap-4 rounded-xl p-7">
        <CircleCheck className="size-8 text-verdigris" aria-hidden />
        <h1 className="font-display text-2xl leading-tight font-semibold">Everything is removed</h1>
        <p role="status" className="text-mist">
          Every vault, setting and cache that Hexis kept on this {phone ? "phone" : "device"} is gone. Nothing is left to clean up. Backups in your S3 buckets were not touched.
        </p>
        {phone ? (
          <p>Now uninstall Hexis: press and hold its icon, choose App info, then Uninstall. If you open it again first, it starts like a fresh install and makes new files.</p>
        ) : (
          <p>Hexis is closing. Now uninstall it the way you installed it: with your package manager, or with the installer and <span className="font-mono text-[0.92em] whitespace-nowrap">--uninstall</span>. Starting it again before that begins like a fresh install and makes new files.</p>
        )}
      </div>
    </main>
  );
}
