import {performanceCountersEnabled,recordPerformanceSample} from "./performanceCounters";
import { invoke } from "@tauri-apps/api/core";
import type {
  AppSnapshot,
  ExternalIntelDetail,
  ExternalIntelFeed,
  GithubLoginPollResult,
  GithubAuthSnapshot,
  GithubLoginStart,
  GithubRepositoryOption,
  RepoAsset,
  RepoOverview,
  RepoScanComparison,
  DependencyPage,
  DependencyQuery,
  RiskTrendPoint,
  RiskTrendRange,
  ReviewActionBasis,
  ReviewPage,
  ReviewFinding,
  ReviewQuery,
  ReviewUpdate,
  LocalEntityResult,
  ScanAllResult,
  ScheduleInterval,
  FindingRemediationProjection,
} from "./types";
import { parseDependencyPage, parseReviewFinding, parseFindingRemediation, parseLocalEntityResults, parseRepoAssets, parseRepoOverview, parseRepoScanComparison, parseReviewPage } from "./contracts";

export async function bootstrap(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("bootstrap");
}

export async function refreshLocalRepositories(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("refresh_local_repositories");
}

export async function checkVulnerabilityDb(checkUpdates = true): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("check_vulnerability_db", { checkUpdates });
}

export async function getSnapshot(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("get_snapshot");
}

export async function loadSavedGithubAuth(): Promise<GithubAuthSnapshot> {
  return invoke<GithubAuthSnapshot>("load_saved_github_auth");
}

export async function configureGithubApp(clientId: string | null, installUrl: string | null): Promise<GithubAuthSnapshot> {
  return invoke<GithubAuthSnapshot>("configure_github_app", { clientId, installUrl });
}

export async function scanAll(): Promise<ScanAllResult> {
  return invoke<ScanAllResult>("scan_all");
}

export async function scanRepo(repoId: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("scan_repo", { repoId });
}

export async function addGithubRepository(url: string): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("add_github_repository", { url });
}

export async function removeRemoteRepository(repoId: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("remove_remote_repository", { repoId });
}

export async function getRepoOverview(repoId: number): Promise<RepoOverview> {
  const overview = parseRepoOverview(await invoke<unknown>("get_repo_overview", { repoId }));
  if (overview.summary.id !== repoId) throw new Error("Scan evidence belongs to a different repository.");
  return overview;
}

export async function listRepositoryAssets(repoId: number): Promise<RepoAsset[]> {
  return parseRepoAssets(await invoke<unknown>("list_repository_assets", { repoId }), repoId);
}

export async function listRepositoryDependencies(repoId: number, query: DependencyQuery): Promise<DependencyPage> {
  return parseDependencyPage(await invoke<unknown>("list_repository_dependencies", { repoId, query }));
}

export async function getRepoScanComparison(repoId: number, runId: number): Promise<RepoScanComparison | null> {
  const comparison = parseRepoScanComparison(await invoke<unknown>("get_repo_scan_comparison", { repoId, runId }));
  if (comparison && comparison.run.id !== runId) throw new Error("Scan evidence belongs to a different run.");
  return comparison;
}

export async function listRiskTrend(
  days: RiskTrendRange = 7,
): Promise<RiskTrendPoint[]> {
  return invoke<RiskTrendPoint[]>("list_risk_trend", { days });
}

export async function listReviewFindings(query: ReviewQuery, restore?: { preserveSelection?: string; loadedRows: number }): Promise<ReviewPage> {
  const trace=performanceCountersEnabled();const start=trace?performance.now():0;
  const raw=await invoke<unknown>("list_review_findings", { query, ...restore });const received=trace?performance.now():0;
  const result=parseReviewPage(raw);
  if(trace)recordPerformanceSample({ipcOperation:1,ipcMs:received-start,validationMs:performance.now()-received});
  return result;
}

