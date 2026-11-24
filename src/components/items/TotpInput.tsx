import { useEffect, useMemo, useRef, useState, type ClipboardEvent } from "react";
import { Camera, CircleAlert, CircleCheck, Eye, EyeOff, ImageUp, Info, ScanLine } from "lucide-react";
import { canScanWithCamera, decodeQrFromImage, openCameraSettings, ScanError, scanWithCamera } from "../../lib/scan";
import { describeOtp, parseOtpInput, type OtpAccount, type OtpParse } from "../../lib/otp";
import { Button, IconButton } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";
import { brandName } from "../../state/brand";

export interface TotpInputProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  /** New items can split a Google Authenticator export into one item per account; editing an existing item cannot. */
  canSplit: boolean;
  /** Called when a link or QR code names the service, so the editor can fill empty fields. */
  onIdentified?: (account: OtpAccount) => void;
  autoFocus?: boolean;
}

/** Everything a person might hand us for a one-time code: a setup key, an otpauth link, a QR image or a camera scan. */
export function TotpInput({ label, value, onChange, canSplit, onIdentified, autoFocus }: TotpInputProps) {
  const [shown, setShown] = useState(false);
  const [note, setNote] = useState<{ tone: "error" | "info"; text: string } | null>(null);
  const [needsSettings, setNeedsSettings] = useState(false);
  const [busy, setBusy] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const parse: OtpParse = useMemo(() => parseOtpInput(value), [value]);
  const mobile = canScanWithCamera();

  // Tell the editor who this code is for once a link names it.
  const identified = parse.kind === "ok" ? parse.account : null;
  const lastIdentified = useRef<string>("");
  useEffect(() => {
    if (identified && identified.value !== lastIdentified.current) {
      lastIdentified.current = identified.value;
      if (identified.issuer || identified.account) onIdentified?.(identified);
    }
  }, [identified, onIdentified]);

  /** Takes whatever text a scan or an image produced and feeds it through the same path as typing. */
  function accept(text: string) {
    setNote(null);
    setNeedsSettings(false);
    const parsed = parseOtpInput(text);
    if (parsed.kind === "export" && !canSplit) {
      setNote({ tone: "error", text: `That is a multi-account export. Add it from New, Code only, and ${brandName()} will split it into one item per account.` });
      return;
    }
    onChange(parsed.kind === "ok" ? parsed.account.value : text.trim());
    setShown(true);
  }

  async function scan() {
    setBusy(true);
    setNote(null);
    setNeedsSettings(false);
    try {
      const text = await scanWithCamera();
      if (text) accept(text);
    } catch (e) {
      if (e instanceof ScanError) {
        setNote({ tone: "error", text: e.message });
        setNeedsSettings(e.kind === "denied");
      } else {
        setNote({ tone: "error", text: "The camera could not start." });
      }
    } finally {
      setBusy(false);
    }
  }

  async function fromImage(blob: Blob) {
    setBusy(true);
    setNote(null);
    try {
      accept(await decodeQrFromImage(blob));
    } catch (e) {
      setNote({ tone: "error", text: e instanceof Error ? e.message : "That image could not be read." });
    } finally {
      setBusy(false);
    }
  }

  function onPaste(e: ClipboardEvent<HTMLInputElement>) {
    const img = [...e.clipboardData.files].find((f) => f.type.startsWith("image/"));
    if (img) { e.preventDefault(); void fromImage(img); }
  }

  const shortKey = parse.kind === "ok" && parse.account.shortKey;
  return (
    <Field label={label} error={parse.kind === "error" ? parse.message : note?.tone === "error" ? note.text : null}>
      {({ id, describedBy, invalid }) => (
        <div className="flex flex-col gap-2.5">
          <div className="relative">
            <TextInput
              id={id}
              mono
              data-autofocus={autoFocus || undefined}
              type={shown ? "text" : "password"}
              autoComplete="off"
              value={value}
              invalid={invalid}
              aria-describedby={describedBy}
              placeholder="Paste a link or setup key, or scan a QR code"
              onChange={(e) => { setNote(null); onChange(e.target.value); }}
              onPaste={onPaste}
              maxLength={8192}
              className="pr-11"
            />
            <IconButton label={shown ? "Hide the setup key" : "Show the setup key"} size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => setShown((v) => !v)}>
              {shown ? <EyeOff className="size-[17px]" /> : <Eye className="size-[17px]" />}
            </IconButton>
          </div>

          <div className="flex flex-wrap gap-2">
            {mobile && (
              <Button variant="key" busy={busy} icon={<Camera className="size-4" />} onClick={() => void scan()}>
                Scan QR code
              </Button>
            )}
            <Button disabled={busy} icon={<ImageUp className="size-4" />} onClick={() => file.current?.click()}>
              {mobile ? "Choose an image" : "Read a QR image"}
            </Button>
            {needsSettings && <Button icon={<ScanLine className="size-4" />} onClick={() => void openCameraSettings().catch(() => undefined)}>Open app settings</Button>}
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

          {parse.kind === "ok" && (
            <p className="flex items-start gap-2 text-[0.93em] text-verdigris" role="status">
              <CircleCheck className="mt-0.5 size-4 shrink-0" aria-hidden />
              <span>
                Recognised: {describeOtp(parse.account)}.
                {shortKey && <span className="text-mist"> This key is shorter than most services issue. It is accepted.</span>}
              </span>
            </p>
          )}
          {parse.kind === "export" && (
            <p className="flex items-start gap-2 text-[0.93em] text-brass-hi" role="status">
              <Info className="mt-0.5 size-4 shrink-0" aria-hidden />
              <span>
                This export holds {parse.accounts.length} accounts{parse.skipped ? ` (${parse.skipped} unsupported ${parse.skipped === 1 ? "entry is" : "entries are"} left out)` : ""}.{" "}
                {canSplit ? "Saving adds one item for each." : "To split it, add it from New, Code only."}
              </span>
            </p>
          )}
          {parse.kind === "empty" && !note && (
            <p className="flex items-start gap-2 text-[0.86em] text-fog">
              <CircleAlert className="mt-0.5 size-3.5 shrink-0" aria-hidden />
              <span>Any of these work: the setup key, an otpauth:// link, a link containing the key, a Google Authenticator export link, or a screenshot of the QR code.</span>
            </p>
          )}
        </div>
      )}
    </Field>
  );
}
