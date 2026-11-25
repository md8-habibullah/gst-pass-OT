import { useCallback, useEffect, useRef, useState, type DragEvent } from "react";
import { Camera, CircleCheck, Eye, EyeOff, ImageUp, ScanLine } from "lucide-react";
import { BackendError, type ItemSummary, type ScanReply } from "../../backend";
import { canScanWithCamera, decodeQrFromImage, openCameraSettings, ScanError, scanWithCamera } from "../../lib/scan";
import { app, be, select, setFilter } from "../../state/app";
import { brandName } from "../../state/brand";
import { useStore } from "../../state/store";
import { closeScanner, ui } from "../../state/ui";
import { ITEM_FILTERS } from "../shell/nav";
import { Button, IconButton } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { Sheet } from "../ui/Sheet";
import { ScanResult } from "./ScanResult";

type Stage =
  | { kind: "start"; note?: string; settings?: boolean }
  | { kind: "reading" }
  | { kind: "found"; text: string; reply: ScanReply }
  | { kind: "done"; item: ItemSummary; created: boolean };

const TITLE: Record<string, string> = { start: "Scan", reading: "Reading", done: "Saved" };

function titleOf(stage: Stage): string {
  if (stage.kind !== "found") return TITLE[stage.kind]!;
  switch (stage.reply.kind) {
    case "totp": return "One-time code";
    case "fido": return "Passkey sign-in";
    case "migration": return "Code export";
    case "link": return "Web link";
    default: return "Not a code";
  }
}

const messageOf = (e: unknown) => (e instanceof BackendError || e instanceof ScanError || e instanceof Error ? e.message : "That did not work. Try again.");

/**
 * The all-in-one scanner. The camera button opens it; the camera starts at once on a phone. Whatever it reads goes to the core,
 * which says what it is (a one-time code, a passkey sign-in code, a link …) and, for a code, which logins it probably belongs
 * to. The person picks one of those logins, any other login, or makes a new one. Nothing is saved before that choice.
 */
