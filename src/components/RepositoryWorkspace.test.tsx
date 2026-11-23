import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StrictMode, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/api", () => ({
  getRepoOverview: vi.fn(),
  getRepoScanComparison: vi.fn(),
  removeRemoteRepository: vi.fn(),
  listRepositoryAssets: vi.fn(),
  listRepositoryDependencies: vi.fn(),
}));

import { getRepoOverview, getRepoScanComparison, listRepositoryAssets, listRepositoryDependencies } from "@/lib/api";
import type { DependencyPage, RepoAsset, RepoFinding, RepoOverview, RepoPackage, RepoScanRun, RepoSummary } from "@/lib/types";
import { createEvidenceReads, EvidenceReadsContext } from "@/lib/revisionReadCache";
import { RepositoryDependencies } from "./RepositoryDependencies";
import { RepositoryWorkspace } from "./RepositoryWorkspace";

class TestResizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}
  observe(target: Element) { this.callback([{ target, contentRect: { width: 1200 } } as ResizeObserverEntry], this as unknown as ResizeObserver); }
  disconnect() {}
  unobserve() {}
}

const repo = (id: number, name: string, sourceType: "local" | "github" = "local"): RepoSummary => ({
  id, name, path: `/tmp/${name}`, sourceType, sourceKey: sourceType === "github" ? `github:${name}` : null,
  remoteUrl: sourceType === "github" ? `https://github.com/cvec/${name}` : null, remoteHost: sourceType === "github" ? "github.com" : null,
  provider: sourceType, defaultBranch: "main", lastRemoteCommitSha: sourceType === "github" ? "abc123" : null,
  lastRemoteSyncedAt: null, remoteFileCount: null, status: "warning", newMedium: 0, newHigh: 1, newCritical: 0,
  openMedium: 0, openHigh: 1, openCritical: 0, resolvedMedium: 0, resolvedHigh: 0, resolvedCritical: 0,
  lastScannedAt: "2026-08-15T10:00:00Z", lastScanStatus: "completed", lastSuccessfulSourceRevision: sourceType === "github" ? "abc123" : null,
});

const finding = (fingerprint: string, packageName: string): RepoFinding => ({
  fingerprint, advisoryId: `CVE-${fingerprint}`, packageName, packageVersion: "1.0.0", packageType: "npm",
  packageUrl: `pkg:npm/${packageName}@1.0.0`, severity: "HIGH", fixVersion: "1.1.0", title: `${packageName} issue`,
  sourceUrl: null, state: "open", intel: null, reviewState: "unreviewed", reviewedAt: null, snoozeUntil: null, reviewNote: null,
});

const packageRow = (name: string): RepoPackage => ({
  name, version: "1.0.0", packageType: "npm", packageUrl: `pkg:npm/${name}@1.0.0`, highestSeverity: "HIGH", findingCount: 1, fixAvailable: true,
});

function detail(summary: RepoSummary, packageName = "router"): RepoOverview {
  const scan: RepoScanRun = {
    id: summary.id * 10, status: "completed", startedAt: "2026-08-15T09:59:00Z", finishedAt: "2026-08-15T10:00:00Z",
    errorMessage: null, newMedium: 0, newHigh: 1, newCritical: 0, resolvedMedium: 0, resolvedHigh: 0, resolvedCritical: 0,
    sourceRevision: summary.lastRemoteCommitSha, packageCount: 1, reopenedCount: 0, warnings: [], evidenceComplete: true,
  };
  const item = finding(`2026-${summary.id}`, packageName);
  return {
    summary, latestScan: scan, previousScan: null, findings: [item], scanHistory: [scan],
    overview: {
      latestCompletedScan: scan, packageCount: 1, affectedCount: 1, openFindingCount: 1, opportunityCount: 1,
      opportunities: [{
        key: packageName, packageName, packageType: item.packageType, packageUrl: item.packageUrl, installedVersion: item.packageVersion,
        fixVersion: item.fixVersion, findingCount: 1, critical: 0, high: 1, medium: 0, fingerprints: [item.fingerprint],
      }],
    },
  };
}

