import { useEffect } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Check, CircleAlert, Copy } from "lucide-react";
import { app, dismissToast, type Toast } from "../../state/app";
import { useStore } from "../../state/store";
import { useLayout } from "../../lib/layout";
import { CountdownRing } from "./Ring";

function ToastItem({ toast }: { toast: Toast }) {
  useEffect(() => {
    const t = setTimeout(() => dismissToast(toast.id), toast.ttlMs);
    return () => clearTimeout(t);
  }, [toast.id, toast.ttlMs]);

  const Icon = toast.kind === "error" ? CircleAlert : toast.countdownMs ? Copy : Check;
  const tone = toast.kind === "error" ? "text-ember" : "text-verdigris";
  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 14, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 8, scale: 0.98 }}
      transition={{ type: "spring", stiffness: 480, damping: 36 }}
      className="surface-float pointer-events-auto flex max-w-[min(30rem,calc(100vw-2rem))] items-center gap-3 rounded-lg py-2.5 pr-2.5 pl-3.5"
    >
      {toast.countdownMs ? (
        <CountdownRing totalMs={toast.countdownMs} remainingMs={toast.countdownMs} windowKey={toast.id} color="var(--color-brass)" size={26} stroke={2.5}>
          <Icon className="size-3 text-brass" aria-hidden />
        </CountdownRing>
      ) : (
        <Icon className={`size-[18px] shrink-0 ${tone}`} aria-hidden />
      )}
      <p className="min-w-0 flex-1 text-[0.95em] leading-5">{toast.text}</p>
      {toast.action && (
        <button
          type="button"
          onClick={() => { toast.action?.run(); dismissToast(toast.id); }}
          className="pressable shrink-0 rounded-sm px-2.5 py-1.5 text-[0.93em] font-semibold text-brass hover:bg-hover pointer-coarse:min-h-11"
        >
          {toast.action.label}
        </button>
      )}
    </motion.div>
  );
}

export function Toasts() {
  const toasts = useStore(app, (s) => s.toasts);
  const compact = useLayout() === "compact";
  return (
    <div
      role="status"
      aria-live="polite"
      className={`pointer-events-none fixed inset-x-0 z-[80] flex flex-col items-center gap-2 px-4 ${compact ? "bottom-[calc(5.75rem+var(--safe-bottom))]" : "bottom-6"}`}
    >
      <AnimatePresence initial={false}>
        {toasts.map((t) => (
          <ToastItem key={t.id} toast={t} />
        ))}
      </AnimatePresence>
    </div>
  );
}
