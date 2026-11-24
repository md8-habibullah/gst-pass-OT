import { useMemo, useState, type FormEvent, type ReactNode } from "react";
import { CircleAlert, CircleCheck, Eye, EyeOff, PlugZap } from "lucide-react";
import { BackendError, type BackupTargetInput, type BackupTargetView, type BackupTestResult } from "../../backend";
import { be } from "../../state/app";
import { Button, IconButton } from "../ui/Button";
import { Field, Switch, TextInput } from "../ui/Inputs";

interface Provider {
  id: string;
  label: string;
  /** null = Amazon S3 (no custom endpoint). */
  endpoint: string | null;
  endpointHint: string;
  region: string;
  pathStyle: boolean;
}

const PROVIDERS: Provider[] = [
  { id: "aws", label: "Amazon S3", endpoint: null, endpointHint: "", region: "us-east-1", pathStyle: false },
  { id: "r2", label: "Cloudflare R2", endpoint: "https://<account-id>.r2.cloudflarestorage.com", endpointHint: "Your R2 S3 endpoint from the Cloudflare dashboard.", region: "auto", pathStyle: true },
  { id: "b2", label: "Backblaze B2", endpoint: "https://s3.<region>.backblazeb2.com", endpointHint: "The S3 endpoint shown on your B2 bucket, for example s3.us-west-004.backblazeb2.com.", region: "us-west-004", pathStyle: true },
  { id: "other", label: "Other S3-compatible", endpoint: "https://", endpointHint: "MinIO, Wasabi, DigitalOcean Spaces, Ceph and similar. Use the endpoint your provider gives you.", region: "us-east-1", pathStyle: true },
];

function guessProvider(t?: BackupTargetView): Provider {
  if (!t) return PROVIDERS[0]!;
  if (!t.endpoint) return PROVIDERS[0]!;
  if (t.endpoint.includes("r2.cloudflarestorage.com")) return PROVIDERS[1]!;
  if (t.endpoint.includes("backblazeb2.com")) return PROVIDERS[2]!;
  return PROVIDERS[3]!;
}

const blank = (provider: Provider): BackupTargetInput => ({
  name: "",
  endpoint: provider.endpoint,
  region: provider.region,
  bucket: "",
  prefix: "hexis/",
  accessKeyId: "",
  secretAccessKey: "",
  pathStyle: provider.pathStyle,
  enabled: true,
  master: false,
  shareInMaster: true,
});

const fromView = (t: BackupTargetView): BackupTargetInput => ({
  name: t.name, endpoint: t.endpoint, region: t.region, bucket: t.bucket, prefix: t.prefix, accessKeyId: t.accessKeyId,
  secretAccessKey: "", pathStyle: t.pathStyle, enabled: t.enabled, master: t.master, shareInMaster: t.shareInMaster,
});

export interface TargetFormProps {
  /** add: new target. edit: change a saved one. restore: credentials only, no target is saved here. */
  mode: "add" | "edit" | "restore";
  initial?: BackupTargetView;
  submitLabel: string;
  onSubmit: (input: BackupTargetInput) => Promise<void>;
  onCancel?: () => void;
  cancelLabel?: string;
  /** Extra controls rendered above the buttons (for example a name field in the restore flow). */
  children?: ReactNode;
  /** Make the first target the master by default (used by the create-vault step). */
  defaultMaster?: boolean;
}

function problems(v: BackupTargetInput, mode: TargetFormProps["mode"]): Partial<Record<keyof BackupTargetInput, string>> {
  const p: Partial<Record<keyof BackupTargetInput, string>> = {};
  if (mode !== "restore" && !v.name.trim()) p.name = "Give this backup a name.";
  if (v.endpoint !== null) {
    const e = v.endpoint.trim();
    if (!/^https?:\/\/[^\s/<>]+/i.test(e) || e.includes("<")) p.endpoint = "Enter the full endpoint address, starting with https://.";
  }
  if (!v.region.trim()) p.region = "Enter the region, or auto if your provider says so.";
  if (!v.bucket.trim()) p.bucket = "Enter the bucket name.";
  if (!v.accessKeyId.trim()) p.accessKeyId = "Enter the access key ID.";
  if ((mode !== "edit") && !v.secretAccessKey) p.secretAccessKey = "Enter the secret access key.";
  return p;
}