describe("RepositoryWorkspace", () => {
  beforeEach(() => {
    vi.stubGlobal("ResizeObserver", TestResizeObserver);
    vi.mocked(getRepoOverview).mockReset();
    vi.mocked(listRepositoryAssets).mockReset().mockResolvedValue([]);
    vi.mocked(listRepositoryDependencies).mockReset();
  });

  it("keeps repository evidence separate from Review and deep-links contextual work", async () => {
    const selected = repo(201, "frontend", "github");
    vi.mocked(getRepoOverview).mockResolvedValue(detail(selected));
    vi.mocked(listRepositoryDependencies).mockResolvedValue({
      items: [packageRow("router")], total: 1, inventoryCount: 1, affectedCount: 1, ecosystems: ["npm"],
      nextCursor: null, scanRunId: selected.id * 10, revision: 0,
    });
    const onOpenReview = vi.fn();
    render(<RepositoryWorkspace isScanning={false} repos={[selected]} selectedRepoId={selected.id} scannerReady schedule="12h" onSelectRepo={() => {}} onSnapshot={() => {}} onScanRepo={async () => {}} onError={() => {}} onOpenReview={onOpenReview} />);

    expect(await screen.findByRole("heading", { name: "Remediation opportunities" })).toBeInTheDocument();
    expect(screen.getByLabelText("1 new and 0 resolved findings").textContent).toBe("+1 / −0");
    expect(screen.queryByText("What to fix first")).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Findings" })).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Chat" })).not.toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: /router/ })[0]);
    expect(onOpenReview).toHaveBeenCalledWith(expect.objectContaining({
      repositoryId: 201,
      packageIdentity: "router",
      findingFingerprint: "2026-201",
      remediationGroup: {
        packageName: "router", packageUrl: "pkg:npm/router@1.0.0", packageType: "npm",
        installedVersion: "1.0.0", fixVersion: "1.1.0", preferredFingerprint: "2026-201",
      },
      origin: expect.objectContaining({ section: "overview" }),
    }));

    expect(listRepositoryDependencies).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("tab", { name: "Dependencies" }));
    expect(screen.getByRole("searchbox", { name: "Search dependencies" })).toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: /router/ }));
    expect(onOpenReview).toHaveBeenLastCalledWith(expect.objectContaining({
      repositoryId: selected.id,
      installedPackage: expect.objectContaining({ packageName: "router", installedVersion: "1.0.0" }),
      origin: { repositoryId: selected.id, section: "dependencies" },
    }));

    fireEvent.click(screen.getByRole("tab", { name: "Scan history" }));
    expect(screen.getByRole("button", { name: "Compare scan #2010" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "Source" }));
    expect(screen.getByRole("heading", { name: "GitHub repository" })).toBeInTheDocument();
    expect(screen.queryByText("Remote files")).not.toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Scan repo" })).toHaveLength(1);
    fireEvent.click(screen.getByRole("tab", { name: "Overview" }));
    expect(screen.getByRole("heading", { name: "Remediation opportunities" })).toBeInTheDocument();
  });

  it("loads product sources only in Source and updates them with repository evidence", async () => {
    const selected = { ...repo(202, "product-signals"), evidenceRevision: 1 };
    const docker: RepoAsset = {
      repoId: selected.id, vendor: "docker", product: "docker", version: null,
      assetKind: "repo_signal", sourcePath: "Dockerfile", evidence: "docker signal", confidence: 70,
    };
    const updated = { ...selected, evidenceRevision: 2 };
    vi.mocked(getRepoOverview).mockResolvedValueOnce(detail(selected)).mockResolvedValueOnce(detail(updated));
    vi.mocked(listRepositoryAssets)
      .mockRejectedValueOnce(new Error("database unavailable"))
      .mockResolvedValueOnce([
        docker,
        ...["build", "deploy", "release"].map((workflow) => ({ ...docker, sourcePath: `.github/workflows/${workflow}.yml` })),
      ])
      .mockResolvedValueOnce([]);

    const evidenceReads = createEvidenceReads();
    const view = (summary: RepoSummary) => (
      <EvidenceReadsContext value={evidenceReads}><RepositoryWorkspace isScanning={false} repos={[summary]} selectedRepoId={summary.id} scannerReady schedule="12h" onSelectRepo={() => {}} onSnapshot={() => {}} onScanRepo={async () => {}} onError={() => {}} onOpenReview={() => {}} /></EvidenceReadsContext>
    );
    const { rerender } = render(view(selected));

    expect(await screen.findByRole("heading", { name: "Remediation opportunities" })).toBeInTheDocument();
    expect(listRepositoryAssets).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("tab", { name: "Source" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Product signals could not be loaded.");
    fireEvent.click(screen.getByRole("button", { name: "Retry loading product signals" }));
    expect(await screen.findByText("Dockerfile")).toBeInTheDocument();
    expect(screen.getByText(".github/workflows/build.yml")).toBeInTheDocument();
    expect(screen.getAllByText("Docker")).toHaveLength(1);
    expect(screen.queryByText(".github/workflows/release.yml")).not.toBeInTheDocument();
    const showMore = screen.getByRole("button", { name: "Show more sources for Docker" });
    showMore.focus();
    fireEvent.click(showMore);
    expect(screen.getByText(".github/workflows/release.yml")).toBeInTheDocument();
    const showFewer = screen.getByRole("button", { name: "Show fewer sources for Docker" });
    expect(showFewer).toHaveFocus();
    fireEvent.click(showFewer);
    expect(screen.queryByText(".github/workflows/release.yml")).not.toBeInTheDocument();
    expect(listRepositoryAssets).toHaveBeenNthCalledWith(2, selected.id);

    fireEvent.click(screen.getByRole("tab", { name: "Overview" }));
    fireEvent.click(screen.getByRole("tab", { name: "Source" }));
    expect(await screen.findByText("Dockerfile")).toBeVisible();
    expect(listRepositoryAssets).toHaveBeenCalledTimes(2);

    rerender(view(updated));
    expect(await screen.findByText("No product signals detected.")).toBeInTheDocument();
    expect(screen.queryByText("Dockerfile")).not.toBeInTheDocument();
    expect(listRepositoryAssets).toHaveBeenCalledTimes(3);
  });

  it("rejects stale repository responses after a faster selection", async () => {
    const first = repo(301, "first");
    const second = repo(302, "second");
    let resolveFirst!: (value: RepoOverview) => void;
    let resolveSecond!: (value: RepoOverview) => void;
    vi.mocked(getRepoOverview).mockImplementation((id) => new Promise((resolve) => { if (id === first.id) resolveFirst = resolve; else resolveSecond = resolve; }));
    const props = { isScanning: false, repos: [first, second], scannerReady: true, schedule: "12h" as const, onSelectRepo: () => {}, onSnapshot: () => {}, onScanRepo: async () => {}, onError: () => {}, onOpenReview: () => {} };
    const view = render(<RepositoryWorkspace {...props} selectedRepoId={first.id} />);
    view.rerender(<RepositoryWorkspace {...props} selectedRepoId={second.id} />);
    resolveSecond(detail(second, "second-package"));
    expect((await screen.findAllByText("second-package")).length).toBeGreaterThan(0);
    resolveFirst(detail(first, "first-package"));
    await waitFor(() => expect(screen.queryByText("first-package")).not.toBeInTheDocument());
  });

  it("opens an exact older run and uses its completed comparison", async () => {
    const selected = repo(1201, "historical-scan");
    const run: RepoScanRun = {
      id: 71, status: "completed", startedAt: "2026-08-15T09:00:00Z", finishedAt: "2026-08-15T09:01:00Z",
      errorMessage: null, newMedium: 0, newHigh: 0, newCritical: 0, resolvedMedium: 0, resolvedHigh: 2,
      resolvedCritical: 0, packageCount: 12,
    };
    const latest = { ...run, id: 81, startedAt: "2026-08-15T10:00:00Z", finishedAt: "2026-08-15T10:01:00Z" };
    const current = detail(selected);
    vi.mocked(getRepoOverview).mockResolvedValue({ ...current, latestScan: latest, scanHistory: [latest], overview: { ...current.overview, latestCompletedScan: latest } });
    vi.mocked(getRepoScanComparison).mockResolvedValueOnce({ run, previousRunId: 62, previousPackageCount: 14 });
    render(<RepositoryWorkspace initialContext={{ repositorySection: "history", repositoryScanRunId: run.id, repositoryDetailOpen: true }} isScanning={false} repos={[selected]} selectedRepoId={selected.id} scannerReady schedule="off" onSelectRepo={() => {}} onSnapshot={() => {}} onScanRepo={async () => {}} onError={() => {}} onOpenReview={() => {}} />);

    expect(await screen.findByText("Package inventory −2 compared with scan #62.")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("heading", { name: "Scan #71" })).toHaveFocus());
    expect(getRepoScanComparison).toHaveBeenCalledWith(selected.id, run.id);

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.getByRole("region", { name: "Repository scan attempts" })).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "Compare scan #81" }));
    expect(screen.getByRole("heading", { name: "Scan #81" })).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.getByRole("button", { name: "Compare scan #81" })).toHaveFocus();
    expect(getRepoScanComparison).toHaveBeenCalledTimes(1);
  });
});

