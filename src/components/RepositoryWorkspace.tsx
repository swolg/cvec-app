import type { WorkspaceContext } from "@/lib/navigation";
import {
  ArrowClockwise,
  ArrowRight,
  CaretLeft,
  DotsThree,
  GitBranch,
  GithubLogo,
  Package,
  ShieldCheck,
  ShieldWarning,
  Trash,
} from "@phosphor-icons/react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ListBox, ListBoxItem } from "react-aria-components/ListBox";
import { ListLayout, Virtualizer } from "react-aria-components/Virtualizer";

import { RepositoryArtifacts } from "./RepositoryArtifacts";
import { RepositoryDependencies } from "./RepositoryDependencies";
import { RepositoryNugetProjects } from "./RepositoryNugetProjects";
import { RepositoryProducts } from "./RepositoryProducts";
import { RepositoryScanHistory } from "./RepositoryScanHistory";
import { AppIcon } from "@/components/ui/app-icon";
import { Badge } from "@/components/ui/badge";
import { Button, IconButton } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { EmptyState, Spinner } from "@/components/ui/feedback";
import { SearchField, Select, Tabs } from "@/components/ui/form-controls";
import { Popover, PopoverContent, PopoverTitle, PopoverTrigger } from "@/components/ui/overlays";
import { getRepoOverview, removeRemoteRepository } from "@/lib/api";
import { formatRelativeTimestamp, formatTimestamp } from "@/lib/format";
import { bundledToolLabel } from "@/lib/findingMatch";
import { deriveRepositoryMonitoring, formatCoverageWarning, RepositoryOverviewCache } from "@/lib/repositoryWorkspace";
import type {
  AppSnapshot,
  RemediationOpportunity,
  RepoOverview,
  RepoFinding,
  RepoScanRun,
  RepoSummary,
  RepositoryMonitoringState,
  ReviewOpenRequest,
  ScannedPackageIdentity,
  ScheduleInterval,
  Severity,
} from "@/lib/types";

export type DetailTab = "overview" | "dependencies" | "history" | "artifacts" | "settings";
type InventorySort = "attention" | "freshness" | "changes" | "name";

interface RepositoryWorkspaceProps {
  isScanning: boolean;
  initialContext?: WorkspaceContext;
  onContextChange?: (context: WorkspaceContext) => void;
  dataRevision?: number;
  repos: RepoSummary[];
  selectedRepoId: number | null;
  scannerReady: boolean;
  schedule: ScheduleInterval;
  onSelectRepo: (repoId: number | null) => void;
  onSnapshot: (snapshot: AppSnapshot) => void;
  onScanRepo: (repoId: number) => Promise<void>;
  onError: (message: string) => void;
  onOpenReview: (request: ReviewOpenRequest) => void;
  onShowActivity?: () => void;
}

const overviewCache = new RepositoryOverviewCache();
const inventoryLayout = { estimatedRowSize: 88 };

