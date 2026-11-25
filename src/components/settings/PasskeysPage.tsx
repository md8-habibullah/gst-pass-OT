import { useState } from "react";
import { ExternalLink } from "lucide-react";
import { app, be, pushToast, setPasskeyAppCheck } from "../../state/app";
import { passkeyProvider, setPasskeyProvider } from "../../state/passkeyProvider";
import { useStore } from "../../state/store";
import { withMaster } from "../reprompt/Reprompt";
import { Button } from "../ui/Button";
import { Switch } from "../ui/Inputs";
import { Group, Row } from "./parts";

/** What stays true whichever way the switch is set. Plain words; each line is a rule the code keeps. */
const RULES = [
  "Every sign-in and every new passkey asks for a fingerprint or your screen lock, each time. A phone with neither asks for your master password instead.",
  "A locked vault opens with the master password only. A fingerprint never opens it.",
  "While the app lock is closed, Hexis shows nothing and answers nothing.",
  "Hexis makes and uses ES256 passkeys, the kind nearly every website offers. A site that accepts only another kind will not list Hexis.",
  "Android lists this app as Hexis, even when its icon and name are disguised.",
];

/** Android 14 and later: let the system offer the passkeys in this vault and save new ones here. */
export function PasskeysPage() {
  const status = useStore(passkeyProvider, (s) => s);
  const [busy, setBusy] = useState(false);
  const enabled = status?.enabled === true;
  const appCheck = useStore(app, (s) => s.device?.passkeyAppCheck !== false);

  async function change(on: boolean) {
    if (busy) return;
    setBusy(true);
    try {
      // Changing what the system may ask this app needs the master password, not only the PIN.
      const done = await withMaster("Use your master password to change the passkey switch.", () => setPasskeyProvider(on));
      if (done) pushToast({ kind: "success", text: on ? "Hexis can now offer passkeys. Choose it in Android's list too." : "Hexis no longer offers passkeys." });
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "Could not change the passkey switch." });
    } finally {
      setBusy(false);
    }
  }

  async function openSystem() {
    try {
      await be().passkeyProviderOpenSettings();
    } catch (e) {
      pushToast({ kind: "error", text: e instanceof Error ? e.message : "Could not open Android's settings." });
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <Group title="Passkeys on this phone" heading={false}>
        <Row title="Offer Hexis for passkeys" description="Websites and apps can then sign you in with a passkey saved in this vault, and save new ones here. It stays off until you turn it on.">
          <Switch checked={enabled} onChange={(on) => void change(on)} label="Offer Hexis for passkeys" />
        </Row>
        <Row
          title="Choose Hexis in Android"
          description={enabled ? "Android decides which app handles passkeys, and an app cannot choose itself. Open Android's list and pick Hexis." : "Turn the switch on first. Then Hexis can be picked in Android's list."}
        >
          <Button icon={<ExternalLink className="size-4" aria-hidden />} onClick={() => void openSystem()} disabled={!enabled}>Open Android settings</Button>
        </Row>
        <Row title="Check an app with its website" description="Before an app may use a website's passkeys, Hexis asks that website, once, whether it allows the app. Nothing from your vault is sent. Off, no request is made and apps get no passkeys; browsers work either way.">
          <Switch checked={appCheck} onChange={(on) => void setPasskeyAppCheck(on)} label="Check an app with its website" />
        </Row>
      </Group>
      <Group title="What stays the same">
        <ul className="flex list-disc flex-col gap-2.5 py-3.5 pr-4 pl-9 text-[0.93em] text-mist marker:text-fog">
          {RULES.map((rule) => <li key={rule}>{rule}</li>)}
        </ul>
      </Group>
    </div>
  );
}
