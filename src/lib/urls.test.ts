import { describe, expect, it } from "vitest";
import { firstUrl, joinUrls, MAX_URLS, openable, shortUrl, urlsOf } from "./urls";

describe("urls", () => {
  it("splits on lines, trims, and drops blanks and repeats", () => {
    expect(urlsOf(" https://a.example \n\nhttps://b.example\r\nhttps://a.example ")).toEqual(["https://a.example", "https://b.example"]);
    expect(urlsOf("")).toEqual([]);
    expect(urlsOf(null)).toEqual([]);
    expect(urlsOf(undefined)).toEqual([]);
  });
  it("joins them the way the core stores them", () => {
    expect(joinUrls(["https://a.example", " ", "https://b.example", "https://a.example"])).toBe("https://a.example\nhttps://b.example");
  });
  it("takes the first address for a plain open", () => {
    expect(firstUrl("https://a.example\nhttps://b.example")).toBe("https://a.example");
    expect(firstUrl("")).toBe("");
  });
  it("adds a scheme to a bare address and leaves a full one alone", () => {
    expect(openable("example.com/login")).toBe("https://example.com/login");
    expect(openable("http://example.com")).toBe("http://example.com");
    expect(openable("ftp://files.example")).toBe("ftp://files.example");
  });
  it("shortens an address for a list", () => {
    expect(shortUrl("https://www.example.com/")).toBe("example.com");
    expect(shortUrl("http://login.example.com/a?b")).toBe("login.example.com/a?b");
  });
  it("matches the core's limit", () => {
    expect(MAX_URLS).toBe(20);
  });
});
