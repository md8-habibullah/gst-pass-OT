import { useEffect } from "react";
import { MotionConfig } from "motion/react";
import { boot, app } from "./state/app";
import { useStore } from "./state/store";
import { LockScreen } from "./components/lock/LockScreen";
import { VaultMark } from "./components/lock/VaultMark";
import { RepromptHost } from "./components/reprompt/Reprompt";
import { Shell } from "./components/shell/Shell";
import { ItemEditor } from "./components/items/ItemEditor";
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
    <main className="grid min-h-dvh place-items-center" aria-busy="true" aria-label="Starting Passman">
      <VaultMark size={56} busy />
    </main>
  );
}

function Failed({ message }: { message: string | null }) {
  return (
    <main className="grid min-h-dvh place-items-center px-6">
      <div className="glass-strong max-w-md rounded-xl p-7 text-center">
        <h1 className="font-display text-2xl font-semibold">Passman could not start</h1>
        <p className="mt-3 text-mist">{message ?? "The secure core did not respond."}</p>
        <p className="mt-2 text-[0.93em] text-fog">Your vault file was not touched. Close and reopen the app; if this keeps happening, check the logs.</p>
      </div>
    </main>
  );
}

export default function App() {
  const phase = useStore(app, (s) => s.phase);
  const bootError = useStore(app, (s) => s.bootError);
  useEffect(() => { void bootOnce(); }, []);

  return (
    <MotionConfig reducedMotion="user">
      {phase === "boot" && <Splash />}
      {phase === "failed" && <Failed message={bootError} />}
      {(phase === "setup" || phase === "locked") && <LockScreen />}
      {phase === "ready" && (
        <>
          <Shell />
          <ItemEditor />
        </>
      )}
      <RepromptHost />
      <Toasts />
    </MotionConfig>
  );
}
