import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { motion } from "motion/react";
import { CircleHelp, CornerDownLeft, Delete, Fingerprint } from "lucide-react";
import { app, gatePanic, gateWithBiometric, submitGate } from "../../state/app";
import { useStore } from "../../state/store";
import { MATH_MAX_LENGTH } from "../../lib/gateRules";
import { Button } from "../ui/Button";
import { GridPad } from "./GridPad";
import { ResetFlow } from "./ResetFlow";

const prettyEquation = (t: string) => t.replace(/\*/g, "×").replace(/\//g, "÷").replace(/-/g, "−");
const LONG_PRESS_RESET_MS = 700;
const LONG_PRESS_PANIC_MS = 1200;
const DOUBLE_TAP_MS = 380;

/** Typed characters stay readable while they could still be a math answer; from the fourth they are masked. */
const masked = (v: string) => (v.length > MATH_MAX_LENGTH ? "•".repeat([...v].length) : v);

type Feedback = { tone: "good" | "plain"; text: string; n: number } | null;

/**
 * Stage 1. To anyone looking over a shoulder this is a small arithmetic exercise. One to three characters are only ever
 * an answer to it. Four or more are a security attempt, judged by the core (never here), and the core decides what a
 * wrong one costs. The screen never learns the answer, the passcode or whether an attempt was a panic code.
 */
export function GateScreen() {
  const gate = useStore(app, (s) => s.gate);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState<Feedback>(null);
  const [shake, setShake] = useState(0);
  const [resetting, setResetting] = useState(false);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => {
    if (!feedback) return;
    const t = window.setTimeout(() => alive.current && setFeedback(null), 2400);
    return () => window.clearTimeout(t);
  }, [feedback]);

  const mode = gate?.mode ?? "pin";

  const submit = useCallback(async (raw: string) => {
    if (!raw || busy) return;
    setBusy(true);
    try {
      const out = await submitGate(raw);
      if (!alive.current) return;
      setValue("");
      if (out.kind === "math") setFeedback({ tone: out.correct ? "good" : "plain", text: out.correct ? "Correct!" : "Incorrect, try again", n: Date.now() });
      else if (out.kind === "failed") { setFeedback(null); setShake((n) => n + 1); }
    } catch {
      if (alive.current) { setValue(""); setFeedback({ tone: "plain", text: "Incorrect, try again", n: Date.now() }); }
    } finally {
      if (alive.current) setBusy(false);
    }
  }, [busy]);

  const type = useCallback((d: string) => setValue((v) => ([...v].length >= 64 ? v : v + d)), []);
  const back = useCallback(() => setValue((v) => [...v].slice(0, -1).join("")), []);

  // Physical keyboard for the number modes (the password mode has a real text field).
  useEffect(() => {
    if (mode === "password" || resetting) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (/^[0-9]$/.test(e.key)) type(e.key);
      else if (e.key === "Backspace") back();
      else if (e.key === "Enter") void submit(value);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mode, resetting, type, back, submit, value]);

  // Long-press on the equation: the discreet way to the reset.
  const hold = useRef<number | undefined>(undefined);
  const startHold = () => { hold.current = window.setTimeout(() => setResetting(true), LONG_PRESS_RESET_MS); };
  const endHold = () => window.clearTimeout(hold.current);

  // The optional panic mark: a deliberate long press or a double tap.
  const panicHold = useRef<number | undefined>(undefined);
  const lastTap = useRef(0);
  const panic = () => { void gatePanic().catch(() => undefined); };
  const panicDown = () => { panicHold.current = window.setTimeout(panic, LONG_PRESS_PANIC_MS); };
  const panicUp = () => {
    window.clearTimeout(panicHold.current);
    const now = Date.now();
    if (now - lastTap.current < DOUBLE_TAP_MS) { lastTap.current = 0; panic(); } else lastTap.current = now;
  };

  async function viaBiometric() {
    setBusy(true);
    try { await gateWithBiometric(); } catch { /* the OS prompt reports its own failures */ } finally { if (alive.current) setBusy(false); }
  }

  const left = gate?.attemptsLeft ?? null;
  const equation = gate?.equation ? prettyEquation(gate.equation) : "";

  return (
    <main className="h-dvh overflow-y-auto overscroll-contain">
      <div className="grid min-h-full place-items-center px-4 pt-[calc(1.5rem+var(--safe-top))] pb-[calc(1.5rem+var(--safe-bottom))]">
        <motion.div className="w-full max-w-[24rem]" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: { duration: 0.3, ease: [0.2, 0.8, 0.2, 1] } }}>
          <div key={shake} className={`glass-strong rounded-xl px-6 pt-9 pb-6 ${shake > 0 ? "animate-shake" : ""}`}>
            <div
              onPointerDown={startHold} onPointerUp={endHold} onPointerLeave={endHold} onPointerCancel={endHold}
              className="mb-6 text-center select-none"
            >
              <p className="font-display text-[2.1rem] leading-tight font-semibold tracking-[-0.01em] tabular-nums" aria-label={`What is ${equation}?`}>
                {equation} <span className="text-fog">= ?</span>
              </p>
            </div>

            {mode === "password" ? (
              <form onSubmit={(e: FormEvent) => { e.preventDefault(); void submit(value); }} className="flex flex-col gap-3" noValidate>
                <input
                  value={value} onChange={(e) => setValue(e.target.value)} disabled={busy}
                  type="text" inputMode="text" autoComplete="off" autoCorrect="off" autoCapitalize="off" spellCheck={false}
                  aria-label="Your answer" data-autofocus enterKeyHint="go"
                  className={`min-h-12 w-full rounded-sm border border-hairline-strong bg-pane px-4 text-center text-[1.2em] tracking-wide outline-none focus-visible:border-brass ${[...value].length > MATH_MAX_LENGTH ? "mask-chars" : ""}`}
                />
                <Button type="submit" variant="key" block disabled={busy || !value} className="min-h-11" icon={<CornerDownLeft className="size-4" />}>Enter</Button>
              </form>
            ) : (
              <div className="flex flex-col items-center gap-5">
                <div className="flex min-h-12 w-full items-center justify-center rounded-sm border border-hairline-strong bg-pane px-4 text-[1.35em] tracking-[0.18em] tabular-nums" role="status" aria-label="Your answer">
                  {value ? masked(value) : <span className="text-fog/60">&nbsp;</span>}
                </div>
                {mode === "grid" ? (
                  <GridPad disabled={busy} onDigit={type} onBackspace={back} onPattern={(d) => void submit(d)} onSubmit={() => void submit(value)} />
                ) : (
                  <Keypad disabled={busy} onDigit={type} onBackspace={back} onSubmit={() => void submit(value)} />
                )}
              </div>
            )}

            <div className="mt-4 min-h-6 text-center text-[0.93em]" aria-live="polite">
              {feedback && <p key={feedback.n} className={feedback.tone === "good" ? "text-verdigris" : "text-mist"}>{feedback.text}</p>}
            </div>
            {left !== null && (
              <p role="alert" className="mt-1 rounded-lg border border-ember/40 bg-danger/10 px-4 py-3 text-center text-[0.93em]">
                {left === 1 ? "That was wrong. One attempt left before everything on this device is erased." : `That was wrong. ${left} attempts left before everything on this device is erased.`}
              </p>
            )}

            <div className="mt-5 flex items-center justify-between gap-3">
              <button type="button" onClick={() => setResetting(true)} aria-label="Reset app data" className="pressable grid size-10 place-items-center rounded-full text-fog/50 hover:text-mist pointer-coarse:size-12">
                <CircleHelp className="size-[18px]" aria-hidden />
              </button>
              {gate?.biometric && (
                <Button variant="ghost" icon={<Fingerprint className="size-5" />} disabled={busy} onClick={() => void viaBiometric()}>Use fingerprint</Button>
              )}
              {gate?.panicButton ? (
                <button type="button" onPointerDown={panicDown} onPointerUp={panicUp} onPointerLeave={() => window.clearTimeout(panicHold.current)} aria-label="Hold or double-tap to clear" className="pressable grid size-10 place-items-center rounded-full pointer-coarse:size-12">
                  <span className="size-2.5 rounded-full bg-fog/35" aria-hidden />
                </button>
              ) : <span className="size-10" aria-hidden />}
            </div>
          </div>
        </motion.div>
      </div>
      <ResetFlow open={resetting} onClose={() => setResetting(false)} />
    </main>
  );
}

