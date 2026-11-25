import { useState } from "react";
import { Copy, Globe } from "lucide-react";
import { copyText } from "../../lib/clipboard";
import { pushToast } from "../../state/app";
import { bridge, registerBrowsers, setBridge, unregisterBrowsers } from "../../state/bridge";
import { useStore } from "../../state/store";
import { withMaster } from "../reprompt/Reprompt";
import { Button } from "../ui/Button";
import { Switch } from "../ui/Inputs";
import { Group, Row } from "./parts";

/** What stays true whichever way the switch is set. Plain words; each line is a rule the code keeps. */
const RULES = [
  "Each request opens in Hexis and waits for you. Nothing is signed or saved until you choose here.",
  "The website's address comes from the browser, not from the page. A page can use only the passkeys of the website it is on.",
  "The private key stays in Hexis. The extension and its helper only carry the request and the answer.",
  "A website that requires verification gets it from your master password, typed in this window each time.",
  "While the app lock is closed, Hexis answers nothing and the browser uses its own passkey screen.",
  "Hexis makes and uses ES256 passkeys, the kind nearly every website offers.",
  "Hexis must be open. When it is not, the browser uses its own passkey screen.",
];

function stateOf(b: { found: boolean; registered: boolean; stale: boolean }): string {
  if (!b.found) return "Not found on this computer. Start it once, then come back.";
  if (b.stale) return "Connected to an older copy of Hexis. Connect again to fix it.";
  return b.registered ? "Connected." : "Not connected yet.";
}

async function copy(text: string, what: string) {
  try {
    await copyText(text); // a folder name is not a secret: no clearing timer
    pushToast({ kind: "success", text: `Copied the ${what}.` });
  } catch {
    pushToast({ kind: "error", text: "Could not copy. Select the folder name and copy it by hand." });
  }
}

/** Linux: let the browsers on this computer ask Hexis for passkeys, through the Hexis extension. */
export function BrowsersPage() {
  const status = useStore(bridge, (s) => s.status);
  const [busy, setBusy] = useState<string | null>(null);
  if (!status) return null;
  const found = status.browsers.filter((b) => b.found);

  async function run(label: string, reason: string, job: () => Promise<unknown>, done: string) {
    if (busy) return;
    setBusy(label);
    try {
      // What browsers may ask of Hexis needs the master password, not only the PIN.
      const result = await withMaster(reason, job);
      if (result !== undefined) pushToast({ kind: "success", text: done });
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "That did not work." });
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <Group title="Passkeys in browsers" heading={false}>
        <Row
          title="Use Hexis for passkeys in browsers"
          description="A browser with the Hexis extension asks this app when a website wants a passkey, and the request opens here. It stays off until you turn it on."
        >
          <Switch
            checked={status.enabled}
            onChange={(on) => void run("switch", "Use your master password to change what browsers may ask Hexis.", () => setBridge(on), on ? "Browsers can now ask Hexis for passkeys." : "Browsers no longer ask Hexis.")}
            label="Use Hexis for passkeys in browsers"
          />
        </Row>
        {status.error && (
          <p role="alert" className="border-b border-hairline px-4 py-3 text-[0.93em] text-ember last:border-b-0">{status.error}</p>
        )}
      </Group>

      <Group title="Browsers">
        {found.length === 0 ? (
          <p className="px-4 py-3.5 text-[0.93em] text-mist">No supported browser was found. Start Firefox or a Chromium-based browser (Chrome, Chromium, Brave, Edge, Vivaldi) once, then come back.</p>
        ) : (
          found.map((b) => (
            <Row key={b.id} title={b.name} description={stateOf(b)}>
              {b.registered && !b.stale ? (
                <Button
                  busy={busy === b.id}
                  disabled={busy !== null}
                  onClick={() => void run(b.id, `Use your master password to disconnect ${b.name}.`, () => unregisterBrowsers([b.id]), `${b.name} is disconnected.`)}
                >
                  Disconnect
                </Button>
              ) : (
                <Button
                  variant="key"
                  busy={busy === b.id}
                  disabled={busy !== null}
                  icon={<Globe className="size-4" aria-hidden />}
                  onClick={() => void run(b.id, `Use your master password to connect ${b.name}.`, () => registerBrowsers([b.id]), `${b.name} is connected.`)}
                >
                  {b.stale ? "Connect again" : "Connect"}
                </Button>
              )}
            </Row>
          ))
        )}
        {status.sandboxed.length > 0 && (
          <p className="border-b border-hairline px-4 py-3.5 text-[0.93em] text-mist last:border-b-0">
            {status.sandboxed.join(", ")} cannot use Hexis: a Snap or Flatpak browser runs in a sandbox that cannot start Hexis' helper. Use the same browser from your distribution's own package or as an AppImage.
          </p>
        )}
      </Group>

      <Group title="The extension">
        {status.extension ? (
          <>
            <FolderRow
              title="Chrome, Chromium, Brave, Edge, Vivaldi"
              how="Open the browser's extensions page, turn on developer mode, choose Load unpacked, and pick this folder."
              folder={status.extension.chromium}
            />
            <FolderRow
              title="Firefox"
              how="Open about:debugging, choose This Firefox, then Load Temporary Add-on, and pick manifest.json in this folder. Firefox keeps an add-on loaded this way only until it closes; a signed copy installs for good."
              folder={status.extension.firefox}
            />
          </>
        ) : (
          <p className="px-4 py-3.5 text-[0.93em] text-mist">Turn the switch on, or connect a browser. Hexis then puts the extension in a folder and shows where it is.</p>
        )}
      </Group>

      <Group title="What stays the same">
        <ul className="flex list-disc flex-col gap-2.5 py-3.5 pr-4 pl-9 text-[0.93em] text-mist marker:text-fog">
          {RULES.map((rule) => <li key={rule}>{rule}</li>)}
        </ul>
      </Group>
    </div>
  );
}

function FolderRow({ title, how, folder }: { title: string; how: string; folder: string }) {
  return (
    <div className="flex flex-col gap-2 border-b border-hairline px-4 py-3.5 last:border-b-0">
      <div className="min-w-0">
        <p className="font-medium">{title}</p>
        <p className="mt-0.5 text-[0.93em] text-mist">{how}</p>
      </div>
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 overflow-x-auto rounded-sm bg-raised px-3 py-2 font-mono text-[0.86em] whitespace-nowrap text-frost select-all">{folder}</code>
        <Button icon={<Copy className="size-4" aria-hidden />} onClick={() => void copy(folder, "folder name")}>Copy</Button>
      </div>
    </div>
  );
}
