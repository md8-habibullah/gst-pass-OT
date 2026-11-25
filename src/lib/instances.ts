import { instanceKey, MAX_INSTANCES, parseFieldKey } from "../backend/types";

/**
 * A login can hold several sign-ins (username + password) and several one-time codes. They are stored as `username`,
 * `username#2` … in the one `fields` map (see model.rs): a "slot" is the number after the `#`, and the first slot has the
 * plain key. These helpers work on that map while it is being edited; the editor shows one row per slot.
 *
 * Slots are only numbers while editing. On save `compactSlots` renumbers them 1, 2, 3 … so that the first sign-in or code is
 * always the plain key (the one the list row, autofill and exports use).
 */

export type Fields = Record<string, string>;

/** Sign-ins are two fields that belong together; a code is one. */
export const SIGN_IN = ["username", "password"] as const;
export const CODE = ["totp"] as const;

/** The slot numbers in use for these fields, lowest first. A key counts even while its value is still blank. */
export function slotsOf(fields: Fields, bases: readonly string[]): number[] {
  const seen = new Set<number>();
  for (const key of Object.keys(fields)) {
    const p = parseFieldKey(key);
    if (p && bases.includes(p.base)) seen.add(p.n);
  }
  return [...seen].sort((a, b) => a - b);
}

/** The slots a form shows: the ones in use, or the first (empty) one when there are none. */
export function rowsOf(fields: Fields, bases: readonly string[]): number[] {
  const slots = slotsOf(fields, bases);
  return slots.length ? slots : [1];
}

/** Does this slot hold anything yet? */
export function slotFilled(fields: Fields, bases: readonly string[], n: number): boolean {
  return bases.some((b) => (fields[instanceKey(b, n)] ?? "") !== "");
}

/** A new blank slot at the lowest free number, or `null` when all are used. The map is copied, never changed. */
export function addSlot(fields: Fields, bases: readonly string[]): Fields | null {
  const used = new Set(slotsOf(fields, bases));
  if (!used.size) used.add(1); // the first slot is shown even when blank
  for (let n = 2; n <= MAX_INSTANCES; n++) {
    if (!used.has(n)) return { ...fields, [instanceKey(bases[0]!, n)]: "" };
  }
  return null;
}

/** Take a slot out (every field of it). */
export function removeSlot(fields: Fields, bases: readonly string[], n: number): Fields {
  const out = { ...fields };
  for (const b of bases) delete out[instanceKey(b, n)];
  return out;
}

/** Drop blank slots and renumber the rest 1, 2, 3 … in their order. Everything else in the map is left alone. */
export function compactSlots(fields: Fields, bases: readonly string[]): Fields {
  const out: Fields = {};
  for (const [k, v] of Object.entries(fields)) {
    const p = parseFieldKey(k);
    if (!p || !bases.includes(p.base)) out[k] = v;
  }
  let next = 1;
  for (const n of slotsOf(fields, bases)) {
    if (!slotFilled(fields, bases, n)) continue;
    for (const b of bases) {
      const v = fields[instanceKey(b, n)];
      if (v !== undefined && v !== "") out[instanceKey(b, next)] = v;
    }
    next++;
  }
  return out;
}

/** The same map without blank values: what the core would keep. Used to tell whether an editor has real changes. */
export function withoutBlanks(fields: Fields): Fields {
  return Object.fromEntries(Object.entries(fields).filter(([, v]) => v !== ""));
}

/**
 * Order the fields of a login for display so each sign-in is together (Username 1, Password 1, Username 2, Password 2) at
 * the place the first of them had. Everything else keeps its order. The core lists all usernames, then all passwords.
 */
export function groupSignIns<T extends { key: string }>(fields: T[]): T[] {
  const num = (f: T) => parseFieldKey(f.key)?.n ?? 1;
  const isSignIn = (f: T) => {
    const p = parseFieldKey(f.key);
    return p !== null && (SIGN_IN as readonly string[]).includes(p.base);
  };
  const first = fields.findIndex(isSignIn);
  if (first < 0) return fields;
  const group = fields
    .filter(isSignIn)
    .sort((a, b) => num(a) - num(b) || (parseFieldKey(a.key)!.base === "username" ? -1 : 1));
  const rest = fields.filter((f) => !isSignIn(f));
  rest.splice(first, 0, ...group);
  return rest;
}