/** The S3-compatible target form, shared by Backup settings, the create-vault step and restore-from-backup. */
export function TargetForm({ mode, initial, submitLabel, onSubmit, onCancel, cancelLabel = "Cancel", children, defaultMaster = false }: TargetFormProps) {
  const [provider, setProvider] = useState<Provider>(() => guessProvider(initial));
  const [v, setV] = useState<BackupTargetInput>(() => (initial ? fromView(initial) : { ...blank(provider), master: defaultMaster }));
  const [showSecret, setShowSecret] = useState(false);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<BackupTestResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [touched, setTouched] = useState(false);
  const errs = useMemo(() => problems(v, mode), [v, mode]);
  const show = (k: keyof BackupTargetInput) => (touched ? errs[k] ?? null : null);
  const set = <K extends keyof BackupTargetInput>(k: K, val: BackupTargetInput[K]) => { setV((x) => ({ ...x, [k]: val })); setResult(null); };

  function pick(p: Provider) {
    setProvider(p);
    setV((x) => ({ ...x, endpoint: p.endpoint, region: x.region === provider.region || !x.region ? p.region : x.region, pathStyle: p.pathStyle }));
    setResult(null);
  }

  const toInput = (): BackupTargetInput => ({ ...v, name: v.name.trim() || "Restore", endpoint: v.endpoint === null ? null : v.endpoint.trim().replace(/\/+$/, ""), region: v.region.trim(), bucket: v.bucket.trim(), prefix: v.prefix.trim(), accessKeyId: v.accessKeyId.trim() });

  async function test() {
    setTouched(true);
    if (mode === "edit" && !v.secretAccessKey) return setError("To test from here, enter the secret access key again. Saved keys are never read back. “Back up now” tests the saved key.");
    if (Object.keys(errs).length) return;
    setTesting(true);
    setError(null);
    setResult(null);
    try {
      setResult(await be().testBackup(toInput()));
    } catch (e) {
      setError(e instanceof Error ? e.message : "The test could not run.");
    } finally {
      setTesting(false);
    }
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    setTouched(true);
    if (Object.keys(errs).length) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit(toInput());
    } catch (err) {
      setBusy(false);
      setError(err instanceof BackendError || err instanceof Error ? err.message : "Could not save this backup.");
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
      {error && <p role="alert" className="rounded-lg border border-ember/40 bg-danger/10 px-4 py-3 text-[0.95em]">{error}</p>}

      {mode !== "restore" && (
        <Field label="Name" error={show("name")} hint="Only for you, for example “Home NAS” or “Offsite”.">
          {({ id, describedBy, invalid }) => <TextInput id={id} data-autofocus value={v.name} invalid={invalid} aria-describedby={describedBy} onChange={(e) => set("name", e.target.value)} maxLength={60} placeholder="Offsite" />}
        </Field>
      )}

      <fieldset className="flex flex-col gap-1.5">
        <legend className="mb-1.5 text-[0.86em] font-medium text-mist">Provider</legend>
        <div className="grid grid-cols-2 gap-2">
          {PROVIDERS.map((p) => (
            <button key={p.id} type="button" aria-pressed={p.id === provider.id} onClick={() => pick(p)} className={`pressable min-h-10 rounded-sm border px-3 text-left text-[0.93em] font-medium ${p.id === provider.id ? "border-brass/60 bg-brass/15 text-brass-hi" : "border-hairline bg-raised text-mist hover:text-frost"}`}>
              {p.label}
            </button>
          ))}
        </div>
      </fieldset>

      {v.endpoint !== null && (
        <Field label="Endpoint" error={show("endpoint")} hint={provider.endpointHint}>
          {({ id, describedBy, invalid }) => <TextInput id={id} mono value={v.endpoint ?? ""} invalid={invalid} aria-describedby={describedBy} onChange={(e) => set("endpoint", e.target.value)} inputMode="url" />}
        </Field>
      )}

      <div className="grid gap-4 min-[480px]:grid-cols-2">
        <Field label="Bucket" error={show("bucket")}>
          {({ id, describedBy, invalid }) => <TextInput id={id} value={v.bucket} invalid={invalid} aria-describedby={describedBy} onChange={(e) => set("bucket", e.target.value)} placeholder="my-hexis-backups" />}
        </Field>
        <Field label="Region" error={show("region")}>
          {({ id, describedBy, invalid }) => <TextInput id={id} value={v.region} invalid={invalid} aria-describedby={describedBy} onChange={(e) => set("region", e.target.value)} />}
        </Field>
      </div>

      <Field label="Folder in the bucket" hint="Backups are kept under this prefix. Leave it as is unless you share the bucket.">
        {({ id, describedBy }) => <TextInput id={id} mono value={v.prefix} aria-describedby={describedBy} onChange={(e) => set("prefix", e.target.value)} />}
      </Field>

      <div className="grid gap-4 min-[480px]:grid-cols-2">
        <Field label="Access key ID" error={show("accessKeyId")}>
          {({ id, describedBy, invalid }) => <TextInput id={id} mono value={v.accessKeyId} invalid={invalid} aria-describedby={describedBy} onChange={(e) => set("accessKeyId", e.target.value)} />}
        </Field>
        <Field label="Secret access key" error={show("secretAccessKey")} hint={mode === "edit" ? "Saved. Leave empty to keep the current one." : undefined}>
          {({ id, describedBy, invalid }) => (
            <div className="relative">
              <TextInput id={id} mono type={showSecret ? "text" : "password"} autoComplete="off" value={v.secretAccessKey} invalid={invalid} aria-describedby={describedBy} onChange={(e) => set("secretAccessKey", e.target.value)} className="pr-11" />
              <IconButton label={showSecret ? "Hide the secret key" : "Show the secret key"} size="sm" className="absolute top-1/2 right-1 -translate-y-1/2" onClick={() => setShowSecret((s) => !s)}>
                {showSecret ? <EyeOff className="size-[17px]" /> : <Eye className="size-[17px]" />}
              </IconButton>
            </div>
          )}
        </Field>
      </div>

      <div className="flex min-h-11 items-center justify-between gap-4 rounded-lg bg-pane/70 px-4 py-2 shadow-e1">
        <div className="min-w-0">
          <p className="font-medium">Path-style addressing</p>
          <p className="text-[0.86em] text-mist">Needed by most S3-compatible servers. Turn off for Amazon S3.</p>
        </div>
        <Switch label="Path-style addressing" checked={v.pathStyle} onChange={(x) => set("pathStyle", x)} />
      </div>

      {mode !== "restore" && (
        <div className="flex flex-col gap-3 rounded-lg bg-pane/70 p-4 shadow-e1">
          <label className="flex cursor-pointer items-start gap-3 text-[0.95em]">
            <input type="checkbox" checked={v.shareInMaster} onChange={(e) => set("shareInMaster", e.target.checked)} className="mt-0.5 size-4.5 shrink-0 accent-[var(--color-brass)] pointer-coarse:size-5.5" />
            <span>
              <span className="block font-medium">Also keep these credentials in the master backup</span>
              <span className="block text-mist">The credentials are sealed inside your vault. When you restore from the master backup on a new device, this backup comes back already set up. You will not have to type its keys again.</span>
            </span>
          </label>
          <label className="flex cursor-pointer items-start gap-3 text-[0.95em]">
            <input type="checkbox" checked={v.master} onChange={(e) => set("master", e.target.checked)} className="mt-0.5 size-4.5 shrink-0 accent-[var(--color-brass)] pointer-coarse:size-5.5" />
            <span>
              <span className="block font-medium">Use this as the master backup</span>
              <span className="block text-mist">The master backup is the one you point a fresh install at. Only one backup is the master; choosing this moves the label here.</span>
            </span>
          </label>
          {mode === "edit" && (
            <div className="flex items-center justify-between gap-4 pt-1">
              <span className="font-medium">Back up to this target</span>
              <Switch label="Back up to this target" checked={v.enabled} onChange={(x) => set("enabled", x)} />
            </div>
          )}
        </div>
      )}

      {children}

      {result && (
        <p role="status" className={`flex items-start gap-2 text-[0.93em] ${result.ok ? "text-verdigris" : "text-ember"}`}>
          {result.ok ? <CircleCheck className="mt-0.5 size-4 shrink-0" aria-hidden /> : <CircleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />}
          <span>
            {result.message}
            {result.ok && result.conditionalWrites === "weak" && <span className="text-mist"> This provider cannot guarantee safe simultaneous writes, so use one device at a time.</span>}
          </span>
        </p>
      )}

      <div className="flex flex-wrap items-center justify-between gap-2 pt-1">
        <Button icon={<PlugZap className="size-4" />} busy={testing} disabled={busy} onClick={() => void test()}>Test connection</Button>
        <div className="flex gap-2">
          {onCancel && <Button variant="ghost" onClick={onCancel} disabled={busy}>{cancelLabel}</Button>}
          <Button type="submit" variant="key" busy={busy} disabled={testing}>{submitLabel}</Button>
        </div>
      </div>
    </form>
  );
}