export async function getReviewFindingDetail(repoId: number, fingerprint: string, evidenceScanRunId: number, resolvedInScanRunId?: number): Promise<ReviewFinding> {
  return parseReviewFinding(await invoke<unknown>("get_review_finding_detail", { repoId, fingerprint, evidenceScanRunId, resolvedInScanRunId }));
}

export async function getFindingRemediation(repoId: number, fingerprint: string, evidenceScanRunId: number): Promise<FindingRemediationProjection> {
  const trace=performanceCountersEnabled();const start=trace?performance.now():0;
  const raw=await invoke<unknown>("get_finding_remediation", {repoId,fingerprint,evidenceScanRunId});const received=trace?performance.now():0;
  const result=parseFindingRemediation(raw);
  if(result.repoId!==repoId || result.fingerprint!==fingerprint || result.evidenceScanRunId!==evidenceScanRunId)throw new Error("Fix evidence belongs to a different finding or scan.");
  if(trace)recordPerformanceSample({ipcOperation:2,ipcMs:received-start,validationMs:performance.now()-received});
  return result;
}

export async function openFindingManifest(repoId: number, fingerprint: string, evidenceScanRunId: number): Promise<void> {
  return invoke<void>("open_finding_manifest", { repoId, fingerprint, evidenceScanRunId });
}

export async function markFindingReviewed(repoId: number, fingerprint: string, evidence: ReviewActionBasis): Promise<ReviewUpdate | null> {
  return invoke<ReviewUpdate | null>("mark_finding_reviewed", { repoId, fingerprint, evidence });
}

export async function snoozeFinding(repoId: number, fingerprint: string, until: string, note: string | undefined, evidence: ReviewActionBasis): Promise<ReviewUpdate | null> {
  return invoke<ReviewUpdate | null>("snooze_finding", { input: { repoId, fingerprint, until, note: note ?? null, evidence } });
}

export async function returnFindingToReview(repoId: number, fingerprint: string, evidence: ReviewActionBasis): Promise<ReviewUpdate | null> {
  return invoke<ReviewUpdate | null>("return_finding_to_review", { repoId, fingerprint, evidence });
}

export async function searchLocalEntities(query: string, limit = 30): Promise<LocalEntityResult[]> {
  return parseLocalEntityResults(await invoke<unknown>("search_local_entities", { query, limit }));
}

export async function setSchedule(interval: ScheduleInterval): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("set_schedule", { interval });
}

export async function chooseDiscoveryRoot(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("choose_discovery_root");
}

export async function updateVulnerabilityDb(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("update_vulnerability_db");
}

export async function clearNotification(notificationId: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("clear_notification", { notificationId });
}

export async function clearAllNotifications(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("clear_all_notifications");
}

export async function completeOnboarding(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("complete_onboarding");
}

export async function skipOnboarding(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("skip_onboarding");
}

export async function listExternalIntelFeed(): Promise<ExternalIntelFeed> {
  return invoke<ExternalIntelFeed>("list_external_intel_feed");
}

export async function getExternalIntelDetail(
  canonicalKey: string,
  intelRevision: number,
  inventoryRevision: number,
): Promise<ExternalIntelDetail | null> {
  return invoke<ExternalIntelDetail | null>("get_external_intel_detail", {
    canonicalKey, intelRevision, inventoryRevision,
  });
}

export async function openExternalUrl(url: string): Promise<void> {
  return invoke<void>("open_external_url", { url });
}

export async function setExternalIntelTopics(
  topicKeys: string[],
): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("set_external_intel_topics", { topicKeys });
}

export async function refreshExternalIntel(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("refresh_external_intel");
}

export async function startGithubLogin(): Promise<GithubLoginStart> {
  return invoke<GithubLoginStart>("start_github_login");
}

export async function pollGithubLogin(loginId: string): Promise<GithubLoginPollResult> {
  return invoke<GithubLoginPollResult>("poll_github_login", { loginId });
}

export async function logoutGithub(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("logout_github");
}

export async function listGithubRepositories(): Promise<GithubRepositoryOption[]> {
  return invoke<GithubRepositoryOption[]>("list_github_repositories");
}
