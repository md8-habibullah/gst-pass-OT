import { describe, expect, it } from "vitest";
import { cleanBase32, describeOtp, parseOtpInput, toBase32 } from "./otp";

// ---- tiny protobuf writer to build Google Authenticator export payloads ----
const varint = (n: number): number[] => {
  const out: number[] = [];
  let v = n;
  while (v >= 0x80) { out.push((v & 0x7f) | 0x80); v = Math.floor(v / 128); }
  out.push(v);
  return out;
};
const field = (num: number, wire: number, body: number[]) => [...varint((num << 3) | wire), ...body];
const lenField = (num: number, bytes: number[]) => field(num, 2, [...varint(bytes.length), ...bytes]);
const str = (s: string) => [...new TextEncoder().encode(s)];
function otpParams(o: { secret: number[]; name: string; issuer?: string; alg?: number; digits?: number; type?: number }) {
  return [
    ...lenField(1, o.secret),
    ...lenField(2, str(o.name)),
    ...(o.issuer ? lenField(3, str(o.issuer)) : []),
    ...field(4, 0, varint(o.alg ?? 1)),
    ...field(5, 0, varint(o.digits ?? 1)),
    ...field(6, 0, varint(o.type ?? 2)),
  ];
}
function migrationLink(entries: Parameters<typeof otpParams>[0][]): string {
  const payload = [...entries.flatMap((e) => lenField(1, otpParams(e))), ...field(2, 0, varint(1)), ...field(3, 0, varint(1)), ...field(4, 0, varint(0))];
  const b64 = btoa(String.fromCharCode(...payload));
  return `otpauth-migration://offline?data=${encodeURIComponent(b64)}`;
}

const SECRET = [0x48, 0x65, 0x6c, 0x6c, 0x6f, 0x21, 0xde, 0xad, 0xbe, 0xef]; // "Hello!" + 4 bytes = 10 bytes

describe("cleanBase32 / toBase32", () => {
  it("normalizes copy-paste noise", () => {
    expect(cleanBase32("jbsw y3dp-ehpk 3pxp==")).toBe("JBSWY3DPEHPK3PXP");
    expect(cleanBase32("not base32!")).toBeNull();
    expect(cleanBase32("   ")).toBeNull();
  });
  it("encodes RFC 4648 vectors", () => {
    const enc = (s: string) => toBase32(new TextEncoder().encode(s));
    expect(enc("f")).toBe("MY");
    expect(enc("fo")).toBe("MZXQ");
    expect(enc("foobar")).toBe("MZXW6YTBOI");
  });
});

describe("parseOtpInput: setup keys", () => {
  it("accepts a bare key with spaces and lower case", () => {
    const r = parseOtpInput("jbsw y3dp ehpk 3pxp");
    expect(r.kind).toBe("ok");
    if (r.kind === "ok") {
      expect(r.account.value).toBe("JBSWY3DPEHPK3PXP");
      expect(r.account.origin).toBe("key");
      expect(r.account.shortKey).toBe(false);
    }
  });
  it("strips labels people copy along", () => {
    for (const s of ["secret=JBSWY3DPEHPK3PXP", "Key: JBSW-Y3DP-EHPK-3PXP", '"JBSWY3DPEHPK3PXP"']) {
      const r = parseOtpInput(s);
      expect(r.kind === "ok" && r.account.value).toBe("JBSWY3DPEHPK3PXP");
    }
  });
  it("flags short keys but still accepts them", () => {
    const r = parseOtpInput("JBSWY3DPEH");
    expect(r.kind === "ok" && r.account.shortKey).toBe(true);
  });
  it("rejects nonsense with a reason", () => {
    expect(parseOtpInput("")).toEqual({ kind: "empty" });
    expect(parseOtpInput("hello world 1 0 8")).toMatchObject({ kind: "error" });
    expect(parseOtpInput("ABC")).toMatchObject({ kind: "error" });
  });
});

