import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const events = vi.hoisted(() => new Map<string, (event: { payload: unknown }) => void>());
const reviewUpdates = vi.hoisted(() => [] as Array<ReviewUpdate | null>);
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, callback: (event: { payload: unknown }) => void) => {
    events.set(name, callback);
    return () => { if (events.get(name) === callback) events.delete(name); };
  }),
}));
vi.mock("@/lib/companionState", () => ({
  companionStore: { initialize: vi.fn(async () => {}) },
  useCompanionSelector: () => ({ activeThreadId: null, index: null }),
}));
vi.mock("@/lib/api", async (importOriginal) => ({
  ...await importOriginal<Record<string, unknown>>(),
  bootstrap: vi.fn(),
  getSnapshot: vi.fn(),
  scanAll: vi.fn(),
  scanRepo: vi.fn(),
  clearNotification: vi.fn(),
  addGithubRepository: vi.fn(),
  configureGithubApp: vi.fn(),
  loadSavedGithubAuth: vi.fn(),
  listRiskTrend: vi.fn(async () => []),
  listExternalIntelFeed: vi.fn(),
  listGithubRepositories: vi.fn(),
  logoutGithub: vi.fn(),
  getExternalIntelDetail: vi.fn(),
  refreshExternalIntel: vi.fn(),
  getRepoOverview: vi.fn(),
  listRepositoryDependencies: vi.fn(),
  searchLocalEntities: vi.fn(async () => []),
}));
vi.mock("@/components/dashboard/DashboardOverview", async () => {
  const { useState } = await import("react");
  const { RiskTrendChart } = await import("@/components/charts/RiskTrendChart");
  return {
    DashboardOverview: ({ context, onIntelDemandChange }: {
      context: DashboardDataContext;
      onIntelDemandChange: (needed: boolean) => void;
    }) => {
      const [showTrend, setShowTrend] = useState(false);
      return (
        <>
          <div data-testid="review-count" data-scan-id={context.snapshot.scanActivity?.id ?? ""} data-scan-running={context.snapshot.workspaceStatus.scanRunning}>{context.snapshot.reviewSummary.needsReview}</div>
          <button onClick={() => onIntelDemandChange(true)}>Show Intelligence widgets</button>
          <output data-testid="home-intel">{context.externalIntelItems.map((item) => item.canonicalKey).join(", ")}</output>
          <button onClick={() => setShowTrend((value) => !value)}>Toggle Risk trend</button>
          {showTrend ? <RiskTrendChart source={context.riskTrend} range={context.riskTrendRange} onRangeChange={context.onRangeChange} /> : null}
          {context.snapshot.reviewSummary.failedScans ? <button onClick={context.onOpenFailedScans}>Failed scans</button> : null}
        </>
      );
    },
  };
});

vi.mock("@/components/NotificationBell", () => ({
  NotificationBell: ({ onClear }: { onClear: (id: number) => void }) => <button onClick={() => onClear(1)}>Clear test notification</button>,
}));

vi.mock("@/components/ReviewWorkspace", () => ({
  ReviewWorkspace: ({ initialContext, onReviewStateChange }: { initialContext?: WorkspaceContext; onReviewStateChange: (update: ReviewUpdate | null) => Promise<void> }) => (
    <>
      <button onClick={() => void onReviewStateChange(reviewUpdates.shift() ?? null)}>Complete test review</button>
      <output data-testid="review-fingerprint">{initialContext?.reviewQuery?.filters.fingerprint}</output>
      <output data-testid="review-view">{initialContext?.viewId}</output>
      <output data-testid="review-resolution-scan">{initialContext?.reviewQuery?.filters.resolvedInScanRunId}</output>
    </>
  ),
}));

import { listen } from "@tauri-apps/api/event";
import App from "@/App";
import { addGithubRepository, getSnapshot, bootstrap, clearNotification, configureGithubApp, getExternalIntelDetail, getRepoOverview, listExternalIntelFeed, listGithubRepositories, listRepositoryDependencies, listRiskTrend, loadSavedGithubAuth, logoutGithub, refreshExternalIntel, scanAll, scanRepo, searchLocalEntities } from "@/lib/api";
import { companionStore } from "@/lib/companionState";
import type { AppSnapshot, ExternalIntelDetail, ExternalIntelFeed, ExternalIntelFeedItem, GithubRepositoryOption, RepoFinding, RepoScanRun, ReviewUpdate, RiskTrendPoint, ScanActivity } from "@/lib/types";
import type { DashboardDataContext } from "@/components/dashboard/types";
import { emptyRepoOverview, EMPTY_SNAPSHOT, READY_SNAPSHOT } from "@/test/fixtures";
import { AppearanceProvider } from "@/components/AppearanceProvider";
import { TooltipProvider } from "@/components/ui/overlays";
import { ToastProvider } from "@/components/ui/toast";
import type { WorkspaceContext } from "@/lib/navigation";

// The real Intelligence scroll area measures its animations, which jsdom does not implement.
if (typeof Element.prototype.getAnimations === "undefined") {
  Object.defineProperty(Element.prototype, "getAnimations", { configurable: true, value: () => [] });
}

beforeEach(() => {
  events.clear();
  reviewUpdates.length = 0;
  vi.mocked(bootstrap).mockReset();
  vi.mocked(addGithubRepository).mockReset();
  vi.mocked(getSnapshot).mockReset();
  vi.mocked(scanAll).mockReset();
  vi.mocked(scanRepo).mockReset();
  vi.mocked(getRepoOverview).mockReset();
  vi.mocked(listRepositoryDependencies).mockReset();
  vi.mocked(listGithubRepositories).mockReset();
  vi.mocked(logoutGithub).mockReset();
  vi.mocked(listRiskTrend).mockReset().mockResolvedValue([]);
  vi.mocked(listExternalIntelFeed).mockReset().mockResolvedValue(intelFeed([]));
  vi.mocked(getExternalIntelDetail).mockReset().mockResolvedValue(null);
  vi.mocked(searchLocalEntities).mockReset().mockResolvedValue([]);
  vi.mocked(companionStore.initialize).mockClear();
  vi.stubGlobal("ResizeObserver", class {
    constructor(private callback: ResizeObserverCallback) {}
    observe(target: Element) {
      this.callback([{ target, contentRect: { width: 1200 } } as ResizeObserverEntry], this as unknown as ResizeObserver);
    }
    disconnect() {}
    unobserve() {}
  });
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
});

it("defers provider discovery until Chat opens", async () => {
  vi.mocked(bootstrap).mockResolvedValue({ ...READY_SNAPSHOT, repos: [] });
  render(<TooltipProvider><App /></TooltipProvider>);
  await screen.findByTestId("review-count");
  expect(companionStore.initialize).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "Chat" }));
  expect(await screen.findByText("No repositories yet")).toBeInTheDocument();
  expect(companionStore.initialize).toHaveBeenCalledTimes(1);
});

it("finds persisted scan failures without a current scan session", async () => {
  const failedRepo = { ...READY_SNAPSHOT.repos[0], id: 38, name: "Failed fixture", lastScanStatus: "failed" as const, status: "error" as const };
  vi.mocked(bootstrap).mockResolvedValue({
    ...READY_SNAPSHOT,
    repos: [...READY_SNAPSHOT.repos, failedRepo],
    reviewSummary: { ...READY_SNAPSHOT.reviewSummary, failedScans: 1 },
    scanActivity: null,
  });
  vi.mocked(getRepoOverview).mockResolvedValue(emptyRepoOverview(failedRepo));
  render(<TooltipProvider><App /></TooltipProvider>);

  fireEvent.click(await screen.findByRole("button", { name: "Failed scans" }));
  expect(await screen.findByRole("heading", { name: "Failed scans" })).toBeInTheDocument();
  expect(screen.getAllByRole("option", { name: /Failed fixture/ })).toHaveLength(1);
  expect(await screen.findByRole("tab", { name: "Scan history" })).toHaveAttribute("aria-selected", "true");
  expect(getRepoOverview).toHaveBeenCalledWith(failedRepo.id);

  fireEvent.keyDown(window, { altKey: true, key: "ArrowLeft" });
  expect(await screen.findByTestId("review-count")).toBeInTheDocument();
});