export function RepositoryWorkspace({
  isScanning,
  initialContext,
  onContextChange,
  dataRevision: _dataRevision = 0,
  repos,
  selectedRepoId,
  scannerReady,
  schedule,
  onSelectRepo,
  onSnapshot,
  onScanRepo,
  onError,
  onOpenReview,
  onShowActivity,
}: RepositoryWorkspaceProps) {
  const [detail, setDetail] = useState<RepoOverview | null>(null);
  const [settledRequest, setSettledRequest] = useState<{ key?: string; refreshRevision: number }>({ refreshRevision: 0 });
  const [detailError, setDetailError] = useState<string | null>(null);
  const packageLinkSequence = initialContext?.repositoryPackageLinkSequence;
  const [query, setQuery] = useState(initialContext?.repositoryQuery??"");
  const [sort, setSort] = useState<InventorySort>(initialContext?.repositorySort??"attention");
  const [tab, setTab] = useState<DetailTab>(initialContext?.repositorySection === "chat" ? "overview" : initialContext?.repositorySection ?? "overview");
  const [packageQuery, setPackageQuery] = useState(initialContext?.repositoryPackageQuery ?? "");
  const [linkedPackage, setLinkedPackage] = useState(initialContext?.repositoryExactPackage);
  const exactPackage = selectedRepoId === initialContext?.repoId ? linkedPackage : undefined;
  const [failedOnly, setFailedOnly] = useState(initialContext?.repositoryFailedOnly ?? false);
  const [scanSelection, setScanSelection] = useState({
    repoId: selectedRepoId,
    runId: initialContext?.repositoryScanRunId,
  });
  const selectedScanRunId = scanSelection.repoId === selectedRepoId ? scanSelection.runId : undefined;
  const [removingRepo, setRemovingRepo] = useState(false);
  const [narrow, setNarrow] = useState(false);
  const [narrowDetailOpen, setNarrowDetailOpen] = useState(initialContext?.repositoryDetailOpen ?? false);
  const rootRef = useRef<HTMLDivElement>(null);
  const backToList = useRef<HTMLButtonElement>(null);
  const restoreListFocus = useRef(false);
  const focusBeforeReturn = useRef(initialContext?.repositoryFocusOnReturn ? document.activeElement : null);
  const requestSequence = useRef(0);
  const selectedRepoRef = useRef(selectedRepoId);
  const onSelectRepoRef = useRef(onSelectRepo);
  selectedRepoRef.current = selectedRepoId;
  onSelectRepoRef.current = onSelectRepo;
  const [refreshRevision, setRefreshRevision] = useState(0);
  const failedRepos = useMemo(() => repos.filter((repo) => repo.lastScanStatus === "failed"), [repos]);
  const inventory = failedOnly ? failedRepos : repos;
  const visibleRepos = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const filtered = needle
      ? inventory.filter((repo) => `${repo.name} ${repo.path} ${repo.remoteHost ?? ""} ${repo.defaultBranch ?? ""}`.toLowerCase().includes(needle))
      : inventory;
    return [...filtered].sort((left, right) => compareRepositories(left, right, sort, schedule));
  }, [query, inventory, schedule, sort]);
  const selectedInInventory = selectedRepoId !== null && visibleRepos.some((repo) => repo.id === selectedRepoId);
  const selectedRepo = repos.find((repo) => repo.id === selectedRepoId) ?? null;
  const evidenceVersion = JSON.stringify([selectedRepo?.evidenceRevision??_dataRevision,selectedRepo]);
  const detailRequestKey = JSON.stringify([evidenceVersion, refreshRevision]);
  const loading = Boolean(selectedRepo) && settledRequest.key !== detailRequestKey;
  const visibleDetail = detail?.summary.id === selectedRepoId ? detail : null;
  const snapshotDependencyRevision = selectedRepo?.dependencyRevision ?? selectedRepo?.evidenceRevision ?? 0;
  const loadedDependencyRevision = visibleDetail?.summary.dependencyRevision ?? visibleDetail?.summary.evidenceRevision ?? 0;
  const dependencyRevision = Math.max(snapshotDependencyRevision, loadedDependencyRevision);
  const refreshingInventory = loading && (
    snapshotDependencyRevision > loadedDependencyRevision || settledRequest.refreshRevision !== refreshRevision
  );
  const detailScroll = useRef(initialContext?.scrollTop ?? 0);
  function saveContext(scrollTop: number) {
    onContextChange?.({
      repoId: selectedRepoId ?? undefined,
      repositorySection: tab,
      repositoryQuery: query,
      repositorySort: sort,
      repositoryPackageQuery: packageQuery,
      repositoryExactPackage: exactPackage,
      repositoryPackageLinkSequence: packageLinkSequence,
      repositoryDetailOpen: narrowDetailOpen,
      repositoryFailedOnly: failedOnly,
      repositoryScanRunId: selectedScanRunId,
      scrollTop,
    });
  }
  useLayoutEffect(() => {
    saveContext(detailScroll.current);
  }, [selectedRepoId, tab, query, sort, packageQuery, exactPackage, packageLinkSequence, narrowDetailOpen, failedOnly, selectedScanRunId, onContextChange]);
  useEffect(() => {
    if (selectedRepoId === null && inventory.length > 0) onSelectRepo(inventory[0].id);
  }, [onSelectRepo, inventory, selectedRepoId]);
  useEffect(() => {
    const node = rootRef.current;
    if (!node) return;
    const observer = new ResizeObserver(([entry]) => setNarrow(entry.contentRect.width < 980));
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    if (!narrow) return;
    if (narrowDetailOpen) {
      if (!rootRef.current?.contains(document.activeElement)
        || document.activeElement?.closest("[data-repository-inventory]")) {
        backToList.current?.focus();
      }
    } else if (restoreListFocus.current) {
      restoreListFocus.current = false;
      if (!selectedInInventory) {
        rootRef.current?.querySelector<HTMLElement>('input[aria-label="Search repositories"]')?.focus();
      }
    }
  }, [narrow, narrowDetailOpen, selectedInInventory]);

  useLayoutEffect(() => {
    if (!visibleDetail || (narrow && !narrowDetailOpen)) return;
    const root = rootRef.current;
    const pane = root?.querySelector<HTMLElement>("[data-repository-scroll]");
    if (pane) pane.scrollTop = detailScroll.current;
    const previousFocus = focusBeforeReturn.current;
    focusBeforeReturn.current = null;
    const active = document.activeElement;
    if (previousFocus && (active === previousFocus || active === document.body || active === backToList.current)) {
      root?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus({ preventScroll: true });
    }
  }, [visibleDetail?.summary.id, narrow, narrowDetailOpen]);

  useEffect(() => {
    const sequence = ++requestSequence.current;setDetailError(null);
    const dispose = () => { requestSequence.current += 1; };
    if (!selectedRepoId || !selectedRepo) { setDetail(null); setSettledRequest({ refreshRevision }); return dispose; }
    const currentRequest = { key: detailRequestKey, refreshRevision };
    const cached = overviewCache.get(selectedRepoId, Date.now(), evidenceVersion);
    if (cached) { setDetail(cached); setSettledRequest(currentRequest); return dispose; }
    setDetail((current)=>current?.summary.id===selectedRepoId?current:null);
    setSettledRequest((current) => current.key ? { refreshRevision: current.refreshRevision } : current);
    getRepoOverview(selectedRepoId)
      .then((next) => {
        if (sequence !== requestSequence.current) return;
        if (next.summary.id !== selectedRepoId) throw new Error("Repository evidence belongs to a different repository.");
        overviewCache.set(selectedRepoId, next, Date.now(), evidenceVersion);
        setDetail(next);
      })
      .catch((cause) => { if (sequence === requestSequence.current) {setDetailError(String(cause));onError(cause instanceof Error ? cause.message : String(cause));} })
      .finally(() => { if (sequence === requestSequence.current) setSettledRequest(currentRequest); });
    return dispose;
  }, [onError, selectedRepoId, evidenceVersion, detailRequestKey, Boolean(selectedRepo)]);

  function refreshOverview() {
    if (selectedRepoId === null) return;
    overviewCache.invalidate(selectedRepoId);
    setRefreshRevision((value) => value + 1);
  }

  const selectRepository = useCallback((repoId: number) => {
    if (repoId !== selectedRepoRef.current) {
      detailScroll.current = 0;
      setScanSelection({ repoId, runId: undefined });
      setPackageQuery("");
      setLinkedPackage(undefined);
    }
    onSelectRepoRef.current(repoId);
    setNarrowDetailOpen(true);
  }, []);

  function closeRepository() {
    restoreListFocus.current = true;
    setNarrowDetailOpen(false);
  }

  function handleScan() {
    if (!selectedRepoId) return;
    if (isScanning) { onShowActivity?.(); return; }
    void onScanRepo(selectedRepoId);
  }

  async function handleRemove() {
    if (!selectedRepo || selectedRepo.sourceType !== "github") return;
    if (!window.confirm(`Remove the local CVEC link and locally stored CVEC history for ${selectedRepo.name}? This does not modify or delete the GitHub repository.`)) return;
    setRemovingRepo(true);
    try {
      const snapshot = await removeRemoteRepository(selectedRepo.id);
      overviewCache.invalidate(selectedRepo.id);
      onSnapshot(snapshot);
      const next = snapshot.repos[0] ?? null;
      if (selectedRepoRef.current === selectedRepo.id) {
        onSelectRepo(next?.id ?? null);
        setNarrowDetailOpen(false);
        setDetail(null);
      }
    } catch (cause) {
      onError(cause instanceof Error ? cause.message : String(cause));
    } finally { setRemovingRepo(false); }
  }

  return (
    <div className="repository-workspace min-h-0 min-w-0 flex-1" ref={rootRef}>
      <div className={`grid min-h-[calc(100vh-11rem)] min-w-0 gap-3 xl:h-[calc(100vh-11rem)] xl:min-h-0 ${narrow ? "grid-cols-1" : "grid-cols-[minmax(300px,340px)_minmax(640px,1fr)]"}`}>
        {(!narrow || !narrowDetailOpen) ? (
          <RepositoryInventory
            query={query} repos={visibleRepos} schedule={schedule} selectedRepoId={selectedInInventory ? selectedRepoId : null}
            sort={sort} failedOnly={failedOnly} hasRepos={repos.length > 0}
            virtualized={inventory.length > 30} restoreFocus={restoreListFocus.current && selectedInInventory}
            onQueryChange={setQuery} onSelect={selectRepository} onSortChange={setSort} onClearFailedFilter={() => setFailedOnly(false)}
          />
        ) : null}
        {(!narrow || narrowDetailOpen) ? (
          <Card className="app-panel flex min-h-0 min-w-0 flex-col overflow-hidden rounded-lg border shadow-none" onKeyDown={(event) => {
            if (narrow && !event.defaultPrevented && event.key === "Escape") {
              event.preventDefault();
              closeRepository();
            }
          }}>
            {narrow ? <div className="shrink-0 border-b border-hairline px-3 py-2"><Button ref={backToList} onClick={closeRepository} size="sm" variant="ghost"><AppIcon className="size-4" icon={CaretLeft} /> Back to repositories</Button></div> : null}
            {!selectedRepo ? (
              <EmptyState icon={GitBranch} title="No repository selected" description="Choose a repository to inspect its monitoring evidence." />
            ) : loading && !visibleDetail ? (
              <div className="flex min-h-80 items-center justify-center"><Spinner label="Loading repository evidence" /></div>
            ) : visibleDetail ? (
              <>
                {loading ? <p role="status" className="px-4 py-2 text-xs text-ink-subtle">Updating repository evidence…</p> : null}
                {detailError ? <div role="alert" className="flex items-center gap-2 px-4 py-2 text-xs text-severity-critical"><p>{detailError}</p><Button size="sm" variant="secondary" onClick={refreshOverview}>Retry</Button></div> : null}
                <RepositoryDetail
                  detail={visibleDetail} dependencyRevision={dependencyRevision} refreshingInventory={refreshingInventory}
                  isScanning={isScanning} packageQuery={packageQuery}
                  exactPackage={exactPackage}
                  packageLinkSequence={packageLinkSequence} removing={removingRepo} scannerReady={scannerReady}
                  selectedRepo={selectedRepo} schedule={schedule} tab={tab} selectedScanRunId={selectedScanRunId}
                  onSelectScanRun={(runId) => setScanSelection({ repoId: selectedRepoId, runId })}
                  onOpenReview={onOpenReview} onPackageQueryChange={setPackageQuery} onRemove={() => void handleRemove()}
                  onClearLinkedPackage={() => setLinkedPackage(undefined)}
                  onRefreshOverview={refreshOverview} onScan={handleScan} onTabChange={(next) => {
                    detailScroll.current = 0;
                    const pane = rootRef.current?.querySelector<HTMLElement>("[data-repository-scroll]");
                    if (pane) pane.scrollTop = 0;
                    setTab(next);
                  }} onScroll={(offset) => {
                    if (detailScroll.current === offset) return;
                    detailScroll.current = offset;
                    saveContext(offset);
                  }}
                />
              </>
            ) : (
              <div role="alert" className="grid place-content-center gap-3 p-6"><p>{detailError??"The locally stored repository evidence could not be loaded."}</p><Button variant="secondary" onClick={refreshOverview}>Retry</Button></div>
            )}
          </Card>
        ) : null}
      </div>
    </div>
  );
}

