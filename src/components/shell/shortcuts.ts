// The keyboard handler. It reads lib/shortcuts.ts for what a key means and does the matching thing here.
// Installed once by the Shell while a vault is open.
import { copyText } from "../../lib/clipboard";
import { CHORD_MS, isMacPlatform, matchKey } from "../../lib/shortcuts";
import { app, be, lockApp, lockVault, pushToast, select, setFilter, visibleList, type Filter } from "../../state/app";
import { handleBack } from "../../state/back";
import { openNewItem, openScanner, setSettingsPage, setShortcutsOpen, ui } from "../../state/ui";
import { cloneItem, copyQuick, editItem, launch, QUICK_COPY, setArchived, trashItem, type Target } from "../items/actions";
import { withItemPassword } from "../reprompt/Reprompt";
import { FILTER_TYPE, ITEM_FILTERS } from "./nav";

const GO: Record<string, Filter> = {
  "go.all": "all", "go.logins": "logins", "go.cards": "cards", "go.identities": "identities", "go.notes": "notes", "go.ssh": "ssh",
  "go.codes": "codes", "go.favorites": "favorites", "go.archive": "archive", "go.trash": "trash", "go.files": "files",
  "go.generator": "generator", "go.import": "import", "go.export": "export", "go.backup": "backup",
};

const FIELD = "input,textarea,select,[contenteditable='true'],[contenteditable='']";
const modalOpen = () => document.querySelector('[aria-modal="true"]') !== null;

function selectedTarget(): Target | null {
  const { selectedId, items } = app.get();
  const found = selectedId ? items.find((i) => i.id === selectedId) : undefined;
  if (!found) {
    pushToast({ kind: "info", text: "Choose an item first." });
    return null;
  }
  return found;
}

function focusSearch() {
  const go = () => {
    const input = document.querySelector<HTMLInputElement>("[data-search]");
    input?.focus();
    input?.select();
  };
  if (!ITEM_FILTERS.has(app.get().filter)) {
    setFilter("all");
    // the list pane has to exist before the box can be focused
    window.setTimeout(go, 60);
  } else go();
}

/** Open Settings, on a given page when one is named. */
export function openSettings(page: Parameters<typeof setSettingsPage>[0] = null) {
  setSettingsPage(page);
  setFilter("settings");
}

async function copyCode(t: Target) {
  if (!app.get().items.find((i) => i.id === t.id)?.hasTotp) return void pushToast({ kind: "info", text: `“${t.title}” has no one-time code.` });
  try {
    const now = await withItemPassword({ reason: `Copy the one-time code for “${t.title}”.`, confirmLabel: "Copy", run: (pw) => be().totpNow(t.id, pw) });
    if (!now) return;
    await copyText(now.code);
    pushToast({ kind: "success", text: `Code for ${t.title} copied.` });
  } catch {
    pushToast({ kind: "error", text: "Could not copy the code." });
  }
}

function step(dir: 1 | -1) {
  const { filter, trash, selectedId } = app.get();
  if (document.querySelector("[data-compact-detail]")) return;
  const ids = filter === "trash" ? trash.map((t) => t.id) : ITEM_FILTERS.has(filter) ? visibleList(app.get()).items.map((i) => i.id) : [];
  if (ids.length === 0) return;
  const at = selectedId ? ids.indexOf(selectedId) : -1;
  const next = ids[Math.max(0, Math.min(ids.length - 1, at < 0 ? (dir === 1 ? 0 : ids.length - 1) : at + dir))]!;
  select(next);
  window.requestAnimationFrame(() => document.querySelector(`[data-item-row][data-item-id="${CSS.escape(next)}"]`)?.scrollIntoView({ block: "nearest" }));
}

function run(id: string) {
  const { filter, profile, gate } = app.get();
  if (id in GO) return setFilter(GO[id]!);
  switch (id) {
    case "help": return setShortcutsOpen(!ui.get().shortcutsOpen);
    case "search": return focusSearch();
    case "new": return openNewItem(FILTER_TYPE[filter] ?? null);
    case "scan": return openScanner("choose");
    case "settings": return openSettings();
    case "go.help": return openSettings("help");
    case "lock":
      // the built-in vault opens itself again, so locking it would only flash the lock screen
      return profile?.demo ? pushToast({ kind: "info", text: "This vault opens without a password, so there is nothing to lock." }) : void lockVault();
    case "lockApp":
      return gate?.enabled ? void lockApp() : pushToast({ kind: "info", text: "Set up an app lock in Settings first.", action: { label: "Open settings", run: () => openSettings("applock") } });
    case "escape": {
      const active = document.activeElement as HTMLElement | null;
      if (active?.matches("[data-search]")) { (active as HTMLInputElement).blur(); return; }
      if (handleBack()) return;
      if (app.get().selectedId) select(null);
      return;
    }
    case "next": return step(1);
    case "prev": return step(-1);
    default: break;
  }
  // everything below acts on the selected item
  const t = selectedTarget();
  if (!t) return;
  switch (id) {
    case "edit": return void editItem(t);
    case "open": return void launch(t);
    case "trash": return void trashItem(t);
    case "clone": return void cloneItem(t);
    case "archive": return void setArchived(t, !t.archived);
    case "copyCode": return void copyCode(t);
    case "copyUser": {
      const q = t.type === "login" || t.type === "identity" ? QUICK_COPY[t.type]?.[0] : undefined;
      return q ? void copyQuick(t, q) : pushToast({ kind: "info", text: "This item has no username or email to copy." });
    }
    case "copyPass": {
      const q = t.type === "login" ? QUICK_COPY.login?.[1] : t.type === "card" ? QUICK_COPY.card?.[0] : t.type === "sshKey" ? QUICK_COPY.sshKey?.[1] : undefined;
      return q ? void copyQuick(t, q) : pushToast({ kind: "info", text: "This item has no password, card number or private key to copy." });
    }
    default: return;
  }
}

/** Starts listening. Returns the function that stops it. */
export function installShortcuts(): () => void {
  const mac = isMacPlatform();
  let pending: string | null = null;
  let timer = 0;
  const clear = () => { pending = null; window.clearTimeout(timer); };

  const onKey = (e: KeyboardEvent) => {
    if (e.isComposing) return;
    const typing = (e.target instanceof Element && e.target.closest(FIELD) !== null) || modalOpen();
    const hit = matchKey(e, { typing, pending, isMac: mac });
    if (!hit) { clear(); return; }
    if (hit.kind === "prefix") {
      if (e.repeat) return;
      e.preventDefault();
      pending = hit.prefix;
      window.clearTimeout(timer);
      timer = window.setTimeout(clear, CHORD_MS);
      return;
    }
    // while a dialog is open only the table's "works everywhere" combos get this far; Escape belongs to the dialog itself
    if (modalOpen() && hit.id === "escape") return;
    if (e.repeat && hit.id !== "next" && hit.id !== "prev") return;
    e.preventDefault();
    clear();
    run(hit.id);
  };
  window.addEventListener("keydown", onKey);
  return () => { window.removeEventListener("keydown", onKey); clear(); };
}