it.each([1440, 820])("opens the canonical finding and restores the repository overview at %i px", async (width) => {
  vi.stubGlobal("ResizeObserver", class {
    constructor(private callback: ResizeObserverCallback) {}
    observe(target: Element) {
      this.callback([{ target, contentRect: { width } } as ResizeObserverEntry], this as unknown as ResizeObserver);
    }
    disconnect() {}
  });
  const repository = { ...READY_SNAPSHOT.repos[0], id: 5000 + width, name: "review fixture" };
  const latestScan: RepoScanRun = {
    id: 4, status: "completed", startedAt: "2026-11-14T00:00:00Z", finishedAt: "2026-11-14T00:01:00Z",
    errorMessage: null, newMedium: 0, newHigh: 0, newCritical: 0, resolvedMedium: 0, resolvedHigh: 0,
    resolvedCritical: 0, packageCount: 2,
  };
  const findings: RepoFinding[] = ["1001", "1002"].map((id) => ({
    fingerprint: `preview-${id}`, advisoryId: `CVE-2099-${id}`, packageName: `fixture-${id}`,
    packageVersion: "1.0.0", packageType: "npm", packageUrl: null, severity: "HIGH", fixVersion: "1.0.1",
    title: `Finding ${id}`, sourceUrl: null, state: "open", intel: null,
    reviewState: "unreviewed", reviewedAt: null, snoozeUntil: null, reviewNote: null,
  }));
  vi.mocked(bootstrap).mockResolvedValue({ ...READY_SNAPSHOT, repos: [repository] });
  const overview = emptyRepoOverview(repository);
  vi.mocked(getRepoOverview).mockResolvedValue({
    ...overview, latestScan, findings, scanHistory: [latestScan],
    overview: { ...overview.overview, latestCompletedScan: latestScan, packageCount: 2, affectedCount: 2, openFindingCount: 2 },
  });
  const view = render(<TooltipProvider><App /></TooltipProvider>);
  await screen.findByTestId("review-count");
  fireEvent.click(screen.getByRole("button", { name: "Repositories" }));
  if (width === 820) fireEvent.click(await screen.findByRole("option", { name: /^review fixture/ }));
  const finding = await screen.findByRole("button", { name: /CVE-2099-1002/ });
  const pane = view.container.querySelector<HTMLElement>("[data-repository-scroll]")!;
  pane.scrollTop = 180;
  fireEvent.scroll(pane);
  fireEvent.click(finding);
  const review = await screen.findByTestId("review-fingerprint");
  expect(review).toHaveTextContent("preview-1002");
  expect(screen.getByRole("heading", { name: "Review" })).toHaveFocus();

  review.addEventListener("keydown", (event) => event.stopPropagation(), { once: true });
  fireEvent.keyDown(review, { altKey: true, key: "ArrowLeft" });
  expect(await screen.findByRole("button", { name: /CVE-2099-1002/ })).toBeVisible();
  const overviewTab = screen.getByRole("tab", { name: "Overview" });
  expect(overviewTab).toHaveAttribute("aria-selected", "true");
  expect(overviewTab).toHaveFocus();
  expect(view.container.querySelector<HTMLElement>("[data-repository-scroll]")?.scrollTop).toBe(180);
  expect(getRepoOverview).toHaveBeenCalledExactlyOnceWith(repository.id);
  expect(listRepositoryDependencies).not.toHaveBeenCalled();

  if (width === 820) {
    const user = userEvent.setup();
    await user.keyboard("{Escape}");
    const repo = await screen.findByRole("option", { name: /^review fixture/, selected: true });
    expect(repo).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("heading", { name: repository.name })).toBeVisible();
  }
});

it("configures a user GitHub App in Settings and reconnects its saved sign-in", async () => {
  vi.mocked(bootstrap).mockResolvedValue(READY_SNAPSHOT);
  const auth = {
    ...READY_SNAPSHOT.githubAuth, configured: true, deferred: true, clientId: "fixture-client", configRevision: 1,
  };
  vi.mocked(configureGithubApp).mockResolvedValueOnce(auth);
  vi.mocked(loadSavedGithubAuth).mockResolvedValueOnce({ ...auth, signedIn: true, deferred: false, login: "fixture-account" });
  render(<AppearanceProvider><TooltipProvider><App /></TooltipProvider></AppearanceProvider>);
  await screen.findByTestId("review-count");
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));

  expect(await screen.findByText(/Configure your own GitHub App/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Sign in" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Choose folder" })).toBeEnabled();

  fireEvent.click(screen.getByRole("button", { name: "Set up GitHub App" }));
  const setup = await screen.findByRole("dialog", { name: "GitHub App" });
  fireEvent.change(within(setup).getByRole("textbox", { name: "Client ID" }), { target: { value: "fixture-client" } });
  fireEvent.click(within(setup).getByRole("button", { name: "Save app" }));
  expect(configureGithubApp).toHaveBeenCalledWith("fixture-client", null);
  await screen.findByRole("button", { name: "Use saved sign-in" });
  await act(async () => events.get("cvec://state")?.({ payload: READY_SNAPSHOT }));
  expect(screen.getByRole("button", { name: "Sign in" })).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Use saved sign-in" }));
  expect(await screen.findByText("Signed in as fixture-account.")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Edit GitHub App" }));
  expect(screen.getByRole("textbox", { name: "Client ID" })).toHaveValue("fixture-client");
});

it("keeps the same import draft when a GitHub rate limit sends the user to account setup", async () => {
  vi.mocked(bootstrap).mockResolvedValue({ ...READY_SNAPSHOT, repos: [] });
  vi.mocked(addGithubRepository).mockRejectedValueOnce(new Error("GitHub API rate limit reached. Sign in with GitHub or wait until the limit resets."));
  const url = "https://github.com/example/fixture";
  render(<AppearanceProvider><TooltipProvider><App /></TooltipProvider></AppearanceProvider>);
  await screen.findByTestId("review-count");
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(await screen.findByRole("button", { name: "Import GitHub repository" }));
  const input = await screen.findByRole("textbox", { name: "GitHub repository URL" });
  fireEvent.change(input, { target: { value: url } });
  fireEvent.click(screen.getByRole("button", { name: "Add repo" }));
  const setup = await screen.findByRole("button", { name: "Set up GitHub App" });
  expect(input).toHaveValue(url);
  expect(input).toHaveAccessibleDescription(/GitHub API rate limit reached/);
  fireEvent.click(setup);
  const account = await screen.findByRole("heading", { name: "GitHub account" });
  expect(account).toHaveFocus();
  fireEvent.click(screen.getByRole("button", { name: "Return to GitHub import" }));
  expect(await screen.findByRole("textbox", { name: "GitHub repository URL" })).toHaveValue(url);
  expect(addGithubRepository).toHaveBeenCalledExactlyOnceWith(url);
});

it("discards a GitHub repository list that finishes after sign-out and a new sign-in", async () => {
  const signedOut = { ...READY_SNAPSHOT, githubAuth: { ...READY_SNAPSHOT.githubAuth, configured: true } };
  const signedIn = (login: string): AppSnapshot => ({
    ...signedOut,
    githubAuth: { ...signedOut.githubAuth, signedIn: true, login },
  });
  const repository = (owner: string): GithubRepositoryOption => ({
    owner, name: "fixture", fullName: `${owner}/fixture`, private: true,
    htmlUrl: `https://github.com/${owner}/fixture`, defaultBranch: "main",
  });
  let finishOldRequest!: (repos: GithubRepositoryOption[]) => void;
  vi.mocked(listGithubRepositories)
    .mockReturnValueOnce(new Promise((resolve) => { finishOldRequest = resolve; }))
    .mockResolvedValueOnce([repository("second-account")]);
  vi.mocked(bootstrap).mockResolvedValue(signedIn("first-account"));
  vi.mocked(logoutGithub).mockResolvedValue(signedOut);
  vi.mocked(getSnapshot).mockResolvedValue(signedOut);
  render(<AppearanceProvider><TooltipProvider><App /></TooltipProvider></AppearanceProvider>);
  await screen.findByTestId("review-count");
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(await screen.findByRole("button", { name: "Load repos" }));
  fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
  await screen.findByRole("button", { name: "Sign in" });
  await act(async () => events.get("cvec://state")?.({ payload: signedIn("second-account") }));
  await act(async () => finishOldRequest([repository("first-account")]));
  expect(screen.queryByText("first-account/fixture")).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Load repos" }));
  expect(await screen.findByText("second-account/fixture")).toBeVisible();
});