function RepositoryInventory({ query, repos, schedule, selectedRepoId, sort, failedOnly, hasRepos, virtualized, restoreFocus, onQueryChange, onSelect, onSortChange, onClearFailedFilter }: {
  query: string; repos: RepoSummary[]; schedule: ScheduleInterval; selectedRepoId: number | null; sort: InventorySort;
  failedOnly: boolean; hasRepos: boolean; virtualized: boolean; restoreFocus: boolean; onClearFailedFilter: () => void;
  onQueryChange: (value: string) => void; onSelect: (repoId: number) => void; onSortChange: (value: InventorySort) => void;
}) {
  const selectedKeys = useMemo(() => selectedRepoId === null ? [] : [selectedRepoId], [selectedRepoId]);
  const list = (
    <ListBox
      aria-label="Repository inventory"
      autoFocus={restoreFocus}
      className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto outline-none [scrollbar-gutter:stable]"
      disallowEmptySelection
      escapeKeyBehavior="none"
      items={repos}
      dependencies={[schedule, onSelect]}
      selectedKeys={selectedKeys}
      selectionBehavior="toggle"
      selectionMode="single"
    >
      {(repo) => {
        const monitoring = deriveRepositoryMonitoring(repo, null, schedule);
        const newFindings = totalNew(repo);
        const resolvedFindings = totalResolved(repo);
        return (
          <ListBoxItem
            className="w-full cursor-pointer border-b border-hairline bg-panel-surface px-3 py-3 text-left outline-none data-[hovered]:bg-row-hover-surface data-[selected]:bg-row-selected-surface data-[selected]:shadow-[inset_0_0_0_1px_var(--hairline-strong)] data-[focus-visible]:ring-2 data-[focus-visible]:ring-inset data-[focus-visible]:ring-primary-focus"
            data-repository-id={repo.id}
            id={repo.id}
            onPress={() => onSelect(repo.id)}
            textValue={repo.name}
          >
            <span className="flex min-w-0 items-start gap-2.5">
              <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-sm bg-surface-3 text-ink-subtle"><AppIcon className="size-4" icon={repo.sourceType === "github" ? GithubLogo : GitBranch} /></span>
              <span className="min-w-0 flex-1">
                <span className="flex items-center justify-between gap-2">
                  <span className="truncate text-sm font-semibold text-ink">{repo.name}</span>
                  <MonitoringBadge state={monitoring.state} />
                </span>
                <span className="mt-1 block truncate text-xs text-ink-subtle">{repoSourceLabel(repo)}</span>
                <span className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-ink-subtle">
                  <span>{repo.lastScannedAt ? `Scanned ${formatRelativeTimestamp(repo.lastScannedAt)}` : "Never scanned"}</span>
                  {newFindings + resolvedFindings > 0 ? (
                    <span aria-label={`${newFindings} new and ${resolvedFindings} resolved findings`}>
                      <strong className="font-medium text-ink">+{newFindings}</strong>
                      {" / "}
                      <strong className="font-medium text-status-success">−{resolvedFindings}</strong>
                    </span>
                  ) : null}
                </span>
              </span>
            </span>
          </ListBoxItem>
        );
      }}
    </ListBox>
  );
  return (
    <Card data-repository-inventory className="app-panel flex max-h-[calc(100vh-11rem)] min-h-0 min-w-0 flex-col overflow-hidden rounded-lg border shadow-none">
      <div className="shrink-0 border-b border-hairline p-3">
        <div className="mb-2 flex items-center justify-between gap-2">
          <h2 className="text-sm font-semibold text-ink">{failedOnly ? "Failed scans" : "Repositories"}</h2>
          <Badge role="status" variant="outline">{repos.length}</Badge>
        </div>
        {failedOnly ? <Button className="mb-2" size="xs" variant="secondary" onClick={onClearFailedFilter}>Show all repositories</Button> : null}
        <SearchField aria-label="Search repositories" placeholder="Search repositories" value={query} onChange={(event) => onQueryChange(event.target.value)} />
        <Select containerClassName="mt-2" aria-label="Sort repositories" value={sort} onValueChange={(value) => onSortChange(value as InventorySort)}><option value="attention">Needs attention</option><option value="freshness">Scan freshness</option><option value="changes">Recent changes</option><option value="name">Name</option></Select>
      </div>
      {repos.length > 0 ? (
        virtualized ? <Virtualizer layout={ListLayout} layoutOptions={inventoryLayout}>{list}</Virtualizer> : list
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="p-5 text-center">
            <p className="text-sm font-medium text-ink">{query.trim() ? "No matching repositories" : failedOnly ? "No failed scans" : "No repositories yet"}</p>
            {query.trim() ? (
              <Button className="mt-3" onClick={() => onQueryChange("")} size="sm" variant="secondary">Clear search</Button>
            ) : !hasRepos && !failedOnly ? (
              <p className="mt-1 text-xs text-ink-subtle">Choose a folder containing Git repositories to get started.</p>
            ) : null}
          </div>
        </div>
      )}
    </Card>
  );
}

