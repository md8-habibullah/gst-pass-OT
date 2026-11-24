import { type ReactNode } from "react";
import { useBrandName } from "../../state/brand";
import { VaultMark } from "./VaultMark";

const FACTS: { title: string; text: string }[] = [
  { title: "Post-quantum encryption", text: "ML-KEM-1024 and X25519 wrap your key together, so a future quantum computer alone cannot open it." },
  { title: "No account, no cloud", text: "Your vault is a file on this device. It leaves only when you add a backup you own." },
  { title: "Backups to your own storage", text: "Send encrypted copies to as many S3 buckets as you like. We never see them." },
];

/**
 * The frame every front-door screen sits in (time zone, accounts, unlock). On a wide screen the product speaks on the
 * left and the form sits on the right; on a phone only the form shows.
 */
export function Door({ children, wide = false, aside = true }: { children: ReactNode; wide?: boolean; aside?: boolean }) {
  const name = useBrandName();
  return (
    <main className="h-dvh overflow-y-auto overscroll-contain">
      <div className={`mx-auto grid min-h-full w-full grid-cols-[minmax(0,1fr)] items-center gap-x-16 px-4 pt-[calc(1.5rem+var(--safe-top))] pb-[calc(1.5rem+var(--safe-bottom))] ${aside ? "min-[62rem]:max-w-[68rem] min-[62rem]:grid-cols-[minmax(0,1fr)_28rem] min-[62rem]:px-10" : "max-w-[30rem]"}`}>
        {aside && (
          <aside className="hidden min-[62rem]:block" aria-hidden="true">
            <VaultMark size={52} />
            <p className="mt-7 font-display text-[2.6rem] leading-[1.08] font-medium tracking-[-0.015em]">{name}</p>
            <p className="mt-3 max-w-[26rem] text-[1.13em] leading-7 text-mist">Passwords, codes and cards, encrypted and kept on your device.</p>
            <dl className="mt-10 max-w-[26rem] divide-y divide-hairline border-y border-hairline">
              {FACTS.map((f) => (
                <div key={f.title} className="py-4">
                  <dt className="font-semibold">{f.title}</dt>
                  <dd className="mt-1 text-[0.93em] leading-6 text-mist">{f.text}</dd>
                </div>
              ))}
            </dl>
          </aside>
        )}
        <div className={`w-full justify-self-center ${wide ? "max-w-[34rem]" : "max-w-[26rem]"} ${aside ? "min-[62rem]:justify-self-end" : ""}`}>{children}</div>
      </div>
    </main>
  );
}

/** The card the forms sit on: one solid surface, a hairline edge and nothing else. */
export function DoorCard({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`surface rounded-xl px-6 pt-7 pb-6 pointer-coarse:px-7 ${className}`}>{children}</div>;
}
