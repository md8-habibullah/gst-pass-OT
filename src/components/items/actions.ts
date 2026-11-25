// What a person can do with one item, shared by the detail pane, the row menu and the Codes view.
// Secret reads need the master password only for items set to ask for it (see withItemPassword); non-secret reads never do.
import { BackendError, type ItemSummary, type ItemType, type ItemView } from "../../backend";
import { copyText } from "../../lib/clipboard";
import { urlsOf } from "../../lib/urls";
import { app, be, pushToast, refresh, select, setFilter } from "../../state/app";
import { openEditor, openFolderDialog } from "../../state/ui";
import { withItemPassword } from "../reprompt/Reprompt";
import { ITEM_FILTERS } from "../shell/nav";

export type Target = Pick<ItemSummary, "id" | "type" | "title" | "archived" | "favorite">;

export interface QuickCopy {
  key: string;
  /** Lower case noun, used as "Copy <label>". */
  label: string;
  secret: boolean;
}

/** The one-tap copies each type offers in its menu (Bitwarden-style "Copy username / Copy password"). */
export const QUICK_COPY: Partial<Record<ItemType, QuickCopy[]>> = {
  login: [
    { key: "username", label: "username", secret: false },
    { key: "password", label: "password", secret: true },
  ],
  card: [
    { key: "number", label: "card number", secret: true },
    { key: "code", label: "security code", secret: true },
  ],
  identity: [
    { key: "email", label: "email", secret: false },
    { key: "phone", label: "phone number", secret: false },
  ],
  sshKey: [
    { key: "publicKey", label: "public key", secret: false },
    { key: "privateKey", label: "private key", secret: true },
  ],
};

const reason = (e: unknown, fallback: string) => (e instanceof BackendError || e instanceof Error ? e.message : fallback);

/** Makes a typed or pasted address openable: adds https:// when there is no scheme. */
export function webAddress(raw: string): string {
  const s = raw.trim();
  return /^[a-z][a-z0-9+.-]*:\/\//i.test(s) ? s : `https://${s}`;
}

/** Every web address an item has: its url fields can hold several, a line each. */
export const findUrls = (view: Pick<ItemView, "fields">): string[] => view.fields.filter((f) => f.kind === "url").flatMap((f) => urlsOf(f.value));

/** The first address, which "Launch" opens. */
export const findUrl = (view: Pick<ItemView, "fields">): string | null => findUrls(view)[0] ?? null;

export async function openLink(raw: string): Promise<void> {
  try {
    await be().openUrl(webAddress(raw));
  } catch (e) {
    pushToast({ kind: "error", text: reason(e, "Could not open that link.") });
  }
}

export async function launch(item: Target): Promise<void> {
  try {
    const url = findUrl(await be().getItem(item.id));
    if (!url) return void pushToast({ kind: "info", text: `“${item.title}” has no website saved.` });
    await openLink(url);
  } catch (e) {
    pushToast({ kind: "error", text: reason(e, "Could not open the website.") });
  }
}

export async function copyQuick(item: Target, q: QuickCopy): Promise<void> {
  const clear = app.get().clipboardClear;
  try {
    let value: string | undefined;
    if (q.secret) {
      const draft = await withItemPassword({
        reason: `Copy the ${q.label} for “${item.title}”. It leaves the clipboard again after ${clear} seconds.`,
        confirmLabel: "Copy",
        run: (pw) => be().revealItem(item.id, pw),
      });
      if (!draft) return;
      value = draft.fields[q.key];
    } else {
      value = (await be().getItem(item.id)).fields.find((f) => f.key === q.key)?.value ?? undefined;
    }
    if (!value) return void pushToast({ kind: "info", text: `“${item.title}” has no ${q.label} saved.` });
    await copyText(value, q.secret ? clear : undefined);
    // A short notice, not a timer: the clipboard is cleared in the background whether or not the notice is still showing.
    pushToast({ kind: "success", text: q.secret ? `The ${q.label} was copied. It leaves the clipboard in ${clear} seconds.` : `The ${q.label} was copied.` });
  } catch (e) {
    pushToast({ kind: "error", text: reason(e, "Copying was blocked on this device.") });
  }
}

/** Shows the item in the detail pane, switching to the list that contains it when needed. */
export function viewItem(item: Target): void {
  const { filter } = app.get();
  if (!ITEM_FILTERS.has(filter) || (item.archived && filter !== "archive")) setFilter(item.archived ? "archive" : "all");
  select(item.id);
}

/** Opens the "Move to folder" dialog for one item. */
export function moveItemToFolder(item: Pick<ItemSummary, "id" | "title" | "folder">): void {
  openFolderDialog({ kind: "move", ids: [item.id], title: item.title, current: item.folder });
}

export async function editItem(item: Target): Promise<void> {
  try {
    const session = await withItemPassword({ reason: `Edit “${item.title}”. Earlier versions stay in the history.`, confirmLabel: "Edit item", run: (pw) => be().beginEdit(item.id, pw) });
    if (session) openEditor(item.id, session);
  } catch (e) {
    pushToast({ kind: "error", text: reason(e, "Could not start editing.") });
  }
}

export async function cloneItem(item: Target): Promise<void> {
  try {
    const copy = await withItemPassword({ reason: `Make a copy of “${item.title}”. Its secrets are duplicated inside the vault.`, confirmLabel: "Clone", run: (pw) => be().cloneItem(item.id, pw) });
    if (!copy) return;
    await refresh();
    viewItem(copy);
    pushToast({ kind: "success", text: `“${copy.title}” was created.` });
  } catch (e) {
    pushToast({ kind: "error", text: reason(e, "Could not clone the item.") });
  }
}

export async function trashItem(item: Target, after?: () => void): Promise<void> {
  try {
    await be().trashItem(item.id);
    await refresh();
    select(null);
    after?.();
    pushToast({
      kind: "info",
      text: `“${item.title}” moved to the trash.`,
      ttlMs: 6000,
      action: { label: "Undo", run: () => void be().restoreItem(item.id).then(refresh).then(() => pushToast({ kind: "success", text: `“${item.title}” is back in your vault.` })).catch((e: unknown) => pushToast({ kind: "error", text: reason(e, "Could not restore it.") })) },
    });
  } catch (e) {
    pushToast({ kind: "error", text: reason(e, "Could not move the item to the trash.") });
  }
}

export async function setArchived(item: Target, archived: boolean, after?: () => void): Promise<void> {
  try {
    await (archived ? be().archiveItem(item.id) : be().unarchiveItem(item.id));
    await refresh();
    select(null);
    after?.();
    pushToast({
      kind: "info",
      text: archived ? `“${item.title}” moved to the archive. It stays encrypted and is hidden from your lists.` : `“${item.title}” is back in your vault.`,
      ttlMs: 6000,
      action: { label: "Undo", run: () => void setArchived(item, !archived) },
    });
  } catch (e) {
    pushToast({ kind: "error", text: reason(e, archived ? "Could not archive the item." : "Could not take the item out of the archive.") });
  }
}