function RepositoryDetail({ detail, dependencyRevision, refreshingInventory, selectedRepo, schedule, tab, packageQuery, exactPackage, packageLinkSequence, scannerReady, isScanning, removing, selectedScanRunId, onSelectScanRun, onTabChange, onPackageQueryChange, onClearLinkedPackage, onOpenReview, onRefreshOverview, onScan, onRemove, onScroll }: {
  detail: RepoOverview; selectedRepo: RepoSummary; schedule: ScheduleInterval; tab: DetailTab; packageQuery: string; packageLinkSequence?: number; scannerReady: boolean; isScanning: boolean; removing: boolean; selectedScanRunId?: number;
  exactPackage?: ScannedPackageIdentity;
  onClearLinkedPackage: () => void;
  dependencyRevision: number;
  refreshingInventory: boolean;
  onSelectScanRun: (id: number | undefined) => void;
  onTabChange: (tab: DetailTab) => void; onPackageQueryChange: (value: string) => void; onOpenReview: (request: ReviewOpenRequest) => void; onRefreshOverview: () => void; onScan: () => void; onRemove: () => void; onScroll: (offset: number) => void;
}) {
  const lastSuccessfulScan = detail.overview.latestCompletedScan;
  const monitoring = deriveRepositoryMonitoring(selectedRepo, detail.latestScan, schedule, Date.now(), lastSuccessfulScan);
  const scanActionLabel = isScanning ? "View scan" : monitoring.state === "failed" ? "Retry scan" : "Scan repo";
  function openReview(section: DetailTab, packageIdentity?: string, findingFingerprint?: string, remediationGroup?: ReviewOpenRequest["remediationGroup"]) {
    onOpenReview({ repositoryId: selectedRepo.id, packageIdentity, findingFingerprint, remediationGroup, origin: { repositoryId: selectedRepo.id, section } });
  }
  return (
    <div className="repository-detail-pane flex h-full min-h-0 min-w-0 flex-col">
      <header className="shrink-0 border-b border-hairline px-4 py-3">
        <div className="flex min-w-0 items-start justify-between gap-3">
          <div className="flex min-w-0 items-start gap-3"><span className="flex size-9 shrink-0 items-center justify-center rounded-md bg-surface-3 text-ink-subtle"><AppIcon className="size-5" icon={selectedRepo.sourceType === "github" ? GithubLogo : GitBranch} /></span><div className="min-w-0"><div className="flex flex-wrap items-center gap-2"><h2 className="truncate text-lg font-semibold text-ink">{selectedRepo.name}</h2><MonitoringBadge state={monitoring.state} /></div><p className="mt-1 truncate text-xs text-ink-subtle">{repoSourceLabel(selectedRepo)}</p></div></div>
          <div className="flex shrink-0 items-center gap-1.5"><Button disabled={(!scannerReady && !isScanning) || removing} onClick={onScan}><AppIcon className={`size-4 ${isScanning ? "motion-safe:animate-spin" : ""}`} icon={ArrowClockwise} />{scanActionLabel}</Button><Popover><PopoverTrigger render={<IconButton aria-label="Repository actions" variant="ghost" />}><AppIcon className="size-5" icon={DotsThree} weight="bold" /></PopoverTrigger><PopoverContent align="end" className="w-64 p-1.5"><PopoverTitle className="sr-only">Repository actions</PopoverTitle><Button className="w-full justify-start" variant="ghost" onClick={() => openReview(tab)}><AppIcon className="size-4" icon={ShieldCheck} />Open all in Review</Button>{selectedRepo.sourceType === "github" ? <Button className="w-full justify-start text-severity-critical" disabled={removing} variant="ghost" onClick={onRemove}><AppIcon className="size-4" icon={Trash} />{removing ? "Removing link" : "Remove local link"}</Button> : null}</PopoverContent></Popover></div>
        </div>
        <p className="mt-2 text-xs text-ink-subtle">{monitoring.reason}</p>
      </header>
      <div className="shrink-0 overflow-x-auto border-b border-hairline px-4 pt-2"><Tabs label="Repository sections" value={tab} onValueChange={onTabChange} options={[{ value: "overview", label: "Overview" }, { value: "dependencies", label: "Dependencies" }, { value: "history", label: "Scan history" }, { value: "artifacts", label: "Artifacts" }, { value: "settings", label: "Source" }]} /></div>
      <div data-repository-scroll onScroll={(event) => onScroll(event.currentTarget.scrollTop)} className="min-h-0 min-w-0 flex-1 overflow-y-auto p-4 [scrollbar-gutter:stable]">
        {tab === "overview" ? <Overview detail={detail} lastSuccessfulScan={lastSuccessfulScan} monitoring={monitoring} onOpenReview={(pkg, fingerprint, group) => openReview("overview", pkg, fingerprint, group)} /> : null}
        {tab === "dependencies" ? lastSuccessfulScan ? (
          <>
          {detail.nugetProjects?.length ? <RepositoryNugetProjects key={lastSuccessfulScan.id} projects={detail.nugetProjects} /> : null}
          <RepositoryDependencies
            key={`${selectedRepo.id}:${packageLinkSequence ?? 0}`}
            packageLinkSequence={packageLinkSequence}
            repoId={selectedRepo.id}
            scanRunId={lastSuccessfulScan.id}
            dependencyRevision={dependencyRevision}
            refreshingInventory={refreshingInventory}
            query={packageQuery}
            exactPackage={exactPackage}
            onClearLinkedPackage={onClearLinkedPackage}
            onQueryChange={onPackageQueryChange}
            onRefreshOverview={onRefreshOverview}
            onOpenReview={(pkg) => onOpenReview({
              repositoryId: selectedRepo.id,
              packageIdentity: pkg.name,
              installedPackage: { anyFix: true, packageName: pkg.name, packageUrl: pkg.packageUrl, packageType: pkg.packageType, installedVersion: pkg.version, fixVersion: null },
              origin: { repositoryId: selectedRepo.id, section: "dependencies" },
            })}
          />
          </>
        ) : <EmptyState icon={Package} title="No completed scan yet" description="Run a repository scan to see its dependencies." /> : null}
        {tab === "history" ? <RepositoryScanHistory
          key={selectedRepo.id}
          detail={detail}
          selectedRunId={selectedScanRunId}
          onSelectRun={onSelectScanRun}
          onOpenResolved={(resolvedInScanRunId) => onOpenReview({
            repositoryId: selectedRepo.id,
            resolvedInScanRunId,
            origin: { repositoryId: selectedRepo.id, section: "history" },
          })}
        /> : null}
        {tab === "artifacts" ? <RepositoryArtifacts key={selectedRepo.id} repoId={selectedRepo.id} canScan={scannerReady && !isScanning && !removing} /> : null}
        {tab === "settings" ? <RepositorySource detail={detail} /> : null}
      </div>
    </div>
  );
}

