import { describe, expect, it } from "vitest";
import type { BackupTargetView, HealthState, TargetHealth } from "../backend";
import { rowState, summarize } from "./backup";

const health = (state: HealthState, queued = 0, over: Partial<TargetHealth> = {}): TargetHealth => ({
  state, queued, lastSuccessAt: state === "untested" ? null : 1000, lastCheckedAt: state === "untested" ? null : 1000, lastError: null, conditionalWrites: "strong", ...over,
});
const target = (h: TargetHealth, over: Partial<BackupTargetView> = {}): BackupTargetView => ({
  id: "t1", name: "Bucket", endpoint: null, region: "auto", bucket: "b", prefix: "", accessKeyId: "k", hasSecret: true, pathStyle: false, enabled: true, master: true, shareInMaster: true, health: h, ...over,
});

describe("what a backup row says", () => {
  it("says 'Up to date' only when nothing is waiting", () => {
    expect(rowState(health("healthy", 0))).toBe("healthy");
  });

  it("says 'Backing up' while the first copy is still going up, even though the worker already recorded a success", () => {
    // A new target: the pull or the key check succeeded, 74 objects are still queued.
    expect(rowState(health("healthy", 74))).toBe("backing");
    expect(rowState(health("healthy", 1))).toBe("backing");
  });

  it("says 'Backing up' for a target that has not been reached yet but already has uploads queued", () => {
    expect(rowState(health("untested", 12))).toBe("backing");
    expect(rowState(health("untested", 0))).toBe("untested");
  });

  it("leaves the states that already explain a problem alone, queue or not", () => {
    expect(rowState(health("failing", 5))).toBe("failing");
    expect(rowState(health("lagging", 5))).toBe("lagging");
    expect(rowState(health("disabled", 5))).toBe("disabled");
  });

  it("agrees with the sidebar summary: a row that says 'Backing up' is never counted as backed up", () => {
    for (const [state, queued] of [["healthy", 3], ["untested", 3], ["healthy", 0], ["lagging", 2]] as const) {
      const h = health(state, queued);
      const reassuring = summarize([target(h)]).tone === "ok";
      expect(reassuring, `${state} with ${queued} waiting`).toBe(rowState(h) === "healthy");
    }
  });
});