function Keypad({ disabled, onDigit, onBackspace, onSubmit }: { disabled?: boolean; onDigit: (d: string) => void; onBackspace: () => void; onSubmit: () => void }) {
  const key = "pressable grid size-[4.25rem] place-items-center rounded-full border border-hairline-strong bg-raised text-[1.45em] font-medium shadow-e2 hover:bg-hover disabled:opacity-40 pointer-coarse:size-[4.5rem]";
  const util = "pressable grid size-[4.25rem] place-items-center rounded-full text-fog hover:bg-raised disabled:opacity-40 pointer-coarse:size-[4.5rem]";
  return (
    <div className="grid grid-cols-3 gap-x-7 gap-y-4" role="group" aria-label="Number keypad">
      {["1", "2", "3", "4", "5", "6", "7", "8", "9"].map((d) => (
        <button key={d} type="button" className={key} disabled={disabled} onClick={() => onDigit(d)} aria-label={`Number ${d}`}>{d}</button>
      ))}
      <button type="button" className={util} disabled={disabled} onClick={onBackspace} aria-label="Delete the last digit"><Delete className="size-6" aria-hidden /></button>
      <button type="button" className={key} disabled={disabled} onClick={() => onDigit("0")} aria-label="Number 0">0</button>
      <button type="button" className={util} disabled={disabled} onClick={onSubmit} aria-label="Submit"><CornerDownLeft className="size-6" aria-hidden /></button>
    </div>
  );
}