it("keeps the current GitHub session when older snapshots arrive", async () => {
  const configured = { ...READY_SNAPSHOT.githubAuth, configured: true, clientId: "fixture-client", configRevision: 1 };
  const first = { ...READY_SNAPSHOT, githubAuth: { ...configured, sessionRevision: 1, signedIn: true, login: "first-account" } };
  const signedOut = { ...READY_SNAPSHOT, githubAuth: { ...configured, sessionRevision: 2 } };
  const second = { ...first, githubAuth: { ...first.githubAuth, sessionRevision: 3, login: "second-account" } };
  vi.mocked(bootstrap).mockResolvedValue(first);
  vi.mocked(logoutGithub).mockResolvedValue(signedOut);
  vi.mocked(getSnapshot).mockResolvedValue(signedOut);
  render(<AppearanceProvider><TooltipProvider><App /></TooltipProvider></AppearanceProvider>);
  await screen.findByTestId("review-count");
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(await screen.findByRole("button", { name: "Sign out" }));
  await screen.findByRole("button", { name: "Sign in" });

  await act(async () => events.get("cvec://state")?.({ payload: first }));
  expect(screen.getByRole("button", { name: "Sign in" })).toBeVisible();

  await act(async () => events.get("cvec://state")?.({ payload: second }));
  await act(async () => events.get("cvec://state")?.({ payload: signedOut }));
  expect(screen.getByText("Signed in as second-account.")).toBeVisible();
});

it("opens onboarding on first launch and lets Settings replay it", async () => {
  vi.mocked(bootstrap).mockResolvedValue(EMPTY_SNAPSHOT);
  render(<AppearanceProvider><TooltipProvider><App /></TooltipProvider></AppearanceProvider>);
  const welcome = await screen.findByRole("heading", { name: "Scan your repositories for vulnerabilities" });
  expect(welcome.closest('[role="dialog"]')).toHaveAccessibleName("cvec onboarding");
  await act(async () => events.get("cvec://state")?.({ payload: READY_SNAPSHOT }));
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "cvec onboarding" })).not.toBeInTheDocument());
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(await screen.findByRole("button", { name: "Replay onboarding" }));
  expect(await screen.findByRole("heading", { name: "Scan your repositories for vulnerabilities" })).toBeVisible();
});

it("opens a command palette finding by exact identity even when its normal review page is not loaded", async () => {
  const repository = READY_SNAPSHOT.repos[0];
  vi.mocked(bootstrap).mockResolvedValue(READY_SNAPSHOT);
  vi.mocked(searchLocalEntities).mockResolvedValueOnce([{
    id: `finding:${repository.id}:outside-page-one`,
    kind: "finding",
    label: "CVE-2099-3000",
    context: `${repository.name} · fixture package`,
    repoId: repository.id,
    fingerprint: "outside-page-one",
    advisoryId: null,
  }]);
  render(<TooltipProvider><App /></TooltipProvider>);
  await screen.findByTestId("review-count");
  fireEvent.keyDown(window, { key: "k", metaKey: true });
  const search = await screen.findByRole("combobox", { name: "Search commands and local data" });
  fireEvent.change(search, { target: { value: "CVE-2099-3000" } });
  fireEvent.click(await screen.findByText("CVE-2099-3000"));
  expect(await screen.findByTestId("review-fingerprint")).toHaveTextContent("outside-page-one");

  await waitFor(() => expect(search).not.toHaveFocus());
  fireEvent.keyDown(window, { altKey: true, key: "ArrowLeft" });
  expect(await screen.findByTestId("review-count")).toBeInTheDocument();
  expect(getRepoOverview).not.toHaveBeenCalled();
});

const intelSnapshot: AppSnapshot = {
  ...READY_SNAPSHOT,
  externalIntelTopics: [{ key: "npm", label: "npm", category: "Packages", totalCount: 1, watched: true, suggested: true }],
  externalIntelSummary: { ...READY_SNAPSHOT.externalIntelSummary, lastFetchedAt: "2026-08-12T08:00:00Z" },
};

const advisory: ExternalIntelFeedItem = {
  id: 21,
  canonicalKey: "CVE-2099-1001",
  title: "Fixture advisory for demo-archive",
  summary: "Affects demo-archive during archive extraction",
  packageName: "demo-archive",
  ecosystem: "npm",
  severity: "high",
  relevance: "relevant",
  productKeys: [],
  topicKeys: ["npm"],
  publishedAt: null,
  withdrawnAt: null,
  cvssScore: null,
  epssPercentage: null,
  isKev: false,
  isRansomwareKnown: false,
  sources: ["osv"],
  aliases: ["GHSA-fixture-aaaa"],
  matchKind: "package_name",
  repoCount: 1,
  cwes: [],
  kevDates: null,
};

const advisoryDetail: ExternalIntelDetail = {
  url: "https://example.com/advisory",
  updatedAt: null,
  modifiedAt: null,
  cvssVector: null,
  cvssVersion: null,
  epssPercentile: null,
  references: [],
  affectedPackages: [],
  localPackages: [],
  kev: null,
  sourceDetails: [],
};

function intelFeed(items: ExternalIntelFeedItem[], revisions = intelSnapshot.dataRevisions): ExternalIntelFeed {
  return { items, intelRevision: revisions.intel, inventoryRevision: revisions.inventory };
}

