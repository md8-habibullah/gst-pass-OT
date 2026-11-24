import { app, be } from "../../state/app";
import { useBrandName } from "../../state/brand";
import { useStore } from "../../state/store";
import { VaultMark } from "../lock/VaultMark";
import { Group, Row } from "./parts";

export function AboutPage() {
  const name = useBrandName();
  const suite = useStore(app, (s) => s.suite);
  const device = useStore(app, (s) => s.device);
  const preview = import.meta.env.DEV && be().kind === "preview";
  return (
    <Group title="About" heading={false}>
      <div className="flex items-center gap-4 border-b border-hairline px-4 py-4">
        <VaultMark size={44} />
        <div className="min-w-0">
          <p className="font-semibold">{name}</p>
          <p className="mt-0.5 text-[0.93em] text-mist">Version {typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "dev"}. Your vaults stay on this device unless you set up a backup.</p>
        </div>
      </div>
      {suite && (
        <>
          <Row title="Key protection" description={`${suite.kem}, so a future quantum computer cannot unwrap your keys from stored data.`} />
          <Row title="Password hashing" description={`${suite.kdf}, ${suite.kdfDefaults.mCostKiB / 1024} MiB of memory, ${suite.kdfDefaults.tCost} passes, ${suite.kdfDefaults.pCost} lanes.`} />
          <Row title="Encryption" description={`${suite.aead} with ${suite.keyBytes * 8}-bit keys and a fresh ${suite.nonceBytes * 8}-bit random nonce for every object.`} />
          <Row title="Format" description={`Version ${suite.version}. Every item and every version is a separate authenticated envelope.`} />
        </>
      )}
      <Row title="Time" description={`Every time is stored in UTC and shown in ${device?.timeZone?.replace(/_/g, " ") ?? "your zone"}. The real time is checked online${device?.timeCheck === false ? ", but that check is off now" : ""}.`} />
      {preview && <Row title="Preview backend" description="This build is talking to an in-memory demo, not the encrypted Rust core. Nothing here is secure." />}
    </Group>
  );
}
