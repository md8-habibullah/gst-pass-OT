import { useState } from "react";
import { ChevronsUpDown, Lock, LockKeyhole, Plus, UserRound } from "lucide-react";
import { app, isLabelledDemo, lockApp, lockVault, selectProfile, showPicker } from "../../state/app";
import { useStore } from "../../state/store";
import { setLockIntent } from "../../state/ui";
import { Avatar } from "../ui/Avatar";
import { Button } from "../ui/Button";
import { Sheet } from "../ui/Sheet";

/** Opens the account list. Switching locks the open vault first (the core guarantees one unlocked vault at a time). */
export function AccountSheet({ open, onClose }: { open: boolean; onClose: () => void }) {
  const profiles = useStore(app, (s) => s.profiles);
  const current = useStore(app, (s) => s.profile);
  const gateOn = useStore(app, (s) => s.gate?.enabled ?? false);
  const device = useStore(app, (s) => s.device);

  async function switchTo(id: string) {
    onClose();
    if (id === current?.id) { if (!current.demo) await lockVault(); return; }
    await showPicker();
    await selectProfile(id);
  }
  async function add() {
    onClose();
    setLockIntent("choose");
    await showPicker();
  }

  return (
    <Sheet open={open} onClose={onClose} title="Accounts" variant="dialog" size="sm" titleIcon={<UserRound className="size-5 text-brass" aria-hidden />}>
      <p className="mb-3 text-[0.93em] text-mist">Each account is its own vault with its own master password. Switching locks this one first.</p>
      <ul className="flex flex-col gap-1">
        {profiles.map((p) => {
          const on = p.id === current?.id;
          return (
            <li key={p.id}>
              <button type="button" onClick={() => void switchTo(p.id)} aria-current={on ? "true" : undefined} className={`pressable flex min-h-14 w-full items-center gap-3 rounded-lg px-3 text-left ${on ? "bg-raised shadow-e2" : "hover:bg-raised/60"}`}>
                <Avatar title={p.displayName} size={38} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-semibold">{p.displayName}</span>
                  <span className="block text-[0.86em] text-mist">{isLabelledDemo(device, p) ? (on ? "Demo vault, open now" : "Demo vault") : on ? "Open now. Choose to lock it." : "Lock this vault and open it"}</span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      <div className="mt-4 flex flex-wrap gap-2">
        <Button icon={<Plus className="size-4" />} onClick={() => void add()}>Add account</Button>
        {!current?.demo && <Button icon={<Lock className="size-4" />} onClick={() => { onClose(); void lockVault(); }}>Lock now</Button>}
        {gateOn && <Button icon={<LockKeyhole className="size-4" />} onClick={() => { onClose(); void lockApp(); }}>Lock app</Button>}
      </div>
    </Sheet>
  );
}

/** The account chip: who is signed in, and the way to switch. `compact` shows only the avatar. */
export function AccountChip({ compact = false, className = "" }: { compact?: boolean; className?: string }) {
  const profile = useStore(app, (s) => s.profile);
  const [open, setOpen] = useState(false);
  if (!profile) return null;
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label={`Account: ${profile.displayName}. Switch account`}
        title={`Account: ${profile.displayName}`}
        className={`pressable flex items-center gap-2.5 rounded-lg text-left hover:bg-raised/60 ${compact ? "p-1" : "w-full px-2 py-1.5"} ${className}`}
      >
        <Avatar title={profile.displayName} size={compact ? 34 : 34} />
        {!compact && (
          <>
            <span className="min-w-0 flex-1">
              <span className="block truncate leading-5 font-semibold">{profile.displayName}</span>
              <span className="block text-[0.8em] leading-4 text-fog">Switch account</span>
            </span>
            <ChevronsUpDown className="size-4 shrink-0 text-fog" aria-hidden />
          </>
        )}
      </button>
      <AccountSheet open={open} onClose={() => setOpen(false)} />
    </>
  );
}