describe("repository evidence consistency",()=>{
 beforeEach(()=>{vi.stubGlobal("ResizeObserver",TestResizeObserver);vi.mocked(getRepoOverview).mockReset();});
 const props={isScanning:false,scannerReady:true,schedule:"12h" as const,onSelectRepo:()=>{},onSnapshot:()=>{},onScanRepo:async()=>{},onError:()=>{},onOpenReview:vi.fn()};
 it("does not present A evidence or deep-link its finding under B while B loads",async()=>{
 const a=repo(8001,"review-a"),b=repo(8002,"review-b");
 vi.mocked(getRepoOverview).mockImplementation(async(id)=>id===a.id?detail(a,"only-in-a"):new Promise(()=>{}));
 const view=render(<RepositoryWorkspace {...props} repos={[a,b]} selectedRepoId={a.id}/>);
 await screen.findAllByText("only-in-a");
 view.rerender(<RepositoryWorkspace {...props} repos={[a,b]} selectedRepoId={b.id}/>);
 expect(screen.queryAllByText("only-in-a")).toHaveLength(0);
 });
 it("invalidates pending B when switching back to cached A",async()=>{
 const a=repo(8101,"cached-a"),b=repo(8102,"pending-b");let done!:(value:RepoOverview)=>void;
 vi.mocked(getRepoOverview).mockImplementation(async(id)=>id===a.id?detail(a,"cache-package-a"):new Promise(r=>{done=r}));
 const view=render(<RepositoryWorkspace {...props} repos={[a,b]} selectedRepoId={a.id}/>);
 await screen.findAllByText("cache-package-a");
 view.rerender(<RepositoryWorkspace {...props} repos={[a,b]} selectedRepoId={b.id}/>);
 view.rerender(<RepositoryWorkspace {...props} repos={[a,b]} selectedRepoId={a.id}/>);
 await act(async()=>{done(detail(b,"late-package-b"));});
 expect(screen.queryAllByText("late-package-b")).toHaveLength(0);
 });
 it("reloads selected evidence after a completed external scan changes repo summary",async()=>{
 const a=repo(8201,"scan-a");vi.mocked(getRepoOverview).mockResolvedValue(detail(a,"before-scan"));
 const view=render(<RepositoryWorkspace {...props} repos={[a]} selectedRepoId={a.id}/>);
 await screen.findAllByText("before-scan");
 const updated={...a,lastScannedAt:"2026-10-08T16:00:00Z",openHigh:0};
 vi.mocked(getRepoOverview).mockResolvedValue(detail(updated,"after-scan"));
 view.rerender(<RepositoryWorkspace {...props} repos={[updated]} selectedRepoId={a.id}/>);
 await waitFor(()=>expect(getRepoOverview).toHaveBeenCalledTimes(2));
 });
});

