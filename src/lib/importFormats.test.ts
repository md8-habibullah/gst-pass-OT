import { describe, expect, it } from "vitest";
import { guessMapping } from "./importFormats";

describe("guessMapping", () => {
  it("maps the common export layouts", () => {
    expect(guessMapping(["name", "url", "username", "password"])).toMatchObject({ title: "name", url: "url", username: "username", password: "password" });
    expect(guessMapping(["Title", "Website", "Login", "Password", "OTP Auth", "Notes", "Folder"])).toMatchObject({ title: "Title", url: "Website", username: "Login", password: "Password", totp: "OTP Auth", notes: "Notes", folder: "Folder" });
  });
  it("does not use one column for two jobs", () => {
    const m = guessMapping(["Site Label", "Login ID", "Secret", "Link"]);
    expect(m).toMatchObject({ title: "Site Label", username: "Login ID", password: "Secret", url: "Link" });
  });
});
