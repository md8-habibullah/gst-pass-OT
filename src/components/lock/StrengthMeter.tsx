import { estimateStrength } from "../../lib/password";

const TONE = ["bg-ember", "bg-ember", "bg-brass-ink", "bg-verdigris", "bg-verdigris"] as const;

/** Four segments plus a sentence; colour is reinforced by text so it is never the only signal. */
export function StrengthMeter({ password }: { password: string }) {
  const s = estimateStrength(password);
  const lit = password ? Math.max(1, s.score) : 0;
  return (
    <div aria-live="polite" className="flex flex-col gap-1.5">
      <div className="flex gap-1.5" aria-hidden>
        {[1, 2, 3, 4].map((n) => (
          <span key={n} className={`h-1 flex-1 rounded-full transition-colors duration-100 ${n <= lit ? TONE[s.score] : "bg-raised"}`} />
        ))}
      </div>
      <p className="text-[0.86em] text-fog">
        {password ? `${s.label}, roughly ${s.bits} bits of entropy.` : "A passphrase of four or five random words is easy to remember and hard to guess."}
      </p>
    </div>
  );
}
