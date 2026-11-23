import { useSyncExternalStore } from "react";

export type Layout = "compact" | "medium" | "wide";

const COMPACT = "(max-width: 719.98px)";
const WIDE = "(min-width: 1100px)";

function subscribe(cb: () => void) {
  const queries = [matchMedia(COMPACT), matchMedia(WIDE)];
  queries.forEach((q) => q.addEventListener("change", cb));
  return () => queries.forEach((q) => q.removeEventListener("change", cb));
}

function snapshot(): Layout {
  if (matchMedia(COMPACT).matches) return "compact";
  return matchMedia(WIDE).matches ? "wide" : "medium";
}

/** Compact = Android-style (bottom nav, sheets); medium = icon rail; wide = full three panes. */
export function useLayout(): Layout {
  return useSyncExternalStore(subscribe, snapshot, () => "wide");
}
