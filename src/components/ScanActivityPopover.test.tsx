import axe from "axe-core";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ScanActivityPopover } from "./ScanActivityPopover";
import type { ScanActivity } from "@/lib/types";

const activity: ScanActivity = {
  sequence: 1,
  id: "scan-1", status: "partial_failure", stage: "completed", scopeRepoId: null,
  startedAt: "2026-08-12T08:00:00Z", finishedAt: "2026-08-12T08:02:00Z",
  totalRepositories: 2, completedRepositories: 1, failedRepositories: 1,
  currentRepoId: 2, currentRepoName: "worker",
  outcomes: [
    { repoId: 1, repoName: "cvec", scanRunId: 10, succeeded: true, error: null, newFindings: 1, resolvedFindings: 1, worsenedFindings: 0 },
    { repoId: 2, repoName: "worker", scanRunId: 11, succeeded: false, error: "Fixture scanner timeout", newFindings: 0, resolvedFindings: 0, worsenedFindings: 0 },
  ],
  warnings: [], newFindings: 1, resolvedFindings: 1, worsenedFindings: 0, terminalError: null,
};

describe("ScanActivityPopover", () => {
  it("shows missing package coverage and retry without reporting resolved findings", () => {
    const failed: ScanActivity = {
      ...activity, scopeRepoId: 2, status: "failed", stage: "failed",
      completedRepositories: 0, totalRepositories: 1, newFindings: 0, resolvedFindings: 0,
      outcomes: [{ ...activity.outcomes[1], error: "Incomplete npm inventory: restore the lockfile and retry." }],
    };
    render(<ScanActivityPopover activity={failed} open onOpenChange={() => {}} onReview={() => {}} onResolved={() => {}} onHistory={() => {}} onRetry={() => {}} />);
    expect(screen.getByRole("button", { name: "Scan activity: Incomplete" })).toBeInTheDocument();
    expect(screen.getByText("restore the lockfile and retry.").closest("details")).toBeNull();
    expect(screen.getByRole("button", { name: "Retry scan for worker" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "View resolved findings" })).not.toBeInTheDocument();
  });

  it("describes discovery before the total is known", () => {
    render(<ScanActivityPopover activity={{
      ...activity, status: "running", stage: "queued", finishedAt: null,
      totalRepositories: 0, completedRepositories: 0, failedRepositories: 0, outcomes: [],
    }} open onOpenChange={() => {}} onReview={() => {}} onResolved={() => {}} onHistory={() => {}} onRetry={() => {}} />);
    expect(screen.getByRole("status")).toHaveTextContent("Finding repositories");
    expect(screen.getByRole("status")).not.toHaveTextContent("0 of 0");
  });

  it("renders truthful partial failure and exact destinations accessibly", async () => {
    const onReview = vi.fn();
    const onResolved = vi.fn();
    const onHistory = vi.fn();
    const onRetry = vi.fn();
    const { container } = render(<ScanActivityPopover activity={activity} open onOpenChange={() => {}} onReview={onReview} onResolved={onResolved} onHistory={onHistory} onRetry={onRetry} />);
    expect(screen.getByText("Completed with failures")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Review changes" }));
    expect(onReview).toHaveBeenCalledWith([10]);
    fireEvent.click(screen.getByRole("button", { name: "View resolved findings" }));
    expect(onResolved).toHaveBeenCalledWith([10]);
    fireEvent.click(screen.getByRole("button", { name: "Scan history for worker, scan #11" }));
    expect(onHistory).toHaveBeenCalledWith(2, 11);
    expect(screen.queryByRole("button", { name: "Scan history" })).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("1 completed · 1 failed");
    expect(screen.getByText("Fixture scanner timeout").closest("details")).not.toHaveAttribute("open");
    fireEvent.click(screen.getByRole("button", { name: "Retry scan for worker" }));
    expect(onRetry).toHaveBeenCalledWith(2);
    const results = await axe.run(container, { rules: { "color-contrast": { enabled: false } } });
    const actionable = results.violations
      .filter((violation) => violation.impact === "critical" || violation.impact === "serious")
      .filter((violation) => violation.nodes.some((node) => !node.html.includes("data-base-ui-focus-guard")));
    expect(actionable).toEqual([]);
  });

  it("keeps optional advisory details folded below the actual progress and repository results", () => {
    const running: ScanActivity = {
      ...activity,
      status: "running",
      stage: "inventorying_packages",
      finishedAt: null,
      totalRepositories: 13,
      completedRepositories: 3,
      failedRepositories: 0,
      currentRepoName: "cvec",
      outcomes: [{ ...activity.outcomes[0], repoName: "worker", newFindings: 0, resolvedFindings: 0 }],
      warnings: [
        "Optional advisory details for GHSA-test are unavailable; local scanner results remain complete.",
        "29 additional optional advisory lookups were unavailable.",
      ],
      newFindings: 6,
      resolvedFindings: 7,
    };
    render(<ScanActivityPopover activity={running} open onOpenChange={() => {}} onReview={() => {}} onResolved={() => {}} onHistory={() => {}} onRetry={() => {}} />);
    const progress = screen.getByRole("progressbar", { name: "Repositories processed" });
    expect(progress).toHaveAttribute("aria-valuenow", "3");
    expect(progress).toHaveAttribute("aria-valuemax", "13");
    expect(screen.getByRole("status")).toHaveTextContent("cvec · Inventorying packages");
    expect(screen.getByRole("group", { name: "Finding changes" })).toHaveTextContent(/6 new.*7 resolved/);
    expect(screen.getByRole("group", { name: "Finding changes" })).not.toHaveTextContent("worsened");
    expect(screen.getByText("No changes")).toBeInTheDocument();
    const note = screen.getByText("Advisory lookup notes");
    expect(note.closest("details")).not.toHaveAttribute("open");
    expect(screen.getByText("29 additional optional advisory lookups were unavailable.")).not.toBeVisible();
    fireEvent.click(note);
    expect(note.closest("details")).toHaveAttribute("open");
    expect(screen.getByText("Local scan results are available.").closest("details")).toHaveAttribute("open");
  });
});
