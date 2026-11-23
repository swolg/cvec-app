import axe from "axe-core";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { NeedsAttention } from "./DashboardOverview";
import { READY_SNAPSHOT } from "@/test/fixtures";
import type { DashboardDataContext } from "./types";
import { CRITICAL_TO_REVIEW_VIEW, OTHER_TO_REVIEW_VIEW } from "@/lib/reviewViews";

function context(onOpenReview = vi.fn()): DashboardDataContext {
  return { snapshot: { ...READY_SNAPSHOT, reviewSummary: { needsReview: 4, unreviewedCritical: 1, unreviewedOther: 3, dueSnoozed: 1, openReviewed: 2, snoozed: 1, recentlyResolved: 1, failedScans: 1 } }, riskTrend: { inventoryRevision: 0, day: "2026-08-12", hasCompletedScan: true, read: async () => [] }, riskTrendRange: 7, externalIntelItems: [], isScanning: false, selectedRepoId: null, onAddGithubRepository: () => {}, onChooseFolder: () => {}, onRangeChange: () => {}, onScanAll: () => {}, onSelectRepo: () => {}, onOpenReview, onOpenFailedScans: () => {} };
}

describe("NeedsAttention", () => {
  it("uses canonical counts and sends exact review view destinations accessibly", async () => {
    const onOpenReview = vi.fn();
    const { container } = render(<NeedsAttention context={context(onOpenReview)} />);
    fireEvent.click(screen.getByRole("button", { name: /1 critical finding$/ }));
    expect(onOpenReview).toHaveBeenCalledWith(CRITICAL_TO_REVIEW_VIEW);
    fireEvent.click(screen.getByRole("button", { name: /3 other findings$/ }));
    expect(onOpenReview).toHaveBeenCalledWith(OTHER_TO_REVIEW_VIEW);
    fireEvent.click(screen.getByRole("button", { name: "Review queue" }));
    expect(onOpenReview).toHaveBeenCalledWith("needs-review");
    const results = await axe.run(container, { rules: { "color-contrast": { enabled: false } } });
    expect(results.violations.filter((violation) => ["critical", "serious"].includes(violation.impact ?? ""))).toEqual([]);
  });
});
