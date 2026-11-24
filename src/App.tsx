import { useEffect } from "react";
import { MotionConfig } from "motion/react";
import { watchActivity } from "./state/activity";
import { boot, app } from "./state/app";
import { brand } from "./state/brand";
import { useStore } from "./state/store";
import { GateScreen } from "./components/gate/GateScreen";
import { LockScreen } from "./components/lock/LockScreen";
import { VaultMark } from "./components/lock/VaultMark";
import { RepromptHost } from "./components/reprompt/Reprompt";
import { Shell } from "./components/shell/Shell";
import { ItemEditor } from "./components/items/ItemEditor";
import { ItemMenuHost } from "./components/items/ItemMenu";
import { ScanOverlay } from "./components/scan/ScanOverlay";
import { Toasts } from "./components/ui/Toasts";

let booted: Promise<void> | null = null;
function bootOnce(): Promise<void> {
  booted ??= boot().then(async () => {
    if (import.meta.env.DEV) await (await import("./state/dev")).devBootstrap();
  });
  return booted;
}

function Splash() {
  return (
    <main className="grid min-h-dvh place-items-center" aria-busy="true" aria-label="Starting">
      <VaultMark size={56} busy />
    </main>
  );
}

function Failed({ message }: { message: string | null }) {
  const name = useStore(brand, (b) => b.name);
  return (
    <main className="grid min-h-dvh place-items-center px-6">
      <div className="glass-strong max-w-md rounded-xl p-7 text-center">
        <h1 className="font-display text-2xl font-semibold">{name} could not start</h1>
        <p className="mt-3 text-mist">{message ?? "The secure core did not respond."}</p>
        <p className="mt-2 text-[0.93em] text-fog">Your vault files were not touched. Close and reopen the app; if this keeps happening, check the logs.</p>
      </div>
    </main>
  );
}

export default function App() {
  const phase = useStore(app, (s) => s.phase);
  const bootError = useStore(app, (s) => s.bootError);
  useEffect(() => { void bootOnce(); }, []);
  // The core runs the inactivity timer (it covers a passed Stage 1 even before a vault is open); the webview only
  // reports input and notices the result. Not while the gate itself is showing.
  const watching = phase === "ready" || phase === "picker" || phase === "locked" || phase === "welcome";
  useEffect(() => (watching ? watchActivity() : undefined), [watching]);

  return (
    <MotionConfig reducedMotion="user">
      {phase === "boot" && <Splash />}
      {phase === "failed" && <Failed message={bootError} />}
      {phase === "gate" && <GateScreen />}
      {(phase === "welcome" || phase === "picker" || phase === "locked") && <LockScreen />}
      {phase === "ready" && (
        <>
          <Shell />
          <ItemEditor />
          <ItemMenuHost />
        </>
      )}
      <RepromptHost />
      <Toasts />
      <ScanOverlay />
    </MotionConfig>
  );
}
