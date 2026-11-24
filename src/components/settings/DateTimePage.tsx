import { useCallback, useEffect, useState } from "react";
import { Clock, RefreshCw } from "lucide-react";
import type { ClockStatus } from "../../backend";
import { exactTime, relativeTime } from "../../lib/format";
import { offsetLabel, offsetMinutes, zoneLabel } from "../../lib/zones";
import { app, be, chooseTimeZone, pushToast, setTimeCheck } from "../../state/app";
import { useStore } from "../../state/store";
import { Button } from "../ui/Button";
import { Switch } from "../ui/Inputs";
import { ZonePicker } from "../ui/ZonePicker";
import { Group, Row } from "./parts";

/** "1.2 seconds", "3 minutes", "2 hours 5 minutes": how far apart two clocks are. */
export function describeSkew(ms: number): string {
  const abs = Math.abs(ms);
  if (abs < 1500) return "less than two seconds";
  if (abs < 60_000) return `${Math.round(abs / 1000)} seconds`;
  if (abs < 3_600_000) { const m = Math.round(abs / 60_000); return `${m} ${m === 1 ? "minute" : "minutes"}`; }
  const h = Math.floor(abs / 3_600_000);
  const m = Math.round((abs % 3_600_000) / 60_000);
  return `${h} ${h === 1 ? "hour" : "hours"}${m ? ` ${m} ${m === 1 ? "minute" : "minutes"}` : ""}`;
}

function statusLine(s: ClockStatus): string {
  if (!s.enabled) return "The online check is off. Waits rest on this device's own clock.";
  if (!s.verified) return s.problem ?? "The time has not been checked yet.";
  const ago = s.checkedAgoMs !== null ? `Checked ${relativeTime(0, s.checkedAgoMs)}` : "Checked";
  return `${ago} with ${s.sources} ${s.sources === 1 ? "website" : "websites"}.`;
}

/** The time zone times are shown in, and the online check of the real time. Everything stored is UTC either way. */
export function DateTimePage() {
  const device = useStore(app, (s) => s.device);
  const [clock, setClock] = useState<ClockStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [tick, setTick] = useState(() => Date.now());

  const load = useCallback(() => void be().clockStatus().then(setClock).catch(() => undefined), []);
  useEffect(() => {
    load();
    const id = window.setInterval(() => { load(); setTick(Date.now()); }, 5000);
    return () => window.clearInterval(id);
  }, [load]);

  const zone = device?.timeZone ?? null;
  const nowUtc = clock?.nowMs ?? tick;

  async function pick(next: string) {
    if (next === zone) return;
    try {
      await chooseTimeZone(next);
      pushToast({ kind: "success", text: `Times are now shown in ${zoneLabel(next)}.` });
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "Could not save the time zone." });
    }
  }
  async function toggle(on: boolean) {
    try {
      await setTimeCheck(on);
      load();
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "Could not change the time check." });
    }
  }
  async function checkNow() {
    setBusy(true);
    try {
      setClock(await be().clockSync());
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "The time could not be checked." });
    } finally {
      setBusy(false);
    }
  }

  const skew = clock?.verified && clock.skewMs !== null ? clock.skewMs : null;
  return (
    <>
      <Group title="Time zone" heading={false}>
        <Row title="Your time zone" description={zone ? `${zoneLabel(zone)}, ${offsetLabel(offsetMinutes(zone, nowUtc))}` : "Not chosen yet"}>
          <span className="flex items-center gap-2 text-mist"><Clock className="size-4" aria-hidden />{exactTime(nowUtc)}</span>
        </Row>
        <div className="border-t border-hairline p-4">
          <ZonePicker value={zone} onChange={(z) => void pick(z)} />
        </div>
      </Group>

      <Group title="Real time" heading={false}>
        <Row
          title="Check the real time online"
          description="Asks a few large websites (Google, Cloudflare, Amazon) what time it is. A wrong clock on this device then cannot shorten a lock wait or back-date what you save."
        >
          <Switch label="Check the real time online" checked={device?.timeCheck ?? true} onChange={(on) => void toggle(on)} />
        </Row>
        <Row title="Status" description={clock ? statusLine(clock) : "Reading the status"}>
          <Button icon={<RefreshCw className="size-4" />} busy={busy} disabled={!device?.timeCheck} onClick={() => void checkNow()}>Check now</Button>
        </Row>
        {skew !== null && (
          <Row
            title="This device's clock"
            description={Math.abs(skew) < 1500 ? "Matches the real time." : `${describeSkew(skew)} ${skew > 0 ? "behind" : "ahead of"} the real time. Hexis uses the real time. Set the device clock right so other apps agree.`}
          />
        )}
      </Group>

      <p className="text-[0.86em] leading-5 text-fog">
        Each check sends one plain request to each website and reads the time from its reply. No account, vault or device detail goes with it. The websites see your internet address, as with any page you open. With the check off, a lock wait still cannot be shortened while the app is open, but after a restart it is judged by this device&apos;s clock.
      </p>
    </>
  );
}
