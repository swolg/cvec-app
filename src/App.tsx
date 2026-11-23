import { ArrowLeft, GithubLogo, MagnifyingGlass, X } from "@phosphor-icons/react";
import {
  useEffect,
  useCallback,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type ComponentProps,
} from "react";
import { listen } from "@tauri-apps/api/event";

import {
  DashboardSidebar,
  type DashboardSection,
} from "@/components/DashboardSidebar";
import { SettingsSidebar, type SettingsPage } from "@/components/SettingsSidebar";
import { GithubRepositoryForm, type GithubImportDraft } from "@/components/GithubRepositoryForm";
import { CommandPalette } from "@/components/CommandPalette";
import { ScanActivityPopover } from "@/components/ScanActivityPopover";
import { NotificationBell } from "@/components/NotificationBell";

import { lazyWorkspace } from "@/components/WorkspaceLoader";

import {
  Alert,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert";
import { AppIcon } from "@/components/ui/app-icon";
import { Button, IconButton } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Dialog, DialogContent } from "@/components/ui/overlays";
import { toast } from "@/components/ui/toast";
import { createEvidenceReads, EvidenceReadsContext } from "@/lib/revisionReadCache";
import { remediationProjectionCache } from "@/lib/remediationProjectionCache";
import {
  addGithubRepository,
  bootstrap,
  getSnapshot,
  checkVulnerabilityDb,
  refreshLocalRepositories,
  clearAllNotifications,
  clearNotification,
  completeOnboarding,
  chooseDiscoveryRoot,
  listExternalIntelFeed,
  listRiskTrend,
  scanRepo,
  openExternalUrl,
  refreshExternalIntel,
  scanAll,
  setExternalIntelTopics,
  setSchedule,
  skipOnboarding,
  updateVulnerabilityDb,
} from "@/lib/api";
import { allReviewViews, DEFAULT_TABLE_PRESENTATION, loadReviewViews, reviewQuery, saveReviewViews } from "@/lib/reviewViews";
import {
  WorkspaceHistory,
  normalizeWorkspace,
  type IntelFeedView,
  type IntelSelection,
  type WorkspaceContext,
  type WorkspaceId,
} from "@/lib/navigation";
import { describeScanResult } from "@/lib/format";
import { buildDbUpdateFeedback, scannerIssue } from "@/lib/grypeStatus";
import type {
  AppSnapshot,
  ExternalIntelFeed,
  GithubAuthSnapshot,
  RiskTrendPoint,
  RiskTrendRange,
  RepoSummary,
  ScannedPackageIdentity,
  ReviewOpenRequest,
  FindingChatRequest,
  ReviewUpdate,
  ScanAllSummary,
  ScanActivity,
  LocalEntityResult,
} from "@/lib/types";

const DashboardOverview=lazyWorkspace<ComponentProps<typeof import("@/components/dashboard/DashboardOverview").DashboardOverview>>(async()=>({default:(await import("@/components/dashboard/DashboardOverview")).DashboardOverview}), "Home");
const ExternalSourcesWorkspace=lazyWorkspace<ComponentProps<typeof import("@/components/ExternalSourcesWorkspace").ExternalSourcesWorkspace>>(async()=>({default:(await import("@/components/ExternalSourcesWorkspace")).ExternalSourcesWorkspace}), "ExternalSources");
const NO_INTEL_SELECTION: IntelSelection = { canonicalKey: null, detailOpen: false, direct: false };
const ChatWorkspace=lazyWorkspace<ComponentProps<typeof import("@/components/ChatWorkspace").ChatWorkspace>>(async()=>({default:(await import("@/components/ChatWorkspace")).ChatWorkspace}), "Chat");
const AiConnectionsSettings=lazyWorkspace<Record<string, never>>(async()=>({default:(await import("@/components/AiConnectionsSettings")).AiConnectionsSettings}), "AiConnections settings");
const SettingsPanel = lazyWorkspace<ComponentProps<typeof import("@/components/SettingsPanel").SettingsPanel>>(
  () => import("@/components/SettingsPanel").then(({ SettingsPanel }) => ({ default: SettingsPanel })),
  "Settings",
);
const WelcomeOnboarding = lazyWorkspace<ComponentProps<typeof import("@/components/WelcomeOnboarding").WelcomeOnboarding>>(
  () => import("@/components/WelcomeOnboarding").then(({ WelcomeOnboarding }) => ({ default: WelcomeOnboarding })),
  "Welcome",
);
const ReviewWorkspace=lazyWorkspace<ComponentProps<typeof import("@/components/ReviewWorkspace").ReviewWorkspace>>(async()=>({default:(await import("@/components/ReviewWorkspace")).ReviewWorkspace}), "Review");
const RepositoryWorkspace=lazyWorkspace<ComponentProps<typeof import("@/components/RepositoryWorkspace").RepositoryWorkspace>>(async()=>({default:(await import("@/components/RepositoryWorkspace")).RepositoryWorkspace}), "Repository");

function prepareWorkspace(section: DashboardSection) {
  const load = section === "review" ? import("@/components/ReviewWorkspace")
    : section === "chat" ? import("@/components/ChatWorkspace")
    : section === "intel" ? import("@/components/ExternalSourcesWorkspace")
    : section === "repositories" ? import("@/components/RepositoryWorkspace")
    : section === "settings" ? import("@/components/SettingsPanel")
    : import("@/components/dashboard/DashboardOverview");
  void load.catch(() => undefined);
}

const BACK_SHORTCUT_EXCLUDED_TARGETS = [
  "input", "textarea", "select", '[contenteditable]:not([contenteditable="false"])',
  '[role="combobox"]', '[role="listbox"]', '[role="menu"]',
  '[role="dialog"]:not([data-closed])', '[role="alertdialog"]:not([data-closed])',
].join(", ");

const EMPTY_STATE: AppSnapshot = {
  repos: [],
  notifications: [],
  externalIntelSummary: {
    totalItems: 0,
    relevantItems: 0,
    triageItems: 0,
    lastFetchedAt: null,
    sources: [],
    sourceCounts: [],
  },
  externalIntelTopics: [],
  overviewSnapshot: {
    reposWatched: 0,
    newMedium: 0,
    newHigh: 0,
    newCritical: 0,
    openMedium: 0,
    openHigh: 0,
    openCritical: 0,
    reposWithNewFindings: 0,
    lastSuccessfulScanAt: null,
  },
  workspaceStatus: {
    scannerReady: false,
    scannerMessage: null,
    schedule: "off",
    lastSuccessfulScanAt: null,
    nextScheduledScanAt: null,
    scanRunning: false,
  },
  discoveryRoot: "~/code",
  versionInfo: {
    app: "0.1.0",
    syft: null,
    grype: null,
    grypeDb: null,
    lastGrypeDbUpdate: null,
  },
  githubAuth: {
    configured: false,
    signedIn: false,
    login: null,
    avatarUrl: null,
    profileUrl: null,
    expiresAt: null,
    rateLimitRemaining: null,
    rateLimitLimit: null,
    rateLimitResetAt: null,
    installUrl: null,
    message: null,
  },
  onboarding: {
    status: "not_started",
    decidedAt: null,
    version: null,
  },
  reviewSummary: {
    needsReview: 0, unreviewedCritical: 0, unreviewedOther: 0, dueSnoozed: 0,
    openReviewed: 0, snoozed: 0, recentlyResolved: 0, failedScans: 0,
  },
  scanActivity: null,
  dataRevisions: { advisory: 0, inventory: 0, scanHistory: 0, review: 0, intel: 0, scanner: 0, notifications: 0, settings: 0 },
};

