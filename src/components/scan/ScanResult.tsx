import { useMemo, useState, type ReactNode } from "react";
import { ChevronDown, CircleAlert, Fingerprint, KeyRound, Link2, Plus, Search, Timer } from "lucide-react";
import type { ItemSummary, LoginSuggestion, ScanChoice, ScanReply } from "../../backend";
import { be, app } from "../../state/app";
import { useStore } from "../../state/store";
import { openNewItem } from "../../state/ui";
import { canScanWithCamera } from "../../lib/scan";
import { plural } from "../../lib/format";
import { Avatar } from "../ui/Avatar";
import { Button } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { withItemPassword } from "../reprompt/Reprompt";

type Reply<K extends ScanReply["kind"]> = Extract<ScanReply, { kind: K }>;

export interface ResultProps {
  /** The text that was scanned or pasted. It holds the setup key, so it lives only in this screen's state. */
  text: string;
  reply: ScanReply;
  /** A code was added to a login, or a login was made for it. */
  onSaved: (item: ItemSummary, created: boolean) => void;
  /** Leave the scanner (a screen that opens the editor hands over to it). */
  onClose: () => void;
}

const REASON: Record<LoginSuggestion["reasons"][number], string> = {
  account: "Same account",
  site: "Same website",
  name: "Same name",
};

const messageOf = (e: unknown) => (e instanceof Error ? e.message : "That did not work. Try again.");

/** Shown for anything that is not a one-time code. */
function Note({ icon, title, children, actions }: { icon: ReactNode; title: string; children: ReactNode; actions?: ReactNode }) {
  return (
    <div className="flex flex-col gap-4 pt-1">
      <div className="flex items-start gap-3">
        <span className="mt-0.5 grid size-9 shrink-0 place-items-center rounded-lg bg-pane text-brass-ink shadow-e1" aria-hidden>{icon}</span>
        <div className="min-w-0">
          <p className="font-semibold">{title}</p>
          <div className="mt-1 flex flex-col gap-2 text-[0.93em] text-mist">{children}</div>
        </div>
      </div>
      {actions && <div className="flex flex-wrap gap-2">{actions}</div>}
    </div>
  );
}

/** What a scanned text turned out to be, and what can be done with it. */
export function ScanResult(props: ResultProps) {
  const { reply } = props;
  switch (reply.kind) {
    case "totp": return <CodeResult {...props} reply={reply} />;
    case "fido": return <FidoResult />;
    case "migration": return <MigrationResult {...props} />;
    case "link": return <LinkResult {...props} reply={reply} />;
    case "invalid": return <Note icon={<CircleAlert className="size-5" />} title="This code cannot be used">{reply.message}</Note>;
    case "text":
      return (
        <Note icon={<CircleAlert className="size-5" />} title="Nothing to save from this">
          <p>It is not a one-time code, a passkey sign-in code or a website link. If it is a setup key, paste it into the box on the first screen.</p>
        </Note>
      );
  }
}

function FidoResult() {
  const phone = canScanWithCamera();
  return (
    <Note icon={<Fingerprint className="size-5" />} title="A computer wants you to sign in with a passkey">
      {phone ? (
        <>
          <p>This code opens your phone's own passkey screen, not this one. Close this scanner, point the phone's camera app at the code, and when Android asks where your passkey is, choose this app.</p>
          <p>This app can answer only if it is switched on as a passkey provider in the phone's settings.</p>
        </>
      ) : (
        <p>This code is meant for a phone: the phone scans it with its camera app and answers over Bluetooth. A computer cannot answer it.</p>
      )}
    </Note>
  );
}

function MigrationResult({ text, onClose }: ResultProps) {
  return (
    <Note
      icon={<Timer className="size-5" />}
      title="Several accounts from an authenticator app"
      actions={<Button variant="key" icon={<Plus className="size-4" />} onClick={() => { onClose(); openNewItem("totp", { totp: text }); }}>Add them as separate codes</Button>}
    >
      <p>This is an export of many one-time codes. The next screen shows how many it holds and saves one code-only item for each account.</p>
    </Note>
  );
}

