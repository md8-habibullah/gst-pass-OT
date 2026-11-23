import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";

interface KdfDefaults {
  mCostKiB: number;
  tCost: number;
  pCost: number;
}

interface SuiteInfo {
  version: number;
  kem: string;
  kdf: string;
  aead: string;
  kdfDefaults: KdfDefaults;
  nonceBytes: number;
  keyBytes: number;
}

function describeKdf(d: KdfDefaults): string {
  return `${d.mCostKiB / 1024} MiB memory · ${d.tCost} passes · ${d.pCost} lanes`;
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5 py-3 sm:flex-row sm:items-baseline sm:justify-between sm:gap-6">
      <dt className="text-sm text-slate-600 dark:text-slate-400">{label}</dt>
      <dd className="font-mono text-sm text-slate-900 dark:text-slate-100">{value}</dd>
    </div>
  );
}

export default function App() {
  const [info, setInfo] = useState<SuiteInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const desktopOrMobileShell = isTauri();

  useEffect(() => {
    if (!desktopOrMobileShell) return;
    invoke<SuiteInfo>("crypto_suite_info")
      .then(setInfo)
      .catch((e: unknown) => setError(String(e)));
  }, [desktopOrMobileShell]);

  return (
    <main className="min-h-dvh bg-slate-50 px-4 py-10 text-slate-900 dark:bg-slate-950 dark:text-slate-100">
      <div className="mx-auto max-w-xl">
        <header className="mb-8">
          <h1 className="text-2xl font-semibold tracking-tight">Passman</h1>
          <p className="mt-1 text-slate-600 dark:text-slate-400">
            Post-quantum, zero-knowledge vault. Local first, with optional direct S3 sync.
          </p>
        </header>

        <section
          aria-labelledby="suite-heading"
          className="rounded-lg border border-slate-200 bg-white px-5 py-2 dark:border-slate-800 dark:bg-slate-900"
        >
          <h2 id="suite-heading" className="pt-3 text-sm font-medium text-teal-700 dark:text-teal-400">
            Crypto core
          </h2>

          {info ? (
            <dl className="divide-y divide-slate-200 dark:divide-slate-800">
              <Row label="Suite version" value={`v${info.version}`} />
              <Row label="Key encapsulation" value={info.kem} />
              <Row label="Password KDF" value={info.kdf} />
              <Row label="Default KDF cost" value={describeKdf(info.kdfDefaults)} />
              <Row label="Encryption" value={info.aead} />
              <Row label="Key / nonce size" value={`${info.keyBytes * 8}-bit key · ${info.nonceBytes * 8}-bit random nonce`} />
            </dl>
          ) : (
            <p className="py-4 text-sm text-slate-600 dark:text-slate-400" role="status">
              {error
                ? `Could not read the crypto suite: ${error}`
                : desktopOrMobileShell
                  ? "Loading suite information…"
                  : "Open the Passman app to see the live crypto suite. This page is running in a plain browser."}
            </p>
          )}
        </section>

        <p className="mt-6 text-sm text-slate-600 dark:text-slate-400">
          Phase 1 ships the cryptographic core only. Vault screens arrive with the storage phase.
        </p>
      </div>
    </main>
  );
}