describe("repository scan selection", () => {
  it("keeps the selected repository after an earlier repository scan finishes", async () => {
    vi.stubGlobal("ResizeObserver", TestResizeObserver);
    const a = repo(8301, "scan-first"), b = repo(8302, "scan-second");
    vi.mocked(getRepoOverview).mockImplementation(async (id) => detail(id === a.id ? a : b, id === a.id ? "scan-package-a" : "scan-package-b"));
    const onScanRepo = vi.fn(async () => {});
    const props = { isScanning: false, repos: [a,b], scannerReady: true, schedule: "12h" as const, onSelectRepo: () => {}, onSnapshot: () => {}, onScanRepo, onError: () => {}, onOpenReview: () => {} };
    const view = render(<RepositoryWorkspace {...props} selectedRepoId={a.id} />);
    await screen.findAllByText("scan-package-a");
    fireEvent.click(screen.getByRole("button", { name: "Scan repo" }));
    expect(onScanRepo).toHaveBeenCalledWith(a.id);
    view.rerender(<RepositoryWorkspace {...props} selectedRepoId={b.id} />);
    await screen.findAllByText("scan-package-b");
    view.rerender(<RepositoryWorkspace {...props} repos={[{ ...a, evidenceRevision: 2 }, b]} selectedRepoId={b.id} />);
    expect(screen.queryAllByText("scan-package-a")).toHaveLength(0);
    expect(screen.getAllByText("scan-package-b").length).toBeGreaterThan(0);
  });
});

