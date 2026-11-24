import { describe, expect, it, vi } from "vitest";
import { announceNativeUi, nativeUiOpen, withNativeUi } from "./nativeUi";

describe("native screens", () => {
  it("is open while a native screen runs and for a moment after", async () => {
    expect(nativeUiOpen()).toBe(false);
    let during = false;
    await withNativeUi(async () => { during = nativeUiOpen(); });
    expect(during).toBe(true);
    expect(nativeUiOpen()).toBe(true); // the web view returns a beat later
  });

  it("closes again even when the native call fails", async () => {
    vi.useFakeTimers();
    try {
      await expect(withNativeUi(async () => { throw new Error("cancelled"); })).rejects.toThrow("cancelled");
      vi.advanceTimersByTime(3000);
      expect(nativeUiOpen()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("only announces the methods that open a native screen and keeps `this`", async () => {
    const seen: boolean[] = [];
    const backend = {
      tag: "b",
      fileAdd() { seen.push(nativeUiOpen()); return Promise.resolve(this.tag); },
      listItems() { seen.push(nativeUiOpen()); return Promise.resolve([] as string[]); },
    };
    const wrapped = announceNativeUi(backend);
    vi.useFakeTimers();
    try {
      vi.advanceTimersByTime(5000);
      expect(await wrapped.listItems()).toEqual([]);
      expect(seen).toEqual([false]);
      expect(await wrapped.fileAdd()).toBe("b");
      expect(seen).toEqual([false, true]);
    } finally {
      vi.useRealTimers();
    }
  });
});
