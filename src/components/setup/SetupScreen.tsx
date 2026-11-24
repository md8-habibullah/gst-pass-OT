import { useState } from "react";
import { motion } from "motion/react";
import { BackendError } from "../../backend";
import { DUR, tween } from "../../lib/motion";
import { deviceZone, offsetLabel, offsetMinutes, zoneLabel } from "../../lib/zones";
import { confirmTimeZone } from "../../state/app";
import { Button } from "../ui/Button";
import { ZonePicker } from "../ui/ZonePicker";
import { Door, DoorCard } from "../lock/Door";
import { VaultMark } from "../lock/VaultMark";

/**
 * The first page, always: which time zone to show times in. Mandatory (there is no skip), suggested from the device.
 * Everything stored stays UTC; this only decides how a moment is written for the person. Changeable in Settings.
 */
export function SetupScreen() {
  const suggested = deviceZone();
  const [zone, setZone] = useState<string>(suggested);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function next() {
    setBusy(true);
    setError(null);
    try {
      await confirmTimeZone(zone);
    } catch (e) {
      setBusy(false);
      setError(e instanceof BackendError || e instanceof Error ? e.message : "The time zone could not be saved.");
    }
  }

  return (
    <Door>
      <motion.div initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} transition={tween(DUR.slow)}>
        <DoorCard>
          <div className="mb-5 flex flex-col items-center gap-4 text-center min-[62rem]:hidden">
            <VaultMark size={52} />
          </div>
          <h1 className="font-display text-[1.75rem] leading-[1.15] font-medium tracking-[-0.01em]">Choose your time zone</h1>
          <p className="mt-2 text-mist">Times are saved in UTC and shown in your zone. You can change it later in Settings.</p>
          <div className="mt-5">
            <ZonePicker value={zone} onChange={setZone} autoFocus />
          </div>
          <p className="mt-3 text-[0.93em] text-mist" aria-live="polite">
            {zoneLabel(zone)}, {offsetLabel(offsetMinutes(zone, Date.now()))}
            {zone === suggested ? ". Suggested by this device." : "."}
          </p>
          {error && <p role="alert" className="mt-3 text-[0.93em] text-ember">{error}</p>}
          <Button variant="key" block busy={busy} onClick={() => void next()} className="mt-5 min-h-11">Continue</Button>
          <p className="mt-4 text-[0.86em] leading-5 text-fog">
            To keep lock waits honest, the app also checks the real time with a few large websites. Changing this device&apos;s clock cannot shorten a wait. You can switch the check off in Settings.
          </p>
        </DoorCard>
      </motion.div>
    </Door>
  );
}
