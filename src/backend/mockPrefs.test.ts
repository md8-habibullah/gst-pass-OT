import { beforeAll, describe, expect, it } from "vitest";
import { previewBackend as be, PREVIEW_PASSWORD, seedPreview } from "./mock";
import type { GeneratorPrefs } from "./types";

const gen = (over: Partial<GeneratorPrefs> = {}): GeneratorPrefs => ({ length: 30, lower: true, upper: true, digits: true, symbols: false, avoidAmbiguous: true, ...over });

// The preview backend follows the core's rules for the generator settings (src-tauri/src/storage/backup/prefs.rs).
describe("preview backend: generator settings", () => {
  let first = "";
  let second = "";
  beforeAll(async () => {
    await seedPreview();
    const list = await be.listProfiles();
    first = list.profiles[0]!.id;
    second = list.profiles[1]!.id;
  });

  it("needs an open vault", async () => {
    await be.selectProfile(first);
    await expect(be.prefsGet()).rejects.toMatchObject({ code: "locked" });
  });

  it("starts with nothing saved, then keeps what was saved, per account", async () => {
    await be.selectProfile(first);
    await be.unlock(PREVIEW_PASSWORD);
    expect((await be.prefsGet()).generator).toBeNull();
    expect((await be.prefsSetGenerator(gen())).generator).toEqual(gen());
    expect((await be.prefsGet()).generator).toEqual(gen());
    await be.lock();
    await be.unlock(PREVIEW_PASSWORD);
    expect((await be.prefsGet()).generator?.length).toBe(30);

    await be.lock();
    await be.selectProfile(second);
    await be.unlock(PREVIEW_PASSWORD);
    expect((await be.prefsGet()).generator).toBeNull();
  });

  it("refuses settings the generator cannot use and keeps the previous ones", async () => {
    await be.selectProfile(first);
    await be.unlock(PREVIEW_PASSWORD);
    for (const bad of [gen({ length: 3 }), gen({ length: 129 }), gen({ length: 20.5 }), gen({ lower: false, upper: false, digits: false, symbols: false })]) {
      await expect(be.prefsSetGenerator(bad)).rejects.toMatchObject({ code: "invalid_input" });
    }
    expect((await be.prefsGet()).generator).toEqual(gen());
    await be.prefsSetGenerator(gen({ length: 4 }));
    await be.prefsSetGenerator(gen({ length: 128 }));
    expect((await be.prefsGet()).generator?.length).toBe(128);
  });
});