const SECTION_META: Record<
  DashboardSection,
  { title: string; description: string }
> = {
  overview: {
    title: "Overview",
    description:
      "See what changed, where it changed, and whether your scanner is healthy.",
  },
  review: {
    title: "Review",
    description:
      "Prioritize, inspect, and acknowledge local security findings.",
  },
  chat: {
    title: "Chat",
    description:
      "Ask Codex about one repository at a time with explicit local security context.",
  },
  intel: {
    title: "Intelligence",
    description:
      "Public advisories and threat intel, kept separate from your local Review queue.",
  },
  repositories: {
    title: "Repositories",
    description:
      "Repository inventory sorted by the repos that need attention first.",
  },
  settings: {
    title: "Settings",
    description:
      "Manage repository sources and keep the scan cadence predictable.",
  },
};

const SETTINGS_PAGE_META: Record<SettingsPage, { title: string; description: string }> = {
  general: {
    title: "General",
    description: "Appearance, repository sources, scan cadence, and local scanner updates.",
  },
  providers: {
    title: "AI Providers",
    description: "Codex, Grok, and OpenCode are discovered automatically for repository chat.",
  },
};

function withReviewUpdate(snapshot: AppSnapshot, { reviewSummary, revision }: ReviewUpdate): AppSnapshot {
  return {
    ...snapshot,
    // Failed attempts are ordered by the scan-history revision in snapshots.
    reviewSummary: { ...reviewSummary, failedScans: snapshot.reviewSummary.failedScans },
    dataRevisions: { ...snapshot.dataRevisions, review: revision },
  };
}

function withScanProgress(snapshot: AppSnapshot, activity: ScanActivity): AppSnapshot {
  return {
    ...snapshot,
    scanActivity: activity,
    workspaceStatus: { ...snapshot.workspaceStatus, scanRunning: activity.status === "running" },
  };
}

function keepCurrentGithubAuth(current: GithubAuthSnapshot, incoming: GithubAuthSnapshot): boolean {
  const currentSession = current.sessionRevision ?? 0;
  const incomingSession = incoming.sessionRevision ?? 0;
  return (current.configRevision ?? 0) > (incoming.configRevision ?? 0)
    || currentSession > incomingSession
    || (currentSession === incomingSession && current.signedIn && Boolean(incoming.checking));
}

