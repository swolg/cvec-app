import { describe, expect, it } from "vitest";

import type { ReviewFinding } from "@/lib/types";
import { adjacentReviewKey } from "./CvecReviewTable";

function finding(id: string): ReviewFinding {
  return {
    repoId: 1,
    repoName: "cvec",
    repoSourceType: "local",
    scanRunId: 1,
    scannedAt: null,
    reviewState: "unreviewed",
    reviewedAt: null,
    snoozeUntil: null,
    reviewNote: null,
    knownExploited: false,
    ransomwareKnown: false,
    materialChange: null,
    priorityReasons: [],
    finding: {
      fingerprint: id,
      advisoryId: id,
      packageName: "pkg",
      packageVersion: "1.0.0",
      packageType: "npm",
      packageUrl: null,
      severity: "HIGH",
      fixVersion: null,
      title: id,
      sourceUrl: null,
      state: "open",
      intel: null,
      reviewState: "unreviewed",
      reviewedAt: null,
      snoozeUntil: null,
      reviewNote: null,
    },
  };
}

describe("adjacentReviewKey", () => {
  it("moves through the visible sorted or grouped row order", () => {
    const displayedItems = [finding("critical"), finding("high"), finding("medium")];

    expect(adjacentReviewKey(displayedItems, "1:critical", 1)).toBe("1:high");
    expect(adjacentReviewKey(displayedItems, "1:medium", -1)).toBe("1:high");
  });
});
