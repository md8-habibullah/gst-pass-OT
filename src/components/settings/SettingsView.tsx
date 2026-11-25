import { useEffect, type ReactNode } from "react";
import { ArrowLeft, ChevronRight, Clock, CloudUpload, Download, Files, Info, Keyboard, ListFilter, LockKeyhole, Palette, ShieldCheck, SunMoon, Upload, UsersRound, type LucideIcon } from "lucide-react";
import { plural } from "../../lib/format";
import { useLayout } from "../../lib/layout";
import { zoneLabel } from "../../lib/zones";
import { app } from "../../state/app";
import { backups, summarize } from "../../state/backup";
import { useBackHandler } from "../../state/back";
import { useBrandName } from "../../state/brand";
import { useStore } from "../../state/store";
import { setSettingsPage, ui, type SettingsPageId } from "../../state/ui";
import { AccountBar } from "../shell/AccountMenu";
import { SETTINGS_CHILDREN } from "../shell/nav";
import { IconButton } from "../ui/Button";
import { AboutPage } from "./AboutPage";
import { AccountsGroup } from "./Accounts";
import { AppLockGroup } from "./AppLock";
import { DateTimePage } from "./DateTimePage";
import { DisguiseGroup } from "./Disguise";
import { HelpPage } from "./HelpPage";
import { ListPage } from "./ListPage";
import { Group, LinkRow } from "./parts";
import { SecurityPage } from "./SecurityPage";
import { ThemePage } from "./ThemePage";

interface Category {
  id: SettingsPageId;
  title: string;
  icon: LucideIcon;
  /** One line under the title on the page itself. */
  about: string;
}

const CATEGORIES: Category[] = [
  { id: "security", title: "Security", icon: ShieldCheck, about: "Your master password, the clipboard and locking the vault." },
  { id: "list", title: "Vault list", icon: ListFilter, about: "Open straight into search, and how items are ordered." },
  { id: "applock", title: "App lock", icon: LockKeyhole, about: "A passcode before the account list, and when open vaults lock by themselves." },
  { id: "accounts", title: "Accounts", icon: UsersRound, about: "The vaults on this device." },
  { id: "time", title: "Date and time", icon: Clock, about: "Hexis saves every time in UTC and writes it in your zone. Changing the zone changes only how times look." },
  { id: "theme", title: "Theme", icon: SunMoon, about: "Light, dark, or the same as your device." },
  { id: "appearance", title: "App icon and name", icon: Palette, about: "Change how the app looks in your launcher." },
  { id: "data", title: "Backup and files", icon: CloudUpload, about: "Backups, encrypted files, import and export." },
  { id: "help", title: "Help and shortcuts", icon: Keyboard, about: "Keyboard shortcuts and common questions." },
  { id: "about", title: "About", icon: Info, about: "Version and how your data is protected." },
];

/** What each category says about its current state, shown in the list. */
function useSummaries(): Record<SettingsPageId, string> {
  const gate = useStore(app, (s) => s.gate);
  const profiles = useStore(app, (s) => s.profiles);
  const device = useStore(app, (s) => s.device);
  const clipboardClear = useStore(app, (s) => s.clipboardClear);
  const backup = summarize(useStore(backups, (s) => s.targets));
  const name = useBrandName();
  return {
    security: `Clipboard clears after ${clipboardClear} seconds`,
    list: device?.openInSearch ? "Opens in search" : "Opens at the list",
    applock: gate?.enabled ? "Passcode on" : "Passcode off",
    accounts: plural(profiles.length, "account"),
    time: device?.timeZone ? zoneLabel(device.timeZone) : "Not chosen",
    theme: device?.theme === "light" ? "Light" : device?.theme === "dark" ? "Dark" : "Same as your device",
    appearance: `Shown as ${name}`,
    data: backup.title,
    help: "Press ? for shortcuts",
    about: `Version ${typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "dev"}`,
  };
}

function BackupPage() {
  const backup = summarize(useStore(backups, (s) => s.targets));
  const remember = () => setSettingsPage("data"); // coming back from Backup, Files, Import or Export lands here
  return (
    <Group title="Backup and files" heading={false}>
      <LinkRow before={remember} title="Backup" description={`${backup.title}. ${backup.detail}`} filter="backup" icon={<CloudUpload className="size-[18px]" />} />
      <LinkRow before={remember} title="Files" description="Encrypted storage for images, PDFs and documents, up to 50 MB each." filter="files" icon={<Files className="size-[18px]" />} />
      <LinkRow before={remember} title="Import" description="Bring in passwords from Bitwarden or a CSV file." filter="import" icon={<Download className="size-[18px]" />} />
      <LinkRow before={remember} title="Export" description="Save an encrypted copy of this vault as a file." filter="export" icon={<Upload className="size-[18px]" />} />
    </Group>
  );
}