it("returns from a local advisory's dependency view to the same filtered feed after a failed refresh", async () => {
  const user = userEvent.setup();
  const repository = READY_SNAPSHOT.repos[0];
  const run: RepoScanRun = {
    id: 4, status: "completed", startedAt: "2026-11-14T00:00:00Z", finishedAt: "2026-11-14T00:01:00Z",
    errorMessage: null, newMedium: 0, newHigh: 0, newCritical: 0, resolvedMedium: 0, resolvedHigh: 0,
    resolvedCritical: 0, packageCount: 2,
  };
  const packageIdentity = { name: "demo-archive", version: "1.2.3", packageType: "npm", packageUrl: "pkg:npm/demo-archive@1.2.3" };
  const matched = { ...packageIdentity, highestSeverity: null, findingCount: 0, fixAvailable: false };
  const other = { ...matched, version: "2.0.0", packageUrl: "pkg:npm/demo-archive@2.0.0" };
  const unrelated = {
    ...advisory, id: 22, canonicalKey: "CVE-2099-1002", title: "Unrelated advisory", summary: "Other package",
    packageName: "other-package", relevance: "general" as const, repoCount: 0, matchKind: null, aliases: [],
  };
  vi.mocked(bootstrap).mockResolvedValue(intelSnapshot);
  vi.mocked(listExternalIntelFeed).mockResolvedValueOnce(intelFeed([advisory, unrelated]));
  vi.mocked(getExternalIntelDetail).mockResolvedValue({
    ...advisoryDetail,
    localPackages: [{ repoId: repository.id, repoName: repository.name, ecosystem: "npm", packageName: packageIdentity.name, package: packageIdentity }],
  });
  vi.mocked(refreshExternalIntel).mockRejectedValueOnce("external intel refresh was not saved; fixture transport failed");
  vi.mocked(getRepoOverview).mockResolvedValueOnce({
    summary: repository, latestScan: run, previousScan: null, findings: [], scanHistory: [run],
    overview: {
      latestCompletedScan: run, packageCount: 2, affectedCount: 0, openFindingCount: 0, opportunityCount: 0, opportunities: [],
    },
  });
  vi.mocked(listRepositoryDependencies).mockImplementation(async (_repoId, query) => ({
    total: query.exactPackage ? 1 : 2, inventoryCount: 2, affectedCount: 0, ecosystems: ["npm"],
    scanRunId: run.id, revision: repository.evidenceRevision ?? 0, nextCursor: null,
    items: query.exactPackage ? [matched] : [matched, other],
  }));
  class NarrowRepositoryObserver {
    constructor(private callback: ResizeObserverCallback) {}
    observe(target: Element) { this.callback([{ target, contentRect: { width: 800 } } as ResizeObserverEntry], this as unknown as ResizeObserver); }
    disconnect() {}
    unobserve() {}
  }
  vi.stubGlobal("ResizeObserver", NarrowRepositoryObserver);
  try {
    render(<TooltipProvider><App /></TooltipProvider>);
    await screen.findByTestId("review-count");
    fireEvent.click(screen.getByRole("button", { name: "Intelligence" }));
    const feed = await screen.findByRole("table", { name: "External intelligence feed" });
    expect(listExternalIntelFeed).toHaveBeenCalledTimes(1);
    expect(getExternalIntelDetail).toHaveBeenCalledWith(advisory.canonicalKey, intelSnapshot.dataRevisions.intel, intelSnapshot.dataRevisions.inventory);
    fireEvent.change(screen.getByRole("searchbox", { name: "Search external intelligence" }), { target: { value: "demo-archive" } });
    await user.click(screen.getByRole("combobox", { name: "Local relevance" }));
    await user.click(await screen.findByRole("option", { name: "Relevant" }));
    fireEvent.click(screen.getByRole("button", { name: "More filters" }));
    await user.click(screen.getByRole("combobox", { name: "Ecosystem" }));
    await user.click(await screen.findByRole("option", { name: "npm" }));
    expect(screen.getByRole("combobox", { name: "Local relevance" })).toHaveTextContent("Relevant");
    expect(screen.getByRole("combobox", { name: "Ecosystem" })).toHaveTextContent("npm");
    fireEvent.click(within(feed).getByRole("button", { name: "Risk" }));
    fireEvent.click(within(feed).getByRole("button", { name: "Risk" }));
    fireEvent.click(within(feed).getByText(advisory.title));

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    const error = await screen.findByRole("alert");
    expect(error).toHaveTextContent("Showing the last saved advisories.");
    expect(error.querySelector("details")).not.toHaveAttribute("open");
    expect(screen.getByRole("table", { name: "External intelligence feed" })).toBeInTheDocument();

    fireEvent.click(await screen.findByRole("button", { name: new RegExp(`${repository.name}.*npm.*demo-archive.*1\\.2\\.3`) }));
    const dependencies = await screen.findByRole("grid", { name: "Repository dependencies" });
    expect(screen.getByRole("tab", { name: "Dependencies" })).toHaveAttribute("aria-selected", "true");
    expect(await within(dependencies).findByText("1.2.3")).toBeVisible();
    expect(within(dependencies).queryByText("2.0.0")).not.toBeInTheDocument();
    expect(listRepositoryDependencies).toHaveBeenLastCalledWith(repository.id, expect.objectContaining({
      exactPackage: packageIdentity, expectedScanRunId: run.id,
    }));
    expect(getRepoOverview).toHaveBeenCalledWith(repository.id);
    fireEvent.click(screen.getByRole("button", { name: "Show all dependencies" }));
    expect(await within(dependencies).findByText("2.0.0")).toBeVisible();
    expect(screen.getByRole("searchbox", { name: "Search dependencies" })).toHaveFocus();
    expect(listRepositoryDependencies).toHaveBeenLastCalledWith(repository.id, expect.objectContaining({ exactPackage: undefined }));
    fireEvent.click(screen.getByRole("button", { name: "Go back" }));
    expect(await screen.findByRole("button", { name: new RegExp(`${repository.name}.*1\\.2\\.3`) })).toBeVisible();
    expect(screen.getByRole("searchbox", { name: "Search external intelligence" })).toHaveValue("demo-archive");
    expect(screen.getByRole("combobox", { name: "Local relevance" })).toHaveTextContent("Relevant");
    expect(screen.getByRole("combobox", { name: "Ecosystem" })).toHaveTextContent("npm");
    const restoredFeed = screen.getByRole("table", { name: "External intelligence feed" });
    expect(within(restoredFeed).getByRole("columnheader", { name: "Risk" })).toHaveAttribute("aria-sort", "ascending");
    expect(within(restoredFeed).getByRole("row", { name: new RegExp(advisory.title) })).toHaveAttribute("aria-selected", "true");
    expect(within(restoredFeed).queryByText(unrelated.title)).not.toBeInTheDocument();
    expect(getExternalIntelDetail).toHaveBeenCalledTimes(1);
    expect(listExternalIntelFeed).toHaveBeenCalledTimes(1);
  } finally {
    vi.unstubAllGlobals();
  }
});

it("shares the Home widget feed with Intelligence and retries a failed local read", async () => {
  vi.mocked(bootstrap).mockResolvedValue(intelSnapshot);
  vi.mocked(listExternalIntelFeed)
    .mockRejectedValueOnce("fixture read error")
    .mockResolvedValueOnce(intelFeed([advisory, {
      ...advisory, id: 22, canonicalKey: "CVE-2099-1002", withdrawnAt: "2026-08-12",
    }]));
  vi.mocked(getExternalIntelDetail).mockResolvedValueOnce(advisoryDetail);
  render(<TooltipProvider><App /></TooltipProvider>);
  await screen.findByTestId("review-count");
  fireEvent.click(screen.getByRole("button", { name: "Intelligence" }));
  const error = await screen.findByRole("alert");
  expect(error).toHaveTextContent("Could not load saved advisories");
  fireEvent.click(within(error).getByRole("button", { name: "Reload saved feed" }));
  await screen.findByRole("table", { name: "External intelligence feed" });
  expect(vi.mocked(refreshExternalIntel)).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  fireEvent.click(await screen.findByRole("button", { name: "Show Intelligence widgets" }));
  expect(screen.getByTestId("home-intel").textContent).toBe(advisory.canonicalKey);
  expect(listExternalIntelFeed).toHaveBeenCalledTimes(2);
  expect(getExternalIntelDetail).toHaveBeenCalledTimes(1);
});

it("loads one compact advisory at a time and ignores a previous detail response", async () => {
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  const other: ExternalIntelFeedItem = { ...advisory, id: 22, canonicalKey: "CVE-2099-1002", title: "Other advisory", summary: "Other description", aliases: [] };
  let resolveFirst!: (detail: ExternalIntelDetail) => void;
  vi.mocked(bootstrap).mockResolvedValue(intelSnapshot);
  vi.mocked(listExternalIntelFeed).mockResolvedValueOnce(intelFeed([advisory, other]));
  vi.mocked(getExternalIntelDetail)
    .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }))
    .mockRejectedValueOnce("local read failed")
    .mockResolvedValueOnce({ ...advisoryDetail, url: "https://example.com/other" });
  render(<TooltipProvider><App /></TooltipProvider>);
  await screen.findByTestId("review-count");
  fireEvent.click(screen.getByRole("button", { name: "Intelligence" }));
  const table = await screen.findByRole("table", { name: "External intelligence feed" });
  expect(getExternalIntelDetail).not.toHaveBeenCalled();

  const search = screen.getByRole("searchbox", { name: "Search external intelligence" });
  for (const query of ["GHSA-fixture-aaaa", "archive extraction"]) {
    fireEvent.change(search, { target: { value: query } });
    expect(within(table).getAllByRole("row")).toHaveLength(2);
    expect(within(table).getByText(advisory.title)).toBeVisible();
  }
  fireEvent.change(search, { target: { value: "" } });
  fireEvent.click(within(table).getByText(advisory.title));
  expect(screen.getByRole("heading", { name: advisory.title })).toHaveFocus();
  expect(screen.getByText("Loading advisory details…")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Back to feed" }));
  expect(within(table).getByRole("row", { name: new RegExp(advisory.title) })).toHaveFocus();

  fireEvent.click(within(table).getByText(other.title));
  fireEvent.click(await screen.findByRole("button", { name: "Retry details" }));
  const source = await screen.findByRole("link", { name: "Open OSV advisory" });
  expect(source).toHaveAttribute("href", "https://example.com/other");
  await act(async () => resolveFirst(advisoryDetail));
  expect(screen.getByRole("link", { name: "Open OSV advisory" })).toHaveAttribute("href", "https://example.com/other");
  expect(getExternalIntelDetail).toHaveBeenCalledTimes(3);
});

