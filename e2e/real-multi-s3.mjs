// Several buckets at once against REAL S3-compatible servers: two Versity S3 Gateway containers (v1.11.0, an open-source S3 server
// over a POSIX folder; loopback only, throw-away credentials), one holding the primary bucket and one holding two secondary
// buckets, so the secondaries differ from the primary in endpoint,
// credentials and bucket. Device A adds all three targets. Device B starts from nothing and is restored from the PRIMARY only;
// the secondary it needs must come back from the sealed settings stored in the primary. No browser: both devices are driven
// through the same IPC the app uses (`node e2e/real-multi-s3.mjs`, needs Docker).
//
// What this proves: a change reaches every enabled target; the primary bucket holds no plaintext and none of the secondary's
// endpoint, bucket or keys; restoring with the primary alone brings back the secondary that was marked to be kept in it (and
// not the one that was not); a secondary that fails does not stop the primary and recovers on the next trigger.
// What it does NOT prove: any real provider (Amazon S3, Cloudflare R2, Backblaze B2). Versity GW is one S3-compatible server.
// (MinIO would be the usual choice, but it no longer publishes its container images.)
import { spawnSync } from "node:child_process";
import { createHash, createHmac, randomBytes } from "node:crypto";
import { Core, SP, check, finish, freshDir, sleep } from "./lib.mjs";

const BIN = process.env.E2E_BIN;
// v1.11.0, pinned by digest so a later image cannot change what this test runs against.
const IMAGE = process.env.S3_TEST_IMAGE ?? "ghcr.io/versity/versitygw@sha256:8319a530d08d537426e2f855bae5280709795dc73a3f0c347973ef915d6343c5";
const rnd = () => randomBytes(14).toString("hex");
const SERVERS = {
  p: { name: "hexis-e2e-s3-p", port: 9360, user: "P" + rnd(), secret: rnd(), buckets: ["primary-a"] },
  q: { name: "hexis-e2e-s3-q", port: 9361, user: "Q" + rnd(), secret: rnd(), buckets: ["secondary-b", "secondary-c"] },
};
const NAME = "Marlowe", PW = "correct horse battery staple 7";
const A = new Core({ bin: BIN, data: freshDir("m-data-a"), port: 1447, args: [] });
const B = new Core({ bin: BIN, data: freshDir("m-data-b"), port: 1448, args: [] });

const sh = (cmd, args, opts = {}) => spawnSync(cmd, args, { encoding: opts.binary ? "buffer" : "utf8", maxBuffer: 64 << 20 });
const EMPTY_SHA = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const hmac = (key, data) => createHmac("sha256", key).update(data).digest();
/**
 * The three headers of an AWS Signature V4 request with an empty body, computed here instead of by `curl --aws-sigv4`:
 * curl 7.81 (Ubuntu 22.04, which is what the CI runners ship) leaves x-amz-content-sha256 out of the signed headers and the
 * server answers 403 to every request, while curl 8.x signs it. Signing it ourselves makes the test independent of the curl version.
 */