function LinkResult({ reply, onClose }: ResultProps & { reply: Reply<"link"> }) {
  return (
    <Note
      icon={<Link2 className="size-5" />}
      title={reply.host ? `A link to ${reply.host}` : "A web link"}
      actions={
        reply.host ? (
          <Button variant="key" icon={<Plus className="size-4" />} onClick={() => { onClose(); openNewItem("login", { url: `https://${reply.host}` }); }}>
            Make a login for {reply.host}
          </Button>
        ) : undefined
      }
    >
      <p>A link holds no one-time code or passkey, so nothing was saved. Only the website's name is used when you make a login from it.</p>
    </Note>
  );
}

/** A suggestion row: who it is, why it fits, and the button that puts the code there. */
function LoginRow({ title, subtitle, reasons, note, busy, disabled, locked, strong, onPick }: {
  title: string;
  subtitle: string | null;
  reasons?: string[];
  note?: string;
  busy: boolean;
  disabled: boolean;
  locked: boolean;
  strong?: boolean;
  onPick: () => void;
}) {
  return (
    <li className="flex items-center gap-3 rounded-lg bg-pane/70 p-3 shadow-e1">
      <Avatar title={title} size={38} />
      <div className="min-w-0 flex-1">
        <p className="flex items-center gap-1.5 truncate font-semibold">
          <span className="truncate">{title}</span>
          {locked && <KeyRound className="size-3.5 shrink-0 text-fog" aria-label="Asks for the master password" />}
        </p>
        {subtitle && <p className="truncate text-[0.93em] text-mist">{subtitle}</p>}
        {(reasons?.length || note) && (
          <p className="mt-0.5 text-[0.86em] text-fog">{note ?? reasons!.join(", ")}</p>
        )}
      </div>
      <Button variant={strong ? "key" : "quiet"} busy={busy} disabled={disabled} aria-label={`Add the code to ${title}`} onClick={onPick}>
        Add here
      </Button>
    </li>
  );
}

