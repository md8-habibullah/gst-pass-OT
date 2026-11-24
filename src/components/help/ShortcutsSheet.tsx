import { Keyboard } from "lucide-react";
import { setSettingsPage, setShortcutsOpen, ui } from "../../state/ui";
import { setFilter } from "../../state/app";
import { useStore } from "../../state/store";
import { Button } from "../ui/Button";
import { Sheet } from "../ui/Sheet";
import { ShortcutList } from "./ShortcutList";

/** The "?" overlay: every shortcut at a glance. */
export function ShortcutsSheet() {
  const open = useStore(ui, (s) => s.shortcutsOpen);
  const close = () => setShortcutsOpen(false);
  return (
    <Sheet
      open={open} onClose={close} title="Keyboard shortcuts" variant="dialog" size="lg" level={2}
      titleIcon={<Keyboard className="size-5 text-brass-ink" aria-hidden />}
      footer={
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-[0.86em] text-fog">Letter shortcuts work when no text box is focused.</p>
          <Button onClick={() => { close(); setSettingsPage("help"); setFilter("settings"); }}>Open help</Button>
        </div>
      }
    >
      <ShortcutList columns />
    </Sheet>
  );
}