it("opens the exact withdrawn advisory from the command palette across a feed refresh", async () => {
  const withdrawn: ExternalIntelFeedItem = {
    ...advisory, id: 22, canonicalKey: "CVE-2099-1002", title: "Withdrawn fixture", withdrawnAt: "2026-08-12",
  };
  const next = { ...intelSnapshot, dataRevisions: { ...intelSnapshot.dataRevisions, intel: intelSnapshot.dataRevisions.intel + 1 } };
  vi.mocked(bootstrap).mockResolvedValue(intelSnapshot);
  vi.mocked(listExternalIntelFeed)
    .mockResolvedValueOnce(intelFeed([advisory, withdrawn]))
    .mockResolvedValueOnce(intelFeed([{ ...advisory, id: withdrawn.id }], next.dataRevisions));
  vi.mocked(getExternalIntelDetail).mockResolvedValueOnce(advisoryDetail);
  vi.mocked(searchLocalEntities).mockResolvedValueOnce([{
    id: "advisory:22", kind: "advisory", label: withdrawn.canonicalKey, context: withdrawn.title,
    repoId: null, fingerprint: null, advisoryId: withdrawn.id,
  }]);
  render(<TooltipProvider><App /></TooltipProvider>);
  await screen.findByTestId("review-count");
  fireEvent.keyDown(window, { key: "k", metaKey: true });
  fireEvent.change(await screen.findByRole("combobox"), { target: { value: withdrawn.canonicalKey } });
  fireEvent.click(await screen.findByText(withdrawn.canonicalKey));
  expect(await screen.findByRole("heading", { name: withdrawn.title })).toHaveFocus();
  expect(await screen.findByText(/The source withdrew this advisory/)).toBeVisible();
  expect(getExternalIntelDetail).toHaveBeenCalledWith(withdrawn.canonicalKey, intelSnapshot.dataRevisions.intel, intelSnapshot.dataRevisions.inventory);

  await act(async () => events.get("cvec://state")?.({ payload: next }));
  expect(await screen.findByRole("heading", { name: "Advisory no longer in the saved feed" })).toHaveFocus();
  expect(getExternalIntelDetail).toHaveBeenCalledTimes(1);
});

