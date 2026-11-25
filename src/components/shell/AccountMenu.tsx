import { useState } from "react";
import { Check, ChevronsUpDown, KeyRound, Lock, LockKeyhole, LogOut, Plus, UserRound } from "lucide-react";
import { useLayout } from "../../lib/layout";
import { app, isLabelledDemo, lockApp, lockVault, logOut, showPicker, switchAccount } from "../../state/app";
import { useStore } from "../../state/store";
import { setLockIntent } from "../../state/ui";
import type { ProfileInfo } from "../../backend";
import { Avatar } from "../ui/Avatar";
import { Button, IconButton } from "../ui/Button";
import { Sheet } from "../ui/Sheet";
import { unlockFully } from "../reprompt/Reprompt";

/** The short line under an account's name: what happens when it is chosen. */
function accountNote(p: ProfileInfo, on: boolean, limited: boolean, labelledDemo: boolean): string {
  if (labelledDemo) return on ? "Demo vault, open now" : "Demo vault";
  if (on) return limited ? "Open now, on its PIN" : "Open now";
  if (p.resumable) return "Its PIN opens it";
  return "Master password";
}

/**
 * The account list. Choosing another account moves there: the one that is open stays behind its PIN when it has one (its PIN
 * opens it again until the lock timer ends) and is locked when it has none. "Log out" locks every account; nothing is deleted.
 */
export function AccountSheet({ open, onClose }: { open: boolean; onClose: () => void }) {
  const profiles = useStore(app, (s) => s.profiles);
  const current = useStore(app, (s) => s.profile);
  const gateOn = useStore(app, (s) => s.gate?.enabled ?? false);
  const device = useStore(app, (s) => s.device);
  const limited = useStore(app, (s) => s.limited);

  async function switchTo(id: string) {
    onClose();
    if (id === current?.id) { if (!current.demo) await lockVault(); return; }
    await switchAccount(id);
  }
  async function add() {
    onClose();
    setLockIntent("choose");
    await showPicker();
  }

  return (
    <Sheet open={open} onClose={onClose} title="Accounts" variant="dialog" size="sm" titleIcon={<UserRound className="size-5 text-brass-ink" aria-hidden />}>
      <p className="mb-3 text-[0.93em] text-mist">Each account is its own vault with its own master password. Switching keeps this one behind its PIN, or locks it when it has none.</p>
      <ul className="flex flex-col gap-1">
        {profiles.map((p) => {
          const on = p.id === current?.id;
          return (
            <li key={p.id}>
              <button type="button" onClick={() => void switchTo(p.id)} aria-current={on ? "true" : undefined} className={`pressable flex min-h-14 w-full items-center gap-3 rounded-lg px-3 text-left ${on ? "bg-raised shadow-e2" : "hover:bg-raised/60"}`}>
                <Avatar title={p.displayName} size={38} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-semibold">{p.displayName}</span>
                  <span className="block text-[0.86em] text-mist">{accountNote(p, on, limited, isLabelledDemo(device, p))}</span>
                </span>
                {on && <Check className="size-[18px] shrink-0 text-brass-ink" aria-label="Open now" />}
              </button>
            </li>
          );
        })}
      </ul>
      {limited && (
        <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-lg border border-hairline bg-pane px-3.5 py-3">
          <p className="min-w-0 flex-1 basis-48 text-[0.93em] text-mist">Opened with the PIN. Secrets and settings ask for your master password.</p>
          <Button icon={<KeyRound className="size-4" />} onClick={() => void unlockFully()}>Use master password</Button>
        </div>
      )}
      <div className="mt-4 flex flex-wrap gap-2">
        <Button icon={<Plus className="size-4" />} onClick={() => void add()}>Add account</Button>
        {!current?.demo && <Button icon={<Lock className="size-4" />} onClick={() => { onClose(); void lockVault(); }}>Lock now</Button>}
        {gateOn && <Button icon={<LockKeyhole className="size-4" />} onClick={() => { onClose(); void lockApp(); }}>Lock app</Button>}
        <Button icon={<LogOut className="size-4" />} title="Locks every account. Nothing is deleted." onClick={() => { onClose(); void logOut(); }}>Log out</Button>
      </div>
    </Sheet>
  );
}

/** The account chip: who is signed in, and the way to switch. `compact` shows only the avatar. */
export function AccountChip({ compact = false, className = "" }: { compact?: boolean; className?: string }) {
  const profile = useStore(app, (s) => s.profile);
  const limited = useStore(app, (s) => s.limited);
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
              <span className="block text-[0.8em] leading-4 text-fog">{limited ? "On its PIN" : "Switch account"}</span>
            </span>
            <ChevronsUpDown className="size-4 shrink-0 text-fog" aria-hidden />
          </>
        )}
      </button>
      <AccountSheet open={open} onClose={() => setOpen(false)} />
    </>
  );
}

/**
 * The phone's account bar, at the right end of a page header: the lock button (the vault closes at once) and the account
 * button (switch, add, log out). A computer has these in the side bar, so this draws nothing there.
 */
export function AccountBar({ className = "" }: { className?: string }) {
  const compact = useLayout() === "compact";
  const profile = useStore(app, (s) => s.profile);
  const limited = useStore(app, (s) => s.limited);
  const [open, setOpen] = useState(false);
  if (!compact || !profile) return null;
  return (
    <div className={`flex shrink-0 items-center gap-1 ${className}`}>
      {!profile.demo && (
        <IconButton label="Lock now" onClick={() => void lockVault()}>
          <Lock className="size-[18px]" />
        </IconButton>
      )}
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label={`Account: ${profile.displayName}. Switch account, add one or log out`}
        title={`Account: ${profile.displayName}`}
        className="pressable relative grid size-10 place-items-center rounded-lg hover:bg-raised/60 pointer-coarse:size-12"
      >
        <Avatar title={profile.displayName} size={32} />
        {limited && <span aria-hidden className="absolute right-1 bottom-1 grid size-4 place-items-center rounded-full bg-marine ring-2 ring-pane"><KeyRound className="size-2.5 text-brass-ink" /></span>}
      </button>
      <AccountSheet open={open} onClose={() => setOpen(false)} />
    </div>
  );
}
