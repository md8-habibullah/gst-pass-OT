import { describe, expect, it } from "vitest";
import { previewBackend as be } from "./mock";

// The preview backend follows the core's rules for the device file (src-tauri/src/storage/device.rs).
describe("preview backend: theme, clipboard time and removing the app", () => {
  it("starts with the device's own colours and 30 seconds", async () => {
    const d = await be.deviceGet();
    expect(d.theme).toBe("system");
    expect(d.clipboardClear).toBe(30);
  });

  it("keeps a chosen theme and refuses a word that is not one", async () => {
    expect((await be.deviceSetTheme("dark")).theme).toBe("dark");
    expect((await be.deviceSetTheme("light")).theme).toBe("light");
    // @ts-expect-error a word the screen never sends
    await expect(be.deviceSetTheme("purple")).rejects.toMatchObject({ code: "invalid_input" });
    expect((await be.deviceGet()).theme).toBe("light");
  });

  it("keeps 15, 30 or 60 seconds and nothing else", async () => {
    expect((await be.deviceSetClipboardClear(60)).clipboardClear).toBe(60);
    // @ts-expect-error not one of the three choices
    await expect(be.deviceSetClipboardClear(20)).rejects.toMatchObject({ code: "invalid_input" });
    expect((await be.deviceGet()).clipboardClear).toBe(60);
  });

  it("removing the app needs the sum's answer, and then leaves the device settings fresh as well", async () => {
    await be.deviceSetTheme("dark");
    await expect(be.appPrepareUninstall("1")).rejects.toMatchObject({ code: "wrong_answer" });
    expect((await be.deviceGet()).theme).toBe("dark");
    const sum = await be.appResetChallenge();
    expect(sum.length).toBeGreaterThan(12);
    // the answer stays in the (mock) core: a guess does not remove anything
    await expect(be.appPrepareUninstall("0")).rejects.toMatchObject({ code: "wrong_answer" });
  });
});
