import { ShortcutList } from "../help/ShortcutList";
import { useBrandName } from "../../state/brand";
import { Group, PageNote } from "./parts";

const ANSWERS: { q: string; a: string }[] = [
  { q: "I forgot my master password", a: "There is no recovery: the password is never stored anywhere, so nobody can reset it. If you made a backup, restore it on the sign-in screen with Add account, then Restore from a backup. That brings the vault back with the password it had." },
  { q: "How do I add my own vault?", a: "Open Settings, then Accounts, then Add account. You can create a new vault, open a vault file exported from another device, or restore from a backup." },
  { q: "Where is my data kept?", a: "In an encrypted file on this device. It leaves only when you add a backup target, and then only as encrypted data to storage you own." },
  { q: "Why did a wrong passcode make me wait?", a: "After three wrong passcodes in a row the app lock makes you wait, and the wait grows with every three more. Sixteen wrong passcodes erase everything on the device. The wait is measured against the real time, so changing the date does not help." },
  { q: "What do the codes do?", a: "A code is a one-time password that changes every 30 seconds. Add the secret key or the otpauth link a website gives you, and Hexis shows the current code. Copy it with C while the item is selected." },
  { q: "How do I move to another phone or computer?", a: "Export an encrypted vault file from Export and open it on the other device. Or add a backup target here and restore from it there." },
];

/** Shortcuts, then the questions people ask most. */
export function HelpPage() {
  const name = useBrandName();
  return (
    <>
      <PageNote>Press ? anywhere in {name} to see the shortcuts. The list below is the same one.</PageNote>
      <Group title="Keyboard shortcuts" heading={false}>
        <div className="p-4"><ShortcutList /></div>
      </Group>
      <section aria-label="Common questions" className="flex flex-col gap-2">
        <h3 className="px-1 font-semibold">Common questions</h3>
        <div className="surface divide-y divide-hairline overflow-hidden rounded-lg">
          {ANSWERS.map((x) => (
            <details key={x.q} className="group px-4 py-3 open:bg-raised/40">
              <summary className="cursor-pointer list-none font-medium marker:hidden [&::-webkit-details-marker]:hidden">{x.q}</summary>
              <p className="mt-2 text-[0.93em] leading-6 text-mist">{x.a}</p>
            </details>
          ))}
        </div>
      </section>
    </>
  );
}