describe("parseOtpInput: links", () => {
  it("reads an otpauth link", () => {
    const r = parseOtpInput("otpauth://totp/GitHub:marlowe-dev?secret=JBSWY3DPEHPK3PXP&issuer=GitHub&digits=8&period=60&algorithm=SHA256");
    expect(r.kind).toBe("ok");
    if (r.kind === "ok") {
      expect(r.account).toMatchObject({ issuer: "GitHub", account: "marlowe-dev", digits: 8, period: 60, algorithm: "SHA256", origin: "link" });
      expect(describeOtp(r.account)).toBe("GitHub, marlowe-dev (8 digits, every 60 s, SHA256)");
    }
  });
  it("is case-insensitive and tolerates encoded labels and a missing issuer parameter", () => {
    const r = parseOtpInput("OTPAUTH://TOTP/Acme%20Corp%3Aalice%40example.com?secret=jbswy3dpehpk3pxp");
    expect(r.kind === "ok" && r.account.issuer).toBe("Acme Corp");
    expect(r.kind === "ok" && r.account.account).toBe("alice@example.com");
  });
  it("explains HOTP, Steam and broken links", () => {
    expect(parseOtpInput("otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP&counter=1")).toMatchObject({ kind: "error", message: expect.stringContaining("HOTP") });
    expect(parseOtpInput("steam://JBSWY3DPEHPK3PXP")).toMatchObject({ kind: "error", message: expect.stringContaining("Steam") });
    expect(parseOtpInput("otpauth://totp/x?issuer=a")).toMatchObject({ kind: "error", message: expect.stringContaining("no secret") });
    expect(parseOtpInput("otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=12")).toMatchObject({ kind: "error" });
    expect(parseOtpInput("otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&algorithm=MD5")).toMatchObject({ kind: "error" });
  });
  it("pulls the key out of a web link", () => {
    const r = parseOtpInput("https://www.example.com/account/2fa?secret=jbsw y3dp ehpk 3pxp&issuer=Example");
    expect(r.kind).toBe("ok");
    if (r.kind === "ok") expect(r.account).toMatchObject({ value: "JBSWY3DPEHPK3PXP", issuer: "Example", origin: "web-link" });
    expect(parseOtpInput("https://example.com/nothing")).toMatchObject({ kind: "error" });
  });
});

describe("parseOtpInput: Google Authenticator export", () => {
  it("splits an export into accounts with correct keys and parameters", () => {
    const link = migrationLink([
      { secret: SECRET, name: "alice@example.com", issuer: "Example" },
      { secret: SECRET, name: "bob", issuer: "Other", alg: 2, digits: 2 },
      { secret: SECRET, name: "counter", type: 1 },
    ]);
    const r = parseOtpInput(link);
    expect(r.kind).toBe("export");
    if (r.kind !== "export") return;
    expect(r.skipped).toBe(1);
    expect(r.accounts).toHaveLength(2);
    expect(r.accounts[0]).toMatchObject({ issuer: "Example", account: "alice@example.com", digits: 6, algorithm: "SHA1", origin: "export" });
    expect(r.accounts[1]).toMatchObject({ issuer: "Other", account: "bob", digits: 8, algorithm: "SHA256" });
    // every produced value is itself a valid link for the single-account parser, with the same key
    const again = parseOtpInput(r.accounts[0]!.value);
    expect(again.kind === "ok" && new URL(again.account.value).searchParams.get("secret")).toBe(toBase32(Uint8Array.from(SECRET)));
  });
  it("reports damaged exports", () => {
    expect(parseOtpInput("otpauth-migration://offline?data=%%%")).toMatchObject({ kind: "error" });
    expect(parseOtpInput("otpauth-migration://offline")).toMatchObject({ kind: "error" });
    expect(parseOtpInput(migrationLink([{ secret: SECRET, name: "c", type: 1 }]))).toMatchObject({ kind: "error", message: expect.stringContaining("counter-based") });
  });
});