describe("workspace background updates", () => {
  const runningScan: ScanActivity = {
    id: "scan-test", sequence: 1, status: "running", stage: "preparing_source", scopeRepoId: 1,
    startedAt: "2026-10-08T16:00:00.002Z", finishedAt: null,
    totalRepositories: 1, completedRepositories: 0, failedRepositories: 0,
    currentRepoId: 1, currentRepoName: "Fixture", outcomes: [], warnings: [],
    newFindings: 0, resolvedFindings: 0, worsenedFindings: 0, terminalError: null,
  };

  it("returns from scan history in the same repository to the filtered dependency page", async () => {
    const repository = { ...READY_SNAPSHOT.repos[0], id: 961, name: "workspace fixture", evidenceRevision: 6 };
    const run: RepoScanRun = {
      id: 962, status: "completed", startedAt: runningScan.startedAt, finishedAt: runningScan.startedAt,
      errorMessage: null, newMedium: 0, newHigh: 1, newCritical: 0, resolvedMedium: 0, resolvedHigh: 0,
      resolvedCritical: 0, packageCount: 81,
    };
    vi.mocked(bootstrap).mockResolvedValue({
      ...READY_SNAPSHOT, repos: [repository], scanActivity: {
        ...runningScan, status: "completed", stage: "completed", scopeRepoId: repository.id,
        currentRepoId: repository.id, currentRepoName: repository.name, finishedAt: run.finishedAt,
        completedRepositories: 1, newFindings: 1,
        outcomes: [{ repoId: repository.id, repoName: repository.name, scanRunId: run.id, succeeded: true,
          error: null, newFindings: 1, resolvedFindings: 0, worsenedFindings: 0 }],
      },
    });
    vi.mocked(getRepoOverview).mockResolvedValue({
      summary: repository, latestScan: run, previousScan: null, findings: [], scanHistory: [run],
      overview: { latestCompletedScan: run, packageCount: 81, affectedCount: 81, openFindingCount: 81, opportunityCount: 0, opportunities: [] },
    });
    vi.mocked(listRepositoryDependencies).mockImplementation(async (_id, query) => ({
      total: 81, inventoryCount: 81, affectedCount: 81, ecosystems: ["npm"],
      nextCursor: query.cursor ? null : "second-page", scanRunId: run.id, revision: 6,
      items: [{ name: query.cursor ? "router-second-page" : "router-first-page", version: "1.0.0", packageType: "npm",
        packageUrl: null, highestSeverity: "HIGH", findingCount: 1, fixAvailable: false }],
    }));
    const view = render(<TooltipProvider><App /></TooltipProvider>);
    await screen.findByTestId("review-count");
    fireEvent.click(screen.getByRole("button", { name: "Repositories" }));
    fireEvent.click(await screen.findByRole("tab", { name: "Dependencies" }));
    await screen.findByRole("button", { name: /router-first-page/ });
    fireEvent.click(screen.getByRole("button", { name: "Affected only" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Next" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    await screen.findByRole("button", { name: /router-second-page/ });
    const pane = view.container.querySelector<HTMLElement>("[data-repository-scroll]")!;
    pane.scrollTop = 180;
    fireEvent.scroll(pane);
    expect(listRepositoryDependencies).toHaveBeenCalledTimes(3);

    fireEvent.click(screen.getByRole("button", { name: "Scan activity: Completed" }));
    fireEvent.click(await screen.findByRole("button", { name: "Scan history" }));
    expect(await screen.findByRole("heading", { name: "Scan #962" })).toHaveFocus();

    const repositorySearch = screen.getByRole("searchbox", { name: "Search repositories" });
    repositorySearch.focus();
    fireEvent.keyDown(repositorySearch, { altKey: true, key: "ArrowLeft" });
    expect(screen.getByRole("heading", { name: "Scan #962" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Go back" }));
    expect(await screen.findByRole("button", { name: /router-second-page/ })).toBeVisible();
    expect(screen.getByRole("button", { name: "Affected only" })).toHaveAttribute("aria-pressed", "true");
    expect(view.container.querySelector<HTMLElement>("[data-repository-scroll]")?.scrollTop).toBe(180);
    expect(listRepositoryDependencies).toHaveBeenCalledTimes(3);
    expect(getRepoOverview).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(screen.getByRole("tab", { name: "Dependencies" }), { altKey: true, key: "ArrowLeft" });
    expect(await screen.findByTestId("review-count")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Go back" })).toBeDisabled();
  });

  it("opens an older scan's resolutions and returns to that scan in the repository history", async () => {
    const repository = { ...READY_SNAPSHOT.repos[0], id: 971, name: "resolution fixture" };
    const resolved: RepoScanRun = {
      id: 972, status: "completed", startedAt: "2026-10-07T10:00:00Z", finishedAt: "2026-10-07T10:01:00Z",
      errorMessage: null, newMedium: 0, newHigh: 0, newCritical: 0, resolvedMedium: 0, resolvedHigh: 1,
      resolvedCritical: 0, packageCount: 1, evidenceComplete: true,
    };
    const latest = { ...resolved, id: 973, startedAt: runningScan.startedAt, finishedAt: runningScan.startedAt, resolvedHigh: 0 };
    vi.mocked(bootstrap).mockResolvedValue({ ...READY_SNAPSHOT, repos: [repository] });
    const overview = emptyRepoOverview(repository);
    vi.mocked(getRepoOverview).mockResolvedValue({
      ...overview, latestScan: latest, previousScan: resolved, scanHistory: [latest, resolved],
      overview: { ...overview.overview, latestCompletedScan: latest, packageCount: 1 },
    });

    render(<TooltipProvider><App /></TooltipProvider>);
    await screen.findByTestId("review-count");
    fireEvent.click(screen.getByRole("button", { name: "Repositories" }));
    fireEvent.click(await screen.findByRole("tab", { name: "Scan history" }));
    fireEvent.click(screen.getByRole("button", { name: "View 1 resolved finding from scan #972" }));
    expect(await screen.findByTestId("review-resolution-scan")).toHaveTextContent("972");

    fireEvent.click(screen.getByRole("button", { name: "Go back" }));
    expect(await screen.findByRole("heading", { name: "Scan #972" })).toHaveFocus();
    expect(screen.getByRole("tab", { name: "Scan history" })).toHaveAttribute("aria-selected", "true");
  });

  it("offers a scoped retry from restored scan activity once local scanners are ready", async () => {
    const repo = { ...READY_SNAPSHOT.repos[0], id: 91, name: "restored repository", lastScanStatus: "failed" as const };
    const interrupted: ScanActivity = {
      ...runningScan, id: "interrupted-92", sequence: 1, status: "failed", stage: "failed", interrupted: true,
      scopeRepoId: repo.id, currentRepoId: repo.id, currentRepoName: repo.name,
      finishedAt: "2026-10-08T16:02:00Z", failedRepositories: 1,
      outcomes: [{ repoId: repo.id, repoName: repo.name, scanRunId: 92, succeeded: false,
        error: "The application exited before this scan completed.", newFindings: 0, resolvedFindings: 0, worsenedFindings: 0 }],
    };
    const snapshot = {
      ...READY_SNAPSHOT, repos: [repo], scanActivity: interrupted,
      workspaceStatus: { ...READY_SNAPSHOT.workspaceStatus, scannerReady: false },
    };
    vi.mocked(bootstrap).mockResolvedValue(snapshot);
    vi.mocked(scanRepo).mockImplementation(() => new Promise(() => {}));
    render(<TooltipProvider><App /></TooltipProvider>);
    fireEvent.click(await screen.findByRole("button", { name: "Scan activity: Interrupted" }));
    await waitFor(() => expect(screen.getByText("CVEC closed before this scan finished")).toBeVisible());
    const retry = await screen.findByRole("button", { name: "Retry scan for restored repository" });
    expect(retry).toBeDisabled();
    expect(screen.getByRole("button", { name: "Scan history" })).toBeEnabled();

    const ready = { ...snapshot, scanActivity: { ...interrupted }, workspaceStatus: { ...snapshot.workspaceStatus, scannerReady: true } };
    await act(async () => events.get("cvec://state")?.({ payload: { ...ready, workspaceStatus: { ...ready.workspaceStatus, scanRunning: true } } }));
    expect(retry).toBeDisabled();
    await act(async () => events.get("cvec://state")?.({ payload: ready }));
    expect(retry).toBeEnabled();
    fireEvent.click(retry);
    fireEvent.click(retry);
    expect(scanRepo).toHaveBeenCalledExactlyOnceWith(repo.id);
  });

  it("opens a batch outcome's exact history and scopes the resolved view to its runs", async () => {
    const repo = { ...READY_SNAPSHOT.repos[0], id: 79, name: "Completed repo" };
    const failed = { ...repo, id: 80, name: "Failed repo", lastScanStatus: "failed" as const };
    const run: RepoScanRun = {
      id: 206, status: "failed", startedAt: runningScan.startedAt, finishedAt: runningScan.startedAt,
      errorMessage: "Fixture checkout changed", newMedium: 0, newHigh: 0, newCritical: 0,
      resolvedMedium: 0, resolvedHigh: 0, resolvedCritical: 0,
    };
    const newerRun = { ...run, id: 207, errorMessage: "Unrelated failure" };
    vi.mocked(bootstrap).mockResolvedValue({
      ...READY_SNAPSHOT,
      repos: [repo, failed],
      scanActivity: {
        ...runningScan, status: "partial_failure", stage: "completed", scopeRepoId: null,
        totalRepositories: 2, completedRepositories: 1, failedRepositories: 1, resolvedFindings: 2,
        finishedAt: runningScan.startedAt,
        outcomes: [
          { repoId: repo.id, repoName: repo.name, scanRunId: 205, succeeded: true, error: null, newFindings: 0, resolvedFindings: 2, worsenedFindings: 0 },
          { repoId: failed.id, repoName: failed.name, scanRunId: run.id, succeeded: false, error: run.errorMessage, newFindings: 0, resolvedFindings: 0, worsenedFindings: 0 },
        ],
      },
    });
    vi.mocked(getRepoOverview).mockResolvedValue({
      ...emptyRepoOverview(failed), latestScan: newerRun, scanHistory: [newerRun, run],
    });
    render(<TooltipProvider><App /></TooltipProvider>);

    const activity = await screen.findByRole("button", { name: "Scan activity: Completed with failures" });
    fireEvent.click(activity);
    fireEvent.click(await screen.findByRole("button", { name: "Scan history for Failed repo, scan #206" }));
    expect(await screen.findByRole("tab", { name: "Scan history" })).toHaveAttribute("aria-selected", "true");
    const selectedRun = await screen.findByRole("heading", { name: "Scan #206" });
    expect(within(selectedRun.closest("section")!).getByText("Fixture checkout changed")).toBeVisible();
    expect(getRepoOverview).toHaveBeenCalledWith(failed.id);

    fireEvent.click(activity);
    fireEvent.click(await screen.findByRole("button", { name: "View resolved findings" }));
    expect(await screen.findByTestId("review-view")).toHaveTextContent("scan-resolved:205");

    fireEvent.keyDown(window, { altKey: true, key: "ArrowLeft" });
    expect(await screen.findByRole("heading", { name: "Scan #206" })).toBeVisible();
  });

  it("keeps the latest scan stage and attempt when snapshots arrive out of order", async () => {
    const latestStage = { ...runningScan, sequence: 3, stage: "matching_vulnerabilities" as const };
    const started = { ...READY_SNAPSHOT, scanActivity: latestStage, workspaceStatus: { ...READY_SNAPSHOT.workspaceStatus, scanRunning: true } };
    vi.mocked(bootstrap).mockResolvedValue(started);
    render(<TooltipProvider><App /></TooltipProvider>);
    expect(await screen.findByRole("button", { name: "Scan activity: Matching vulnerabilities" })).toBeInTheDocument();
    await act(async () => events.get("cvec://state")?.({ payload: { ...started, scanActivity: runningScan } }));
    expect(screen.getByRole("button", { name: "Scan activity: Matching vulnerabilities" })).toBeInTheDocument();
    const completed = { ...latestStage, sequence: 4, status: "completed", stage: "completed" };
    await act(async () => events.get("cvec://scan-activity")?.({ payload: completed }));
    expect(screen.getByRole("button", { name: "Scan activity: Completed" })).toBeInTheDocument();
    await act(async () => events.get("cvec://scan-activity")?.({ payload: { ...runningScan, id: "retry", sequence: 5, startedAt: "2026-10-08T16:00:00.001Z" } }));
    await act(async () => events.get("cvec://state")?.({ payload: { ...started, scanActivity: completed } }));
    expect(screen.getByRole("button", { name: "Scan activity: Preparing source" })).toBeInTheDocument();
    expect(screen.getByTestId("review-count")).toHaveAttribute("data-scan-id", "retry");
  });

  it("uses the completed manual scan snapshot without a second read", async () => {
    vi.mocked(bootstrap).mockResolvedValue(READY_SNAPSHOT);
    vi.mocked(getRepoOverview).mockResolvedValue(emptyRepoOverview(READY_SNAPSHOT.repos[0]));
    const next = { ...READY_SNAPSHOT, dataRevisions: { ...READY_SNAPSHOT.dataRevisions, inventory: 1 }, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 17 } };
    vi.mocked(scanAll).mockResolvedValue({ summary: { startedAt: "now", finishedAt: "now", reposAttempted: 1, reposScanned: 1, reposFailed: 0, alertsCreated: 0 }, snapshot: next });
    render(<TooltipProvider><App /></TooltipProvider>);
    await screen.findByTestId("review-count");
    fireEvent.click(screen.getByRole("button", { name: "Repositories" }));
    fireEvent.click(await screen.findByRole("button", { name: "Scan all" }));
    await waitFor(() => expect(scanAll).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "Home" }));
    expect(await screen.findByTestId("review-count")).toHaveTextContent("17");
    expect(getSnapshot).not.toHaveBeenCalled();
  });

  it("consumes the complete snapshot and refreshes derived data after a scheduled scan", async () => {
    vi.mocked(bootstrap).mockResolvedValue({ ...READY_SNAPSHOT, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 1 } });
    render(<TooltipProvider><App /></TooltipProvider>);
    await waitFor(() => expect(screen.getByTestId("review-count")).toHaveTextContent("1"));
    const updated = {
      ...READY_SNAPSHOT,
      dataRevisions: { ...READY_SNAPSHOT.dataRevisions, inventory: 1, review: 1 },
      reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 9 },
    };
    await act(async () => {
      events.get("cvec://scan-activity")?.({ payload: {
        id: "review-scan", sequence: 2, status: "completed", stage: "completed", scopeRepoId: null,
        startedAt: "2026-10-08T16:00:00Z", finishedAt: "2026-10-08T16:01:00Z",
        totalRepositories: 1, completedRepositories: 1, failedRepositories: 0,
        outcomes: [], warnings: [], newFindings: 8, resolvedFindings: 0,
        worsenedFindings: 0, terminalError: null,
      } });
      events.get("cvec://state")?.({ payload: updated });
    });
    const reviewCount = screen.getByTestId("review-count");
    expect(reviewCount).toHaveTextContent("9");
    expect(reviewCount).toHaveAttribute("data-scan-id", "review-scan");
    await act(async () => events.get("cvec://state")?.({
      payload: {
        ...READY_SNAPSHOT,
        workspaceStatus: { ...READY_SNAPSHOT.workspaceStatus, scanRunning: true },
        reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 2 },
      },
    }));
    expect(reviewCount).toHaveTextContent("9");
    expect(reviewCount).toHaveAttribute("data-scan-running", "false");
    expect(listRiskTrend).not.toHaveBeenCalled();
    expect(listExternalIntelFeed).not.toHaveBeenCalled();
  });

  it("updates failed-scan attention and reuses the completed inventory after a failed attempt", async () => {
    const initial = {
      ...intelSnapshot,
      dataRevisions: { ...intelSnapshot.dataRevisions, scanHistory: 1 },
    };
    vi.mocked(bootstrap).mockResolvedValue(initial);
    vi.mocked(listExternalIntelFeed).mockResolvedValue(intelFeed([advisory], initial.dataRevisions));
    render(<TooltipProvider><App /></TooltipProvider>);
    fireEvent.click(await screen.findByRole("button", { name: "Show Intelligence widgets" }));
    fireEvent.click(screen.getByRole("button", { name: "Toggle Risk trend" }));
    await waitFor(() => expect(screen.getByTestId("home-intel")).toHaveTextContent(advisory.canonicalKey));
    expect(listRiskTrend).toHaveBeenCalledOnce();

    const failed = {
      ...initial,
      dataRevisions: { ...initial.dataRevisions, scanHistory: 2 },
      reviewSummary: { ...initial.reviewSummary, failedScans: 1 },
    };
    await act(async () => events.get("cvec://state")?.({ payload: failed }));
    expect(screen.getByRole("button", { name: "Failed scans" })).toBeInTheDocument();
    expect(listExternalIntelFeed).toHaveBeenCalledOnce();
    expect(listRiskTrend).toHaveBeenCalledOnce();

    await act(async () => events.get("cvec://state")?.({ payload: initial }));
    expect(screen.getByRole("button", { name: "Failed scans" })).toBeInTheDocument();

    // A review action may carry a summary read before the attempt failed.
    reviewUpdates.push({ reviewSummary: { ...initial.reviewSummary, needsReview: 3 }, revision: 1 });
    fireEvent.click(screen.getByRole("button", { name: /^Review/ }));
    fireEvent.click(await screen.findByText("Complete test review"));
    fireEvent.click(screen.getByRole("button", { name: "Home" }));
    expect(await screen.findByRole("button", { name: "Failed scans" })).toBeInTheDocument();
    expect(screen.getByTestId("review-count")).toHaveTextContent("3");
  });

  it("loads the risk chart on demand, switches ranges locally and discards pre-scan data", async () => {
    let beforeScan!: (points: RiskTrendPoint[]) => void;
    let afterScan!: (points: RiskTrendPoint[]) => void;
    vi.mocked(bootstrap).mockResolvedValue(READY_SNAPSHOT);
    vi.mocked(listRiskTrend)
      .mockImplementationOnce(() => new Promise((resolve) => { beforeScan = resolve; }))
      .mockImplementationOnce(() => new Promise((resolve) => { afterScan = resolve; }));
    render(<TooltipProvider><App /></TooltipProvider>);
    await screen.findByTestId("review-count");
    expect(listRiskTrend).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Toggle Risk trend" }));
    expect(await screen.findByText("Loading risk trend…")).toBeVisible();
    expect(listRiskTrend).toHaveBeenCalledExactlyOnceWith(30);
    fireEvent.click(screen.getByRole("button", { name: "14d" }));
    expect(screen.getByRole("button", { name: "14d" })).toHaveAttribute("aria-pressed", "true");
    expect(listRiskTrend).toHaveBeenCalledTimes(1);

    await act(async () => events.get("cvec://state")?.({
      payload: { ...READY_SNAPSHOT, dataRevisions: { ...READY_SNAPSHOT.dataRevisions, inventory: 1 } },
    }));
    await waitFor(() => expect(listRiskTrend).toHaveBeenCalledTimes(2));
    const points = Array.from({ length: 30 }, (_, index) => ({
      date: `2026-10-${String(index + 1).padStart(2, "0")}`, medium: 0, high: index + 1, critical: 0,
    }));
    const dates = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
    const labelFor = (day: number) => dates.format(new Date(2026, 9, day));
    await act(async () => afterScan(points));
    expect(screen.getByText(labelFor(17))).toBeInTheDocument();
    await act(async () => beforeScan([]));
    expect(screen.getByText(labelFor(17))).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "30d" }));
    expect(screen.getByText(labelFor(1))).toBeInTheDocument();
    expect(listRiskTrend).toHaveBeenCalledTimes(2);

    fireEvent.click(screen.getByRole("button", { name: "Toggle Risk trend" }));
    fireEvent.click(screen.getByRole("button", { name: "Toggle Risk trend" }));
    expect(await screen.findByText(labelFor(1))).toBeInTheDocument();
    expect(listRiskTrend).toHaveBeenCalledTimes(2);
  });

  it("keeps a risk read failure in the chart and retries the local read", async () => {
    vi.mocked(bootstrap).mockResolvedValue(READY_SNAPSHOT);
    vi.mocked(listRiskTrend).mockRejectedValueOnce("fixture read failure");
    render(<TooltipProvider><App /></TooltipProvider>);
    fireEvent.click(await screen.findByRole("button", { name: "Toggle Risk trend" }));
    expect(await screen.findByText("Could not load the risk trend.")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("No open findings in this range")).toBeVisible();
    expect(listRiskTrend).toHaveBeenCalledTimes(2);
  });

  it("shows the unscanned state without requesting a risk history", async () => {
    vi.mocked(bootstrap).mockResolvedValue({ ...EMPTY_SNAPSHOT, onboarding: READY_SNAPSHOT.onboarding });
    render(<TooltipProvider><App /></TooltipProvider>);
    fireEvent.click(await screen.findByRole("button", { name: "Toggle Risk trend" }));
    expect(screen.getByText("No completed scans in this range")).toBeVisible();
    expect(listRiskTrend).not.toHaveBeenCalled();
  });

  it("keeps a newer background snapshot when an older bootstrap response arrives", async () => {
    let finishBootstrap!: (value: AppSnapshot) => void;
    vi.mocked(bootstrap).mockImplementation(() => new Promise((resolve) => { finishBootstrap = resolve; }));
    const view = render(<TooltipProvider><ToastProvider><App /></ToastProvider></TooltipProvider>);
    await waitFor(() => expect(events.has("cvec://state")).toBe(true));
    const newer = { ...READY_SNAPSHOT, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 7 } };
    await act(async () => events.get("cvec://state")?.({ payload: newer }));
    await act(async () => finishBootstrap({ ...READY_SNAPSHOT, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 1 } }));
    expect(await screen.findByTestId("review-count")).toHaveTextContent("7");
    await act(async () => events.get("cvec://error")?.({ payload: "Scheduled scan failed: test failure" }));
    expect(screen.getByText("Scheduled scan failed: test failure", { selector: "h2" })).toBeInTheDocument();
    view.unmount();
    expect(events.size).toBe(0);
  });

  it("shows GitHub rate limits as a dismissible notice and expires it", async () => {
    vi.mocked(bootstrap).mockResolvedValue(READY_SNAPSHOT);
    render(<TooltipProvider><ToastProvider><App /></ToastProvider></TooltipProvider>);
    await screen.findByTestId("review-count");
    vi.useFakeTimers();
    try {
      const message = "GitHub API rate limit reached. Sign in with GitHub or wait until the limit resets.";
      await act(async () => events.get("cvec://error")?.({ payload: message }));
      expect(screen.getByText(message, { selector: "h2" })).toBeInTheDocument();
      expect(screen.getAllByLabelText("Dismiss notification").length).toBeGreaterThan(0);
      await act(async () => vi.advanceTimersByTimeAsync(6400));
      expect(screen.queryByText(message, { selector: "h2" })).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("disposes subscriptions that finish registering after the workspace unmounts", async () => {
    vi.mocked(bootstrap).mockResolvedValue(READY_SNAPSHOT);
    const ready: Array<(dispose: () => void) => void> = [];
    for (let index = 0; index < 3; index += 1) {
      vi.mocked(listen).mockImplementationOnce(() => new Promise((resolve) => ready.push(resolve)));
    }
    const view = render(<TooltipProvider><App /></TooltipProvider>);
    await waitFor(() => expect(ready).toHaveLength(3));
    view.unmount();
    const dispose = vi.fn();
    await act(async () => ready.forEach((resolve) => resolve(dispose)));
    expect(dispose).toHaveBeenCalledTimes(3);
  });
});


describe("mutation and background snapshot ordering", () => {
  it("keeps newer scan data and reconciles a late notification response", async () => {
    const initial = { ...READY_SNAPSHOT, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 1 } };
    const newer = { ...READY_SNAPSHOT, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 9 } };
    vi.mocked(bootstrap).mockResolvedValueOnce(initial);
    vi.mocked(getSnapshot).mockResolvedValue(newer);
    let finish!: (value: AppSnapshot) => void;
    vi.mocked(clearNotification).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    render(<TooltipProvider><App /></TooltipProvider>);
    await waitFor(() => expect(screen.getByTestId("review-count")).toHaveTextContent("1"));
    fireEvent.click(screen.getByText("Clear test notification"));
    await act(async () => events.get("cvec://state")?.({ payload: newer }));
    await act(async () => finish(initial));
    expect(screen.getByTestId("review-count")).toHaveTextContent("9");
    expect(getSnapshot).toHaveBeenCalledTimes(1);
  });

  it("also discards a reconciliation response overtaken by another background snapshot", async () => {
    vi.mocked(bootstrap).mockResolvedValueOnce(READY_SNAPSHOT);
    let reconcile!: (value: AppSnapshot) => void;
    vi.mocked(getSnapshot).mockImplementationOnce(() => new Promise(resolve => { reconcile = resolve; }));
    let finish!: (value: AppSnapshot) => void;
    vi.mocked(clearNotification).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    render(<TooltipProvider><App /></TooltipProvider>);
    await screen.findByTestId("review-count");
    fireEvent.click(screen.getByText("Clear test notification"));
    await act(async () => events.get("cvec://state")?.({ payload: READY_SNAPSHOT }));
    await act(async () => finish(READY_SNAPSHOT));
    await waitFor(() => expect(getSnapshot).toHaveBeenCalledTimes(1));
    const newer = { ...READY_SNAPSHOT, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 12 } };
    await act(async () => events.get("cvec://state")?.({ payload: newer }));
    await act(async () => reconcile(READY_SNAPSHOT));
    expect(screen.getByTestId("review-count")).toHaveTextContent("12");
  });
});