function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot>(EMPTY_STATE);
  const [dataRevision, setDataRevision] = useState(0);
  const [reviewEvidenceRevision, setReviewEvidenceRevision] = useState(0);
  const lastSnapshot = useRef<AppSnapshot | null>(null);
  const snapshotSequence = useRef(0);
  const snapshotRequestSequence = useRef(0);
  const scanProgress = useRef<ScanActivity | null>(null);
  const intelSequence = useRef(0);
  const applyGithubAuth = useCallback((githubAuth: GithubAuthSnapshot) => {
    const previous = lastSnapshot.current;
    if (previous && keepCurrentGithubAuth(previous.githubAuth, githubAuth)) return;
    if (previous) lastSnapshot.current = { ...previous, githubAuth };
    setSnapshot((current) => ({ ...current, githubAuth }));
  }, []);
  const applySnapshot = useCallback((state: AppSnapshot) => {
    const previous = lastSnapshot.current;
    const old = previous?.dataRevisions;
    const currentActivity = scanProgress.current;
    const receivedActivity = state.scanActivity;
    const newerActivity = receivedActivity && receivedActivity.sequence > (currentActivity?.sequence ?? 0);
    // Independent background tasks can deliver their SQLite snapshots out of order.
    // Review-only updates are merged below because they have their own IPC path.
    if (old && (
      old.inventory > state.dataRevisions.inventory ||
      old.scanHistory > state.dataRevisions.scanHistory ||
      old.intel > state.dataRevisions.intel ||
      (old.advisory ?? 0) > (state.dataRevisions.advisory ?? 0) ||
      old.scanner > state.dataRevisions.scanner ||
      old.notifications > state.dataRevisions.notifications ||
      old.settings > state.dataRevisions.settings
    )) {
      if (newerActivity) {
        scanProgress.current = receivedActivity;
        lastSnapshot.current = withScanProgress(previous, receivedActivity);
        setSnapshot(lastSnapshot.current);
      }
      return false;
    }
    if (newerActivity) scanProgress.current = receivedActivity;
    if (scanProgress.current && scanProgress.current !== receivedActivity) {
      // A newer snapshot can hold the scanner lock for a database update.
      // An earlier, terminal scan only replaces another copy of that activity.
      state = scanProgress.current.status === "running" || receivedActivity?.status === "running"
        ? withScanProgress(state, scanProgress.current)
        : { ...state, scanActivity: scanProgress.current };
    }
    snapshotSequence.current += 1;
    // Keep a newer review result while accepting the rest of a background snapshot.
    if (previous && previous.dataRevisions.review > state.dataRevisions.review) {
      state = withReviewUpdate(state, { reviewSummary: previous.reviewSummary, revision: previous.dataRevisions.review });
    }
    if (previous && keepCurrentGithubAuth(previous.githubAuth, state.githubAuth)) {
      state = { ...state, githubAuth: previous.githubAuth };
    }
    const next = state.dataRevisions;
    const inventoryChanged = !old || old.inventory !== next.inventory;
    const intelChanged = inventoryChanged || old?.intel !== next.intel;
    const reviewEvidenceChanged = intelChanged || old?.advisory !== next.advisory;
    if (inventoryChanged) setDataRevision((current) => current + 1);
    if(inventoryChanged||intelChanged||old?.advisory!==next?.advisory){
      const prior=new Map((previous?.repos??[]).map((repo)=>[repo.id,JSON.stringify(repo)]));
      const incoming=new Map(state.repos.map((repo)=>[repo.id,JSON.stringify(repo)]));
      for(const id of new Set([...prior.keys(),...incoming.keys()]))if(prior.get(id)!==incoming.get(id))remediationProjectionCache.invalidateRepository(id);
    }
    if (intelChanged) intelSequence.current += 1;
    if (reviewEvidenceChanged) setReviewEvidenceRevision((current) => current + 1);
    lastSnapshot.current=state;
    setSnapshot(state);
    return true;
  }, []);
  const [isLoading, setIsLoading] = useState(true);
  const [isScanning, setIsScanning] = useState(false);
  const requestedScan = useRef(false);
  const [isUpdatingVulnerabilityDb, setIsUpdatingVulnerabilityDb] =
    useState(false);
  const [isClearingNotifications, setIsClearingNotifications] =
    useState(false);
  const [isNotificationBellOpen, setIsNotificationBellOpen] = useState(false);
  const [homeNeedsIntel,setHomeNeedsIntel]=useState(false);
  const [intelFeed, setIntelFeed] = useState<ExternalIntelFeed | null>(null);
  const [intelFeedError, setIntelFeedError] = useState<string | null>(null);
  const [riskTrendRange, setRiskTrendRange] = useState<RiskTrendRange>(7);
  const trendRequest = useRef<{ revision: number; day: string; data: Promise<RiskTrendPoint[]> } | null>(null);
  const [isRefreshingExternalIntel, setIsRefreshingExternalIntel] =
    useState(false);
  const [isSavingExternalIntelTopics, setIsSavingExternalIntelTopics] =
    useState(false);
  const [externalIntelRefreshError, setExternalIntelRefreshError] = useState<string | null>(null);
  const [intelSelection, setIntelSelection] = useState(NO_INTEL_SELECTION);
  const [error, setError] = useState<string | null>(null);
  const [activeSection, setActiveSection] =
    useState<DashboardSection>(() => {
      if (!import.meta.env.DEV) return "overview";
      const workspace = new URLSearchParams(window.location.search).get("workspace");
      return workspace && ["overview", "review", "chat", "intel", "repositories", "settings"].includes(workspace) ? workspace as DashboardSection : "overview";
    });
  const [settingsPage, setSettingsPage] = useState<SettingsPage>(() => {
    if (!import.meta.env.DEV) return "general";
    return new URLSearchParams(window.location.search).get("settingsPage") === "providers" ? "providers" : "general";
  });
  const [settingsFocus, setSettingsFocus] = useState<{ section: "github" | "scanner"; fromImport?: boolean }>();
  const settingsReturnSection = useRef<DashboardSection>("overview");
  const [selectedRepoId, setSelectedRepoId] = useState<number | null>(null);
  const [isGithubComposerOpen, setIsGithubComposerOpen] = useState(false);
  const [githubImportDraft, setGithubImportDraft] = useState<GithubImportDraft>({ url: "", error: null });
  const [githubComposerFocusToken, setGithubComposerFocusToken] = useState(0);
  const [isOnboardingOpen, setIsOnboardingOpen] = useState(false);
  const [{ state: initialViewsState, warning: initialViewsWarning }] = useState(loadReviewViews);
  const [reviewViewsState, setReviewViewsState] = useState(initialViewsState);
  const [reviewOpenSequence, setReviewOpenSequence] = useState(0);
  const [evidenceReads] = useState(createEvidenceReads);
  const [navigationHistory] = useState(() => {
    const history = new WorkspaceHistory();
    history.push({ workspace: activeSection, context: activeSection === "review" ? { viewId: initialViewsState.defaultViewId } : {} });
    return history;
  });
  const [repositoryNavigationContext, setRepositoryNavigationContext] = useState<WorkspaceContext>({});
  const [reviewNavigationContext, setReviewNavigationContext] = useState<WorkspaceContext>({ viewId: initialViewsState.defaultViewId });
  const [isCommandPaletteOpen, setIsCommandPaletteOpen] = useState(false);
  const [isScanActivityOpen, setIsScanActivityOpen] = useState(false);
  const [repositoryOpenSequence, setRepositoryOpenSequence] = useState(0);
  const repositoryPackageLinkSequence = useRef(0);
  const [chatFindingRequest, setChatFindingRequest] = useState<FindingChatRequest>();

  const handleReviewContextChange = useCallback((context: WorkspaceContext) => {
    setReviewNavigationContext(context);
    navigationHistory.replaceContext("review", context);
  }, [navigationHistory]);

  const handleRepositoryContextChange = useCallback((context: WorkspaceContext) => {
    navigationHistory.replaceContext("repositories", context);
  }, [navigationHistory]);

  const handleIntelFeedViewChange = useCallback((intelFeedView: IntelFeedView) => {
    navigationHistory.replaceContext("intel", { ...navigationHistory.context("intel"), intelFeedView });
  }, [navigationHistory]);

  function handleIntelSelectionChange(selection: IntelSelection) {
    setIntelSelection(selection);
    navigationHistory.replaceContext("intel", { ...navigationHistory.context("intel"), intelSelection: selection });
  }

  const refreshSnapshot = useCallback(async (initial = false) => {
    const request = ++snapshotRequestSequence.current;
    const sequence = snapshotSequence.current;
    const state = await (initial ? bootstrap() : getSnapshot());
    if (request === snapshotRequestSequence.current && sequence === snapshotSequence.current) applySnapshot(state);
    return state;
  }, [applySnapshot]);

  const handleReviewStateChange = useCallback(async (update: ReviewUpdate | null) => {
    // The write can succeed even if its subsequent counter read fails.
    if (!update) {
      await refreshSnapshot();
      return;
    }
    const previous = lastSnapshot.current;
    if (!previous || update.revision < previous.dataRevisions.review) return;
    lastSnapshot.current = withReviewUpdate(previous, update);
    setSnapshot((current) => withReviewUpdate(current, update));
  }, [refreshSnapshot]);

  async function runSnapshotMutation(action: () => Promise<AppSnapshot>) {
    const sequence = snapshotSequence.current;
    const state = await action();
    if (sequence === snapshotSequence.current) {
      applySnapshot(state);
      return state;
    }
    // Preserve the newer event, but fetch again so this mutation is included too.
    return refreshSnapshot();
  }

  const handleChildSnapshot = useCallback(() => {
    // Child callbacks do not carry the revision at which their command started.
    void refreshSnapshot().catch((cause) => setError(String(cause)));
  }, [refreshSnapshot]);

  const goBack = useCallback(() => {
    const previous = navigationHistory.back();
    if (!previous) return false;
    if (previous.workspace === "review") {
      setReviewNavigationContext(previous.context);
      setReviewOpenSequence((sequence) => sequence + 1);
    }
    if (previous.workspace === "intel") {
      setIntelSelection(previous.context.intelSelection ?? NO_INTEL_SELECTION);
    }
    if (previous.workspace === "chat") setChatFindingRequest(previous.context.chatFinding);
    if (previous.workspace === "repositories") {
      setSelectedRepoId(previous.context.repoId ?? null);
      setRepositoryNavigationContext({
        ...previous.context,
        repositoryFocusOnReturn: previous.context.repositoryDetailOpen,
      });
      setRepositoryOpenSequence((sequence) => sequence + 1);
    } else if (previous.context.repoId) {
      setSelectedRepoId(previous.context.repoId);
    }
    setActiveSection(previous.workspace);
    return true;
  }, [navigationHistory]);

  useEffect(() => {
    const handleHistoryKey = (event: KeyboardEvent) => {
      if (event.key !== "ArrowLeft" || !event.altKey || event.ctrlKey || event.metaKey || event.shiftKey
        || event.defaultPrevented || event.isComposing || event.repeat) return;
      const target = event.target instanceof Element ? event.target : document.activeElement;
      if (target?.closest(BACK_SHORTCUT_EXCLUDED_TARGETS)) return;
      if (goBack()) {
        event.preventDefault();
        event.stopPropagation();
      }
    };
    // Handle the workspace shortcut before a grid consumes the arrow key.
    window.addEventListener("keydown", handleHistoryKey, true);
    return () => window.removeEventListener("keydown", handleHistoryKey, true);
  }, [goBack]);

  async function loadSnapshot(initial = false): Promise<AppSnapshot | null> {
    setError(null);
    const sequence = snapshotSequence.current;
    try {
      return await refreshSnapshot(initial);
    } catch (cause) {
      if (sequence === snapshotSequence.current) setError(cause instanceof Error ? cause.message : String(cause));
      return null;
    } finally {
      setIsLoading(false);
    }
  }

  useEffect(() => {
    void loadSnapshot(true);
  }, []);

  useEffect(() => {
    if (initialViewsWarning) setError(initialViewsWarning);
  }, [initialViewsWarning]);

  useEffect(() => {
    const handler = (event: globalThis.KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setIsCommandPaletteOpen((value) => !value);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  useEffect(() => {
    let cancelled = false;
    const disposers: Array<() => void> = [];
    const register = async (subscription: Promise<() => void>) => {
      try {
        const dispose = await subscription;
        if (cancelled) dispose(); else disposers.push(dispose);
      } catch (cause) {
        if (!cancelled) setError(String(cause));
      }
    };
    void register(listen<ScanActivity>("cvec://scan-activity", (event) => {
      if (cancelled) return;
      if (event.payload.sequence <= (scanProgress.current?.sequence ?? 0)) return;
      scanProgress.current = event.payload;
      if (lastSnapshot.current) lastSnapshot.current = withScanProgress(lastSnapshot.current, event.payload);
      setSnapshot((current) => withScanProgress(current, event.payload));
    }));
    void register(listen<AppSnapshot>("cvec://state", (event) => {
      if (!cancelled) { applySnapshot(event.payload); setIsLoading(false); }
    }));
    void register(listen<string>("cvec://error", (event) => {
      if (!cancelled) setError(event.payload);
    }));
    return () => {
      cancelled = true;
      disposers.forEach((dispose) => dispose());
      snapshotSequence.current += 1;
      intelSequence.current += 1;
    };
  }, [applySnapshot]);

  const needsIntel = activeSection === "intel" || (activeSection === "overview" && homeNeedsIntel);
  const { intel: currentIntelRevision, inventory: currentInventoryRevision } = snapshot.dataRevisions;
  const trendDay = new Date().toISOString().slice(0, 10);
  const readRiskTrend = useCallback(() => {
    const pending = trendRequest.current;
    if (pending?.revision === currentInventoryRevision && pending.day === trendDay) return pending.data;
    const data = listRiskTrend(30).catch((cause) => {
      if (trendRequest.current?.data === data) trendRequest.current = null;
      throw cause;
    });
    trendRequest.current = { revision: currentInventoryRevision, day: trendDay, data };
    return data;
  }, [currentInventoryRevision, trendDay]);
  const hasSnapshot = lastSnapshot.current !== null;
  const intelFeedCurrent =
    intelFeed !== null &&
    intelFeed.intelRevision >= currentIntelRevision &&
    intelFeed.inventoryRevision >= currentInventoryRevision;

  const loadIntelFeed = useCallback(async () => {
    const sequence = ++intelSequence.current;
    setIntelFeedError(null);
    try {
      const feed = await listExternalIntelFeed();
      if (sequence === intelSequence.current) setIntelFeed(feed);
    } catch (cause) {
      if (sequence === intelSequence.current) setIntelFeedError(String(cause));
    }
  }, []);

  useEffect(() => {
    if (needsIntel && hasSnapshot && !intelFeedCurrent) void loadIntelFeed();
  }, [
    hasSnapshot, intelFeedCurrent, currentIntelRevision, currentInventoryRevision,
    needsIntel, loadIntelFeed,
  ]);

  async function handleScanAll(): Promise<ScanAllSummary | null> {
    if (requestedScan.current || scanProgress.current?.status === "running") {
      setIsScanActivityOpen(true);
      return null;
    }
    if (scannerUnavailable || !snapshot.repos.length) return null;
    requestedScan.current = true;
    setIsScanning(true);
    setError(null);
    try {
      const { summary, snapshot } = await scanAll();
      toast(describeScanResult(summary));
      if (!snapshot || !applySnapshot(snapshot)) await loadSnapshot();
      return summary;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return null;
    } finally {
      requestedScan.current = false;
      setIsScanning(false);
    }
  }

  async function handleChooseFolder(): Promise<AppSnapshot | null> {
    setError(null);
    try {
      const nextSnapshot = await runSnapshotMutation(() => chooseDiscoveryRoot());
      return nextSnapshot;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return null;
    }
  }

  async function handleAddGithubRepository(url: string) {
    setError(null);
    const nextSnapshot = await runSnapshotMutation(() => addGithubRepository(url));

    const addedRepo = findAddedGithubRepo(nextSnapshot.repos, url);
    if (addedRepo) {
      handleSelectRepo(addedRepo.id);
    }
  }

  function handleShowGithubRepositoryComposer() {
    if (activeSection === "settings" && settingsFocus?.fromImport && goBack()) {
      setSettingsFocus(undefined);
    } else {
      handleNavigate("repositories");
    }
    setIsGithubComposerOpen(true);
    setGithubComposerFocusToken((token) => token + 1);
  }

  function handleOpenGithubAccount() {
    setSettingsPage("general");
    handleNavigate("settings");
    setSettingsFocus({ section: "github", fromImport: true });
  }

  async function handleSettingsRefresh(action: () => Promise<AppSnapshot>) {
    try {
      return await runSnapshotMutation(action);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return null;
    }
  }

  async function handleScheduleChange(nextInterval: "off" | "12h"): Promise<AppSnapshot | null> {
    setError(null);
    try {
      const nextSnapshot = await runSnapshotMutation(() => setSchedule(nextInterval));
      return nextSnapshot;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return null;
    }
  }

  async function handleUpdateVulnerabilityDb(): Promise<AppSnapshot | null> {
    setIsUpdatingVulnerabilityDb(true);
    setError(null);
    try {
      const nextSnapshot = await runSnapshotMutation(() => updateVulnerabilityDb());
      const feedback = buildDbUpdateFeedback(nextSnapshot.versionInfo.lastGrypeDbUpdate);
      toast({ kind: "success", ...feedback });
      return nextSnapshot;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return null;
    } finally {
      setIsUpdatingVulnerabilityDb(false);
    }
  }

  async function handleCompleteOnboarding(section?: DashboardSection) {
    setError(null);
    try {
      await runSnapshotMutation(() => completeOnboarding());
      setIsOnboardingOpen(false);
      if (section) {
        setActiveSection(section);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  async function handleSkipOnboarding() {
    setError(null);
    try {
      await runSnapshotMutation(() => skipOnboarding());
      setIsOnboardingOpen(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  async function handleClearNotification(notificationId: number) {
    setIsClearingNotifications(true);
    setError(null);
    try {
      await runSnapshotMutation(() => clearNotification(notificationId));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsClearingNotifications(false);
    }
  }

  async function handleClearAllNotifications() {
    setIsClearingNotifications(true);
    setError(null);
    try {
      await runSnapshotMutation(() => clearAllNotifications());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsClearingNotifications(false);
    }
  }

  async function handleRefreshExternalIntel() {
    setIsRefreshingExternalIntel(true);
    setExternalIntelRefreshError(null);

    try {
      await runSnapshotMutation(() => refreshExternalIntel());
    } catch (cause) {
      setExternalIntelRefreshError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsRefreshingExternalIntel(false);
    }
  }

  function handleSelectRepo(repoId: number, packageQuery?: string) {
    openRepositoryContext({
      repoId,
      repositorySection: packageQuery !== undefined ? "dependencies" : "overview",
      repositoryPackageQuery: packageQuery,
      repositoryPackageLinkSequence: packageQuery === undefined ? undefined : ++repositoryPackageLinkSequence.current,
      repositoryDetailOpen: true,
    });
  }

  function handleOpenIntelPackage(repoId: number, exactPackage: ScannedPackageIdentity) {
    openRepositoryContext({
      repoId,
      repositorySection: "dependencies",
      repositoryExactPackage: exactPackage,
      repositoryPackageLinkSequence: ++repositoryPackageLinkSequence.current,
      repositoryDetailOpen: true,
    });
  }

  function openRepositoryContext(context: WorkspaceContext) {
    const current = navigationHistory.context("repositories");
    if (activeSection === "repositories" && current.repoId === context.repoId
      && current.repositorySection === context.repositorySection
      && current.repositoryScanRunId === context.repositoryScanRunId
      && Boolean(current.repositoryFailedOnly) === Boolean(context.repositoryFailedOnly)
      && Boolean(current.repositoryDetailOpen) === Boolean(context.repositoryDetailOpen)
      && context.repositoryPackageLinkSequence === undefined) return;
    setChatFindingRequest(undefined);
    setSelectedRepoId(context.repoId ?? null);
    setRepositoryNavigationContext(context);
    setRepositoryOpenSequence((previous) => previous + 1);
    navigationHistory.push({ workspace: "repositories", context }, { preserveCurrent: true });
    setActiveSection("repositories");
  }

  function handleOpenFailedScans() {
    openRepositoryContext({
      repoId: snapshot.repos.find((repo) => repo.lastScanStatus === "failed")?.id,
      repositorySection: "history",
      repositoryFailedOnly: true,
    });
  }

  function handleOpenScanHistory(repoId: number, scanRunId: number) {
    setIsScanActivityOpen(false);
    openRepositoryContext({
      repoId,
      repositorySection: "history",
      repositoryScanRunId: scanRunId,
      repositoryDetailOpen: true,
    });
  }

  function handleContinueFindingInChat(finding: FindingChatRequest) {
    setSelectedRepoId(finding.repositoryId);
    setChatFindingRequest(finding);
    navigationHistory.push({ workspace: "chat", context: { repoId: finding.repositoryId, chatFinding: finding } });
    setActiveSection("chat");
  }

  function handleOpenReviewView(viewId: string) {
    const context = { viewId };
    setReviewNavigationContext(context);
    setReviewOpenSequence((previous) => previous + 1);
    navigationHistory.push({ workspace: "review", context });
    setActiveSection("review");
  }

  function handleOpenRepositoryReview({ origin, ...request }: ReviewOpenRequest) {
    const fingerprint = request.remediationGroup ? undefined : request.findingFingerprint;
    const { resolvedInScanRunId } = request;
    const query = resolvedInScanRunId !== undefined ? reviewQuery({
      repoIds: [request.repositoryId],
      scannerStates: ["resolved"],
      resolvedInScanRunId,
    }) : reviewQuery({
      repoIds: [request.repositoryId],
      // An exact finding remains addressable if its state changes after
      // the search result was loaded.
      scannerStates: fingerprint ? [] : ["new", "open"],
      fingerprint,
      packages: request.installedPackage || request.remediationGroup || fingerprint ? [] : request.packageIdentity ? [request.packageIdentity] : [],
      packageIdentity:request.installedPackage??null,
    });
    const context: WorkspaceContext = {
      viewId: resolvedInScanRunId === undefined ? "all-open" : `scan-resolution:${resolvedInScanRunId}`,
      reviewQuery: query,
      reviewPresentation: DEFAULT_TABLE_PRESENTATION,
      selectedKey: request.findingFingerprint || request.remediationGroup?.preferredFingerprint ? `${request.repositoryId}:${request.findingFingerprint ?? request.remediationGroup?.preferredFingerprint}` : undefined,
      ephemeralReviewContext: true,
      remediationGroup: request.remediationGroup,
    };
    if (origin?.section === "chat") {
      navigationHistory.push({ workspace: "chat", context: { repoId: origin.repositoryId, chatFinding: chatFindingRequest } });
    } else if (origin) {
      navigationHistory.push({
        workspace: "repositories",
        context: {
          ...navigationHistory.context("repositories"),
          repoId: origin.repositoryId,
          repositorySection: origin.section,
          repositoryDetailOpen: true,
          ...(resolvedInScanRunId === undefined ? {} : { repositoryScanRunId: resolvedInScanRunId }),
          repositoryScrollAnchor: origin.scrollAnchor,
        },
      });
    }
    setReviewNavigationContext(context);
    setReviewOpenSequence((previous) => previous + 1);
    navigationHistory.push({ workspace: "review", context });
    setActiveSection("review");
  }

  function handleNavigate(workspace: WorkspaceId) {
    const normalized = normalizeWorkspace(workspace);
    setSettingsFocus(undefined);
    if (normalized === "settings") {
      if (activeSection !== "settings") settingsReturnSection.current = activeSection;
    } else {
      settingsReturnSection.current = normalized;
    }
    const context: WorkspaceContext = normalized === "review" ? reviewNavigationContext
      : normalized === "repositories" ? { repoId: selectedRepoId ?? undefined, ...navigationHistory.context("repositories") }
      : normalized === "chat" ? { repoId: selectedRepoId ?? undefined, chatFinding: chatFindingRequest }
      : normalized === "intel" ? navigationHistory.context("intel")
      : {};
    if (normalized === "repositories") {
      setSelectedRepoId(context.repoId ?? null);
      setRepositoryNavigationContext(context);
    }
    navigationHistory.push({ workspace: normalized, context });
    setActiveSection(normalized);
  }

  function handleOpenAiConnections() {
    setSettingsPage("providers");
    handleNavigate("settings");
  }

  function handleOpenEntity(result: LocalEntityResult) {
    if (result.repoId) {
      if (result.kind === "finding" && result.fingerprint) {
        handleOpenRepositoryReview({ repositoryId: result.repoId, findingFingerprint: result.fingerprint });
        return;
      }
      handleSelectRepo(result.repoId, result.kind === "package" ? result.label : undefined);
    } else if (result.kind === "advisory") {
      handleIntelSelectionChange({ canonicalKey: result.label, detailOpen: true, direct: true });
      handleNavigate("intel");
    }
  }

  async function handleScanRepoFromShell(repoId: number) {
    setIsScanActivityOpen(true);
    return runRepositoryScan(repoId);
  }

  async function runRepositoryScan(repoId: number): Promise<void> {
    if (requestedScan.current || scanProgress.current?.status === "running") {
      setIsScanActivityOpen(true);
      return;
    }
    if (scannerUnavailable) return;
    requestedScan.current = true;
    setIsScanning(true);
    setError(null);
    try {
      const state = await scanRepo(repoId);
      if (!applySnapshot(state)) await loadSnapshot();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      requestedScan.current = false;
      setIsScanning(false);
    }
  }

  function handleOpenScannerSettings() {
    setSettingsPage("general");
    handleNavigate("settings");
    setSettingsFocus({ section: "scanner" });
  }

  async function handleExternalIntelTopicsChange(topicKeys: string[]) {
    setIsSavingExternalIntelTopics(true);
    setExternalIntelRefreshError(null);
    setError(null);

    try {
      await runSnapshotMutation(() => setExternalIntelTopics(topicKeys));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsSavingExternalIntelTopics(false);
    }
  }

  const workspaceStatus = snapshot.workspaceStatus;
  const scannerUnavailable = scannerIssue(workspaceStatus, snapshot.versionInfo.grypeDb);
  const { scanActivity } = snapshot;
  const sectionMeta = activeSection === "settings" ? SETTINGS_PAGE_META[settingsPage] : SECTION_META[activeSection];
  const isBusy = isScanning || workspaceStatus.scanRunning || scanActivity?.status === "running";
  const reviewViews = useMemo(() => allReviewViews(reviewViewsState), [reviewViewsState]);
  const favoriteViews = reviewViewsState.favoriteIds
    .map((id) => reviewViews.find((view) => view.id === id))
    .filter((view): view is NonNullable<typeof view> => Boolean(view));
  const shouldShowOnboarding =
    snapshot.onboarding.status === "not_started" || isOnboardingOpen;
  const hasWorkspace = lastSnapshot.current !== null;
  const startupError = hasWorkspace ? null : error;
  useEffect(() => {
    if (!error || !hasWorkspace || shouldShowOnboarding) return;
    toast({ title: error, kind: "error", timeout: 6000 });
    setError(null);
  }, [error, hasWorkspace, shouldShowOnboarding]);
  const githubDisabledReason = scannerUnavailable
    ? scannerUnavailable
    : isBusy
      ? "Adding is paused while a scan is running."
      : null;
  const activeDashboardAdvisories = useMemo(
    () => intelFeed?.items.filter((item) => !item.withdrawnAt) ?? [],
    [intelFeed],
  );
  const dashboardContext = {
    snapshot,
    riskTrend: {
      inventoryRevision: currentInventoryRevision,
      day: trendDay,
      hasCompletedScan: Boolean(snapshot.workspaceStatus.lastSuccessfulScanAt),
      read: readRiskTrend,
    },
    riskTrendRange,
    externalIntelItems: activeDashboardAdvisories,
    externalIntelTotal: intelFeed?.items.length ?? 0,
    externalIntelLoading: intelFeed === null && intelFeedError === null,
    externalIntelFailed: intelFeedError !== null,
    onRetryIntel: loadIntelFeed,
    isScanning: isBusy,
    selectedRepoId,
    onAddGithubRepository: handleShowGithubRepositoryComposer,
    onChooseFolder: handleChooseFolder,
    onRangeChange: setRiskTrendRange,
    onScanAll: handleScanAll,
    onOpenScannerSettings: handleOpenScannerSettings,
    onSelectRepo: handleSelectRepo,
    onOpenReview: handleOpenReviewView,
    onOpenFailedScans: handleOpenFailedScans,
  };
  const onboardingContext = {
    snapshot,
    githubDisabledReason,
    error,
    isScanning: isBusy,
    isUpdatingVulnerabilityDb,
    onAddGithubRepository: handleAddGithubRepository,
    onChooseFolder: handleChooseFolder,
    onClose: () => {
      if (snapshot.onboarding.status === "not_started") {
        void handleSkipOnboarding();
        return;
      }
      setIsOnboardingOpen(false);
    },
    onComplete: handleCompleteOnboarding,
    onGithubAuthChange: applyGithubAuth,
    onRefresh: () => handleSettingsRefresh(() => checkVulnerabilityDb(false)),
    onScheduleChange: handleScheduleChange,
    onScanAll: handleScanAll,
    onSkip: handleSkipOnboarding,
    onUpdateVulnerabilityDb: handleUpdateVulnerabilityDb,
  };

  if (isLoading) {
    return (
      <main className="relative flex min-h-screen items-center justify-center bg-[var(--background)] p-0">
        <div className="absolute inset-x-0 top-0 h-[var(--shell-topbar-height)]" data-tauri-drag-region="deep" />
        <Card className="app-panel w-full max-w-md rounded-lg border shadow-none">
          <CardContent className="flex items-center justify-center py-16 text-sm text-ink-subtle">
            Loading cvec...
          </CardContent>
        </Card>
      </main>
    );
  }

  const WorkspaceViewport = activeSection === "chat" ? "div" : ScrollArea;
  return (
    <EvidenceReadsContext value={evidenceReads}>
    <main className="h-full min-h-0 bg-transparent p-0">
      <div className="app-shell relative h-full min-h-0 overflow-hidden">
        <div className="app-topbar absolute inset-x-0 top-0 z-30 flex h-[var(--shell-topbar-height)]">
          <div
            className="app-topbar-sidebar flex w-[var(--shell-sidebar-width)] shrink-0 items-center gap-2.5 px-3 pl-[var(--window-controls-inset)]"
            data-tauri-drag-region="deep"
          >
            <img
              src="/app-icon.png"
              alt=""
              aria-hidden="true"
              width={24}
              height={24}
              draggable={false}
              className="pointer-events-none size-6 shrink-0 rounded-md"
            />
            <span className="pointer-events-none text-xs font-semibold uppercase tracking-[0.16em] text-ink-subtle">
              cvec
            </span>
          </div>
          <div
            className="app-topbar-content flex min-w-0 flex-1 items-center justify-between px-4 lg:px-5"
            data-tauri-drag-region="deep"
          >
            <div data-no-window-drag className="flex min-w-0 items-center gap-2">
              <IconButton
                aria-label="Go back"
                aria-keyshortcuts="Alt+ArrowLeft"
                title="Go back (Option + Left Arrow)"
                variant="ghost"
                onClick={goBack}
                disabled={!navigationHistory.canGoBack}
              >
                <AppIcon className="size-4" icon={ArrowLeft} />
              </IconButton>
              <Button aria-label="Search or run command" className="justify-between text-ink-subtle md:min-w-64" size="sm" variant="secondary" onClick={() => setIsCommandPaletteOpen(true)}>
                <AppIcon className="size-4 md:hidden" icon={MagnifyingGlass} />
                <span className="hidden md:inline">Search or run command</span><kbd className="hidden rounded-sm border border-hairline px-1.5 py-0.5 text-xs md:inline">⌘K</kbd>
              </Button>
            </div>
            <div data-no-window-drag className="flex items-center gap-1">
              <ScanActivityPopover
                activity={scanActivity}
                open={isScanActivityOpen}
                onOpenChange={setIsScanActivityOpen}
                retryDisabledReason={isBusy ? "Retry is available after the current operation finishes."
                  : scannerUnavailable ? scannerUnavailable
                  : undefined}
                onReview={(scanRunIds) => { setIsScanActivityOpen(false); handleOpenReviewView(`scan:${scanRunIds.join(",")}`); }}
                onResolved={(scanRunIds) => { setIsScanActivityOpen(false); handleOpenReviewView(`scan-resolved:${scanRunIds.join(",")}`); }}
                onHistory={handleOpenScanHistory}
                onRetry={(repoId) => void handleScanRepoFromShell(repoId)}
              />
              <NotificationBell
              isClearing={isClearingNotifications}
              isOpen={isNotificationBellOpen}
              notifications={snapshot.notifications}
              openTarget={(notification) => {
                if (notification.kind === "db_update") return { label: "Open scanner settings", open: handleOpenScannerSettings };
                const repoId = notification.repoId;
                if (repoId === null || !snapshot.repos.some((repository) => repository.id === repoId)) return null;
                const runId = notification.scanRunId;
                if (runId) return { label: "Review scan #" + runId, open: () => handleOpenReviewView("scan:" + runId) };
                return { label: "Open repository scan history", open: () => openRepositoryContext({
                  repoId,
                  repositorySection: "history",
                  repositoryDetailOpen: true,
                }) };
              }}
              onClear={handleClearNotification}
              onClearAll={handleClearAllNotifications}
              onClose={() => setIsNotificationBellOpen(false)}
              onToggle={() =>
                setIsNotificationBellOpen((current) => !current)
              }
              />
            </div>
          </div>
        </div>

        <div className="flex h-full pt-[var(--shell-topbar-height)]">
          <aside className="app-sidebar relative h-full w-[var(--shell-sidebar-width)] shrink-0">
            {activeSection === "settings" ? (
              <SettingsSidebar
                activePage={settingsPage}
                onBack={() => { if (!goBack()) handleNavigate(settingsReturnSection.current); }}
                onSelect={setSettingsPage}
              />
            ) : (
              <DashboardSidebar
                onPrepare={prepareWorkspace}
                activeSection={activeSection}
                reviewCount={snapshot.reviewSummary.needsReview}
                favoriteViews={favoriteViews.map((view) => ({ id: view.id, name: view.name }))}
                onSelectView={handleOpenReviewView}
                onSelect={(section) => handleNavigate(section)}
              />
            )}
          </aside>

          <section data-active-workspace={activeSection} className="app-content relative min-w-0 flex-1 overflow-hidden">
            <WorkspaceViewport className={`relative z-10 h-full min-h-0 px-4 pb-4 pt-3.5 lg:px-5 lg:pb-5 lg:pt-3.5${activeSection === "chat" ? " chat-workspace-viewport overflow-hidden" : ""}`}>
              <div
                className={
                  activeSection === "overview" || activeSection === "review" || activeSection === "chat"
                    ? "flex h-full min-h-0 flex-col pr-1.5"
                    : "min-h-full pr-1.5"
                }
              >
                {startupError ? <Alert className="mb-4" variant="destructive">
                  <AlertTitle>Something went wrong</AlertTitle>
                  <AlertDescription>{startupError}</AlertDescription>
                </Alert> : null}

                <section
                  className={
                    activeSection === "overview" || activeSection === "review" || activeSection === "chat"
                      ? "flex min-h-0 flex-1 flex-col space-y-3.5"
                      : "space-y-3.5"
                  }
                >
                  {activeSection !== "overview" && activeSection !== "chat" ? (
                    <PageHeader
                      action={
                        activeSection === "repositories" ? (
                          <div className="flex flex-wrap items-center gap-2">
                            <Button variant={snapshot.repos.length === 0 ? "default" : "secondary"} onClick={handleChooseFolder}>Choose folder</Button>
                            <Button variant="secondary" onClick={handleShowGithubRepositoryComposer}>Add GitHub repo</Button>
                            <Button disabled={(Boolean(scannerUnavailable) || snapshot.repos.length === 0) && !isBusy} onClick={() => isBusy ? setIsScanActivityOpen(true) : void handleScanAll()}>
                              {isBusy ? "View scan" : "Scan all"}
                            </Button>
                          </div>
                        ) : undefined
                      }
                      description={sectionMeta.description}
                      title={sectionMeta.title}
                      titleRef={activeSection === "review" && reviewNavigationContext?.ephemeralReviewContext ? focusWorkspaceTitle : undefined}
                    />
                  ) : null}

                  {activeSection === "repositories" && scannerUnavailable ? (
                    <Alert variant="warning">
                      <AlertTitle>Scan setup required</AlertTitle><AlertDescription>{scannerUnavailable}</AlertDescription>
                      <Button className="mt-2" size="sm" variant="secondary" onClick={handleOpenScannerSettings}>Open scanner settings</Button>
                    </Alert>
                  ) : null}

                  {activeSection === "overview" && (
                    <DashboardOverview
                      onIntelDemandChange={setHomeNeedsIntel}
                      context={dashboardContext}
                      renderHeader={(actions) => (
                        <PageHeader
                          action={actions}
                          description={sectionMeta.description}
                          title={sectionMeta.title}
                        />
                      )}
                    />
                  )}

                  {activeSection === "review" && (
                    <ReviewWorkspace
                      key={reviewOpenSequence}
                      dataRevision={reviewEvidenceRevision}
                      reviewRevision={snapshot.dataRevisions.review}
                      initialContext={reviewNavigationContext}
                      onContextChange={handleReviewContextChange}
                      onReviewStateChange={handleReviewStateChange}
                      onError={setError}
                      onOpenRepo={handleSelectRepo}
                      onContinueInChat={handleContinueFindingInChat}
                      onOpenUrl={(url) => void openExternalUrl(url).catch((cause) => setError(String(cause)))}
                      onViewsStateChange={(state) => { saveReviewViews(state); setReviewViewsState(state); }}
                      scannerReady={!scannerUnavailable}
                      viewsState={reviewViewsState}
                    />
                  )}

                  {activeSection === "chat" && (
                    <ChatWorkspace
                      findingRequest={chatFindingRequest}
                      onError={setError}
                      onOpenAiConnections={handleOpenAiConnections}
                      onOpenRepository={handleSelectRepo}
                      onChooseLocalCheckout={handleChooseFolder}
                      onOpenScannerSettings={handleOpenScannerSettings}
                      scannerUnavailable={scannerUnavailable}
                      onOpenReview={handleOpenRepositoryReview}
                      onSelectRepo={(repoId) => {
                        setChatFindingRequest(undefined);
                        setSelectedRepoId(repoId);
                        navigationHistory.replaceContext("chat", { repoId });
                      }}
                      onClearFinding={() => {
                        setChatFindingRequest(undefined);
                        navigationHistory.replaceContext("chat", { repoId: selectedRepoId ?? undefined });
                      }}
                      repos={snapshot.repos}
                      selectedRepoId={selectedRepoId}
                    />
                  )}

                  {activeSection === "intel" && (
                    <ExternalSourcesWorkspace
                      feed={intelFeed}
                      feedError={intelFeedError}
                      initialFeedView={navigationHistory.context("intel").intelFeedView}
                      isCompactDetailOpen={intelSelection.detailOpen}
                      isDirectSelection={intelSelection.direct}
                      isLoading={!intelFeedCurrent && intelFeedError === null}
                      isRefreshing={isRefreshingExternalIntel}
                      isSavingTopics={isSavingExternalIntelTopics}
                      onCloseDetail={() => handleIntelSelectionChange({ ...intelSelection, detailOpen: false, direct: false })}
                      onFeedViewChange={handleIntelFeedViewChange}
                      onOpenRepository={handleOpenIntelPackage}
                      onRefresh={handleRefreshExternalIntel}
                      onReloadFeed={loadIntelFeed}
                      onSelect={(canonicalKey, mode) => handleIntelSelectionChange({ canonicalKey, detailOpen: mode === "open", direct: false })}
                      onTopicsChange={handleExternalIntelTopicsChange}
                      refreshError={externalIntelRefreshError}
                      selectedKey={intelSelection.canonicalKey}
                      summary={snapshot.externalIntelSummary}
                      topics={snapshot.externalIntelTopics}
                    />
                  )}

                  {activeSection === "repositories" && (
                    <>
                      {isGithubComposerOpen ? (
                        <Card className="app-panel shrink-0 rounded-lg border shadow-none">
                          <CardContent className="pt-0">
                            <div className="flex items-start justify-between gap-3">
                              <div className="flex min-w-0 items-start gap-3">
                                <div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-surface-3 text-ink-subtle">
                                  <AppIcon className="size-4" icon={GithubLogo} />
                                </div>
                                <div className="min-w-0">
                                  <p className="text-sm font-semibold text-ink">Add GitHub repository</p>
                                  <p className="mt-1 text-xs text-ink-subtle">Link a github.com repository and start its first local scan.</p>
                                </div>
                              </div>
                              <IconButton aria-label="Close GitHub repository composer" onClick={() => setIsGithubComposerOpen(false)} variant="ghost" type="button">
                                <AppIcon className="size-4" icon={X} />
                              </IconButton>
                            </div>
                            <GithubRepositoryForm className="mt-3" disabled={Boolean(githubDisabledReason)} disabledReason={githubDisabledReason} focusToken={githubComposerFocusToken}
                              draft={githubImportDraft} onDraftChange={setGithubImportDraft}
                              accountAction={snapshot.githubAuth.canConfigure === false ? undefined : {
                                label: snapshot.githubAuth.signedIn ? "GitHub account" : snapshot.githubAuth.configured ? "Sign in" : "Set up GitHub App",
                                onOpen: handleOpenGithubAccount,
                              }}
                              onAdd={handleAddGithubRepository} onCancel={() => setIsGithubComposerOpen(false)} onSuccess={() => setIsGithubComposerOpen(false)} submitLabel="Add repo" />
                          </CardContent>
                        </Card>
                      ) : null}
                      <RepositoryWorkspace
                        key={repositoryOpenSequence}
                        initialContext={repositoryNavigationContext}
                        onContextChange={handleRepositoryContextChange}
                        dataRevision={dataRevision}
                        isScanning={isBusy}
                        onError={setError}
                        onScanRepo={runRepositoryScan}
                        onSelectRepo={(repoId) => {
                          if (repoId !== selectedRepoId) setChatFindingRequest(undefined);
                          setSelectedRepoId(repoId);
                        }}
                        onSnapshot={handleChildSnapshot}
                        repos={snapshot.repos}
                        onOpenReview={handleOpenRepositoryReview}
                        onShowActivity={() => setIsScanActivityOpen(true)}
                        schedule={workspaceStatus.schedule}
                        scannerReady={!scannerUnavailable}
                        selectedRepoId={selectedRepoId}
                      />
                    </>
                  )}

                  {activeSection === "settings" && settingsPage === "providers" ? (
                    <AiConnectionsSettings />
                  ) : null}

                  {activeSection === "settings" && settingsPage === "general" && (
                    <SettingsPanel
                      discoveryRoot={snapshot.discoveryRoot}
                      isScanning={isBusy}
                      isUpdatingVulnerabilityDb={isUpdatingVulnerabilityDb}
                      githubDisabledReason={githubDisabledReason}
                      githubAuth={snapshot.githubAuth}
                      onGithubAuthChange={applyGithubAuth}
                      onGithubSnapshot={handleChildSnapshot}
                      onAddGithubRepository={handleAddGithubRepository}
                      onChooseFolder={handleChooseFolder}
                      focusRequest={settingsFocus}
                      onOpenGithubImport={handleShowGithubRepositoryComposer}
                      onRefreshRepositories={() => handleSettingsRefresh(refreshLocalRepositories)}
                      onCheckVulnerabilityDb={(checkUpdates) => handleSettingsRefresh(() => checkVulnerabilityDb(checkUpdates))}
                      onReplayOnboarding={() => setIsOnboardingOpen(true)}
                      onScheduleChange={handleScheduleChange}
                      onUpdateVulnerabilityDb={handleUpdateVulnerabilityDb}
                      status={workspaceStatus}
                      versionInfo={snapshot.versionInfo}
                    />
                  )}
                </section>
              </div>
            </WorkspaceViewport>
          </section>
        </div>

        <Dialog
          open={shouldShowOnboarding}
          onOpenChange={(open) => {
            if (!open) onboardingContext.onClose();
          }}
        >
          <DialogContent aria-label="cvec onboarding" fullScreen showClose={false}>
            <WelcomeOnboarding context={onboardingContext} />
          </DialogContent>
        </Dialog>
        <CommandPalette
          onNavigate={handleNavigate}
          onOpenChange={setIsCommandPaletteOpen}
          onOpenEntity={handleOpenEntity}
          onOpenRepo={handleSelectRepo}
          onOpenView={handleOpenReviewView}
          onScanAll={() => void handleScanAll()}
          onScanRepo={(repoId) => void handleScanRepoFromShell(repoId)}
          onShowActivity={() => setIsScanActivityOpen(true)}
          open={isCommandPaletteOpen}
          repos={snapshot.repos}
          scanRunning={isBusy}
          scannerReady={!scannerUnavailable}
          views={reviewViews}
        />
      </div>
    </main>
    </EvidenceReadsContext>
  );
}

function findAddedGithubRepo(repos: RepoSummary[], url: string): RepoSummary | null {
  const normalized = normalizeGithubUrl(url);
  if (!normalized) {
    return repos.find((repo) => repo.sourceType === "github") ?? null;
  }
  return (
    repos.find((repo) => normalizeGithubUrl(repo.remoteUrl ?? repo.path) === normalized) ??
    null
  );
}

function normalizeGithubUrl(value: string): string | null {
  try {
    const parsed = new URL(value.trim());
    if (
      parsed.protocol !== "https:" ||
      parsed.hostname !== "github.com" ||
      parsed.search ||
      parsed.hash
    ) {
      return null;
    }
    const [owner, rawRepo, ...rest] = parsed.pathname
      .split("/")
      .filter(Boolean);
    if (!owner || !rawRepo || rest.length > 0) {
      return null;
    }
    const repo = rawRepo.endsWith(".git") ? rawRepo.slice(0, -4) : rawRepo;
    return `https://github.com/${owner.toLowerCase()}/${repo.toLowerCase()}`;
  } catch {
    return null;
  }
}

interface PageHeaderProps {
  title: string;
  description: string;
  action?: ReactNode;
  titleRef?: (node: HTMLHeadingElement | null) => void;
}

function focusWorkspaceTitle(node: HTMLHeadingElement | null) {
  node?.focus({ preventScroll: true });
}

function PageHeader({ title, description, action, titleRef }: PageHeaderProps) {
  return (
    <header className="flex shrink-0 flex-col gap-4 md:flex-row md:items-end md:justify-between">
      <div>
        <h1 className="ui-page-title" ref={titleRef} tabIndex={titleRef ? -1 : undefined}>{title}</h1>
        <p className="ui-body mt-1">{description}</p>
      </div>
      {action}
    </header>
  );
}

export default App;