function PageContent({ id }: { id: SettingsPageId }): ReactNode {
  switch (id) {
    case "security": return <SecurityPage />;
    case "list": return <ListPage />;
    case "applock": return <AppLockGroup heading={false} />;
    case "accounts": return <AccountsGroup heading={false} />;
    case "time": return <DateTimePage />;
    case "theme": return <ThemePage />;
    case "appearance": return <DisguiseGroup heading={false} />;
    case "data": return <BackupPage />;
    case "help": return <HelpPage />;
    case "about": return <AboutPage />;
  }
}

function PageBox({ category, back, onBack }: { category: Category; back: boolean; onBack: () => void }) {
  return (
    <section aria-label={category.title} className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain">
      <div className="mx-auto flex w-full max-w-[40rem] flex-col gap-6 px-5 pt-[calc(1rem+var(--safe-top))] pb-28 min-[720px]:mx-0 min-[720px]:pt-6 min-[720px]:pb-10">
        <div className="flex items-start gap-2">
          {back && (
            <IconButton label="Back to settings" className="-ml-2 shrink-0" onClick={onBack}><ArrowLeft className="size-5" /></IconButton>
          )}
          <div className="min-w-0 flex-1">
            <h2 className="font-display text-[1.45rem] leading-[1.2] font-medium tracking-[-0.005em]">{category.title}</h2>
            {!back && <p className="mt-1 text-[0.93em] text-mist">{category.about}</p>}
          </div>
          <AccountBar className="-mt-1 -mr-1" />
        </div>
        <PageContent id={category.id} />
      </div>
    </section>
  );
}

/**
 * Settings, layer by layer. On a phone the categories are a list; a tap opens one full page and Back returns to the list.
 * On a tablet or desktop the categories sit on the left and the chosen page on the right, so there is no long scroll
 * and nothing to hunt for.
 */
export function SettingsView() {
  const layout = useLayout();
  const compact = layout === "compact";
  const page = useStore(ui, (s) => s.settingsPage);
  const summaries = useSummaries();
  const active = CATEGORIES.find((c) => c.id === (page ?? (compact ? null : "security"))) ?? null;

  // The Back button closes an open page before it leaves Settings.
  useBackHandler(compact && page !== null, () => setSettingsPage(null));
  // Leaving Settings for anywhere but one of its own sub-pages starts the next visit from the list.
  useEffect(() => () => { if (!SETTINGS_CHILDREN.has(app.get().filter)) setSettingsPage(null); }, []);

  if (compact) {
    if (active) return <PageBox category={active} back onBack={() => setSettingsPage(null)} />;
    return (
      <section aria-label="Settings" className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain">
        <div className="mx-auto flex w-full max-w-[40rem] flex-col gap-4 px-5 pt-[calc(1rem+var(--safe-top))] pb-28">
          <div className="flex items-center justify-between gap-3">
            <h2 className="font-display text-[1.45rem] leading-none font-medium">Settings</h2>
            <AccountBar className="-my-1 -mr-1" />
          </div>
          <ul className="surface overflow-hidden rounded-lg">
            {CATEGORIES.map((c) => (
              <li key={c.id} className="border-b border-hairline last:border-b-0">
                <button type="button" onClick={() => setSettingsPage(c.id)} className="pressable flex min-h-16 w-full items-center gap-4 px-4 text-left hover:bg-raised/60">
                  <span className="grid size-10 shrink-0 place-items-center rounded-sm bg-raised text-brass-ink"><c.icon className="size-[18px]" aria-hidden /></span>
                  <span className="min-w-0 flex-1">
                    <span className="block font-medium">{c.title}</span>
                    <span className="block truncate text-[0.93em] text-mist">{summaries[c.id]}</span>
                  </span>
                  <ChevronRight className="size-4 shrink-0 text-fog" aria-hidden />
                </button>
              </li>
            ))}
          </ul>
        </div>
      </section>
    );
  }

  const current = active ?? CATEGORIES[0]!;
  return (
    <div className="flex min-h-0 flex-1" role="region" aria-label="Settings">
      <nav aria-label="Settings categories" className="w-[13.5rem] shrink-0 overflow-y-auto overscroll-contain border-r border-hairline px-3 pt-6 pb-6 min-[1100px]:w-[17rem]">
        <h2 className="mb-3 px-3 font-display text-[1.45rem] leading-none font-medium">Settings</h2>
        <ul className="flex flex-col gap-0.5">
          {CATEGORIES.map((c) => {
            const on = c.id === current.id;
            return (
              <li key={c.id}>
                <button
                  type="button" onClick={() => setSettingsPage(c.id)} aria-current={on ? "page" : undefined}
                  className={`pressable flex min-h-11 w-full items-center gap-3 rounded-sm px-3 text-left ${on ? "bg-raised text-frost" : "text-mist hover:bg-raised/60 hover:text-frost"}`}
                >
                  <c.icon className={`size-[18px] shrink-0 ${on ? "text-brass-ink" : ""}`} aria-hidden />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium">{c.title}</span>
                    <span className="hidden truncate text-[0.8em] text-fog min-[1100px]:block">{summaries[c.id]}</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </nav>
      <PageBox key={current.id} category={current} back={false} onBack={() => undefined} />
    </div>
  );
}