export function ScanSheet() {
  const req = useStore(ui, (s) => s.scanner);
  const open = req !== null;
  const [stage, setStage] = useState<Stage>({ kind: "start" });
  const [typed, setTyped] = useState("");
  const [shown, setShown] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  /** Each opening gets a number, so a camera or image that finishes after the sheet closed is ignored. */
  const run = useRef(0);
  const phone = canScanWithCamera();

  const reset = useCallback(() => {
    run.current++;
    setStage({ kind: "start" });
    setTyped("");
    setShown(false);
  }, []);

  const inspect = useCallback(async (text: string, isTyped: boolean) => {
    const mine = ++run.current;
    setStage({ kind: "reading" });
    try {
      const reply = await be().scanClassify(text, isTyped);
      if (mine === run.current) setStage({ kind: "found", text, reply });
    } catch (e) {
      if (mine === run.current) setStage({ kind: "start", note: messageOf(e) });
    }
  }, []);

  const camera = useCallback(async (closeOnCancel: boolean) => {
    const mine = ++run.current;
    setStage({ kind: "reading" });
    try {
      const text = await scanWithCamera();
      if (mine !== run.current) return;
      if (text === null) return closeOnCancel ? closeScanner() : setStage({ kind: "start" });
      await inspect(text, false);
    } catch (e) {
      if (mine === run.current) setStage({ kind: "start", note: messageOf(e), settings: e instanceof ScanError && e.kind === "denied" });
    }
  }, [inspect]);

  const fromImage = useCallback(async (blob: Blob) => {
    const mine = ++run.current;
    setStage({ kind: "reading" });
    try {
      const text = await decodeQrFromImage(blob);
      if (mine === run.current) await inspect(text, false);
    } catch (e) {
      if (mine === run.current) setStage({ kind: "start", note: messageOf(e) });
    }
  }, [inspect]);

  // Each time the scanner opens it starts clean, and on a phone the camera starts without another tap.
  useEffect(() => {
    reset();
    if (req?.start === "camera" && phone) void camera(true);
    return () => { run.current++; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- a new request object is a new opening
  }, [req]);

  // A picture pasted anywhere on the first screen is read; so is pasted text that did not land in the box.
  useEffect(() => {
    if (!open || stage.kind !== "start") return;
    const onPaste = (e: ClipboardEvent) => {
      const img = [...(e.clipboardData?.files ?? [])].find((f) => f.type.startsWith("image/"));
      if (img) { e.preventDefault(); void fromImage(img); return; }
      const inBox = e.target instanceof Element && e.target.closest("input,textarea") !== null;
      const text = e.clipboardData?.getData("text/plain") ?? "";
      if (!inBox && text.trim()) { e.preventDefault(); void inspect(text, true); }
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, [open, stage.kind, fromImage, inspect]);

  function onDrop(e: DragEvent) {
    e.preventDefault();
    const img = [...e.dataTransfer.files].find((f) => f.type.startsWith("image/"));
    if (img) void fromImage(img);
  }

  function saved(item: ItemSummary, created: boolean) {
    setStage({ kind: "done", item, created });
  }

  function openLogin(id: string) {
    closeScanner();
    if (!ITEM_FILTERS.has(app.get().filter)) setFilter("all");
    select(id);
  }

  const footer =
    stage.kind === "found" ? (
      <Button variant="ghost" onClick={reset}>Scan something else</Button>
    ) : stage.kind === "done" ? (
      <div className="flex flex-wrap justify-end gap-2">
        <Button variant="ghost" onClick={reset}>Scan another</Button>
        <Button variant="key" onClick={() => openLogin(stage.item.id)}>Open {stage.item.title}</Button>
      </div>
    ) : undefined;

  return (
    <Sheet open={open} onClose={closeScanner} title={titleOf(stage)} variant="form" size="md" level={1} footer={footer} dismissible titleIcon={<ScanLine className="size-5 text-brass-ink" aria-hidden />}>
      {stage.kind === "start" && (
        <div className="flex flex-col gap-5 pt-1" onDragOver={(e) => e.preventDefault()} onDrop={onDrop}>
          <p className="text-mist">
            Scan the QR code of a one-time code or a passkey sign-in code. {brandName()} sees which it is and offers the login it belongs to, or makes a new one.
          </p>
          {stage.note && (
            <div role="alert" className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-lg border border-ember/40 bg-pane px-3.5 py-3">
              <p className="min-w-0 flex-1 basis-56 text-[0.93em] text-ember">{stage.note}</p>
              {stage.settings && <Button onClick={() => void openCameraSettings().catch(() => undefined)}>Open app settings</Button>}
            </div>
          )}
          <div className="flex flex-wrap gap-2">
            {phone && <Button variant="key" icon={<Camera className="size-4" />} data-autofocus onClick={() => void camera(false)}>Open the camera</Button>}
            <Button variant={phone ? "quiet" : "key"} data-autofocus={phone ? undefined : true} icon={<ImageUp className="size-4" />} onClick={() => file.current?.click()}>
              {phone ? "Choose an image" : "Choose a picture of the QR code"}
            </Button>
            <input
              ref={file}
              type="file"
              accept="image/*"
              className="hidden"
              tabIndex={-1}
              aria-hidden
              onChange={(e) => {
                const f = e.target.files?.[0];
                e.target.value = "";
                if (f) void fromImage(f);
              }}
            />
          </div>
          {!phone && <p className="-mt-2 text-[0.86em] text-fog">You can also drop a picture here, or paste a screenshot.</p>}
          <form
            className="flex flex-col gap-3"
            onSubmit={(e) => { e.preventDefault(); if (typed.trim()) void inspect(typed, true); }}
          >
            <Field label="Or paste a setup key or link">
              {({ id, describedBy }) => (
                <div className="relative">
                  <TextInput
                    id={id}
                    aria-describedby={describedBy}
                    mono
                    type={shown ? "text" : "password"}
                    autoComplete="off"
                    value={typed}
                    maxLength={4096}
                    placeholder="Setup key or otpauth:// link"
                    className="pr-11 pointer-coarse:pr-14"
                    onChange={(e) => setTyped(e.target.value)}
                  />
                  <IconButton label={shown ? "Hide what you typed" : "Show what you typed"} size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => setShown((v) => !v)}>
                    {shown ? <EyeOff className="size-[17px]" /> : <Eye className="size-[17px]" />}
                  </IconButton>
                </div>
              )}
            </Field>
            <Button type="submit" className="self-start" disabled={!typed.trim()}>Check it</Button>
          </form>
        </div>
      )}

      {stage.kind === "reading" && (
        <p role="status" className="flex items-center gap-2 pt-2 text-mist">
          <ScanLine className="size-4 animate-pulse text-brass-ink" aria-hidden />
          Reading the code…
        </p>
      )}

      {stage.kind === "found" && <ScanResult text={stage.text} reply={stage.reply} onSaved={saved} onClose={closeScanner} />}

      {stage.kind === "done" && (
        <div role="status" className="flex items-start gap-3 pt-2">
          <CircleCheck className="mt-0.5 size-5 shrink-0 text-verdigris" aria-hidden />
          <div>
            <p className="font-semibold">{stage.created ? `Made the login “${stage.item.title}”` : `Added the code to “${stage.item.title}”`}</p>
            <p className="mt-1 text-[0.93em] text-mist">The code is saved in the vault and works right away.</p>
          </div>
        </div>
      )}
    </Sheet>
  );
}