it("updates review counters without reading the application snapshot again", async () => {
  const initial = { ...READY_SNAPSHOT, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 9 } };
  const reviewed = { ...initial.reviewSummary, needsReview: 8 };
  vi.mocked(bootstrap).mockResolvedValueOnce(initial);
  reviewUpdates.push({ reviewSummary: reviewed, revision: 1 });
  render(<TooltipProvider><App /></TooltipProvider>);
  await screen.findByTestId("review-count");
  fireEvent.click(screen.getByRole("button", { name: /^Review/ }));
  fireEvent.click(await screen.findByText("Complete test review"));
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  expect(await screen.findByTestId("review-count")).toHaveTextContent("8");
  expect(getSnapshot).not.toHaveBeenCalled();
});

it("keeps newer review counters and accepts scan status from a delayed background snapshot", async () => {
  const initial = { ...READY_SNAPSHOT, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 9 } };
  vi.mocked(bootstrap).mockResolvedValueOnce(initial);
  reviewUpdates.push({ reviewSummary: { ...initial.reviewSummary, needsReview: 8 }, revision: 1 });
  render(<TooltipProvider><App /></TooltipProvider>);
  await screen.findByTestId("review-count");
  fireEvent.click(screen.getByRole("button", { name: /^Review/ }));
  fireEvent.click(await screen.findByText("Complete test review"));
  await act(async () => events.get("cvec://state")?.({
    payload: { ...initial, workspaceStatus: { ...initial.workspaceStatus, scanRunning: true } },
  }));
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  const counter = await screen.findByTestId("review-count");
  expect(counter).toHaveTextContent("8");
  expect(counter).toHaveAttribute("data-scan-running", "true");
});