function Overview({ detail, lastSuccessfulScan, monitoring, onOpenReview }: { detail: RepoOverview; lastSuccessfulScan: RepoScanRun | null; monitoring: ReturnType<typeof deriveRepositoryMonitoring>; onOpenReview: (packageIdentity?: string, fingerprint?: string, remediationGroup?: ReviewOpenRequest["remediationGroup"]) => void }) {
  const { opportunities, affectedCount, opportunityCount, openFindingCount } = detail.overview;
  const preview = detail.findings;
  if (!lastSuccessfulScan) return <EmptyState icon={ShieldWarning} title="No trusted scan evidence yet" description="Run a local scan to establish the repository baseline." />;
  const packageCount = lastSuccessfulScan.packageCount ?? detail.overview.packageCount;
  return (
    <div className="grid gap-4">
      <section className="repository-evidence-metrics grid gap-2">
        <EvidenceMetric label="Last successful scan" value={monitoring.lastSuccessfulScanAt ? formatRelativeTimestamp(monitoring.lastSuccessfulScanAt) : "Unavailable"} detail={lastSuccessfulScan.finishedAt ? formatTimestamp(lastSuccessfulScan.finishedAt) : undefined} />
        <EvidenceMetric label="Last scanned inventory" value={`${packageCount} package${packageCount === 1 ? "" : "s"}`} detail={`${affectedCount} affected`} />
        <EvidenceMetric label="Since comparison" value={`+${totalNew(detail.summary)} / −${totalResolved(detail.summary)}`} detail={detail.previousScan ? "New / resolved findings" : "Baseline scan"} />
        <EvidenceMetric label="Scanned revision" value={shortSha(lastSuccessfulScan.sourceRevision) ?? "Not recorded"} detail={detail.summary.sourceType === "github" && detail.summary.lastRemoteCommitSha ? "Latest known on GitHub · " + shortSha(detail.summary.lastRemoteCommitSha) : lastSuccessfulScan.sourceDirty ? "Includes local changes" : undefined} />
      </section>
      {lastSuccessfulScan.coverageWarnings?.length ? (
        <section className="rounded-lg border border-hairline bg-surface-1 px-4 py-3">
          <h3 className="text-sm font-semibold text-ink">Incomplete package coverage</h3>
          <ul className="mt-2 list-disc space-y-1 break-words pl-4 text-xs text-ink-subtle">
            {lastSuccessfulScan.coverageWarnings.map((warning) => <li key={warning}>{formatCoverageWarning(warning, detail.summary.sourceType)}</li>)}
          </ul>
        </section>
      ) : null}
      <RepositoryRiskTrend runs={detail.scanHistory} />
      <section className="rounded-lg border border-hairline bg-surface-1"><div className="flex items-start justify-between gap-3 border-b border-hairline px-4 py-3"><div><h3 className="text-sm font-semibold text-ink">Remediation opportunities</h3><p className="mt-1 text-xs text-ink-subtle">Review findings grouped by package and target version.</p></div>{opportunities.length ? <Badge variant="outline">{opportunityCount}</Badge> : null}</div>{opportunities.length ? <div className="divide-y divide-hairline">{opportunities.map((item) => <RemediationRow key={item.key} item={item} onOpen={() => onOpenReview(item.packageName, item.fingerprints[0], { packageName: item.packageName, packageUrl: item.packageUrl, packageType: item.packageType, installedVersion: item.installedVersion, fixVersion: item.fixVersion, preferredFingerprint: item.fingerprints[0] })} />)}</div> : <p className="px-4 py-5 text-sm text-ink-subtle">No supported package fix target is recorded for current findings.</p>}</section>
      <section className="rounded-lg border border-hairline bg-surface-1"><div className="flex items-start justify-between gap-3 border-b border-hairline px-4 py-3"><div><h3 className="text-sm font-semibold text-ink">Recent repository findings</h3><p className="mt-1 text-xs text-ink-subtle">Open findings from the last successful scan.</p></div><Button size="sm" variant="ghost" onClick={() => onOpenReview()}>Open {openFindingCount} in Review <AppIcon className="size-4" icon={ArrowRight} /></Button></div>{preview.length ? <div className="divide-y divide-hairline">{preview.map((finding) => <FindingPreview key={finding.fingerprint} finding={finding} onOpen={() => onOpenReview(finding.packageName, finding.fingerprint)} />)}</div> : <p className="px-4 py-5 text-sm text-ink-subtle">No open findings in the scanned inventory.</p>}</section>
    </div>
  );
}

