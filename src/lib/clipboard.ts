// Web clipboard with an auto-clear timer (spec: configurable 15/30/60 s). A native clipboard plugin
// (wl-copy / arboard / Android ClipboardManager with the sensitive flag) replaces this in the lock/hardening phase.

let clearTimer: ReturnType<typeof setTimeout> | undefined;

async function write(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    /* fall through to the legacy path (older WebViews, missing permission) */
  }
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.setAttribute("readonly", "");
  ta.className = "fixed -left-[9999px] top-0 opacity-0";
  document.body.appendChild(ta);
  ta.select();
  const ok = document.execCommand("copy");
  ta.remove();
  if (!ok) throw new Error("copy failed");
}

/** Copies text. When clearAfterSec is set, clears the clipboard later unless something else replaced it. */
export async function copyText(text: string, clearAfterSec?: number): Promise<void> {
  await write(text);
  if (clearTimer) clearTimeout(clearTimer);
  if (!clearAfterSec) return;
  clearTimer = setTimeout(async () => {
    try {
      const current = await navigator.clipboard.readText();
      if (current !== text) return; // the user copied something else meanwhile; leave it alone
    } catch {
      /* cannot read the clipboard: clear anyway, the spec favours privacy over preservation */
    }
    try {
      await navigator.clipboard.writeText("");
    } catch {
      /* nothing more we can do from a webview */
    }
  }, clearAfterSec * 1000);
}
