import { useEffect, useRef, useState } from "react";
import { FileWarning } from "lucide-react";
import type { FileEntry } from "../../backend";
import { MAX_FILE_LABEL, previewKind } from "../../lib/files";
import { be } from "../../state/app";
import { Spinner } from "../ui/Button";
import { Sheet } from "../ui/Sheet";

const TEXT_LIMIT = 2 * 1024 * 1024;
const PDF_PAGES = 40;

type Loaded =
  | { kind: "image"; url: string }
  | { kind: "text"; text: string; cut: boolean }
  | { kind: "pdf"; bytes: ArrayBuffer }
  | { kind: "none" };

/** Decrypted preview inside the app. Bytes live in memory only while this is open; the blob URL is revoked on close. */
export function FilePreview({ file, onClose }: { file: FileEntry | null; onClose: () => void }) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Android: no screenshots or screen recording while a decrypted file is on screen. A no-op everywhere else.
  const open = file !== null;
  useEffect(() => {
    if (!open) return;
    void be().screenSecure(true).catch(() => undefined);
    return () => { void be().screenSecure(false).catch(() => undefined); };
  }, [open]);

  useEffect(() => {
    if (!file) return;
    let alive = true;
    let url: string | null = null;
    setLoaded(null); setError(null);
    const kind = previewKind(file);
    if (kind === "none") { setLoaded({ kind: "none" }); return; }
    be().fileRead(file.id).then((buf) => {
      if (!alive) return;
      if (kind === "image") { url = URL.createObjectURL(new Blob([buf], { type: file.mime ?? "image/*" })); setLoaded({ kind: "image", url }); }
      else if (kind === "text") setLoaded({ kind: "text", text: new TextDecoder("utf-8", { fatal: false }).decode(buf.slice(0, TEXT_LIMIT)), cut: buf.byteLength > TEXT_LIMIT });
      else setLoaded({ kind: "pdf", bytes: buf });
    }).catch((e: unknown) => alive && setError(e instanceof Error ? e.message : "Could not open this file."));
    return () => { alive = false; if (url) URL.revokeObjectURL(url); setLoaded(null); };
  }, [file]);

  return (
    <Sheet open={file !== null} onClose={onClose} title={file?.name ?? "Preview"} variant="form" size="lg">
      <div className="flex min-h-48 flex-col gap-3">
        {error && <p role="alert" className="rounded-lg border border-ember/40 bg-danger/10 px-4 py-3">{error}</p>}
        {!error && !loaded && <p className="flex items-center gap-2 text-mist" role="status"><Spinner /> Decrypting</p>}
        {loaded?.kind === "image" && <img src={loaded.url} alt={file?.name ?? ""} className="mx-auto max-h-[70dvh] max-w-full rounded-lg bg-pane object-contain" draggable={false} />}
        {loaded?.kind === "text" && (
          <>
            <pre className="max-h-[70dvh] overflow-auto rounded-lg bg-pane p-4 font-mono text-[0.9em] leading-6 whitespace-pre-wrap break-words">{loaded.text}</pre>
            {loaded.cut && <p className="text-[0.86em] text-fog">Showing the first 2 MB. Save a decrypted copy to read the rest.</p>}
          </>
        )}
        {loaded?.kind === "pdf" && <PdfPages bytes={loaded.bytes} onError={setError} />}
        {loaded?.kind === "none" && (
          <p className="flex items-start gap-3 text-mist"><FileWarning className="mt-0.5 size-5 shrink-0" aria-hidden />This kind of file cannot be shown inside the app. You can save a decrypted copy instead (it needs your master password). Files can be up to {MAX_FILE_LABEL}.</p>
        )}
      </div>
    </Sheet>
  );
}

/** Pages are drawn on canvases with pdf.js (a PDF cannot be shown natively in the Linux or Android web views). */
function PdfPages({ bytes, onError }: { bytes: ArrayBuffer; onError: (m: string) => void }) {
  const box = useRef<HTMLDivElement>(null);
  const [pages, setPages] = useState<{ shown: number; total: number } | null>(null);

  useEffect(() => {
    let alive = true;
    let task: { destroy(): Promise<void> } | null = null;
    (async () => {
      const pdfjs = await import("pdfjs-dist");
      pdfjs.GlobalWorkerOptions.workerSrc = (await import("pdfjs-dist/build/pdf.worker.min.mjs?url")).default;
      const loading = pdfjs.getDocument({ data: new Uint8Array(bytes.slice(0)), useSystemFonts: false, enableXfa: false, useWorkerFetch: false });
      task = loading;
      const pdf = await loading.promise;
      if (!alive || !box.current) return;
      const shown = Math.min(pdf.numPages, PDF_PAGES);
      setPages({ shown, total: pdf.numPages });
      const width = Math.min(box.current.clientWidth || 640, 900);
      for (let n = 1; n <= shown && alive; n++) {
        const page = await pdf.getPage(n);
        const base = page.getViewport({ scale: 1 });
        const viewport = page.getViewport({ scale: (width / base.width) * Math.min(window.devicePixelRatio || 1, 2) });
        const canvas = document.createElement("canvas");
        canvas.width = Math.floor(viewport.width); canvas.height = Math.floor(viewport.height);
        canvas.className = "mx-auto h-auto max-w-full rounded-lg bg-white shadow-e2";
        canvas.setAttribute("role", "img"); canvas.setAttribute("aria-label", `Page ${n} of ${pdf.numPages}`);
        box.current?.append(canvas);
        await page.render({ canvas, viewport }).promise;
      }
    })().catch(() => alive && onError("This PDF could not be shown. Save a decrypted copy to open it elsewhere."));
    return () => { alive = false; void task?.destroy(); box.current?.replaceChildren(); };
  }, [bytes, onError]);

  return (
    <>
      <div ref={box} className="flex max-h-[70dvh] flex-col gap-3 overflow-y-auto rounded-lg bg-pane p-3" />
      {pages && pages.total > pages.shown && <p className="text-[0.86em] text-fog">Showing the first {pages.shown} of {pages.total} pages. Save a decrypted copy to read the rest.</p>}
    </>
  );
}
