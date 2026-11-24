import { describe, expect, it } from "vitest";
import { FIELD_SCHEMA } from "../../backend/types";
import { parseCsvRows } from "./csv";
import { ImportError, parseImport } from "./index";

const bw = (items: unknown[], extra: object = {}) => JSON.stringify({ encrypted: false, folders: [{ id: "f1", name: "Work" }], items, ...extra });

describe("Bitwarden JSON", () => {
  const login = { type: 1, name: "GitHub", favorite: true, folderId: "f1", notes: "n", login: { username: "u", password: "p w", uris: [{ uri: "https://github.com" }, { uri: "https://gist.github.com" }], totp: "jbsw y3dp ehpk 3pxp" }, fields: [{ name: "pin", value: "1234", type: 1 }, { name: "ok", value: "true", type: 2 }, { name: "t", value: "x", type: 0 }] };
  const card = { type: 3, name: "Visa", card: { cardholderName: "A B", brand: "Visa", number: "4111111111111111", expMonth: "4", expYear: "2030", code: "123" } };
  const ident = { type: 4, name: "Me", identity: { title: "Dr", firstName: "A", lastName: "B", address1: "1 Rd", address2: "x", address3: "y", ssn: "123", passportNumber: "P1", licenseNumber: "L1", username: "ab" } };
  const ssh = { type: 5, name: "box", sshKey: { privateKey: "-----BEGIN OPENSSH PRIVATE KEY-----", publicKey: "ssh-ed25519 AAA", keyFingerprint: "SHA256:abc" } };

  it("maps every type onto fields that exist in the schema", () => {
    const r = parseImport(bw([login, { type: 2, name: "Note", notes: "body" }, card, ident, ssh]));
    expect(r.format).toBe("Bitwarden JSON");
    expect(r.drafts.map((d) => d.type)).toEqual(["login", "secureNote", "card", "identity", "sshKey"]);
    for (const d of r.drafts) for (const k of Object.keys(d.fields)) expect(FIELD_SCHEMA[d.type].some((s) => s.key === k)).toBe(true);
  });
  it("keeps login details, extra links, custom fields, folder and favorite", () => {
    const d = parseImport(bw([login])).drafts[0]!;
    expect(d.fields).toMatchObject({ username: "u", password: "p w", url: "https://github.com", totp: "JBSWY3DPEHPK3PXP", notes: "n" });
    expect(d.folder).toBe("Work");
    expect(d.favorite).toBe(true);
    expect(d.customFields).toEqual([
      { name: "Website 2", value: "https://gist.github.com", hidden: false },
      { name: "pin", value: "1234", hidden: true },
      { name: "ok", value: "true", hidden: false },
      { name: "t", value: "x", hidden: false },
    ]);
  });
  it("maps cards and identities", () => {
    const [c, i] = parseImport(bw([card, ident])).drafts;
    expect(c!.fields).toMatchObject({ cardholder: "A B", number: "4111111111111111", expMonth: "4", code: "123" });
    expect(i!.fields).toMatchObject({ salutation: "Dr", address2: "x, y", nationalId: "123", passport: "P1", license: "L1" });
    expect(i!.customFields).toEqual([{ name: "Username", value: "ab", hidden: false }]);
  });
  it("never loses an unreadable one-time code", () => {
    const r = parseImport(bw([{ type: 1, name: "X", login: { username: "u", totp: "steam://ABCDEFGH" } }]));
    expect(r.drafts[0]!.fields.totp).toBeUndefined();
    expect(r.drafts[0]!.customFields[0]).toMatchObject({ name: "One-time code (not imported)", value: "steam://ABCDEFGH", hidden: true });
    expect(r.warnings).toHaveLength(1);
  });
  it("skips Bitwarden's trash and unknown types, and says so", () => {
    const r = parseImport(bw([{ type: 1, name: "Old", deletedDate: "2024-01-01", login: {} }, { type: 99, name: "Weird" }, login]));
    expect(r.drafts).toHaveLength(1);
    expect(r.skipped.map((s) => s.name)).toEqual(["Old", "Weird"]);
  });
  it("refuses encrypted and foreign files with a clear message", () => {
    expect(() => parseImport(bw([], { encrypted: true }))).toThrow(/password-protected/);
    expect(() => parseImport('{"items": "no"}')).toThrow(ImportError);
    expect(() => parseImport("")).toThrow(/empty/);
    expect(() => parseImport("hello")).toThrow(/Bitwarden JSON/);
  });
});

describe("CSV", () => {
  it("parses quotes, doubled quotes, CRLF and multi-line cells", () => {
    expect(parseCsvRows('a,b\r\n"x, ""y""","line1\nline2"\r\n')).toEqual([["a", "b"], ['x, "y"', "line1\nline2"]]);
    expect(() => parseCsvRows('a,"b')).toThrow(/unclosed quote/);
  });
  it("imports a Chrome export", () => {
    const csv = "name,url,username,password,note\nGitHub,https://github.com/login,me,\"p,w\",\"two\nlines\"\n,https://news.example.com,reader,secret,\n";
    const r = parseImport(csv);
    expect(r.format).toBe("CSV");
    expect(r.drafts).toHaveLength(2);
    expect(r.drafts[0]).toMatchObject({ title: "GitHub", type: "login", fields: { username: "me", password: "p,w", url: "https://github.com/login", notes: "two\nlines" } });
    expect(r.drafts[1]!.title).toBe("news.example.com");
  });
  it("understands Bitwarden-style and 1Password-style headers, with totp and folders", () => {
    const r = parseImport("folder,favorite,name,login_uri,login_username,login_password,login_totp\nWork,1,Box,https://b.example,u,p,JBSWY3DPEHPK3PXP\n");
    expect(r.drafts[0]).toMatchObject({ folder: "Work", favorite: true, fields: { totp: "JBSWY3DPEHPK3PXP" } });
  });
  it("skips empty rows and rejects files without a password column", () => {
    expect(parseImport("name,username,password\n,,\nx,y,z\n").drafts).toHaveLength(1);
    expect(() => parseImport("name,username\nx,y\n")).toThrow(ImportError);
  });
});