function RemediationRow({ item, onOpen }: { item: RemediationOpportunity; onOpen: () => void }) {
  return <Button aria-label={"Review " + item.packageName + " to " + item.fixVersion} className="h-auto w-full justify-start rounded-none px-4 py-3 text-left whitespace-normal hover:bg-row-hover-surface focus-visible:ring-inset" onClick={onOpen} type="button" variant="ghost"><span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-surface-3"><AppIcon className="size-4 text-ink-subtle" icon={Package} /></span><span className="min-w-0 flex-1"><span className="block truncate text-sm font-semibold text-ink">{item.packageName}</span><span className="mt-1 block truncate text-xs text-ink-subtle">{item.installedVersion ?? "unknown"} → <strong className="font-medium text-[color:var(--status-success)]">{item.fixVersion}</strong></span></span><span className="flex shrink-0 items-center gap-1"><Badge variant="outline">{item.findingCount} {item.findingCount === 1 ? "finding" : "findings"}</Badge>{item.critical ? <Badge variant="destructive">{item.critical} critical</Badge> : item.high ? <Badge variant="warning">{item.high} high</Badge> : null}<AppIcon className="size-4 text-ink-subtle" icon={ArrowRight} /></span></Button>;
}

function FindingPreview({ finding, onOpen }: { finding: RepoFinding; onOpen: () => void }) {
  const tool = finding.matchEvidence?.bundledTool;
  return (
    <Button className="h-auto w-full justify-start rounded-none px-4 py-3 text-left whitespace-normal hover:bg-row-hover-surface focus-visible:ring-inset" onClick={onOpen} type="button" variant="ghost">
      <Badge variant={severityVariant(finding.severity)}>{finding.severity}</Badge>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-semibold text-ink">{finding.advisoryId}</span>
        <span className="mt-1 block truncate text-xs text-ink-subtle">{finding.title ?? finding.packageName}</span>
        {tool ? <span className="mt-1 block text-xs text-ink-subtle [overflow-wrap:anywhere]">{bundledToolLabel(tool)}</span> : null}
      </span>
      <span className="hidden shrink-0 text-xs text-ink-subtle sm:block">{finding.packageName}</span>
      <AppIcon className="size-4 shrink-0 text-ink-subtle" icon={ArrowRight} />
    </Button>
  );
}

