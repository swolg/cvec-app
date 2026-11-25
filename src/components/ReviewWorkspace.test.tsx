import axe from "axe-core";
import { useState } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/api", () => ({
  listReviewFindings: vi.fn(),
  getReviewFindingDetail: vi.fn(async () => finding),
  markFindingReviewed: vi.fn(async () => ({})),
  returnFindingToReview: vi.fn(async () => ({})),
  snoozeFinding: vi.fn(async () => ({})),
  getFindingRemediation: vi.fn(async (repoId: number, fingerprint: string, evidenceScanRunId: number) => ({
    schemaVersion: 1, repoId, repoName: "cvec", fingerprint, advisoryId: "CVE-2026-1000",
    packageName: "router", installedVersion: "1.0.0", packageType: "npm", packageUrl: "pkg:npm/router@1.0.0",
    scannerReportedTarget: "1.0.1", evidenceScanRunId, evidenceScannedAt: "2026-08-12T08:00:00Z",
    sourceSnapshot: { revision: "abc123", dirty: false }, relativePath: "package.json", locationKind: "manifest",
    relationship: "unknown", parentPackage: null, evidenceQuality: "partial", evidenceSource: "syft_location",
    locationCount: 1, verificationState: "still_detected", verificationScanRunId: evidenceScanRunId,
    verificationScannedAt: "2026-08-12T08:00:00Z", canOpenManifest: true,
    priorityReasons: ["CRITICAL severity", "Fix available"], briefMarkdown: "## CVEC fix brief\n\n- Repository: cvec",
  })),
  openFindingManifest: vi.fn(async () => undefined),
}));

import { getReviewFindingDetail, getFindingRemediation, listReviewFindings, markFindingReviewed, returnFindingToReview, snoozeFinding } from "@/lib/api";
import { reviewSessionCache } from "@/lib/reviewPageCache";
import { remediationProjectionCache } from "@/lib/remediationProjectionCache";
import { isFixPlanEnabled, ReviewWorkspace } from "./ReviewWorkspace";
import { CRITICAL_TO_REVIEW_VIEW, OTHER_TO_REVIEW_VIEW, defaultReviewViewsState, DEFAULT_TABLE_PRESENTATION, reviewQuery } from "@/lib/reviewViews";
import type { WorkspaceContext } from "@/lib/navigation";
import type { FindingRemediationProjection, ReviewFinding, ReviewGroupCounts, ReviewPage } from "@/lib/types";
import { EMPTY_SNAPSHOT } from "@/test/fixtures";

const finding: ReviewFinding = {
  repoId: 1, repoName: "cvec", repoSourceType: "local", scanRunId: 10, scannedAt: "2026-08-12T08:00:00Z",
  finding: { fingerprint: "fp-1", advisoryId: "CVE-2026-1000", packageName: "router", packageVersion: "1.0.0", packageType: "npm", packageUrl: "pkg:npm/router@1.0.0", severity: "CRITICAL", fixVersion: "1.0.1", title: "Router issue", sourceUrl: "https://example.com/advisory", state: "open", intel: null, reviewState: "unreviewed", reviewedAt: null, snoozeUntil: null, reviewNote: null },
  reviewState: "unreviewed", reviewedAt: null, snoozeUntil: null, reviewNote: null, knownExploited: true, ransomwareKnown: false, materialChange: "new", priorityReasons: ["Known exploited vulnerability", "CRITICAL severity"],
};

function renderWorkspace(initialViewId?: string, dataRevision = 0, table = false) {
  return render(<ReviewWorkspace dataRevision={dataRevision} viewsState={defaultReviewViewsState()} onViewsStateChange={() => {}} initialViewId={initialViewId} initialContext={table ? { reviewPresentation: DEFAULT_TABLE_PRESENTATION } : undefined} scannerReady onOpenRepo={() => {}} onOpenUrl={() => {}} onError={() => {}} />);
}

function StatefulRepositoryReview({ context: initialContext }: { context: WorkspaceContext }) {
  const [context, setContext] = useState(initialContext);
  return <ReviewWorkspace viewsState={defaultReviewViewsState()} onViewsStateChange={() => {}} initialContext={context} onContextChange={setContext} scannerReady onOpenRepo={() => {}} onOpenUrl={() => {}} onError={() => {}} />;
}

function reviewPage(items: ReviewFinding[], options: { total?: number; nextCursor?: string | null; groupCounts?: ReviewGroupCounts } = {}): ReviewPage {
  const count = (predicate: (item: ReviewFinding) => boolean) => items.filter(predicate).length;
  return {
    items,
    revision: 0,
    nextCursor: options.nextCursor ?? null,
    total: options.total ?? items.length,
    groupCounts: options.groupCounts ?? {
      reviewState: [["unreviewed", "Needs review"], ["reviewed", "Reviewed"], ["snoozed", "Snoozed"]].map(([id, label]) => ({ id, label, count: count((item) => item.reviewState === id) })),
      severity: [["CRITICAL", "Critical"], ["HIGH", "High"], ["MEDIUM", "Medium"]].map(([id, label]) => ({ id, label, count: count((item) => item.finding.severity === id) })),
      repository: [...new Map(items.map((item) => [item.repoId, item.repoName])).entries()].map(([repoId, label]) => ({ id: `repository:${repoId}`, label, count: count((item) => item.repoId === repoId) })),
      fixAvailability: [{ id: "available", label: "Fix available", count: count((item) => Boolean(item.finding.fixVersion)) }, { id: "unavailable", label: "No fix listed", count: count((item) => !item.finding.fixVersion) }],
    },
  };
}

function remediationProjection(item: ReviewFinding, target = item.finding.fixVersion): FindingRemediationProjection {
  return {
    schemaVersion: 1, repoId: item.repoId, repoName: item.repoName, fingerprint: item.finding.fingerprint,
    advisoryId: item.finding.advisoryId, packageName: item.finding.packageName, installedVersion: item.finding.packageVersion,
    packageType: item.finding.packageType, packageUrl: item.finding.packageUrl, scannerReportedTarget: target,
    evidenceScanRunId: item.scanRunId!, evidenceScannedAt: item.scannedAt, sourceSnapshot: { revision: "abc123", dirty: false },
    relativePath: "package.json", locationKind: "manifest", relationship: "unknown", parentPackage: null,
    evidenceQuality: "partial", evidenceSource: "syft_location", locationCount: 1, verificationState: "still_detected",
    verificationScanRunId: item.scanRunId, verificationScannedAt: item.scannedAt, canOpenManifest: true,
    priorityReasons: ["CRITICAL severity", "Fix available"], briefMarkdown: `## CVEC fix brief\n\n- Finding: ${item.finding.advisoryId}`,
  };
}

async function chooseSelectOption(name: string, optionName: string) {
  const user = userEvent.setup();
  await user.click(screen.getByRole("combobox", { name }));
  await user.click(await screen.findByRole("option", { name: optionName }));
}