function sigv4Headers(server, method, path) {
  const [uri, query = ""] = path.split("?");
  const amzDate = new Date().toISOString().replace(/[:-]|\.\d{3}/g, ""); // 20261124T143000Z
  const day = amzDate.slice(0, 8);
  const scope = `${day}/us-east-1/s3/aws4_request`;
  const signed = "host;x-amz-content-sha256;x-amz-date";
  const canonicalQuery = query.split("&").filter(Boolean).map((p) => (p.includes("=") ? p : `${p}=`)).sort().join("&");
  const canonical = [method, uri, canonicalQuery, `host:127.0.0.1:${server.port}\nx-amz-content-sha256:${EMPTY_SHA}\nx-amz-date:${amzDate}\n`, signed, EMPTY_SHA].join("\n");
  const toSign = ["AWS4-HMAC-SHA256", amzDate, scope, createHash("sha256").update(canonical).digest("hex")].join("\n");
  const key = hmac(hmac(hmac(hmac(`AWS4${server.secret}`, day), "us-east-1"), "s3"), "aws4_request");
  return [
    `Authorization: AWS4-HMAC-SHA256 Credential=${server.user}/${scope}, SignedHeaders=${signed}, Signature=${hmac(key, toSign).toString("hex")}`,
    `x-amz-date: ${amzDate}`,
    `x-amz-content-sha256: ${EMPTY_SHA}`,
  ];
}
function s3(server, method, path, binary = false) {
  const r = sh("curl", ["-sS", "--max-time", "20", "-X", method, ...sigv4Headers(server, method, path).flatMap((h) => ["-H", h]),
    "-w", "\n%{http_code}", `http://127.0.0.1:${server.port}${path}`], { binary });
  const out = r.stdout ?? (binary ? Buffer.alloc(0) : "");
  const cut = out.lastIndexOf(binary ? 10 : "\n");
  return { code: Number((binary ? out.subarray(cut + 1) : out.slice(cut + 1)).toString()), body: binary ? out.subarray(0, cut) : out.slice(0, cut) };
}
const keysOf = (server, bucket) => [...s3(server, "GET", `/${bucket}?list-type=2&max-keys=1000`).body.matchAll(/<Key>([^<]+)<\/Key>/g)].map((m) => m[1].replace(/&amp;/g, "&"));
const itemKeys = (server, bucket) => keysOf(server, bucket).filter((k) => /\/items\/.+\.enc$/.test(k) || /\/items\/[^/]+$/.test(k));
const allBytes = (server, bucket) => keysOf(server, bucket).map((k) => s3(server, "GET", `/${bucket}/${k.split("/").map(encodeURIComponent).join("/")}`, true).body);

async function startServer(s) {
  sh("docker", ["rm", "-f", s.name]);
  const r = sh("docker", ["run", "-d", "--rm", "--name", s.name, "--memory", "300m", "--tmpfs", "/data:rw,mode=1777,size=200m", "-p", `127.0.0.1:${s.port}:7070`,
    "-e", `ROOT_ACCESS_KEY_ID=${s.user}`, "-e", `ROOT_SECRET_ACCESS_KEY=${s.secret}`, IMAGE, "--port", ":7070", "posix", "/data"]);
  if (r.status !== 0) throw new Error(`could not start ${IMAGE}: ${r.stderr}`);
  for (let i = 0; ; i++) {
    const probe = s3(s, "GET", "/"); // listing the buckets answers once the gateway is up
    if (probe.code === 200) break;
    if (i === 100) {
      const logs = sh("docker", ["logs", "--tail", "5", s.name]);
      throw new Error(`${s.name} did not become ready: last answer ${probe.code} ${String(probe.body).slice(0, 200)} | ${logs.stdout}${logs.stderr}`);
    }
    await sleep(300);
  }
  for (const b of s.buckets) if (s3(s, "PUT", `/${b}`).code !== 200) throw new Error(`could not create bucket ${b}`);
}
const stopServers = () => { for (const s of Object.values(SERVERS)) sh("docker", ["rm", "-f", s.name]); };

const ok = async (core, cmd, args = {}) => { const r = await core.ipc(cmd, args); if (r.err) throw new Error(`${cmd}: ${r.err.code} ${r.err.message}`); return r.ok; };
const target = (s, bucket, name, extra = {}) => ({ name, endpoint: `http://127.0.0.1:${s.port}`, region: "us-east-1", bucket, prefix: "vaults/ana", accessKeyId: s.user, secretAccessKey: s.secret, pathStyle: true, enabled: true, master: false, shareInMaster: true, ...extra });
const login = (title, username, password) => ({ type: "login", title, favorite: false, reprompt: false, tags: [], folder: null, fields: { username, password }, customFields: [] });
const titles = async (core) => (await ok(core, "item_list")).map((i) => i.title).sort();

/** Waits until every enabled target of a device is healthy with nothing queued. */
async function settled(core, label, ms = 40000) {
  const t0 = Date.now();
  let last = [];
  while (Date.now() - t0 < ms) {
    last = await ok(core, "backup_list");
    if (last.length && last.filter((t) => t.enabled).every((t) => t.health.state === "healthy" && t.health.queued === 0)) return last;
    await sleep(300);
  }
  check(`${label}: every target caught up`, false, JSON.stringify(last.map((t) => [t.name, t.health.state, t.health.queued, t.health.lastError])));
  return last;
}