it("keeps the latest background review counts when an older mutation response arrives", async () => {
  const initial = { ...READY_SNAPSHOT, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 9 } };
  vi.mocked(bootstrap).mockResolvedValueOnce(initial);
  reviewUpdates.push({ reviewSummary: { ...initial.reviewSummary, needsReview: 8 }, revision: 1 });
  render(<TooltipProvider><App /></TooltipProvider>);
  await screen.findByTestId("review-count");
  fireEvent.click(screen.getByRole("button", { name: /^Review/ }));
  await act(async () => events.get("cvec://state")?.({
    payload: { ...initial, dataRevisions: { ...initial.dataRevisions, review: 2 }, reviewSummary: { ...initial.reviewSummary, needsReview: 7 } },
  }));
  fireEvent.click(await screen.findByText("Complete test review"));
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  expect(await screen.findByTestId("review-count")).toHaveTextContent("7");
  expect(getSnapshot).not.toHaveBeenCalled();
});

it("scan progress during bootstrap must not discard the repository snapshot", async () => {
  let finish!: (value: AppSnapshot) => void;
  vi.mocked(bootstrap).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  render(<TooltipProvider><App /></TooltipProvider>);
  await waitFor(() => expect(events.has("cvec://scan-activity")).toBe(true));
  await act(async () => events.get("cvec://scan-activity")?.({ payload: {
    id: "running-scan", sequence: 1, status: "running", stage: "matching_vulnerabilities", scopeRepoId: null,
    startedAt: "2026-10-08T16:00:00Z", finishedAt: null,
    totalRepositories: 10, completedRepositories: 1, failedRepositories: 0,
    outcomes: [], warnings: [], newFindings: 0, resolvedFindings: 0,
    worsenedFindings: 0, terminalError: null,
  } }));
  await act(async () => finish({ ...READY_SNAPSHOT, reviewSummary: { ...READY_SNAPSHOT.reviewSummary, needsReview: 9 } }));
  expect(screen.getByTestId("review-count")).toHaveTextContent("9");
  expect(screen.getByTestId("review-count")).toHaveAttribute("data-scan-id", "running-scan");
  expect(screen.getByTestId("review-count")).toHaveAttribute("data-scan-running", "true");
});