describe("ReviewWorkspace", () => {
  beforeEach(() => {
    window.history.replaceState({}, "", "/");
    reviewSessionCache(0).clear();
    vi.mocked(markFindingReviewed).mockReset().mockResolvedValue({} as never);
    vi.mocked(returnFindingToReview).mockReset().mockResolvedValue({} as never);
    vi.mocked(snoozeFinding).mockReset().mockResolvedValue({} as never);
    remediationProjectionCache.clear();
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([finding]));
    vi.mocked(getFindingRemediation).mockReset().mockImplementation(async () => remediationProjection(finding));
    vi.mocked(getReviewFindingDetail).mockReset().mockResolvedValue(finding);
  });


  it("keeps the exact installed package identity when loading Review",async()=>{
    const identity={anyFix:true,packageName:"router",packageUrl:"pkg:npm/router@1.0.0",packageType:"npm",installedVersion:"1.0.0",fixVersion:null};
    render(<ReviewWorkspace viewsState={defaultReviewViewsState()} onViewsStateChange={()=>{}} initialContext={{viewId:"all-open",reviewQuery:reviewQuery({repoIds:[1],packageIdentity:identity})}} scannerReady onOpenRepo={()=>{}} onOpenUrl={()=>{}} onError={()=>{}} />);
    await waitFor(()=>expect(listReviewFindings).toHaveBeenCalledWith(expect.objectContaining({filters:expect.objectContaining({packageIdentity:identity})})));
  });

  it("opens the recorded findings for Scan changes with the selected runs", async () => {
    renderWorkspace("scan:17,18");
    await waitFor(() => expect(listReviewFindings).toHaveBeenCalledWith(expect.objectContaining({
      filters: expect.objectContaining({ scanChangeRunIds: [17, 18], scannerStates: ["new", "open"] }),
    })));
  });

  it("opens the resolutions from the selected scans using the resolving run IDs", async () => {
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([]));
    renderWorkspace("scan-resolved:92,93");
    await waitFor(() => expect(listReviewFindings).toHaveBeenCalledWith(expect.objectContaining({
      filters: expect.objectContaining({ scanResolvedRunIds: [92, 93], scanRunIds: [], scannerStates: ["resolved"] }),
    })));
    expect(await screen.findByText("No current resolutions from these scans")).toBeInTheDocument();
  });

  it("inspects the saved finding from a historical resolution without loading a current fix plan", async () => {
    const recorded: ReviewFinding = { ...finding, finding: { ...finding.finding, state: "resolved" } };
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([recorded]));
    vi.mocked(getReviewFindingDetail).mockResolvedValueOnce(recorded);
    vi.mocked(getFindingRemediation).mockClear();

    renderWorkspace("scan-resolution:11");
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    const inspector = within(await screen.findByRole("dialog"));
    expect(inspector.getByText("Scan #11")).toBeInTheDocument();
    expect(inspector.getByText("Scan #10")).toBeInTheDocument();
    expect(inspector.getByText("router 1.0.0")).toBeInTheDocument();
    await waitFor(() => expect(getReviewFindingDetail).toHaveBeenCalledWith(1, "fp-1", 10, 11));
    expect(listReviewFindings).toHaveBeenCalledWith(expect.objectContaining({
      filters: expect.objectContaining({ resolvedInScanRunId: 11, scannerStates: ["resolved"] }),
    }));
    expect(inspector.queryByRole("button", { name: /Mark reviewed|Snooze/ })).not.toBeInTheDocument();
    expect(getFindingRemediation).not.toHaveBeenCalled();
  });

  it("reloads the open inspector when verification changes without a new evidence id", async () => {
    const props = { viewsState: defaultReviewViewsState(), onViewsStateChange: vi.fn(), initialViewId: "all-open", scannerReady: true, onOpenRepo: vi.fn(), onOpenUrl: vi.fn(), onError: vi.fn() };
    const view = render(<ReviewWorkspace {...props} dataRevision={1} />);
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    await screen.findByText(/Still detected/i);
    vi.mocked(getFindingRemediation).mockResolvedValue({ ...remediationProjection(finding), verificationState: "unverifiable", verificationScanRunId: 11 });
    remediationProjectionCache.clear();
    view.rerender(<ReviewWorkspace {...props} dataRevision={2} />);
    await waitFor(() => expect(listReviewFindings).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(getFindingRemediation).toHaveBeenCalledTimes(2));
    await screen.findByText(/latest result could not verify/i);
    expect(screen.queryByText("The latest complete scan still detected this advisory for the package.")).not.toBeInTheDocument();
  });

  it.each([true, false])("defers background reload until a board move settles (success=%s)", async (succeeds) => {
    let complete!: () => void;
    let currentFinding = finding;
    vi.mocked(listReviewFindings).mockImplementation(async () => reviewPage([currentFinding]));
    vi.mocked(markFindingReviewed).mockImplementationOnce(() => new Promise((resolve, reject) => {
      complete = () => {
        if (!succeeds) { reject(new Error("write failed")); return; }
        currentFinding = { ...finding, reviewState: "reviewed", finding: { ...finding.finding, reviewState: "reviewed" } };
        resolve({} as never);
      };
    }));
    const props = { initialViewId: "all-open", viewsState: defaultReviewViewsState(), onViewsStateChange: vi.fn(), scannerReady: true, onOpenRepo: vi.fn(), onOpenUrl: vi.fn(), onError: vi.fn() };
    const view = render(<ReviewWorkspace {...props} dataRevision={1} />);
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(screen.getByRole("button", { name: "Actions for CVE-2026-1000" }));
    fireEvent.click(await screen.findByRole("button", { name: "Move to Reviewed" }));
    expect(screen.getByRole("region", { name: "Reviewed, 1 finding" })).toBeInTheDocument();
    view.rerender(<ReviewWorkspace {...props} dataRevision={2} />);
    view.rerender(<ReviewWorkspace {...props} dataRevision={3} />);
    expect(listReviewFindings).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("region", { name: "Reviewed, 1 finding" })).toBeInTheDocument();
    await act(async () => complete());
    await waitFor(() => expect(listReviewFindings).toHaveBeenCalledTimes(2));
    expect(screen.getByRole("region", { name: succeeds ? "Reviewed, 1 finding" : "Needs review, 1 finding" })).toBeInTheDocument();
    if (!succeeds) expect(props.onError).toHaveBeenCalledWith("Error: write failed");
  });


  it("invalidates a background read that started before the board move", async () => {
    let finishRead!: (page: ReviewPage) => void;
    let finishMove!: () => void;
    const reviewed: ReviewFinding = { ...finding, reviewState: "reviewed", finding: { ...finding.finding, reviewState: "reviewed" } };
    vi.mocked(listReviewFindings)
      .mockResolvedValueOnce(reviewPage([finding]))
      .mockImplementationOnce(() => new Promise(resolve => { finishRead = resolve; }))
      .mockResolvedValue(reviewPage([reviewed]));
    vi.mocked(markFindingReviewed).mockImplementationOnce(() => new Promise(resolve => { finishMove = () => resolve({} as never); }));
    const props = { initialViewId: "all-open", viewsState: defaultReviewViewsState(), onViewsStateChange: vi.fn(), scannerReady: true, onOpenRepo: vi.fn(), onOpenUrl: vi.fn(), onError: vi.fn() };
    const view = render(<ReviewWorkspace {...props} dataRevision={1} />);
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    view.rerender(<ReviewWorkspace {...props} dataRevision={1} reviewRevision={1} />);
    await waitFor(() => expect(listReviewFindings).toHaveBeenCalledTimes(2));
    fireEvent.click(screen.getByRole("button", { name: "Actions for CVE-2026-1000" }));
    fireEvent.click(await screen.findByRole("button", { name: "Move to Reviewed" }));
    await act(async () => finishRead(reviewPage([finding])));
    expect(screen.getByRole("region", { name: "Reviewed, 1 finding" })).toBeInTheDocument();
    await act(async () => finishMove());
    await waitFor(() => expect(listReviewFindings).toHaveBeenCalledTimes(3));
    expect(screen.getByRole("region", { name: "Reviewed, 1 finding" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Drag CVE-2026-1000" })).toHaveClass("cursor-grab");
  });

  it("enables the Fix Plan by default and retains explicit rollback controls", () => {
    expect(isFixPlanEnabled(undefined, "", false)).toBe(true);
    expect(isFixPlanEnabled("on", "", false)).toBe(true);
    expect(isFixPlanEnabled("off", "", false)).toBe(false);
    expect(isFixPlanEnabled(undefined, "?qa=exact", true)).toBe(true);
    expect(isFixPlanEnabled(undefined, "?qa=exact&fixPlan=off", true)).toBe(false);
  });

  it("uses Needs review for pending decisions and All open for the workflow board", async () => {
    const reviewed: ReviewFinding = { ...finding, reviewState: "reviewed", finding: { ...finding.finding, fingerprint: "reviewed", advisoryId: "CVE-2026-1001" } };
    const snoozed: ReviewFinding = { ...finding, reviewState: "snoozed", finding: { ...finding.finding, fingerprint: "snoozed", advisoryId: "CVE-2026-1002" } };
    vi.mocked(listReviewFindings).mockImplementation(async (query) =>
      reviewPage(query?.filters.reviewStates.includes("unreviewed") ? [finding] : [finding, reviewed, snoozed]));
    renderWorkspace();
    const table = await screen.findByRole("grid", { name: "Needs review findings" });
    expect(within(table).getByRole("row", { name: /CVE-2026-1000/ })).toBeVisible();
    expect(within(table).queryByRole("row", { name: /CVE-2026-1001|CVE-2026-1002/ })).not.toBeInTheDocument();
    expect(listReviewFindings).toHaveBeenCalledWith(expect.objectContaining({
      filters: expect.objectContaining({ reviewStates: ["unreviewed"], scannerStates: ["new", "open"] }),
    }));

    fireEvent.click(screen.getByRole("button", { name: "All open" }));
    const board = await screen.findByRole("region", { name: "Review board" });
    expect(within(board).getByRole("grid", { name: "Reviewed findings" })).toHaveTextContent("CVE-2026-1001");
    expect(within(board).getByRole("grid", { name: "Snoozed findings" })).toHaveTextContent("CVE-2026-1002");
    expect(within(board).getByRole("button", { name: "Drag CVE-2026-1000" })).toHaveClass("cursor-grab");
  });

  it("keeps scanner and review state distinct and executes review actions", async () => {
    const { container } = renderWorkspace("all-open", 0, true);
    const row = await screen.findByRole("row", { name: /CVE-2026-1000/ });
    expect(container.querySelectorAll(".review-result-scroller")).toHaveLength(1);
    expect(screen.getByRole("columnheader", { name: /Finding/ })).toHaveAttribute("aria-sort", "descending");
    expect(screen.getByRole("columnheader", { name: /Finding/ })).toHaveClass("sticky");
    expect(row).toHaveTextContent("unreviewed");
    expect(row).toHaveTextContent("open");
    fireEvent.click(row);
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(container.querySelectorAll(".review-result-scroller")).toHaveLength(1);
    fireEvent.click(await screen.findByRole("button", { name: "Mark reviewed" }));
    await waitFor(() => expect(markFindingReviewed).toHaveBeenCalledWith(1, "fp-1", {
      scanRunId: 10,
      riskBasis: { severity: "CRITICAL", fixVersion: "1.0.1", knownExploited: true, ransomwareKnown: false, scannerState: "open" },
      reviewState: "unreviewed",
      reviewedAt: null,
      snoozeUntil: null,
      reviewNote: null,
    }));
    const results = await axe.run(container, { rules: { "color-contrast": { enabled: false } } });
    const actionable = results.violations.filter((violation) => ["critical", "serious"].includes(violation.impact ?? "")).filter((violation) => violation.nodes.some((node) => !node.html.includes("data-base-ui-focus-guard")));
    expect(actionable).toEqual([]);
  });

  it("hands a stable repository and finding identity to repository chat", async () => {
    const onContinueInChat = vi.fn();
    render(<ReviewWorkspace viewsState={defaultReviewViewsState()} onViewsStateChange={() => {}} initialViewId="all-open" scannerReady onOpenRepo={() => {}} onOpenUrl={() => {}} onContinueInChat={onContinueInChat} onError={() => {}} />);
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Fix in Chat" }));
    expect(onContinueInChat).toHaveBeenCalledWith({
      repositoryId: 1, fingerprint: "fp-1", advisoryId: "CVE-2026-1000", packageName: "router",
      installedVersion: "1.0.0", targetVersion: "1.0.1", scanRunId: 10, mode: "fix",
    });
    expect(markFindingReviewed).not.toHaveBeenCalled();
  });

  it.each([
    { kind: "go_symbol", label: "Bundled tool" },
    { kind: "go_module", label: "Go version match" },
  ] as const)("shows the npm tool and offers analysis for $kind evidence", async ({ kind, label }) => {
    const item: ReviewFinding = {
      ...finding,
      finding: {
        ...finding.finding,
        packageType: "go-module",
        fixVersion: null,
        matchEvidence: {
          kind,
          relativePath: "node_modules/@example/cli/lib/tool",
          bundledTool: { packageName: "@example/cli", componentFixVersion: "1.0.1" },
        },
      },
    };
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([item]));
    vi.mocked(getReviewFindingDetail).mockResolvedValue(item);
    const onContinueInChat = vi.fn();
    render(<ReviewWorkspace viewsState={defaultReviewViewsState()} onViewsStateChange={() => {}} initialViewId="all-open" scannerReady onOpenRepo={() => {}} onOpenUrl={() => {}} onContinueInChat={onContinueInChat} onError={() => {}} />);
    const row = await screen.findByRole("row", { name: /CVE-2026-1000/ });
    expect(row).toHaveTextContent(label);
    fireEvent.click(row);
    const evidence = within(await screen.findByRole("region", { name: "Scanner match" }));
    expect(evidence.getByText("@example/cli")).toBeVisible();
    expect(evidence.getByText("Go module fixed in")).toBeVisible();
    expect(evidence.getByText("1.0.1")).toBeVisible();
    const action = screen.getByRole("button", { name: "Analyze in Chat" });
    expect(action).toHaveAccessibleDescription(/npm tool’s releases/);
    expect(screen.queryByRole("button", { name: "Fix in Chat" })).not.toBeInTheDocument();
    fireEvent.click(action);
    expect(onContinueInChat).toHaveBeenCalledWith(expect.objectContaining({ mode: "analysis", targetVersion: null }));
  });

  it.each([
    { repoSourceType: "github", state: "open", view: "all-open", heading: "Remediation evidence", reason: /local checkout/ },
    { repoSourceType: "local", state: "resolved", view: "recently-resolved", heading: "Historical evidence", reason: /historical finding/ },
  ] as const)("explains the analysis-only handoff for $repoSourceType $state evidence", async ({ repoSourceType, state, view, heading, reason }) => {
    const item: ReviewFinding = { ...finding, repoSourceType, finding: { ...finding.finding, state } };
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([item]));
    vi.mocked(getReviewFindingDetail).mockResolvedValue(item);
    const onContinueInChat = vi.fn();
    render(<ReviewWorkspace viewsState={defaultReviewViewsState()} onViewsStateChange={() => {}} initialViewId={view} scannerReady onOpenRepo={() => {}} onOpenUrl={() => {}} onContinueInChat={onContinueInChat} onError={() => {}} />);
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    const action = await screen.findByRole("button", { name: "Analyze in Chat" });
    expect(action).toHaveAccessibleDescription(reason);
    expect(screen.getByRole("heading", { name: heading })).toBeVisible();
    fireEvent.click(action);
    expect(onContinueInChat).toHaveBeenCalledWith(expect.objectContaining({ mode: "analysis", scanRunId: 10 }));
  });

  it("renders the calm completion state without treating built-in filters as search", async () => {
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([]));
    renderWorkspace();
    expect(await screen.findByRole("heading", { name: "Review complete" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "View all open" })).toBeInTheDocument();
  });

  it.each([
    { id: CRITICAL_TO_REVIEW_VIEW, name: "Critical to review", severities: ["CRITICAL"] },
    { id: OTHER_TO_REVIEW_VIEW, name: "Other findings to review", severities: ["HIGH", "MEDIUM", "LOW", "NEGLIGIBLE", "UNKNOWN"] },
  ])("opens the dashboard's $name count with the matching query", async ({ id, name, severities }) => {
    render(<ReviewWorkspace viewsState={defaultReviewViewsState()} onViewsStateChange={() => {}} initialContext={{ viewId: id }} scannerReady onOpenRepo={() => {}} onOpenUrl={() => {}} onError={() => {}} />);
    expect(screen.getByRole("button", { name })).toHaveAttribute("aria-current", "true");
    await waitFor(() => expect(listReviewFindings).toHaveBeenCalledWith(expect.objectContaining({
      search: null,
      filters: expect.objectContaining({ scannerStates: ["new", "open"], reviewStates: ["unreviewed"], severities }),
    })));
  });

  it("clears an unsuccessful search without declaring the review queue complete", async () => {
    vi.mocked(listReviewFindings).mockImplementation(async (query) => reviewPage(query?.search ? [] : [finding]));
    renderWorkspace();
    await screen.findByRole("row", { name: /CVE-2026-1000/ });

    const search = screen.getByRole("searchbox", { name: "Search review findings" });
    fireEvent.change(search, { target: { value: "missing-fixture" } });
    expect(await screen.findByRole("heading", { name: "No matching findings" })).toBeInTheDocument();
    expect(screen.queryByText(/Nothing currently needs review/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Clear search" }));
    expect(search).toHaveFocus();
    expect(search).toHaveValue("");
    expect(await screen.findByRole("row", { name: /CVE-2026-1000/ })).toBeInTheDocument();
  });

  it("lets the user retry a failed load without showing a completed queue", async () => {
    vi.mocked(listReviewFindings).mockRejectedValueOnce(new Error("Could not read test database"));
    renderWorkspace();
    expect(await screen.findByRole("heading", { name: "Findings could not be loaded" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Review complete" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry loading findings" }));
    expect(await screen.findByRole("row", { name: /CVE-2026-1000/ })).toBeInTheDocument();
  });

  it("restores the session query, selection, and sort without reopening persisted state", async () => {
    const filters = { reviewStates: ["unreviewed" as const], scannerStates: ["open" as const], severities: ["CRITICAL" as const], repoIds: [], sourceTypes: [], fixAvailable: true, packages: [], ecosystems: [], knownExploited: null, ransomwareKnown: null, changedSince: null, scanRunIds: [] };
    render(<ReviewWorkspace viewsState={defaultReviewViewsState()} onViewsStateChange={() => {}} initialContext={{ viewId: "critical", selectedKey: "1:fp-1", scrollTop: 34, reviewQuery: { filters, search: "router", sort: "severity", cursor: null, pageSize: 50 } }} scannerReady onOpenRepo={() => {}} onOpenUrl={() => {}} onError={() => {}} />);
    expect(await screen.findByRole("searchbox", { name: "Search review findings" })).toHaveValue("router");
    expect(screen.getByRole("combobox", { name: "Sort review findings" })).toHaveTextContent("Severity");
    expect(await screen.findByRole("row", { name: /CVE-2026-1000/ })).toHaveAttribute("aria-selected", "true");
  });

  it.each(["all-open", "needs-review"])("returns keyboard focus to the finding when closing a restored %s inspector", async (viewId) => {
    render(<ReviewWorkspace viewsState={defaultReviewViewsState()} onViewsStateChange={() => {}} initialContext={{ viewId, selectedKey: "1:fp-1", focusKey: "1:fp-1", inspectorKey: "1:fp-1", inspectorPinned: true }} scannerReady onOpenRepo={() => {}} onOpenUrl={() => {}} onError={() => {}} />);
    await screen.findByRole("dialog");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await waitFor(() => {
      const row = screen.getByRole("row", { name: /CVE-2026-1000/ });
      expect(row === document.activeElement || row.contains(document.activeElement)).toBe(true);
    });
  });

  it("navigates between findings inside the overlay inspector", async () => {
    const second = { ...finding, repoId: 2, repoName: "web-app", finding: { ...finding.finding, fingerprint: "fp-2", advisoryId: "CVE-2026-2000", title: "Second issue" } };
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([finding, second]));
    renderWorkspace();
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    expect(await screen.findByRole("dialog")).toHaveTextContent("Router issue");
    fireEvent.click(screen.getByRole("button", { name: "Next finding" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("Second issue");
  });

  it("discards a stale remediation response during rapid next navigation", async () => {
    window.history.replaceState({}, "", "/?qa=exact");
    const second = { ...finding, repoId: 2, repoName: "web-app", scanRunId: 20, finding: { ...finding.finding, fingerprint: "fp-2", advisoryId: "CVE-2026-2000", title: "Second issue", fixVersion: "2.0.0" } };
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([finding, second]));
    let resolveFirst!: (value: FindingRemediationProjection) => void;
    vi.mocked(getFindingRemediation)
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }))
      .mockResolvedValueOnce(remediationProjection(second));

    renderWorkspace();
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    await waitFor(() => expect(resolveFirst).toBeTypeOf("function"));
    fireEvent.click(await screen.findByRole("button", { name: "Next finding" }));
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("→ 2.0.0"));
    await screen.findByText(/The latest complete scan still detected/);
    resolveFirst(remediationProjection(finding, "9.9.9"));
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("Second issue"));
    expect(screen.getByRole("dialog")).not.toHaveTextContent("9.9.9");
    expect(markFindingReviewed).not.toHaveBeenCalled();
  });

  it("marks only settled matching Inspector evidence ready when moving to the next finding", async () => {
    const second={...finding,repoId:2,repoName:"second",scanRunId:20,finding:{...finding.finding,fingerprint:"fp-2",advisoryId:"CVE-2026-2000",title:"Second issue"}};
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([finding,second]));
    let finishDetail!:(value:ReviewFinding)=>void;let finishFix!:(value:FindingRemediationProjection)=>void;
    vi.mocked(getReviewFindingDetail).mockResolvedValueOnce(finding).mockImplementationOnce(()=>new Promise(resolve=>{finishDetail=resolve;}));
    vi.mocked(getFindingRemediation).mockResolvedValueOnce(remediationProjection(finding)).mockImplementationOnce(()=>new Promise(resolve=>{finishFix=resolve;}));
    renderWorkspace();fireEvent.click(await screen.findByRole("row",{name:/CVE-2026-1000/}));
    const pane=await screen.findByRole("complementary",{name:"Finding CVE-2026-1000"});
    await waitFor(()=>expect(pane).toHaveAttribute("data-inspector-remediation-state","ready"));
    expect(pane).toHaveAttribute("data-inspector-detail-state","ready");
    fireEvent.click(screen.getByRole("button",{name:"Next finding"}));
    expect(pane).toHaveAttribute("data-inspector-finding-key","2:fp-2");
    expect(pane).toHaveAttribute("data-inspector-detail-state","loading");
    expect(pane).toHaveAttribute("data-inspector-remediation-state","loading");
    await waitFor(()=>expect(finishDetail).toBeTypeOf("function"));
    await waitFor(()=>expect(finishFix).toBeTypeOf("function"));
    await act(async()=>finishDetail(second));
    expect(pane).toHaveAttribute("data-inspector-detail-state","ready");
    expect(pane).toHaveAttribute("data-inspector-remediation-state","loading");
    await act(async()=>finishFix(remediationProjection(second)));
    await waitFor(()=>expect(pane).toHaveAttribute("data-inspector-remediation-state","ready"));
  });

  it("reuses immutable remediation when the inspector closes and reopens", async () => {
    window.history.replaceState({}, "", "/?qa=exact");
    renderWorkspace();
    const row = await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(row);
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("→ 1.0.1"));
    await screen.findByText(/The latest complete scan still detected/);
    fireEvent.click(screen.getByRole("button", { name: "Close inspector" }));
    fireEvent.click(row);
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("→ 1.0.1"));
    await screen.findByText(/The latest complete scan still detected/);
    expect(getFindingRemediation).toHaveBeenCalledTimes(1);
    expect(markFindingReviewed).not.toHaveBeenCalled();
  });

  it("retries a failed local load without changing selection or Review state", async () => {
    window.history.replaceState({}, "", "/?qa=exact");
    vi.mocked(getFindingRemediation).mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce(remediationProjection(finding));
    renderWorkspace();
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    expect(await screen.findByText("Local fix evidence could not be loaded.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("→ 1.0.1"));
    await screen.findByText(/The latest complete scan still detected/);
    expect(getFindingRemediation).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("dialog")).toHaveTextContent("Router issue");
    expect(screen.getByRole("button", { name: "Mark reviewed" })).toBeInTheDocument();
    expect(markFindingReviewed).not.toHaveBeenCalled();
  });

  it("loads a fresh projection after scan completion changes the immutable scan id", async () => {
    window.history.replaceState({}, "", "/?qa=exact");
    const firstRender = renderWorkspace();
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("→ 1.0.1"));
    await screen.findByText(/The latest complete scan still detected/);
    firstRender.unmount();

    const rescanned = { ...finding, scanRunId: 11, scannedAt: "2026-08-17T12:00:00Z", finding: { ...finding.finding, fixVersion: "1.0.2" } };
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([rescanned]));
    vi.mocked(getFindingRemediation).mockResolvedValue(remediationProjection(rescanned));
    renderWorkspace(undefined, 1);
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("→ 1.0.2"));
    await screen.findByText(/The latest complete scan still detected/);
    expect(getFindingRemediation).toHaveBeenNthCalledWith(1, 1, "fp-1", 10);
    expect(getFindingRemediation).toHaveBeenNthCalledWith(2, 1, "fp-1", 11);
  });

  it("keeps repository remediation groups exact without changing saved views", async () => {
    const sameGroup = { ...finding, finding: { ...finding.finding, fingerprint: "fp-2", advisoryId: "CVE-2026-1001" } };
    const otherTarget = { ...finding, finding: { ...finding.finding, fingerprint: "fp-3", advisoryId: "CVE-2026-1002", fixVersion: "2.0.0" } };
    vi.mocked(listReviewFindings).mockImplementation(async (query) => reviewPage(
      query.filters.packageIdentity?.fixVersion === "1.0.1" ? [finding, sameGroup] : [finding, sameGroup, otherTarget],
    ));
    const views = defaultReviewViewsState();
    const onViewsStateChange = vi.fn();
    render(<ReviewWorkspace viewsState={views} onViewsStateChange={onViewsStateChange} initialContext={{
      viewId: "all-open", selectedKey: "1:missing", ephemeralReviewContext: true,
      reviewQuery: reviewQuery({ repoIds: [1], scannerStates: ["new", "open"], packages: ["router"] }),
      remediationGroup: { packageName: "router", packageUrl: "pkg:npm/router@1.0.0", packageType: "npm", installedVersion: "1.0.0", fixVersion: "1.0.1", preferredFingerprint: "missing" },
    }} scannerReady onOpenRepo={() => {}} onOpenUrl={() => {}} onError={() => {}} />);
    expect(await screen.findByRole("dialog")).toHaveTextContent("Router issue");
    expect(screen.queryByText("CVE-2026-1002")).not.toBeInTheDocument();
    expect(screen.getByText(/requested finding is no longer available/i)).toBeInTheDocument();
    expect(listReviewFindings).toHaveBeenCalledWith(expect.objectContaining({
      filters: expect.objectContaining({ packageIdentity: expect.objectContaining({ fixVersion: "1.0.1" }) }),
    }), { loadedRows: 0, preserveSelection: "1:missing" });
    expect(onViewsStateChange).not.toHaveBeenCalled();
  });

  it("dismisses a repository-opened inspector outside and reopens the same finding", async () => {
    render(<StatefulRepositoryReview context={{
      viewId: "all-open", reviewPresentation: DEFAULT_TABLE_PRESENTATION,
      selectedKey: "1:fp-1",
      ephemeralReviewContext: true,
      reviewQuery: reviewQuery({ repoIds: [1], scannerStates: ["new", "open"], packages: ["router"] }),
    }} />);

    expect(await screen.findByRole("dialog")).toHaveTextContent("Router issue");
    fireEvent.pointerDown(document.body);
    fireEvent.mouseDown(document.body);
    fireEvent.pointerUp(document.body);
    fireEvent.click(document.body);

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

    const row = screen.getByRole("row", { name: /CVE-2026-1000/ });
    const findingCell = screen.getByRole("rowheader", { name: /CVE-2026-1000/ });
    fireEvent.pointerDown(findingCell, { pointerId: 1, pointerType: "mouse", isPrimary: true, button: 0 });
    fireEvent.mouseDown(findingCell, { button: 0 });
    await waitFor(() => expect(row).toHaveAttribute("aria-selected", "true"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.pointerUp(findingCell, { pointerId: 1, pointerType: "mouse", isPrimary: true, button: 0 });
    fireEvent.mouseUp(findingCell, { button: 0 });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.click(findingCell, { button: 0 });
    expect(await screen.findByRole("dialog")).toHaveTextContent("Router issue");
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("Router issue"));

    fireEvent.click(screen.getByRole("button", { name: "Close inspector" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    fireEvent.keyDown(screen.getByRole("row", { name: /CVE-2026-1000/ }), { key: "Enter" });
    expect(await screen.findByRole("dialog")).toHaveTextContent("Router issue");
  });

  it("switches to the board without refetching or duplicating canonical findings", async () => {
    renderWorkspace("all-open", 0, true);
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    expect(listReviewFindings).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "Board" }));
    expect(screen.getByTestId("review-toolbar")).toHaveClass("min-w-0", "xl:grid-cols-[minmax(220px,1fr)_auto_180px_auto_auto]");
    expect(await screen.findByRole("region", { name: /Needs review, 1 finding/ })).toHaveClass("min-w-0");
    expect(screen.getByRole("region", { name: /Reviewed, 0 findings/ })).toHaveClass("min-w-0");
    expect(screen.getByRole("region", { name: /Snoozed, 0 findings/ })).toHaveClass("min-w-0");
    expect(screen.getByTestId("review-board-lanes")).toHaveStyle({ gridTemplateColumns: "minmax(10rem, 1fr) minmax(10rem, 1fr) minmax(10rem, 1fr)" });
    expect(screen.getByRole("grid", { name: "Reviewed findings" })).toHaveTextContent("Drop into Reviewed");
    const boardCard = screen.getByRole("row", { name: /CVE-2026-1000 Router issue/ });
    expect(boardCard).toHaveStyle({ height: "138px" });
    expect(boardCard.querySelector("p")).toHaveTextContent("Router issue");
    expect(boardCard.querySelector('[data-testid="review-card-repository-icon"]')).toHaveAttribute("aria-label", "Local repository");
    expect(boardCard.querySelector('[data-testid="review-card-repository-icon"]')).toHaveAttribute("data-repository-source", "local");
    expect(boardCard).toHaveTextContent(/CVE-2026-1000.*cvec/);
    expect(boardCard).toHaveTextContent(/router.*1\.0\.0.*1\.0\.1/);
    expect(screen.getByText("1.0.1").closest("p")).toHaveClass("border-t");
    expect(screen.getAllByText("CVE-2026-1000")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: /Customize/ }));
    await chooseSelectOption("Density", "Compact");
    expect(boardCard).toHaveStyle({ height: "108px" });
    fireEvent.click(screen.getByRole("checkbox", { name: "Review state" }));
    expect(boardCard).toHaveStyle({ height: "132px" });
    expect(boardCard).toHaveTextContent(/unreviewed.*router.*1\.0\.0.*1\.0\.1/);
    fireEvent.click(boardCard);
    expect(boardCard).toHaveAttribute("aria-selected", "true");
    expect(boardCard).toHaveClass("review-board-card-selected");
    expect(boardCard.className).not.toContain("inset_3px");
    expect(listReviewFindings).toHaveBeenCalledTimes(1);
  });

  it("distinguishes local and GitHub repositories on board cards", async () => {
    const githubFinding = { ...finding, repoId: 2, repoName: "remote-app", repoSourceType: "github" as const, finding: { ...finding.finding, fingerprint: "fp-github", advisoryId: "GHSA-remote-1234", title: "Remote issue" } };
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([finding, githubFinding]));
    renderWorkspace();
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(screen.getByRole("button", { name: "Board" }));
    const repositoryIcons = await screen.findAllByTestId("review-card-repository-icon");
    expect(repositoryIcons.map((icon) => icon.getAttribute("data-repository-source"))).toEqual(["local", "github"]);
    expect(screen.getByRole("img", { name: "Local repository" })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "GitHub repository" })).toBeInTheDocument();
  });

  it("reveals a clipped board title on focus and hover without opening the inspector", async () => {
    const title = "Router has a long advisory title that is clipped in the compact board";
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([{ ...finding, finding: { ...finding.finding, title } }]));
    renderWorkspace("all-open");
    const card = await screen.findByRole("row", { name: `CVE-2026-1000 ${title}` });
    const heading = screen.getByText(title);
    Object.defineProperties(heading, {
      clientHeight: { configurable: true, value: 20 },
      scrollHeight: { configurable: true, value: 60 },
    });

    const user = userEvent.setup();
    await user.tab();
    act(() => card.focus());
    expect(await screen.findAllByText(title)).toHaveLength(2);
    expect(card).toHaveFocus();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.getAllByText(title)).toHaveLength(1));
    expect(card).toHaveFocus();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await user.hover(card);
    await waitFor(() => expect(screen.getAllByText(title)).toHaveLength(2));
    await user.unhover(card);
    await waitFor(() => expect(screen.getAllByText(title)).toHaveLength(1));

    Object.defineProperty(heading, "scrollHeight", { value: 20 });
    act(() => screen.getByRole("button", { name: "Table" }).focus());
    act(() => card.focus());
    expect(screen.getAllByText(title)).toHaveLength(1);
  });

  it("restores a visited Review view from the session cache", async () => {
    vi.mocked(listReviewFindings).mockImplementation(async (query) => query?.filters?.severities.includes("CRITICAL") ? reviewPage([]) : reviewPage([finding]));
    renderWorkspace();
    expect(await screen.findByRole("row", { name: /CVE-2026-1000/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Critical" }));
    expect(await screen.findByRole("heading", { name: "No findings in this view" })).toBeInTheDocument();
    expect(listReviewFindings).toHaveBeenCalledTimes(2);
    fireEvent.click(screen.getByRole("button", { name: "Needs review" }));
    expect(await screen.findByRole("row", { name: /CVE-2026-1000/ })).toBeInTheDocument();
    expect(listReviewFindings).toHaveBeenCalledTimes(2);
  });

  it("projects a semantic move immediately while the local command is pending", async () => {
    let completeMove!: () => void;
    vi.mocked(markFindingReviewed).mockImplementationOnce(() => new Promise((resolve) => { completeMove = () => resolve({} as never); }));
    renderWorkspace("all-open");
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(await screen.findByRole("button", { name: "Actions for CVE-2026-1000" }));
    fireEvent.click(await screen.findByRole("button", { name: "Move to Reviewed" }));
    await waitFor(() => expect(markFindingReviewed).toHaveBeenCalledWith(1, "fp-1", expect.objectContaining({ scanRunId: 10 })));
    expect(screen.getByRole("region", { name: "Needs review, 0 findings" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Reviewed, 1 finding" })).toBeInTheDocument();
    expect(screen.getByRole("grid", { name: "Reviewed findings" })).toHaveTextContent("CVE-2026-1000");
    expect(screen.getByRole("button", { name: "Drag CVE-2026-1000" })).toHaveClass("cursor-wait");
    expect(listReviewFindings).toHaveBeenCalledTimes(1);
    expect(returnFindingToReview).not.toHaveBeenCalled();
    completeMove();
    await waitFor(() => expect(screen.getByRole("button", { name: "Drag CVE-2026-1000" })).toHaveClass("cursor-grab"));
    expect(listReviewFindings).toHaveBeenCalledTimes(1);
  });

  it("keeps a moved finding visible in its destination workflow lane", async () => {
    let currentFinding = finding;
    vi.mocked(listReviewFindings).mockImplementation(async () => reviewPage([currentFinding]));
    vi.mocked(markFindingReviewed).mockImplementationOnce(async () => {
      currentFinding = { ...currentFinding, reviewState: "reviewed", reviewedAt: "2026-08-12T09:00:00Z", finding: { ...currentFinding.finding, reviewState: "reviewed", reviewedAt: "2026-08-12T09:00:00Z" } };
      return {} as never;
    });

    renderWorkspace("all-open");
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    expect(vi.mocked(listReviewFindings).mock.calls[0][0]?.filters.reviewStates).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: "Actions for CVE-2026-1000" }));
    fireEvent.click(await screen.findByRole("button", { name: "Move to Reviewed" }));

    await waitFor(() => expect(screen.getByRole("region", { name: "Reviewed, 1 finding" })).toBeInTheDocument());
    expect(screen.getByRole("grid", { name: "Reviewed findings" })).toHaveTextContent("CVE-2026-1000");
    expect(screen.getByRole("region", { name: "Needs review, 0 findings" })).toBeInTheDocument();
  });

  it("moves a card with the board-owned mouse pointer path exactly once", async () => {
    let currentFinding = finding;
    vi.mocked(listReviewFindings).mockImplementation(async () => reviewPage([currentFinding]));
    vi.mocked(markFindingReviewed).mockImplementationOnce(async () => {
      currentFinding = { ...currentFinding, reviewState: "reviewed", reviewedAt: "2026-08-12T09:00:00Z", finding: { ...currentFinding.finding, reviewState: "reviewed", reviewedAt: "2026-08-12T09:00:00Z" } };
      return {} as never;
    });

    renderWorkspace("all-open");
    const sourceCard = await screen.findByRole("row", { name: /CVE-2026-1000/ });
    vi.spyOn(sourceCard, "getBoundingClientRect").mockReturnValue({ left: 100, right: 700, top: 300, bottom: 476, width: 600, height: 176, x: 100, y: 300, toJSON: () => ({}) });
    const reviewedLane = screen.getByRole("region", { name: "Reviewed, 0 findings" });
    const reviewedBounds = vi.spyOn(reviewedLane, "getBoundingClientRect").mockReturnValue({ left: 800, right: 1400, top: 250, bottom: 1000, width: 600, height: 750, x: 800, y: 250, toJSON: () => ({}) });

    expect(sourceCard).toHaveAttribute("draggable", "true");
    fireEvent.pointerDown(sourceCard, { pointerId: 7, pointerType: "mouse", isPrimary: true, button: 0, clientX: 300, clientY: 350 });
    expect(document.documentElement).toHaveAttribute("data-review-pointer-phase", "armed-card");
    expect(sourceCard).toHaveAttribute("draggable", "false");
    fireEvent.pointerMove(window, { pointerId: 7, pointerType: "mouse", isPrimary: true, buttons: 1, clientX: 1000, clientY: 500 });
    for (let offset = 1; offset <= 20; offset += 1) {
      fireEvent.pointerMove(window, { pointerId: 7, pointerType: "mouse", isPrimary: true, buttons: 1, clientX: 1000 + offset, clientY: 500 + offset });
    }

    expect(screen.getByTestId("review-pointer-drag-preview").firstElementChild).toHaveStyle({ height: sourceCard.style.height });
    expect(sourceCard).toHaveTextContent("Moving…");
    expect(sourceCard).toHaveClass("cursor-grabbing");
    expect(document.documentElement).toHaveAttribute("data-review-pointer-phase", "dragging");
    expect(reviewedLane).toHaveAttribute("data-magnetic-target", "true");
    expect(reviewedLane).toHaveTextContent("Release to move to Reviewed");
    expect(reviewedBounds).toHaveBeenCalledTimes(1);
    fireEvent.pointerUp(window, { pointerId: 7, pointerType: "mouse", isPrimary: true, button: 0, clientX: 1000, clientY: 500 });
    fireEvent.pointerUp(window, { pointerId: 7, pointerType: "mouse", isPrimary: true, button: 0, clientX: 1000, clientY: 500 });
    expect(document.documentElement).not.toHaveAttribute("data-review-pointer-phase");
    expect(sourceCard).toHaveAttribute("draggable", "true");

    await waitFor(() => expect(markFindingReviewed).toHaveBeenCalledWith(1, "fp-1", expect.objectContaining({ scanRunId: 10 })));
    expect(markFindingReviewed).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByRole("region", { name: "Reviewed, 1 finding" })).toBeInTheDocument());
    expect(screen.getByRole("grid", { name: "Reviewed findings" })).toHaveTextContent("CVE-2026-1000");
  });

  it("does not move before the pointer threshold and cancels an active pointer drag with Escape", async () => {
    renderWorkspace("all-open");
    const sourceCard = await screen.findByRole("row", { name: /CVE-2026-1000/ });
    vi.spyOn(sourceCard, "getBoundingClientRect").mockReturnValue({ left: 100, right: 700, top: 300, bottom: 476, width: 600, height: 176, x: 100, y: 300, toJSON: () => ({}) });
    const reviewedLane = screen.getByRole("region", { name: "Reviewed, 0 findings" });
    vi.spyOn(reviewedLane, "getBoundingClientRect").mockReturnValue({ left: 800, right: 1400, top: 250, bottom: 1000, width: 600, height: 750, x: 800, y: 250, toJSON: () => ({}) });
    fireEvent.pointerDown(sourceCard, { pointerId: 8, pointerType: "mouse", isPrimary: true, button: 0, clientX: 300, clientY: 350 });
    expect(document.documentElement).toHaveAttribute("data-review-pointer-phase", "armed-card");
    expect(sourceCard).toHaveAttribute("draggable", "false");
    fireEvent.pointerMove(window, { pointerId: 8, pointerType: "mouse", isPrimary: true, buttons: 1, clientX: 303, clientY: 352 });
    expect(screen.queryByTestId("review-pointer-drag-preview")).not.toBeInTheDocument();
    expect(document.documentElement).toHaveAttribute("data-review-pointer-phase", "armed-card");
    fireEvent.pointerCancel(window, { pointerId: 8, pointerType: "mouse", isPrimary: true, clientX: 303, clientY: 352 });
    expect(document.documentElement).not.toHaveAttribute("data-review-pointer-phase");
    expect(sourceCard).toHaveAttribute("draggable", "true");
    expect(markFindingReviewed).not.toHaveBeenCalled();

    fireEvent.pointerDown(sourceCard, { pointerId: 9, pointerType: "mouse", isPrimary: true, button: 0, clientX: 300, clientY: 350 });
    fireEvent.pointerMove(window, { pointerId: 9, pointerType: "mouse", isPrimary: true, buttons: 1, clientX: 1000, clientY: 500 });
    expect(screen.getByTestId("review-pointer-drag-preview")).toBeInTheDocument();
    expect(document.documentElement).toHaveAttribute("data-review-pointer-phase", "dragging");
    expect(reviewedLane).toHaveAttribute("data-magnetic-target", "true");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTestId("review-pointer-drag-preview")).not.toBeInTheDocument();
    expect(document.documentElement).not.toHaveAttribute("data-review-pointer-phase");
    expect(sourceCard).toHaveAttribute("draggable", "true");
    expect(reviewedLane).not.toHaveAttribute("data-magnetic-target");
    expect(markFindingReviewed).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.getByRole("row", { name: /CVE-2026-1000/ })).toHaveFocus());
  });

  it("routes a pointer drop into Snoozed through the required confirmation", async () => {
    renderWorkspace("all-open");
    const sourceCard = await screen.findByRole("row", { name: /CVE-2026-1000/ });
    vi.spyOn(sourceCard, "getBoundingClientRect").mockReturnValue({ left: 100, right: 700, top: 300, bottom: 476, width: 600, height: 176, x: 100, y: 300, toJSON: () => ({}) });
    const snoozedLane = screen.getByRole("region", { name: "Snoozed, 0 findings" });
    vi.spyOn(snoozedLane, "getBoundingClientRect").mockReturnValue({ left: 1450, right: 2050, top: 250, bottom: 1000, width: 600, height: 750, x: 1450, y: 250, toJSON: () => ({}) });
    fireEvent.pointerDown(sourceCard, { pointerId: 10, pointerType: "mouse", isPrimary: true, button: 0, clientX: 300, clientY: 350 });
    fireEvent.pointerMove(window, { pointerId: 10, pointerType: "mouse", isPrimary: true, buttons: 1, clientX: 1700, clientY: 500 });
    expect(snoozedLane).toHaveAttribute("data-magnetic-target", "true");
    fireEvent.pointerUp(window, { pointerId: 10, pointerType: "mouse", isPrimary: true, button: 0, clientX: 1700, clientY: 500 });

    expect(await screen.findByRole("heading", { name: "Snooze finding" })).toBeInTheDocument();
    expect(snoozeFinding).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("region", { name: "Needs review, 1 finding" })).toBeInTheDocument();
  });

  it("opens the inspector without claiming a card pointer before the drag threshold", async () => {
    renderWorkspace("all-open");
    const sourceCard = await screen.findByRole("row", { name: /CVE-2026-1000/ });
    vi.spyOn(sourceCard, "getBoundingClientRect").mockReturnValue({ left: 100, right: 700, top: 300, bottom: 476, width: 600, height: 176, x: 100, y: 300, toJSON: () => ({}) });
    const board = screen.getByRole("region", { name: "Review board" });
    const setPointerCapture = vi.fn();
    Object.defineProperty(board, "setPointerCapture", { configurable: true, value: setPointerCapture });

    expect(fireEvent.pointerDown(sourceCard, { pointerId: 12, pointerType: "mouse", isPrimary: true, button: 0, clientX: 300, clientY: 350 })).toBe(true);
    expect(document.documentElement).toHaveAttribute("data-review-pointer-phase", "armed-card");
    expect(sourceCard).toHaveAttribute("draggable", "false");
    expect(setPointerCapture).not.toHaveBeenCalled();
    fireEvent.pointerMove(window, { pointerId: 12, pointerType: "mouse", isPrimary: true, buttons: 1, clientX: 303, clientY: 352 });
    expect(document.documentElement).toHaveAttribute("data-review-pointer-phase", "armed-card");
    expect(setPointerCapture).not.toHaveBeenCalled();
    fireEvent.pointerUp(sourceCard, { pointerId: 12, pointerType: "mouse", isPrimary: true, button: 0, clientX: 303, clientY: 352 });
    expect(document.documentElement).not.toHaveAttribute("data-review-pointer-phase");
    expect(sourceCard).toHaveAttribute("draggable", "true");
    fireEvent.click(screen.getByRole("row", { name: /CVE-2026-1000/ }));
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
  });

  it("keeps embedded card controls out of the surface drag path", async () => {
    renderWorkspace("all-open");
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    const actionButton = screen.getByRole("button", { name: "Actions for CVE-2026-1000" });

    fireEvent.pointerDown(actionButton, { pointerId: 11, pointerType: "mouse", isPrimary: true, button: 0, clientX: 620, clientY: 324 });
    fireEvent.pointerMove(window, { pointerId: 11, pointerType: "mouse", isPrimary: true, buttons: 1, clientX: 1000, clientY: 500 });
    fireEvent.pointerUp(window, { pointerId: 11, pointerType: "mouse", isPrimary: true, button: 0, clientX: 1000, clientY: 500 });
    expect(screen.queryByTestId("review-pointer-drag-preview")).not.toBeInTheDocument();
    expect(document.documentElement).not.toHaveAttribute("data-review-pointer-phase");
    expect(markFindingReviewed).not.toHaveBeenCalled();
  });

  it("keeps the drag handle grabbing while armed and does not open the inspector on a short handle press", async () => {
    renderWorkspace("all-open");
    const sourceCard = await screen.findByRole("row", { name: /CVE-2026-1000/ });
    const dragHandle = screen.getByRole("button", { name: "Drag CVE-2026-1000" });

    fireEvent.pointerDown(dragHandle, { pointerId: 13, pointerType: "mouse", isPrimary: true, button: 0, clientX: 620, clientY: 324 });
    expect(document.documentElement).toHaveAttribute("data-review-pointer-phase", "armed-handle");
    expect(sourceCard).toHaveAttribute("draggable", "false");
    fireEvent.pointerUp(window, { pointerId: 13, pointerType: "mouse", isPrimary: true, button: 0, clientX: 620, clientY: 324 });

    expect(document.documentElement).not.toHaveAttribute("data-review-pointer-phase");
    expect(sourceCard).toHaveAttribute("draggable", "true");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("cleans the global pointer cursor state when the board unmounts mid-gesture", async () => {
    const workspace = renderWorkspace("all-open");
    const sourceCard = await screen.findByRole("row", { name: /CVE-2026-1000/ });

    fireEvent.pointerDown(sourceCard, { pointerId: 14, pointerType: "mouse", isPrimary: true, button: 0, clientX: 300, clientY: 350 });
    expect(document.documentElement).toHaveAttribute("data-review-pointer-phase", "armed-card");
    workspace.unmount();

    expect(document.documentElement).not.toHaveAttribute("data-review-pointer-phase");
    expect(sourceCard).toHaveAttribute("draggable", "true");
  });

  it("requires confirmation for snooze and preserves state on cancel", async () => {
    renderWorkspace("all-open");
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(await screen.findByRole("button", { name: "Actions for CVE-2026-1000" }));
    fireEvent.click(await screen.findByRole("button", { name: "Snooze…" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Snooze finding" }));
    expect(dialog.getByText("CVE-2026-1000 · cvec")).toBeVisible();
    fireEvent.change(dialog.getByLabelText("Return at"), { target: { value: "" } });
    expect(dialog.getByText("Choose a return time in the future.")).toBeVisible();
    expect(dialog.getByRole("button", { name: "Snooze" })).toBeDisabled();
    fireEvent.click(dialog.getByRole("button", { name: "Cancel" }));
    expect(snoozeFinding).not.toHaveBeenCalled();
    expect(screen.getByRole("region", { name: /Needs review, 1 finding/ })).toBeInTheDocument();
  });

  it("keeps a confirmed snooze visible in the Snoozed lane", async () => {
    let currentFinding = finding;
    vi.mocked(listReviewFindings).mockImplementation(async () => reviewPage([currentFinding]));
    vi.mocked(snoozeFinding).mockImplementationOnce(async (_repoId, _fingerprint, until, note) => {
      currentFinding = {
        ...currentFinding,
        reviewState: "snoozed",
        snoozeUntil: until,
        reviewNote: note ?? null,
        finding: { ...currentFinding.finding, reviewState: "snoozed", snoozeUntil: until, reviewNote: note ?? null },
      };
      return {} as never;
    });

    renderWorkspace("all-open");
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(screen.getByRole("button", { name: "Actions for CVE-2026-1000" }));
    fireEvent.click(await screen.findByRole("button", { name: "Snooze…" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Note (optional)" }), { target: { value: "Waiting for patch" } });
    fireEvent.click(screen.getByRole("button", { name: "Snooze" }));

    await waitFor(() => expect(snoozeFinding).toHaveBeenCalledWith(1, "fp-1", expect.any(String), "Waiting for patch", expect.objectContaining({ scanRunId: 10 })));
    await waitFor(() => expect(screen.getByRole("region", { name: "Snoozed, 1 finding" })).toBeInTheDocument());
    expect(screen.getByRole("grid", { name: "Snoozed findings" })).toHaveTextContent("CVE-2026-1000");
    expect(screen.getByRole("region", { name: "Needs review, 0 findings" })).toBeInTheDocument();
  });

  it("keeps a returned finding visible in the Needs review lane", async () => {
    let currentFinding: ReviewFinding = {
      ...finding,
      reviewState: "reviewed",
      reviewedAt: "2026-08-12T09:00:00Z",
      finding: { ...finding.finding, reviewState: "reviewed", reviewedAt: "2026-08-12T09:00:00Z" },
    };
    vi.mocked(listReviewFindings).mockImplementation(async () => reviewPage([currentFinding]));
    vi.mocked(returnFindingToReview).mockImplementationOnce(async () => {
      currentFinding = {
        ...currentFinding,
        reviewState: "unreviewed",
        reviewedAt: null,
        finding: { ...currentFinding.finding, reviewState: "unreviewed", reviewedAt: null },
      };
      return {} as never;
    });

    renderWorkspace("all-open");
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(screen.getByRole("button", { name: "Actions for CVE-2026-1000" }));
    fireEvent.click(await screen.findByRole("button", { name: "Move to Needs review" }));

    await waitFor(() => expect(returnFindingToReview).toHaveBeenCalledWith(1, "fp-1", expect.objectContaining({ scanRunId: 10, reviewState: "reviewed" })));
    await waitFor(() => expect(screen.getByRole("region", { name: "Needs review, 1 finding" })).toBeInTheDocument());
    expect(screen.getByRole("grid", { name: "Needs review findings" })).toHaveTextContent("CVE-2026-1000");
    expect(screen.getByRole("region", { name: "Reviewed, 0 findings" })).toBeInTheDocument();
  });

  it("disables movement for derived groupings", async () => {
    renderWorkspace();
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(screen.getByRole("button", { name: "Board" }));
    fireEvent.click(screen.getByRole("button", { name: /^Customize/ }));
    await chooseSelectOption("Group by", "Severity");
    expect(await screen.findAllByText("Read-only grouping")).toHaveLength(3);
    expect(screen.queryByRole("button", { name: "Actions for CVE-2026-1000" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Drag CVE-2026-1000" })).not.toBeInTheDocument();
  });

  it("uses a contained fixed-width horizontal rail for repository lanes", async () => {
    const second = { ...finding, repoId: 2, repoName: "web-app", finding: { ...finding.finding, fingerprint: "fp-2", advisoryId: "CVE-2026-2000" } };
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([finding, second]));
    renderWorkspace();
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(screen.getByRole("button", { name: "Board" }));
    fireEvent.click(screen.getByRole("button", { name: /^Customize/ }));
    await chooseSelectOption("Group by", "Repository");

    expect(screen.getByTestId("review-board-rail")).toHaveClass("overflow-x-auto", "overflow-y-hidden");
    expect(screen.getByTestId("review-board-rail")).toHaveAttribute("tabindex", "0");
    expect(screen.getByTestId("review-board-rail")).toHaveAccessibleName("Repository lanes. Scroll horizontally to see more repositories.");
    expect(screen.getByTestId("review-board-lanes")).toHaveClass("w-max", "snap-x", "snap-proximity");
    expect(screen.getByTestId("review-board-lanes")).toHaveStyle({ gridTemplateColumns: "20rem 20rem" });
  });

  it("preserves collapsed lanes while switching presentations", async () => {
    renderWorkspace();
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(screen.getByRole("button", { name: "Board" }));
    fireEvent.click(await screen.findByRole("button", { name: "Collapse Needs review" }));
    expect(screen.getByRole("button", { name: "Expand Needs review" })).toBeInTheDocument();
    expect(screen.getByTestId("review-board-lanes")).toHaveStyle({ gridTemplateColumns: "3.5rem minmax(10rem, 1fr) minmax(10rem, 1fr)" });
    fireEvent.click(screen.getByRole("button", { name: "Table" }));
    fireEvent.click(screen.getByRole("button", { name: "Board" }));
    expect(screen.getByRole("button", { name: "Expand Needs review" })).toBeInTheDocument();
  });

  it("keeps the source lane and reports a failed transition", async () => {
    const onError = vi.fn();
    vi.mocked(markFindingReviewed).mockRejectedValueOnce(new Error("write failed"));
    render(<ReviewWorkspace initialViewId="all-open" viewsState={defaultReviewViewsState()} onViewsStateChange={() => {}} scannerReady onOpenRepo={() => {}} onOpenUrl={() => {}} onError={onError} />);
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(await screen.findByRole("button", { name: "Actions for CVE-2026-1000" }));
    fireEvent.click(await screen.findByRole("button", { name: "Move to Reviewed" }));
    await waitFor(() => expect(onError).toHaveBeenCalledWith("Error: write failed"));
    expect(screen.getByRole("region", { name: /Needs review, 1 finding/ })).toBeInTheDocument();
  });

  it("exposes accessible Board collections, drag parity, and non-color movement state", async () => {
    const user = userEvent.setup();
    const { container } = renderWorkspace("all-open");
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    expect(await screen.findByRole("grid", { name: "Needs review findings" })).toHaveClass("min-w-0", "w-full", "overflow-x-hidden", "overflow-y-auto");
    const dragHandle = screen.getByRole("button", { name: "Drag CVE-2026-1000" });
    expect(dragHandle).toHaveAttribute("aria-describedby");
    expect(screen.getByRole("button", { name: "Actions for CVE-2026-1000" })).toBeInTheDocument();
    dragHandle.focus();
    await user.keyboard(" ");
    expect(screen.getByText("Moving…")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Insert (before|between|after)/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Drop on Needs review findings" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Drop on Reviewed findings" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Drop on Snoozed findings" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: "Drop on Reviewed findings" })).toHaveFocus());
    await user.keyboard("{Escape}");
    expect(await screen.findByRole("row", { name: /CVE-2026-1000/ })).toBeInTheDocument();
    expect(screen.queryByText("Moving…")).not.toBeInTheDocument();
    const results = await axe.run(container, { rules: { "color-contrast": { enabled: false } } });
    expect(results.violations.filter((violation) => ["critical", "serious"].includes(violation.impact ?? ""))).toEqual([]);
  });

  it("can roll back to the legacy renderer and re-enable v2 without changing findings", async () => {
    vi.stubEnv("VITE_CVEC_REVIEW_SURFACE_V2", "legacy");
    const legacy = renderWorkspace();
    expect(await screen.findByRole("row", { name: /CVE-2026-1000/ })).toHaveTextContent("CVE-2026-1000");
    expect(screen.queryByRole("button", { name: "Board" })).not.toBeInTheDocument();
    legacy.unmount();
    vi.unstubAllEnvs();
    renderWorkspace();
    expect(await screen.findByRole("row", { name: /CVE-2026-1000/ })).toHaveTextContent("CVE-2026-1000");
    expect(screen.getByRole("button", { name: "Board" })).toBeInTheDocument();
  });

  it("shows canonical lane totals separately from the loaded page", async () => {
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([finding], {
      total: 374,
      nextCursor: "100",
      groupCounts: {
        reviewState: [{ id: "unreviewed", label: "Needs review", count: 374 }, { id: "reviewed", label: "Reviewed", count: 0 }, { id: "snoozed", label: "Snoozed", count: 0 }],
        severity: [{ id: "CRITICAL", label: "Critical", count: 374 }, { id: "HIGH", label: "High", count: 0 }, { id: "MEDIUM", label: "Medium", count: 0 }],
        repository: [{ id: "repository:1", label: "cvec", count: 374 }],
        fixAvailability: [{ id: "available", label: "Fix available", count: 374 }, { id: "unavailable", label: "No fix listed", count: 0 }],
      },
    }));
    renderWorkspace();
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    fireEvent.click(screen.getByRole("button", { name: "Board" }));
    expect(await screen.findByRole("region", { name: "Needs review, 374 findings" })).toBeInTheDocument();
    expect(screen.getByText("of 374 findings loaded")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Load more findings" })).toBeInTheDocument();
  });
});

describe("Review scan refresh", () => {
  beforeEach(() => {
    vi.mocked(listReviewFindings).mockReset();
    reviewSessionCache(0).clear();
    vi.mocked(markFindingReviewed).mockReset().mockResolvedValue({} as never);
    vi.mocked(returnFindingToReview).mockReset().mockResolvedValue({} as never);
    vi.mocked(snoozeFinding).mockReset().mockResolvedValue({} as never);
  });

  it("retries a failed refresh inside the open finding after a stale decision is rejected", async () => {
    const newer: ReviewFinding = {
      ...finding,
      scanRunId: 11,
      finding: { ...finding.finding, title: "Router issue after scan", fixVersion: "1.0.2" },
    };
    vi.mocked(listReviewFindings)
      .mockResolvedValueOnce(reviewPage([finding]))
      .mockRejectedValueOnce(new Error("fixture database could not be read"))
      .mockResolvedValue({ ...reviewPage([newer]), revision: 1 });
    vi.mocked(getReviewFindingDetail).mockImplementation(async (_repo, _fingerprint, scanRunId) => scanRunId === 11 ? newer : finding);
    vi.mocked(markFindingReviewed).mockRejectedValueOnce("REVIEW_EVIDENCE_STALE: New evidence was saved");
    renderWorkspace("all-open", 0, true);
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    const dialog = within(await screen.findByRole("dialog"));
    fireEvent.click(dialog.getByRole("button", { name: "Mark reviewed" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent("Finding evidence could not be refreshed.");
    expect(dialog.getByRole("heading", { name: "Router issue" })).toBeVisible();
    expect(dialog.getByRole("button", { name: "Mark reviewed" })).toBeDisabled();
    expect(dialog.queryByText("Updating finding evidence…")).not.toBeInTheDocument();

    fireEvent.click(dialog.getByRole("button", { name: "Retry loading evidence" }));
    expect(await dialog.findByRole("heading", { name: "Router issue after scan" })).toBeVisible();
    expect(dialog.getByText(/The finding changed before this action was saved/)).toBeVisible();
    expect(dialog.getByRole("button", { name: "Mark reviewed" })).toBeEnabled();
    expect(markFindingReviewed).toHaveBeenCalledTimes(1);
    expect(listReviewFindings).toHaveBeenCalledTimes(3);
  });

  it("publishes a reopened snooze from the loaded page without querying that page twice", async () => {
    const summary = { ...EMPTY_SNAPSHOT.reviewSummary, needsReview: 1, dueSnoozed: 1 };
    const first = { ...reviewPage([finding]), revision: 1, reviewSummary: summary };
    vi.mocked(listReviewFindings).mockResolvedValue(first);
    const onReviewStateChange = vi.fn();
    const props = { viewsState: defaultReviewViewsState(), onViewsStateChange: vi.fn(), scannerReady: true, onOpenRepo: vi.fn(), onOpenUrl: vi.fn(), onError: vi.fn(), onReviewStateChange };
    const view = render(<ReviewWorkspace {...props} reviewRevision={0} />);
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    expect(onReviewStateChange).toHaveBeenCalledWith({ reviewSummary: summary, revision: 1 });
    view.rerender(<ReviewWorkspace {...props} reviewRevision={1} />);
    expect(listReviewFindings).toHaveBeenCalledTimes(1);

    const other = { ...finding, finding: { ...finding.finding, advisoryId: "CVE-2026-2000" } };
    vi.mocked(listReviewFindings).mockResolvedValue({ ...reviewPage([other]), revision: 2 });
    view.rerender(<ReviewWorkspace {...props} reviewRevision={2} />);
    expect(await screen.findByRole("row", { name: /CVE-2026-2000/ })).toBeInTheDocument();
    expect(listReviewFindings).toHaveBeenCalledTimes(2);
  });

  it("reloads unchanged queries when new scan data arrives", async () => {
    const initial = defaultReviewViewsState();
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([finding]));
    const props = { viewsState: initial, onViewsStateChange: () => {}, initialViewId: "all-open", scannerReady: true, onOpenRepo: () => {}, onOpenUrl: () => {}, onError: () => {} };
    const view = render(<ReviewWorkspace {...props} dataRevision={0} />);
    await screen.findByRole("row", { name: /CVE-2026-1000/ });
    const next = { ...finding, finding: { ...finding.finding, advisoryId: "CVE-2026-9999", title: "New scan evidence" } };
    vi.mocked(listReviewFindings).mockResolvedValue(reviewPage([next]));
    view.rerender(<ReviewWorkspace {...props} dataRevision={1} />);
    expect(await screen.findByRole("row", { name: /CVE-2026-9999/ })).toBeInTheDocument();
    expect(screen.queryByRole("row", { name: /CVE-2026-1000/ })).not.toBeInTheDocument();
    expect(screen.getByRole("searchbox", { name: "Search review findings" })).toHaveValue("");
  });

  it("keeps an inspected finding from a later page selected when the scan changes its sort position", async () => {
    const later = {
      ...finding,
      finding: { ...finding.finding, fingerprint: "fp-later", advisoryId: "CVE-2026-2222", title: "Later finding" },
    };
    const firstPage = reviewPage([finding], { total: 2, nextCursor: "first-page" });
    const refreshed = { ...later, scanRunId: 11, finding: { ...later.finding, title: "Later finding after scan" } };
    vi.mocked(listReviewFindings).mockImplementation(async ({ cursor }) =>
      cursor ? reviewPage([later], { total: 2 }) : firstPage);
    vi.mocked(getReviewFindingDetail).mockImplementation(async (_repoId, fingerprint, scanRunId) => {
      const item = fingerprint === later.finding.fingerprint ? later : finding;
      return scanRunId === 11 ? refreshed : item;
    });

    const props = {
      viewsState: defaultReviewViewsState(), onViewsStateChange: vi.fn(), scannerReady: true,
      onOpenRepo: vi.fn(), onOpenUrl: vi.fn(), onError: vi.fn(),
      initialContext: { viewId: "all-open", reviewPresentation: DEFAULT_TABLE_PRESENTATION, reviewQuery: { ...reviewQuery(), search: "router", pageSize: 1 } },
    };
    const view = render(<ReviewWorkspace {...props} />);
    fireEvent.click(await screen.findByRole("button", { name: "Load more findings" }));
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-2222/ }));
    expect(await screen.findByRole("dialog")).toHaveTextContent("Later finding");

    let publishRefresh!: () => void;
    vi.mocked(listReviewFindings).mockImplementation(async (_query, restore) => {
      const inserted = { ...finding, finding: { ...finding.finding, fingerprint: "fp-new", advisoryId: "CVE-2026-3333" } };
      const refreshedItems = restore?.preserveSelection === "1:fp-later" ? [finding, inserted, refreshed] : [finding];
      return new Promise((resolve) => {
        publishRefresh = () => resolve({ ...reviewPage(refreshedItems, { total: 3 }), revision: 1 });
      });
    });
    view.rerender(<ReviewWorkspace {...props} dataRevision={1} reviewRevision={1} />);

    expect(screen.getByRole("dialog")).toHaveTextContent("Later finding");
    expect(screen.getByText("Updating finding evidence…")).toBeVisible();
    expect(screen.getByRole("button", { name: "Mark reviewed" })).toBeDisabled();
    await act(async () => publishRefresh());
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("Later finding after scan"));
    expect(screen.getByRole("button", { name: "Mark reviewed" })).toBeEnabled();
    expect(listReviewFindings).toHaveBeenLastCalledWith(
      expect.objectContaining({ search: "router", cursor: null }),
      { preserveSelection: "1:fp-later", loadedRows: 2 },
    );
    await userEvent.keyboard("{Escape}");
    expect(await screen.findByRole("searchbox", { name: "Search review findings" })).toHaveValue("router");
    await waitFor(() => {
      const row = screen.getByRole("row", { name: /CVE-2026-2222/ });
      expect(row === document.activeElement || row.contains(document.activeElement)).toBe(true);
    });
  });

  it("returns to the filtered list when a scan removes the inspected finding and keeps its loaded rows", async () => {
    const earlier = { ...finding, finding: { ...finding.finding, fingerprint: "fp-earlier", advisoryId: "CVE-2026-4444", title: "Earlier finding" } };
    const remaining = { ...finding, finding: { ...finding.finding, fingerprint: "fp-remaining", advisoryId: "CVE-2026-5555", title: "Remaining finding" } };
    vi.mocked(listReviewFindings).mockImplementation(async ({ cursor }) =>
      cursor ? reviewPage([finding], { total: 2 }) : reviewPage([earlier], { total: 2, nextCursor: "more" }));
    vi.mocked(getReviewFindingDetail).mockImplementation(async () => finding);
    const props = {
      viewsState: defaultReviewViewsState(), onViewsStateChange: vi.fn(), initialViewId: "all-open", scannerReady: true,
      onOpenRepo: vi.fn(), onOpenUrl: vi.fn(), onError: vi.fn(),
    };
    const view = render(<ReviewWorkspace {...props} />);
    fireEvent.click(await screen.findByRole("button", { name: "Load more findings" }));
    fireEvent.click(await screen.findByRole("row", { name: /CVE-2026-1000/ }));
    expect(await screen.findByRole("dialog")).toHaveTextContent("Router issue");

    vi.mocked(listReviewFindings).mockImplementation(async (_query, restore) =>
      ({ ...reviewPage(restore?.loadedRows === 2 ? [earlier, remaining] : [earlier], { total: 2 }), revision: 1 }));
    view.rerender(<ReviewWorkspace {...props} dataRevision={1} reviewRevision={1} />);

    expect(await screen.findByText("The inspected finding is no longer in the loaded results.")).toBeVisible();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByRole("row", { name: /CVE-2026-4444/ })).toBeVisible();
    expect(screen.getByRole("row", { name: /CVE-2026-5555/ })).toBeVisible();
    expect(listReviewFindings).toHaveBeenCalledTimes(3);
  });
});
