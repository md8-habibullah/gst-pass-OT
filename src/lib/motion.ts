// One easing and three durations for every animation in the app (seconds, for `motion`; the CSS twins are --dur-*).
// Short on purpose: a change of state should read as instant, never as something that is loading.
export const EASE = [0.2, 0.8, 0.2, 1] as const;
export const DUR = { fast: 0.08, base: 0.12, slow: 0.18 } as const;

export const tween = (duration: number = DUR.base) => ({ duration, ease: EASE });