function RepositoryRiskTrend({ runs }: { runs: RepoScanRun[] }) {
  const points = runs.filter((run) => run.status === "completed").slice(0, 7).reverse();
  if (points.length < 2 || points.every((run) => run.openCritical === undefined && run.openHigh === undefined && run.openMedium === undefined && run.openOther === undefined)) {
    return <section className="rounded-lg border border-hairline bg-surface-1 px-4 py-3"><h3 className="text-sm font-semibold text-ink">Risk trend</h3><p className="mt-1 text-xs text-ink-subtle">A trend becomes available after two successful scans with comparable finding evidence.</p></section>;
  }
  const max = Math.max(1, ...points.map((run) => (run.openCritical ?? 0) + (run.openHigh ?? 0) + (run.openMedium ?? 0) + (run.openOther ?? 0)));
  return <section className="rounded-lg border border-hairline bg-surface-1"><div className="border-b border-hairline px-4 py-3"><h3 className="text-sm font-semibold text-ink">Risk trend</h3><p className="mt-1 text-xs text-ink-subtle">Open findings across the latest comparable successful scans.</p></div><ol aria-label="Repository risk trend" className="flex h-28 items-end gap-2 px-4 pb-3 pt-4">{points.map((run) => { const critical = run.openCritical ?? 0; const high = run.openHigh ?? 0; const medium = run.openMedium ?? 0; const other = run.openOther ?? 0; const total = critical + high + medium + other; return <li aria-label={`${formatTimestamp(run.finishedAt ?? run.startedAt)}: ${critical} critical, ${high} high, ${medium} medium and ${other} other open findings`} className="group flex min-w-0 flex-1 flex-col items-center justify-end gap-1" key={run.id}><span className="text-xs tabular-nums text-ink-tertiary opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">{total}</span><span className="flex min-h-2 w-full max-w-14 flex-col-reverse overflow-hidden rounded-sm bg-surface-3" style={{ height: `${Math.max(8, Math.round((total / max) * 70))}px` }}><span className="bg-ink-subtle" style={{ height: `${total ? (other / total) * 100 : 0}%` }} /><span className="bg-[color:var(--severity-medium)]" style={{ height: `${total ? (medium / total) * 100 : 0}%` }} /><span className="bg-[color:var(--severity-high)]" style={{ height: `${total ? (high / total) * 100 : 0}%` }} /><span className="bg-[color:var(--severity-critical)]" style={{ height: `${total ? (critical / total) * 100 : 0}%` }} /></span><time className="truncate text-xs text-ink-tertiary">{new Date(run.finishedAt ?? run.startedAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })}</time></li>; })}</ol></section>;
}

