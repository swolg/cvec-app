import { describe, expect, it } from "vitest";

import { EMPTY_SNAPSHOT, OFFLINE_SNAPSHOT, READY_SNAPSHOT } from "./fixtures";

describe("frontend fixtures", () => {
  it("cover blocked, populated, and offline-compatible states", () => {
    expect(EMPTY_SNAPSHOT.workspaceStatus.scannerReady).toBe(false);
    expect(READY_SNAPSHOT.repos[0]?.openCritical).toBe(1);
    expect(OFFLINE_SNAPSHOT.externalIntelSummary.lastFetchedAt).not.toBeNull();
  });
});
