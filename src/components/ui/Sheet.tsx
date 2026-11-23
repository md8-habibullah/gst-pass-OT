import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion, useDragControls } from "motion/react";
import { X } from "lucide-react";
import { useLayout } from "../../lib/layout";
import { IconButton } from "./Button";

const FOCUSABLE = 'a[href],button:not([disabled]),input:not([disabled]),textarea:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex="-1"])';

/** Traps Tab inside `ref` while active, focuses the first control, restores focus on close. */
function useFocusTrap(ref: React.RefObject<HTMLElement | null>, active: boolean) {
  useEffect(() => {
    if (!active) return;
    const el = ref.current;
    if (!el) return;
    const previous = document.activeElement as HTMLElement | null;
    const first = el.querySelector<HTMLElement>("[data-autofocus]") ?? el.querySelector<HTMLElement>(FOCUSABLE) ?? el;
    first.focus({ preventScroll: true });
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      const nodes = [...el.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((n) => n.offsetParent !== null);
      if (nodes.length === 0) return e.preventDefault();
      const firstN = nodes[0]!, lastN = nodes[nodes.length - 1]!;
      if (e.shiftKey && document.activeElement === firstN) { e.preventDefault(); lastN.focus(); }
      else if (!e.shiftKey && document.activeElement === lastN) { e.preventDefault(); firstN.focus(); }
    };
    el.addEventListener("keydown", onKey);
    return () => {
      el.removeEventListener("keydown", onKey);
      previous?.focus?.({ preventScroll: true });
    };
  }, [ref, active]);
}

export interface SheetProps {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  footer?: ReactNode;
  /** "form": bottom sheet on compact, right drawer elsewhere. "dialog": bottom sheet on compact, centred elsewhere. */
  variant?: "form" | "dialog";
  size?: "sm" | "md" | "lg";
  /** Stacking level; a re-prompt opens above the editor that asked for it. */
  level?: 1 | 2;
  dismissible?: boolean;
  /** Tone of the title icon area is left to the caller via `titleIcon`. */
  titleIcon?: ReactNode;
}

const WIDTH = { sm: "w-[min(26rem,100vw)]", md: "w-[min(32rem,100vw)]", lg: "w-[min(42rem,100vw)]" } as const;
const SPRING = { type: "spring", stiffness: 420, damping: 38 } as const;

export function Sheet({ open, onClose, title, children, footer, variant = "form", size = "md", level = 1, dismissible = true, titleIcon }: SheetProps) {
  const layout = useLayout();
  const compact = layout === "compact";
  const place = compact ? "bottom" : variant === "form" ? "right" : "center";
  const panel = useRef<HTMLDivElement>(null);
  const drag = useDragControls();
  useFocusTrap(panel, open);

  useEffect(() => {
    if (!open || !dismissible) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && (e.stopPropagation(), onClose());
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, dismissible, onClose]);

  const z = level === 2 ? "z-[70]" : "z-[50]";
  const titleId = `sheet-${title.replace(/\W+/g, "-").toLowerCase()}`;

  const shell =
    place === "bottom"
      ? "inset-x-0 bottom-0 max-h-[92dvh] rounded-t-xl pb-[var(--safe-bottom)]"
      : place === "right"
        ? `inset-y-0 right-0 ${WIDTH[size]} rounded-l-xl pt-[var(--safe-top)]`
        : `left-1/2 top-1/2 max-h-[86dvh] ${WIDTH[size]} rounded-xl`;

  const motionProps =
    place === "bottom"
      ? { initial: { y: "100%" }, animate: { y: 0 }, exit: { y: "100%" } }
      : place === "right"
        ? { initial: { x: "100%" }, animate: { x: 0 }, exit: { x: "100%" } }
        : { initial: { opacity: 0, scale: 0.96, x: "-50%", y: "-46%" }, animate: { opacity: 1, scale: 1, x: "-50%", y: "-50%" }, exit: { opacity: 0, scale: 0.97, x: "-50%", y: "-48%" } };

  return createPortal(
    <AnimatePresence>
      {open && (
        <div className={`fixed inset-0 ${z}`}>
          <motion.div
            className="absolute inset-0 bg-marine/70 backdrop-blur-[3px]"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            onClick={dismissible ? onClose : undefined}
            aria-hidden
          />
          <motion.div
            ref={panel}
            role="dialog"
            aria-modal="true"
            aria-labelledby={titleId}
            tabIndex={-1}
            className={`glass-strong absolute flex flex-col overflow-hidden ${shell}`}
            {...motionProps}
            transition={place === "center" ? { duration: 0.2, ease: [0.2, 0.8, 0.2, 1] } : SPRING}
            {...(place === "bottom" && dismissible
              ? {
                  drag: "y" as const,
                  dragControls: drag,
                  dragListener: false,
                  dragConstraints: { top: 0, bottom: 0 },
                  dragElastic: { top: 0, bottom: 0.6 },
                  onDragEnd: (_: unknown, info: { offset: { y: number }; velocity: { y: number } }) => {
                    if (info.offset.y > 110 || info.velocity.y > 700) onClose();
                  },
                }
              : {})}
          >
            {place === "bottom" && (
              <div className="flex shrink-0 cursor-grab touch-none justify-center pt-2.5 pb-1 active:cursor-grabbing" onPointerDown={(e) => drag.start(e)} aria-hidden>
                <span className="h-1 w-10 rounded-full bg-mist/40" />
              </div>
            )}
            <header className={`flex shrink-0 items-center gap-3 px-5 ${place === "bottom" ? "pt-1 pb-3" : "pt-4 pb-3"}`}>
              {titleIcon}
              <h2 id={titleId} className="min-w-0 flex-1 truncate text-[1.13em] font-semibold tracking-[-0.005em]">
                {title}
              </h2>
              {place !== "bottom" && dismissible && (
                <IconButton label="Close" onClick={onClose}>
                  <X className="size-[18px]" />
                </IconButton>
              )}
            </header>
            <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-5 pb-5">{children}</div>
            {footer && <footer className="shrink-0 border-t border-hairline bg-pane/60 px-5 py-3.5">{footer}</footer>}
          </motion.div>
        </div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
