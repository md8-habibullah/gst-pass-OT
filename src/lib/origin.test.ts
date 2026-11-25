import { describe, expect, it } from "vitest";
import { originParts } from "./origin";

describe("originParts", () => {
  it("shows the website on its own, without the scheme", () => {
    expect(originParts("https://example.com")).toEqual({ host: "example.com", port: null, secure: true, local: false });
  });

  it("keeps a port that is not the usual one", () => {
    expect(originParts("https://example.com:8443")).toMatchObject({ host: "example.com", port: "8443", secure: true });
    expect(originParts("https://example.com:443").port).toBeNull();
  });

  it("calls a page on localhost a page on this computer, and not secure unless https", () => {
    expect(originParts("http://localhost:3000")).toEqual({ host: "localhost", port: "3000", secure: false, local: true });
  });

  it("shows what it cannot read as it is, and never as secure or local", () => {
    expect(originParts("not an address")).toEqual({ host: "not an address", port: null, secure: false, local: false });
  });

  it("does not let a look-alike host pass as localhost", () => {
    expect(originParts("http://localhost.evil.example").local).toBe(false);
    expect(originParts("https://evil.example/@localhost").local).toBe(false);
  });
});
