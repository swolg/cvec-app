import { incompleteInventoryReason } from "./format";
import type {
  RepoOverview,
  RepoScanRun,
  RepoSourceType,
  RepoSummary,
  RepositoryMonitoringStatus,
  ScheduleInterval,
} from "./types";

const STALE_GRACE_MS = 15 * 60 * 60 * 1000;

export function formatCoverageWarning(warning: string, sourceType: RepoSourceType): string {
  // Saved scans use both forms. Preserve their original scope evidence.
  const missingNuget = /^NuGet dependency coverage for (.+) requires (?:a resolved packages\.lock\.json|packages\.lock\.json or a current project\.assets\.json for this project directory)\.?$/.exec(warning);
  if (!missingNuget) return warning;

  const action = sourceType === "github"
    ? "A checked-in NuGet lockfile, a local restore, or a build SBOM can supply resolved versions."
    : "Run dotnet restore in a local checkout and scan it again.";
  return `Resolved NuGet versions are unavailable for ${missingNuget[1]}. ${action}`;
}

export function deriveRepositoryMonitoring(
  repo: RepoSummary,
  latestScan: RepoScanRun | null,
  schedule: ScheduleInterval,
  now = Date.now(),
  lastSuccessfulScan: RepoScanRun | null = latestScan?.status === "completed" ? latestScan : null,
): RepositoryMonitoringStatus {
  const latestAttemptAt = latestScan?.startedAt ?? repo.lastScannedAt;
  const lastSuccessfulScanAt = lastSuccessfulScan
    ? lastSuccessfulScan.finishedAt ?? lastSuccessfulScan.startedAt
    : repo.lastScanStatus === "completed"
      ? repo.lastScannedAt
      : null;

  if (repo.lastScanStatus === "running" || repo.status === "scanning" || latestScan?.status === "running") {
    return { state: "scanning", reason: "A local scan is currently running.", lastSuccessfulScanAt, latestAttemptAt };
  }
  if (repo.lastScanStatus === "failed" || latestScan?.status === "failed" || repo.status === "error") {
    const incomplete = incompleteInventoryReason(latestScan?.errorMessage);
    const reason = incomplete
      ? lastSuccessfulScan ? "The latest scan could not verify dependencies. Findings reflect the last complete scan." : "The latest scan could not verify dependencies. See Scan history to retry."
      : "The latest scan attempt failed.";
    return { state: "failed", reason, lastSuccessfulScanAt, latestAttemptAt };
  }
  if (!latestScan && !repo.lastScannedAt) {
    return { state: "never_scanned", reason: "No completed scan is available yet.", lastSuccessfulScanAt: null, latestAttemptAt: null };
  }
  if (
    repo.sourceType === "github" &&
    repo.lastRemoteCommitSha &&
    (latestScan?.sourceRevision ?? repo.lastSuccessfulSourceRevision) &&
    repo.lastRemoteCommitSha !== (latestScan?.sourceRevision ?? repo.lastSuccessfulSourceRevision)
  ) {
    return { state: "source_ahead", reason: "The linked source has advanced since the trusted scan.", lastSuccessfulScanAt, latestAttemptAt };
  }
  if (schedule === "12h" && lastSuccessfulScanAt) {
    const timestamp = Date.parse(lastSuccessfulScanAt);
    if (Number.isFinite(timestamp) && now - timestamp > STALE_GRACE_MS) {
      return { state: "stale", reason: "The last successful scan is outside the 12h cadence and grace period.", lastSuccessfulScanAt, latestAttemptAt };
    }
  }
  if (repo.sourceType === "github" && repo.lastRemoteCommitSha && !(latestScan?.sourceRevision ?? repo.lastSuccessfulSourceRevision)) {
    return { state: "unknown", reason: "The stored scan does not contain comparable source-revision evidence.", lastSuccessfulScanAt, latestAttemptAt };
  }
  if (repo.coverageIncomplete || lastSuccessfulScan?.coverageWarnings?.length) {
    return { state: "coverage_incomplete", reason: "Some dependency inputs could not be verified. See the coverage details for this repository.", lastSuccessfulScanAt, latestAttemptAt };
  }
  if (repo.lastScanStatus === "completed" || latestScan?.status === "completed") {
    return { state: "current", reason: "The latest locally recorded scan completed successfully.", lastSuccessfulScanAt, latestAttemptAt };
  }
  return { state: "unknown", reason: "There is not enough local evidence to prove scan freshness.", lastSuccessfulScanAt, latestAttemptAt };
}

export class RepositoryOverviewCache {
  private entries = new Map<number, { detail: RepoOverview; expiresAt: number; version?: string }>();

  constructor(private readonly maxEntries = 8, private readonly ttlMs = 5 * 60 * 1000) {}

  get(repoId: number, now = Date.now(), version?: string): RepoOverview | null {
    const entry = this.entries.get(repoId);
    if (!entry) return null;
    if (entry.expiresAt <= now || entry.version !== version) {
      this.entries.delete(repoId);
      return null;
    }
    this.entries.delete(repoId);
    this.entries.set(repoId, entry);
    return entry.detail;
  }

  set(repoId: number, detail: RepoOverview, now = Date.now(), version?: string): void {
    this.entries.delete(repoId);
    this.entries.set(repoId, { detail, expiresAt: now + this.ttlMs, version });
    while (this.entries.size > this.maxEntries) {
      const oldest = this.entries.keys().next().value as number | undefined;
      if (oldest === undefined) break;
      this.entries.delete(oldest);
    }
  }

  invalidate(repoId?: number): void {
    if (repoId === undefined) this.entries.clear();
    else this.entries.delete(repoId);
  }

  get size(): number { return this.entries.size; }
}