function CodeResult({ text, reply, onSaved }: ResultProps & { reply: Reply<"totp"> }) {
  const items = useStore(app, (s) => s.items);
  const who = reply.account || reply.issuer || "this account";
  const [title, setTitle] = useState(reply.issuer || reply.account || "");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [more, setMore] = useState(false);
  const [query, setQuery] = useState("");

  const suggested = useMemo(() => new Set(reply.suggestions.map((s) => s.id)), [reply.suggestions]);
  const others = useMemo(() => {
    const q = query.trim().toLowerCase();
    return items
      .filter((i) => i.type === "login" && !i.archived && !suggested.has(i.id))
      .filter((i) => !q || i.title.toLowerCase().includes(q) || (i.subtitle ?? "").toLowerCase().includes(q))
      .sort((a, b) => a.title.localeCompare(b.title));
  }, [items, suggested, query]);

  async function apply(choice: ScanChoice, key: string, name: string) {
    setBusy(key);
    setError(null);
    try {
      const item = await withItemPassword({
        reason: "existing" in choice ? `Add a one-time code to “${name}”.` : `Make a login for “${name}”.`,
        confirmLabel: "existing" in choice ? "Add code" : "Make login",
        run: (pw) => be().scanApply(text, choice, pw),
      });
      if (item) onSaved(item, "new" in choice);
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(null);
    }
  }

  const top = reply.suggestions[0];
  const sureOfTop = top !== undefined && top.reasons.includes("account") && !top.hasThisCode && !top.full;
  const make = (
    <section aria-label="Make a new login" className="flex flex-col gap-3 rounded-lg border border-hairline p-3.5">
      <Field label={reply.suggestions.length ? "Or make a new login" : "Make a new login"}>
        {({ id, describedBy }) => (
          <TextInput id={id} aria-describedby={describedBy} value={title} maxLength={200} placeholder="Name of the login" onChange={(e) => setTitle(e.target.value)} />
        )}
      </Field>
      <Button
        className="self-start"
        variant={reply.suggestions.length ? "quiet" : "key"}
        icon={<Plus className="size-4" />}
        busy={busy === "new"}
        disabled={!title.trim() || busy !== null}
        onClick={() => void apply({ new: { title: title.trim() } }, "new", title.trim())}
      >
        Make login
      </Button>
    </section>
  );

  return (
    <div className="flex flex-col gap-4 pt-1">
      <div className="flex items-start gap-3">
        <span className="mt-0.5 grid size-9 shrink-0 place-items-center rounded-lg bg-pane text-brass-ink shadow-e1" aria-hidden><Timer className="size-5" /></span>
        <div className="min-w-0">
          <p className="font-semibold">One-time code{reply.issuer ? ` for ${reply.issuer}` : ""}</p>
          <p className="truncate text-[0.93em] text-mist">{reply.account || "No account name in the code"}</p>
          <p className="text-[0.86em] text-fog">{reply.digits} digits, new code every {reply.period} seconds</p>
        </div>
      </div>

      {error && <p role="alert" className="flex items-start gap-2 text-[0.93em] text-ember"><CircleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />{error}</p>}

      {reply.suggestions.length > 0 ? (
        <section aria-label="Logins it may belong to" className="flex flex-col gap-2">
          <p className="text-[0.93em] font-medium text-mist">{plural(reply.suggestions.length, "login")} that fit{reply.suggestions.length === 1 ? "s" : ""}</p>
          <ul className="flex flex-col gap-2">
            {reply.suggestions.map((s, i) => (
              <LoginRow
                key={s.id}
                title={s.title}
                subtitle={s.subtitle}
                reasons={s.reasons.map((r) => REASON[r])}
                note={s.hasThisCode ? "Already has this code" : s.full ? "No room for another code" : undefined}
                locked={s.reprompt}
                strong={i === 0 && sureOfTop}
                busy={busy === s.id}
                disabled={busy !== null || s.hasThisCode || s.full}
                onPick={() => void apply({ existing: s.id }, s.id, s.title)}
              />
            ))}
          </ul>
        </section>
      ) : (
        <p className="flex items-start gap-2 text-[0.93em] text-mist"><Search className="mt-0.5 size-4 shrink-0 text-fog" aria-hidden />No login in this vault matches {who}.</p>
      )}

      {make}

      {others.length > 0 || query ? (
        <section aria-label="Other logins" className="flex flex-col gap-2">
          <button type="button" aria-expanded={more} onClick={() => setMore((v) => !v)} className="pressable flex items-center gap-1.5 self-start rounded-sm py-1 text-[0.93em] font-medium text-mist hover:text-frost pointer-coarse:min-h-11">
            <ChevronDown className={`size-4 transition-transform ${more ? "rotate-180" : ""}`} aria-hidden />
            Add to another login
          </button>
          {more && (
            <>
              <div className="relative">
                <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-fog" aria-hidden />
                <TextInput type="search" aria-label="Search logins" placeholder="Search logins" value={query} onChange={(e) => setQuery(e.target.value)} className="pl-9" />
              </div>
              {others.length === 0 ? (
                <p className="text-[0.93em] text-fog">No login matches that.</p>
              ) : (
                <ul className="flex flex-col gap-2">
                  {others.slice(0, 30).map((i) => (
                    <LoginRow
                      key={i.id}
                      title={i.title}
                      subtitle={i.subtitle}
                      locked={i.reprompt}
                      busy={busy === i.id}
                      disabled={busy !== null}
                      onPick={() => void apply({ existing: i.id }, i.id, i.title)}
                    />
                  ))}
                </ul>
              )}
              {others.length > 30 && <p className="text-[0.86em] text-fog">Showing 30 of {others.length}. Type to narrow the list.</p>}
            </>
          )}
        </section>
      ) : null}
    </div>
  );
}