it("starts at page one on a dependency search or filter change without sending the old cursor", async () => {
  vi.stubGlobal("ResizeObserver", TestResizeObserver);
  const first: DependencyPage = { items: [], total: 81, inventoryCount: 81, affectedCount: 1, ecosystems: ["npm"], nextCursor: "page-two", scanRunId: 901, revision: 1 };
  vi.mocked(listRepositoryDependencies).mockReset().mockImplementation(async (_repo, query) => ({
    ...first,
    total: query.affectedOnly || query.search ? 2 : 81,
    nextCursor: query.cursor || query.affectedOnly || query.search ? null : first.nextCursor,
  }));
  function View() {
    const [query, setQuery] = useState("");
    return <RepositoryDependencies repoId={9001} scanRunId={901} dependencyRevision={1} query={query} onQueryChange={setQuery} onOpenReview={() => {}} />;
  }
  render(<View />);
  const next = screen.getByRole("button", { name: "Next" });
  await waitFor(() => expect(next).toBeEnabled());
  fireEvent.click(next);
  expect(await screen.findByText(/Page 2/)).toBeInTheDocument();
  await waitFor(() => expect(screen.getByRole("button", { name: "Previous" })).toBeEnabled());
  fireEvent.change(screen.getByRole("searchbox", { name: "Search dependencies" }), { target: { value: "router" } });
  expect(next).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Affected only" }));
  await screen.findByText(/2 matching dependencies · Page 1/);
  const payloads = vi.mocked(listRepositoryDependencies).mock.calls.map(([, query]) => query);
  expect(payloads.map(({ search, affectedOnly, cursor }) => [search, affectedOnly, cursor])).toEqual([
    ["", false, null], ["", false, "page-two"], ["router", true, null],
  ]);
});

