import { useState } from "react";
import { Check } from "lucide-react";
import type { Theme } from "../../backend";
import { app, setTheme } from "../../state/app";
import { useStore } from "../../state/store";
import { PageNote } from "./parts";

/** Which palette a preview paints with. "System" is drawn as both, split along a diagonal. */
type Look = "light" | "dark";

const CHOICES: { value: Theme; label: string; note: string }[] = [
  { value: "system", label: "Device setting", note: "Light by day and dark by night, or whatever your device is set to." },
  { value: "light", label: "Light", note: "Pale panes and dark ink." },
  { value: "dark", label: "Dark", note: "Deep panes and pale ink." },
];

/** A small drawing of the app in one palette: the ground, a pane with two lines of text, and the brass key. */
function Preview({ look }: { look: Look }) {
  return (
    <span data-palette={look} aria-hidden className="relative block h-full w-full overflow-hidden bg-marine">
      <span className="absolute inset-x-2.5 top-2.5 bottom-0 rounded-t-sm border border-b-0 border-hairline-strong bg-pane px-2 pt-2">
        <span className="block h-1.5 w-2/3 rounded-full bg-frost" />
        <span className="mt-1.5 block h-1 w-full rounded-full bg-mist/70" />
        <span className="mt-1 block h-1 w-4/5 rounded-full bg-mist/70" />
        <span className="mt-2 flex items-center gap-1.5">
          <span className="block h-3 w-8 rounded-[4px] bg-brass" />
          <span className="block size-1.5 rounded-full bg-verdigris" />
          <span className="block size-1.5 rounded-full bg-ember" />
        </span>
      </span>
    </span>
  );
}

function Swatch({ value }: { value: Theme }) {
  if (value !== "system") return <Preview look={value} />;
  return (
    <span aria-hidden className="relative block h-full w-full">
      <span className="absolute inset-0"><Preview look="dark" /></span>
      <span className="absolute inset-0 [clip-path:polygon(0_0,62%_0,38%_100%,0_100%)]"><Preview look="light" /></span>
    </span>
  );
}

/**
 * Theme: light, dark, or follow the device. The choice is applied at once and kept by the core (`device.json`), so it is
 * there before the first screen is drawn on the next start and nothing is stored in the web view.
 */
export function ThemePage() {
  const theme = useStore(app, (s) => s.device?.theme ?? "system");
  const [busy, setBusy] = useState(false);

  async function choose(next: Theme) {
    if (busy || next === theme) return;
    setBusy(true);
    try {
      await setTheme(next);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <PageNote>The colours of every screen, including the lock screens. It applies to this device only.</PageNote>
      <div role="radiogroup" aria-label="Theme" className="grid grid-cols-1 gap-3 min-[560px]:grid-cols-3">
        {CHOICES.map((c) => {
          const on = c.value === theme;
          return (
            <button
              key={c.value}
              type="button"
              role="radio"
              aria-checked={on}
              onClick={() => void choose(c.value)}
              className={`pressable flex min-w-0 flex-col gap-3 rounded-lg border p-3 text-left ${on ? "border-brass-ink bg-raised ring-1 ring-brass-ink" : "border-hairline bg-pane hover:bg-raised/60"}`}
            >
              <span className="block h-24 overflow-hidden rounded-sm border border-hairline"><Swatch value={c.value} /></span>
              <span className="flex items-center gap-2">
                <span className="font-medium">{c.label}</span>
                {on && <Check className="size-4 text-brass-ink" aria-hidden />}
              </span>
              <span className="-mt-2 text-[0.86em] text-mist">{c.note}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
