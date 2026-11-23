import { useSyncExternalStore } from "react";

export interface Store<S> {
  get(): S;
  set(next: S | ((prev: S) => S)): void;
  subscribe(listener: () => void): () => void;
}

/** Minimal external store (React `useSyncExternalStore` friendly). */
export function createStore<S>(initial: S): Store<S> {
  let state = initial;
  const listeners = new Set<() => void>();
  return {
    get: () => state,
    set(next) {
      const value = typeof next === "function" ? (next as (p: S) => S)(state) : next;
      if (Object.is(value, state)) return;
      state = value;
      listeners.forEach((l) => l());
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

/** Subscribe to a slice. The selector must return a referentially stable value (a field, not a fresh object). */
export function useStore<S, R>(store: Store<S>, selector: (s: S) => R): R {
  return useSyncExternalStore(store.subscribe, () => selector(store.get()), () => selector(store.get()));
}