describe("repository dependency evidence", () => {
  const firstPage: DependencyPage = {
    items: [packageRow("first-package")],
    total: 81, inventoryCount: 81, affectedCount: 2, ecosystems: ["npm"],
    nextCursor: "page-two", scanRunId: 921, revision: 7,
  };
  const secondPage = {
    ...firstPage,
    items: [packageRow("second-package")],
    nextCursor: null,
  };
  const props = { isScanning: false, scannerReady: true, schedule: "off" as const, onSelectRepo: () => {}, onSnapshot: () => {}, onScanRepo: async () => {}, onError: () => {}, onOpenReview: () => {} };
  function overview(summary: RepoSummary, scanId: number): RepoOverview {
    const result = detail(summary);
    const scan = { ...result.latestScan!, id: scanId };
    return { ...result, latestScan: scan, scanHistory: [scan], overview: {
      latestCompletedScan: scan, packageCount: 81, affectedCount: 2, openFindingCount: 2, opportunityCount: 0, opportunities: [],
    } };
  }

  beforeEach(() => {
    vi.stubGlobal("ResizeObserver", TestResizeObserver);
    vi.mocked(getRepoOverview).mockReset();
    vi.mocked(listRepositoryDependencies).mockReset();
  });

  it("keeps the previous page available without labeling its rows as a failed next page", async () => {
    const onOpenReview = vi.fn();
    vi.mocked(listRepositoryDependencies)
      .mockResolvedValueOnce(firstPage)
      .mockRejectedValueOnce(new Error("Could not load page two"));
    render(<StrictMode><RepositoryDependencies repoId={9201} scanRunId={921} dependencyRevision={7} query="" onQueryChange={() => {}} onOpenReview={onOpenReview} /></StrictMode>);

    await screen.findByRole("button", { name: /first-package/ });
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load page two");
    expect(screen.queryByRole("button", { name: /first-package/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Previous" }));
    fireEvent.click(await screen.findByRole("button", { name: /first-package/ }));
    expect(onOpenReview).toHaveBeenCalledWith(expect.objectContaining({ name: "first-package" }));
    expect(listRepositoryDependencies).toHaveBeenCalledTimes(2);
  });

  it("keeps the dependency page after a failed scan and restarts pagination for a completed scan", async () => {
    const selected = { ...repo(9301, "cached-dependencies"), evidenceRevision: 7, dependencyRevision: 7 };
    vi.mocked(getRepoOverview).mockResolvedValueOnce(overview(selected, 921));
    vi.mocked(listRepositoryDependencies)
      .mockResolvedValueOnce(firstPage)
      .mockResolvedValueOnce(firstPage)
      .mockResolvedValueOnce(secondPage)
      .mockResolvedValue({ ...firstPage, scanRunId: 922, revision: 8 });
    const view = render(<RepositoryWorkspace {...props} selectedRepoId={selected.id} repos={[selected]} initialContext={{ repositorySection: "dependencies", repositoryPackageQuery: "package" }} />);

    await screen.findByRole("button", { name: /first-package/ });
    fireEvent.click(screen.getByRole("button", { name: "Affected only" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Next" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    await screen.findByRole("button", { name: /second-package/ });
    fireEvent.click(screen.getByRole("tab", { name: "Overview" }));
    fireEvent.click(screen.getByRole("tab", { name: "Dependencies" }));
    expect(await screen.findByRole("button", { name: /second-package/ })).toBeInTheDocument();
    expect(listRepositoryDependencies).toHaveBeenCalledTimes(3);

    const failed = { ...selected, evidenceRevision: 8, lastScanStatus: "failed" as const, status: "error" as const };
    let finishFailedOverview!: (value: RepoOverview) => void;
    vi.mocked(getRepoOverview).mockImplementationOnce(() => new Promise((resolve) => { finishFailedOverview = resolve; }));
    view.rerender(<RepositoryWorkspace {...props} selectedRepoId={selected.id} repos={[failed]} />);
    expect(screen.getByText("Updating repository evidence…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /second-package/ })).toBeVisible();
    expect(screen.getByRole("button", { name: "Previous" })).toBeEnabled();
    expect(screen.getByRole("searchbox", { name: "Search dependencies" })).toHaveValue("package");
    expect(listRepositoryDependencies).toHaveBeenCalledTimes(3);
    const failedDetail = overview(failed, 921);
    failedDetail.latestScan = { ...failedDetail.latestScan!, id: 923, status: "failed" };
    await act(async () => finishFailedOverview(failedDetail));
    expect(screen.getByRole("button", { name: "Retry scan" })).toBeEnabled();
    expect(screen.getByRole("button", { name: /second-package/ })).toBeVisible();
    expect(listRepositoryDependencies).toHaveBeenCalledTimes(3);

    const updated = { ...selected, evidenceRevision: 9, dependencyRevision: 8 };
    vi.mocked(getRepoOverview).mockResolvedValueOnce(overview(updated, 922));
    view.rerender(<RepositoryWorkspace {...props} selectedRepoId={selected.id} repos={[updated]} />);
    expect(await screen.findByRole("button", { name: /first-package/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /second-package/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Affected only" })).toHaveAttribute("aria-pressed", "true");
    expect(listRepositoryDependencies).toHaveBeenLastCalledWith(selected.id, expect.objectContaining({ search: "package", affectedOnly: true, cursor: null, expectedScanRunId: 922 }));
    expect(listRepositoryDependencies).toHaveBeenCalledTimes(4);
  });

  it("refreshes the repository overview before retrying when the latest completed scan has changed", async () => {
    const selected = { ...repo(9401, "outdated-dependency-scan"), evidenceRevision: 7 };
    let completeOverview!: (value: RepoOverview) => void;
    vi.mocked(getRepoOverview)
      .mockResolvedValueOnce(overview(selected, 921))
      .mockImplementationOnce(() => new Promise((resolve) => { completeOverview = resolve; }));
    vi.mocked(listRepositoryDependencies)
      .mockRejectedValueOnce("DEPENDENCY_CURSOR_STALE: Scan evidence changed. Refresh this repository.")
      .mockResolvedValueOnce({ ...secondPage, scanRunId: 922, revision: 8 });
    render(<RepositoryWorkspace {...props} selectedRepoId={selected.id} repos={[selected]} initialContext={{ repositorySection: "dependencies" }} />);

    fireEvent.click(await screen.findByRole("button", { name: "Refresh repository" }));
    expect(getRepoOverview).toHaveBeenCalledTimes(2);
    expect(listRepositoryDependencies).toHaveBeenCalledTimes(1);
    await act(async () => completeOverview(overview({ ...selected, evidenceRevision: 8 }, 922)));
    expect(await screen.findByRole("button", { name: /second-package/ })).toBeInTheDocument();
    expect(listRepositoryDependencies).toHaveBeenLastCalledWith(selected.id, expect.objectContaining({ expectedScanRunId: 922, cursor: null }));
    expect(listRepositoryDependencies).toHaveBeenCalledTimes(2);
  });
});
