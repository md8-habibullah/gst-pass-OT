import { useEffect } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Check, CircleAlert, X } from "lucide-react";
import { app, dismissToast, type Toast } from "../../state/app";
import { useStore } from "../../state/store";
import { useLayout } from "../../lib/layout";

/** How far (px) or how fast (px/s) a sideways drag must go for the notice to leave. */
const SWIPE_DISTANCE = 72;
const SWIPE_SPEED = 480;

/**
 * One notice. It leaves by itself after `ttlMs` (5 seconds unless it is a warning), and a swipe to the left or the right
 * sends it away at once, so it never sits on top of a button for long and never has to be waited out.
 */
function ToastItem({ toast }: { toast: Toast }) {
  useEffect(() => {
    const t = setTimeout(() => dismissToast(toast.id), toast.ttlMs);
    return () => clearTimeout(t);
  }, [toast.id, toast.ttlMs]);

  const Icon = toast.kind === "error" ? CircleAlert : Check;
  const tone = toast.kind === "error" ? "text-ember" : "text-verdigris";
  return (
    <motion.div
      layout
      drag="x"
      dragSnapToOrigin
      dragElastic={0.7}
      dragConstraints={{ left: 0, right: 0 }}
      onDragEnd={(_, info) => {
        if (Math.abs(info.offset.x) > SWIPE_DISTANCE || Math.abs(info.velocity.x) > SWIPE_SPEED) dismissToast(toast.id);
      }}
      initial={{ opacity: 0, y: 14, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, scale: 0.98, transition: { duration: 0.12 } }}
      transition={{ type: "spring", stiffness: 480, damping: 36 }}
      className="surface-float pointer-events-auto flex max-w-[min(30rem,calc(100vw-2rem))] cursor-grab items-center gap-3 rounded-lg py-2.5 pr-1.5 pl-3.5 active:cursor-grabbing"
    >
      <Icon className={`size-[18px] shrink-0 ${tone}`} aria-hidden />
      <p className="min-w-0 flex-1 text-[0.95em] leading-5">{toast.text}</p>
      {toast.action && (
        <button
          type="button"
          onClick={() => { toast.action?.run(); dismissToast(toast.id); }}
          className="pressable shrink-0 rounded-sm px-2.5 py-1.5 text-[0.93em] font-semibold text-brass-ink hover:bg-hover pointer-coarse:min-h-11"
        >
          {toast.action.label}
        </button>
      )}
      <button
        type="button"
        aria-label="Dismiss notice"
        onClick={() => dismissToast(toast.id)}
        className="pressable grid size-8 shrink-0 place-items-center rounded-full text-fog hover:bg-hover hover:text-frost pointer-coarse:size-10"
      >
        <X className="size-4" aria-hidden />
      </button>
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
