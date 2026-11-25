import { describe, expect, it } from "vitest";
import type { ReviewFinding, Severity } from "./types";
import { canonicalReviewFindings, nearestEligibleKey, projectReviewLanes, reviewFindingKey, transitionReviewFinding, transitionReviewGroupCounts } from "./reviewProjection";

const finding = (id: string, state: "unreviewed" | "reviewed" | "snoozed" = "unreviewed", severity: Severity = "HIGH"): ReviewFinding => ({ repoId: 1, repoName: "cvec", repoSourceType: "local", scanRunId: 1, scannedAt: null, reviewState: state, reviewedAt: null, snoozeUntil: null, reviewNote: null, knownExploited: false, ransomwareKnown: false, materialChange: null, priorityReasons: [], finding: { fingerprint: id, advisoryId: id, packageName: "pkg", packageVersion: "1", packageType: "npm", packageUrl: null, severity, fixVersion: null, title: id, sourceUrl: null, state: "open", intel: null, reviewState: state, reviewedAt: null, snoozeUntil: null, reviewNote: null } });

describe("Review projections", () => {
  it("deduplicates canonical records and projects exactly once", () => { const a = finding("a"); const b = finding("b", "reviewed"); const canonical = canonicalReviewFindings([a, a, b]); expect(canonical).toHaveLength(2); expect(projectReviewLanes(canonical, "reviewState").flatMap((lane) => lane.items).map(reviewFindingKey)).toEqual(["1:a", "1:b"]); });
  it("keeps deterministic empty lanes and mutability", () => { const lanes = projectReviewLanes([finding("a", "unreviewed", "CRITICAL")], "severity"); expect(lanes.map((lane) => [lane.id, lane.items.length, lane.mutable])).toEqual([["CRITICAL", 1, false], ["HIGH", 0, false], ["MEDIUM", 0, false]]); });
  it("includes low, negligible, and unknown findings in the board", () => {
    const items = [finding("a", "unreviewed", "LOW"), finding("b", "unreviewed", "NEGLIGIBLE"), finding("c", "unreviewed", "UNKNOWN")];
    const lanes = projectReviewLanes(items, "severity");
    expect(lanes.filter((lane) => lane.totalCount).map((lane) => lane.id)).toEqual(["LOW", "NEGLIGIBLE", "UNKNOWN"]);
  });
  it("keeps canonical totals distinct from the loaded page", () => {
    const counts = { reviewState: [{ id: "unreviewed", label: "Needs review", count: 374 }, { id: "reviewed", label: "Reviewed", count: 0 }, { id: "snoozed", label: "Snoozed", count: 0 }], severity: [], repository: [], fixAvailability: [] };
    const lanes = projectReviewLanes([finding("a")], "reviewState", counts);
    expect(lanes.map((lane) => [lane.id, lane.items.length, lane.totalCount])).toEqual([["unreviewed", 1, 374], ["reviewed", 0, 0], ["snoozed", 0, 0]]);
  });
  it("chooses the nearest eligible selection", () => { const a = finding("a"); const b = finding("b"); const c = finding("c"); expect(nearestEligibleKey([a, b, c], [a, c], "1:b")).toBe("1:c"); });
  it("projects every supported board grouping exactly once", () => {
    const items = [finding("a", "unreviewed", "CRITICAL"), finding("b", "reviewed", "HIGH"), finding("c", "snoozed", "MEDIUM")];
    for (const grouping of ["reviewState", "severity", "repository", "fixAvailability"] as const) {
      const projected = projectReviewLanes(items, grouping).flatMap((lane) => lane.items).map(reviewFindingKey);
      expect(projected.sort()).toEqual(items.map(reviewFindingKey).sort());
    }
  });
  it("only marks review-state lanes as mutable", () => {
    expect(projectReviewLanes([finding("a")], "reviewState").every((lane) => lane.mutable)).toBe(true);
    for (const grouping of ["severity", "repository", "fixAvailability"] as const) expect(projectReviewLanes([finding("a")], grouping).every((lane) => !lane.mutable)).toBe(true);
  });
  it("projects a review transition immediately without mutating the source record", () => {
    const source = finding("a");
    const reviewed = transitionReviewFinding(source, "reviewed", { reviewedAt: "2026-08-17T18:00:00Z" });
    const counts = { reviewState: [{ id: "unreviewed", label: "Needs review", count: 4 }, { id: "reviewed", label: "Reviewed", count: 2 }, { id: "snoozed", label: "Snoozed", count: 0 }], severity: [], repository: [], fixAvailability: [] };

    expect(source.reviewState).toBe("unreviewed");
    expect(reviewed.reviewState).toBe("reviewed");
    expect(reviewed.finding.reviewState).toBe("reviewed");
    expect(reviewed.reviewedAt).toBe("2026-08-17T18:00:00Z");
    expect(transitionReviewGroupCounts(counts, "unreviewed", "reviewed")?.reviewState.map((count) => count.count)).toEqual([3, 3, 0]);
  });

});