let failed = false;
try {
  await startServer(SERVERS.p); await startServer(SERVERS.q);
  check("two S3-compatible servers are up, each with its own credentials", s3(SERVERS.p, "GET", "/primary-a?list-type=2").code === 200 && s3(SERVERS.q, "GET", "/secondary-b?list-type=2").code === 200);
  check("a server refuses the other one's credentials", s3({ ...SERVERS.p, port: SERVERS.q.port }, "GET", "/secondary-b?list-type=2").code === 403);

  await A.start();
  await ok(A, "profile_create", { displayName: NAME, password: PW });
  if (!(await ok(A, "vault_status")).unlocked) await ok(A, "vault_unlock", { password: PW });
  await ok(A, "item_create", { draft: login("GitHub", "marlowe-dev", "Tr0ub4dor&3-horse") });
  await ok(A, "item_create", { draft: login("Cloudflare", "ops@marlowe.example", "Zk9-plain-secret-77") });

  const probe = await ok(A, "backup_test", { input: target(SERVERS.q, "secondary-b", "probe") });
  check("the connection test answers for a secondary server", probe.ok === true, JSON.stringify(probe));

  const p = await ok(A, "backup_add", { input: target(SERVERS.p, "primary-a", "Primary", { master: true }) });
  const b = await ok(A, "backup_add", { input: target(SERVERS.q, "secondary-b", "Secondary B", { shareInMaster: true }) });
  const c = await ok(A, "backup_add", { input: target(SERVERS.q, "secondary-c", "Secondary C", { shareInMaster: false }) });
  check("one primary and two secondaries on device A", p.master === true && b.master === false && c.master === false, JSON.stringify([p.master, b.master, c.master]));
  let list = await settled(A, "A");
  check("all three targets are healthy with an empty queue", list.length === 3 && list.every((t) => t.health.state === "healthy" && t.health.queued === 0), JSON.stringify(list.map((t) => [t.name, t.health.state])));
  check("no target view carries a secret key", !JSON.stringify(list).includes(SERVERS.p.secret) && !JSON.stringify(list).includes(SERVERS.q.secret));

  // every bucket received the vault
  const kP = itemKeys(SERVERS.p, "primary-a"), kB = itemKeys(SERVERS.q, "secondary-b"), kC = itemKeys(SERVERS.q, "secondary-c");
  check("the primary bucket holds the encrypted items", kP.length >= 2, `${kP.length} item objects: ${kP.slice(0, 3)}`);
  check("secondary B holds the same item objects as the primary", kB.length === kP.length && kB.every((k) => kP.includes(k)), `${kB.length} vs ${kP.length}`);
  check("secondary C holds the same item objects as the primary", kC.length === kP.length && kC.every((k) => kP.includes(k)), `${kC.length} vs ${kP.length}`);

  // ciphertext only, and the secondaries' settings are not readable in the primary bucket
  const primaryBytes = Buffer.concat(allBytes(SERVERS.p, "primary-a"));
  const needles = ["Tr0ub4dor", "marlowe-dev", "Zk9-plain-secret", "Cloudflare", SERVERS.q.user, SERVERS.q.secret, "secondary-b", "secondary-c", `127.0.0.1:${SERVERS.q.port}`];
  const leaked = needles.filter((n) => primaryBytes.includes(Buffer.from(n)));
  check("the primary bucket has no plaintext and nothing of the secondaries' settings", primaryBytes.length > 1000 && leaked.length === 0, `leaked: ${leaked}`);
  const secondaryBytes = Buffer.concat([...allBytes(SERVERS.q, "secondary-b"), ...allBytes(SERVERS.q, "secondary-c")]);
  const leaked2 = ["Tr0ub4dor", "marlowe-dev", "Zk9-plain-secret", SERVERS.p.user, SERVERS.p.secret].filter((n) => secondaryBytes.includes(Buffer.from(n)));
  check("the secondary buckets are ciphertext too and know nothing of the primary", secondaryBytes.length > 1000 && leaked2.length === 0, `leaked: ${leaked2}`);

  // device B: restored from the primary alone
  await B.start();
  await ok(B, "profile_restore_from_backup", { displayName: "Device B", input: target(SERVERS.p, "primary-a", "Primary", { master: true }) });
  await ok(B, "vault_unlock", { password: PW });
  check("device B has both items after restoring from the primary only", (await titles(B)).join("|") === "Cloudflare|GitHub", (await titles(B)).join("|"));
  list = await ok(B, "backup_list");
  const names = list.map((t) => t.name).sort().join("|");
  check("the secondary marked to be kept in the primary came back with it", names.includes("Secondary B"), names);
  check("the secondary that was not marked did not come back", !names.includes("Secondary C"), names);
  const bB = list.find((t) => t.name === "Secondary B");
  check("the restored secondary is complete (endpoint, bucket, key id, a stored secret) but shows no secret", bB && bB.endpoint === `http://127.0.0.1:${SERVERS.q.port}` && bB.bucket === "secondary-b" && bB.accessKeyId === SERVERS.q.user && bB.hasSecret === true && !JSON.stringify(list).includes(SERVERS.q.secret), JSON.stringify(bB));
  check("it is a secondary on B as well: only one primary", list.filter((t) => t.master).length === 1 && list.find((t) => t.master)?.name === "Primary");

  // a change on B reaches the restored secondary and the primary; A (which also has C) pulls it from the primary
  await ok(B, "item_create", { draft: login("Added on B", "from-b", "pw-from-b-1") });
  await settled(B, "B");
  const after = (s, bucket) => itemKeys(s, bucket).length;
  check("the item added on B is in the primary bucket", after(SERVERS.p, "primary-a") === kP.length + 1);
  check("and in the restored secondary (its stored secret really works)", after(SERVERS.q, "secondary-b") === kP.length + 1, String(after(SERVERS.q, "secondary-b")));
  check("but not in secondary C, which B does not know", after(SERVERS.q, "secondary-c") === kP.length);
  await ok(A, "backup_run", {});
  for (let i = 0; i < 40 && !(await titles(A)).includes("Added on B"); i++) await sleep(300);
  check("device A pulls it from the primary", (await titles(A)).includes("Added on B"), (await titles(A)).join("|"));
  await settled(A, "A (after the pull)");
  check("and A passes it on to secondary C, which only A knows", after(SERVERS.q, "secondary-c") === kP.length + 1, String(after(SERVERS.q, "secondary-c")));

  // a secondary that fails does not stop the primary, and recovers
  const sb = (await ok(A, "backup_list")).find((t) => t.name === "Secondary B");
  await ok(A, "backup_update", { id: sb.id, input: { ...target(SERVERS.q, "no-such-bucket", "Secondary B"), secretAccessKey: "" } });
  await ok(A, "item_create", { draft: login("While B is down", "u", "pw-while-down") });
  let st;
  for (let i = 0; i < 60; i++) {
    st = await ok(A, "backup_list");
    const pr = st.find((t) => t.name === "Primary"), sec = st.find((t) => t.name === "Secondary B");
    if (pr.health.state === "healthy" && pr.health.queued === 0 && sec.health.state !== "healthy") break;
    await sleep(300);
  }
  const prim = st.find((t) => t.name === "Primary"), sec = st.find((t) => t.name === "Secondary B");
  check("the primary is up to date while a secondary is failing", prim.health.state === "healthy" && prim.health.queued === 0, JSON.stringify(prim.health));
  check("the failing secondary says so, with a reason and its queue", sec.health.state !== "healthy" && sec.health.queued >= 1 && !!sec.health.lastError, JSON.stringify(sec.health));
  check("the new item is in the primary bucket regardless", after(SERVERS.p, "primary-a") === kP.length + 2);
  await ok(A, "backup_update", { id: sb.id, input: { ...target(SERVERS.q, "secondary-b", "Secondary B"), secretAccessKey: "" } });
  await ok(A, "backup_run", { id: sb.id });
  await settled(A, "A (secondary back)");
  check("the secondary catches up on the next trigger and gets the item", after(SERVERS.q, "secondary-b") === kP.length + 2, String(after(SERVERS.q, "secondary-b")));
} catch (e) {
  failed = true;
  check(`script error: ${e.message}`, false, e.stack);
} finally {
  await A.stop(); await B.stop(); stopServers();
}
finish(failed ? "real-multi-s3 (stopped early)" : "real-multi-s3");
