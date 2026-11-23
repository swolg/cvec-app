import type { AppSnapshot, RepoOverview, RepoSummary } from "@/lib/types";

export function emptyRepoOverview(summary: RepoSummary): RepoOverview {
  return {
    summary, latestScan: null, previousScan: null, findings: [], scanHistory: [],
    overview: { latestCompletedScan: null, packageCount: 0, affectedCount: 0, openFindingCount: 0, opportunityCount: 0, opportunities: [] },
  };
}

export const EMPTY_REPOSITORY: RepoSummary = {
  id: 1,
  name: "cvec",
  path: "/tmp/cvec",
  sourceType: "local",
  sourceKey: null,
  remoteUrl: null,
  remoteHost: null,
  provider: "local",
  defaultBranch: "main",
  lastRemoteCommitSha: null,
  lastRemoteSyncedAt: null,
  remoteFileCount: null,
  status: "unknown",
  newMedium: 0,
  newHigh: 0,
  newCritical: 0,
  openMedium: 0,
  openHigh: 0,
  openCritical: 0,
  resolvedMedium: 0,
  resolvedHigh: 0,
  resolvedCritical: 0,
  lastScannedAt: null,
  lastScanStatus: null,
  lastSuccessfulSourceRevision: null,
};

export const EMPTY_SNAPSHOT: AppSnapshot = {
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
    scannerMessage: "Install syft and grype to start scanning.",
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
    needsReview: 0,
    unreviewedCritical: 0,
    unreviewedOther: 0,
    dueSnoozed: 0,
    openReviewed: 0,
    snoozed: 0,
    recentlyResolved: 0,
    failedScans: 0,
  },
  scanActivity: null,
  dataRevisions: { advisory: 0, inventory: 0, scanHistory: 0, review: 0, intel: 0, scanner: 0, notifications: 0, settings: 0 },
};

export const READY_SNAPSHOT: AppSnapshot = {
  ...EMPTY_SNAPSHOT,
  repos: [
    {
      ...EMPTY_REPOSITORY,
      status: "critical",
      openCritical: 1,
      openHigh: 2,
      newCritical: 1,
      lastScannedAt: "2026-08-12T08:00:00Z",
      lastScanStatus: "completed",
    },
  ],
  overviewSnapshot: {
    ...EMPTY_SNAPSHOT.overviewSnapshot,
    reposWatched: 1,
    newCritical: 1,
    openCritical: 1,
    openHigh: 2,
    reposWithNewFindings: 1,
    lastSuccessfulScanAt: "2026-08-12T08:00:00Z",
  },
  workspaceStatus: {
    ...EMPTY_SNAPSHOT.workspaceStatus,
    scannerReady: true,
    scannerMessage: null,
    lastSuccessfulScanAt: "2026-08-12T08:00:00Z",
  },
  versionInfo: {
    ...EMPTY_SNAPSHOT.versionInfo,
    syft: "1.0.0",
    grype: "1.0.0",
    grypeDb: {
      present: true, valid: true, path: "/tmp/grype-db", schemaVersion: "6",
      builtAt: "2026-08-12T08:00:00Z", sourceUrl: null, updateAvailable: null,
      latestSchemaVersion: null, latestBuiltAt: null, error: null, updateCheckError: null,
    },
  },
  onboarding: {
    status: "completed",
    decidedAt: "2026-08-12T08:00:00Z",
    version: "1",
  },
};

export const OFFLINE_SNAPSHOT: AppSnapshot = {
  ...READY_SNAPSHOT,
  externalIntelSummary: {
    ...READY_SNAPSHOT.externalIntelSummary,
    lastFetchedAt: "2026-08-11T08:00:00Z",
  },
};