function RepositorySource({ detail }: { detail: RepoOverview }) {
  const repo = detail.summary;
  const remote = repo.sourceType === "github";
  return (
    <div className="grid gap-4">
      <section className="rounded-lg border border-hairline bg-surface-1">
        <div className="border-b border-hairline px-4 py-3">
          <h3 className="text-sm font-semibold text-ink">{remote ? "GitHub repository" : "Local checkout"}</h3>
        </div>
        <dl className="grid px-4">
          <SourceField label={remote ? "Repository URL" : "Checkout folder"} value={(remote ? repo.remoteUrl : repo.path) ?? "Not recorded"} />
          {!remote && repo.remoteUrl ? <SourceField label="Git remote" value={repo.remoteUrl} /> : null}
          {remote ? <>
            {repo.defaultBranch ? <SourceField label="Default branch" value={repo.defaultBranch} /> : null}
            <SourceField label="Latest known revision" value={shortSha(repo.lastRemoteCommitSha) ?? "Not recorded"} />
            {repo.lastRemoteSyncedAt ? <SourceField label="Last synced" value={formatTimestamp(repo.lastRemoteSyncedAt)} /> : null}
          </> : null}
        </dl>
      </section>
      <RepositoryProducts key={repo.id} repoId={repo.id} evidenceRevision={repo.evidenceRevision} />
    </div>
  );
}

function SourceField({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid gap-1 border-b border-hairline py-3 last:border-0 sm:grid-cols-[170px_minmax(0,1fr)]">
      <dt className="text-xs text-ink-subtle">{label}</dt>
      <dd className="min-w-0 text-sm text-ink [overflow-wrap:anywhere]">{value}</dd>
    </div>
  );
}

function EvidenceMetric({ label, value, detail }: { label: string; value: string; detail?: string }) { return <div className="rounded-lg border border-hairline bg-surface-1 px-3 py-3"><p className="text-xs text-ink-subtle">{label}</p><p className="mt-1 truncate text-base font-semibold text-ink" title={value}>{value}</p>{detail ? <p className="mt-1 truncate text-xs text-ink-subtle">{detail}</p> : null}</div>; }
function MonitoringBadge({ state }: { state: RepositoryMonitoringState }) { const variants: Record<RepositoryMonitoringState, "outline" | "secondary" | "warning" | "destructive" | "success"> = { scanning: "secondary", failed: "destructive", never_scanned: "warning", source_ahead: "warning", stale: "warning", coverage_incomplete: "warning", current: "success", unknown: "outline" }; const labels: Record<RepositoryMonitoringState, string> = { scanning: "Scanning", failed: "Failed", never_scanned: "Never scanned", source_ahead: "Source ahead", stale: "Stale", coverage_incomplete: "Coverage incomplete", current: "Current", unknown: "Unknown" }; return <Badge variant={variants[state]}>{labels[state]}</Badge>; }
function repoSourceLabel(repo: RepoSummary) { if (repo.sourceType === "github") return `GitHub${repo.defaultBranch ? ` · ${repo.defaultBranch}` : ""}${shortSha(repo.lastRemoteCommitSha) ? ` · ${shortSha(repo.lastRemoteCommitSha)}` : ""}`; return repo.remoteHost ? `Local · ${repo.remoteHost}` : "Local repository"; }
function totalNew(repo: RepoSummary) { return repo.newCritical + repo.newHigh + repo.newMedium + (repo.newOther ?? 0); }
function totalResolved(repo: RepoSummary) { return repo.resolvedCritical + repo.resolvedHigh + repo.resolvedMedium + (repo.resolvedOther ?? 0); }
function compareRepositories(left: RepoSummary, right: RepoSummary, sort: InventorySort, schedule: ScheduleInterval) { if (sort === "name") return left.name.localeCompare(right.name) || left.id - right.id; if (sort === "changes") return totalNew(right) - totalNew(left) || totalResolved(right) - totalResolved(left) || left.name.localeCompare(right.name); if (sort === "freshness") return (left.lastScannedAt ?? "").localeCompare(right.lastScannedAt ?? "") || left.name.localeCompare(right.name); const rank: Record<RepositoryMonitoringState, number> = { failed: 7, scanning: 6, never_scanned: 5, source_ahead: 4, stale: 3, coverage_incomplete: 3, unknown: 2, current: 1 }; const leftState = deriveRepositoryMonitoring(left, null, schedule).state; const rightState = deriveRepositoryMonitoring(right, null, schedule).state; return rank[rightState] - rank[leftState] || right.newCritical - left.newCritical || right.newHigh - left.newHigh || left.name.localeCompare(right.name) || left.id - right.id; }
function severityVariant(severity: Severity): "destructive" | "warning" | "secondary" { return severity === "CRITICAL" ? "destructive" : severity === "HIGH" ? "warning" : "secondary"; }
function shortSha(value: string | null | undefined) { return value ? value.slice(0, 8) : null; }
