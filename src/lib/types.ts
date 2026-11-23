export type RepoStatus = "unknown" | "clean" | "warning" | "critical" | "error" | "scanning";
export type RepoSourceType = "local" | "github" | "unknown";
export type Severity = "MEDIUM" | "HIGH" | "CRITICAL";
export type ScheduleInterval = "off" | "12h";
export type ScanRunStatus = "running" | "completed" | "failed" | "unknown";
export type RepoFindingState = "new" | "open" | "resolved";
export type ReviewState = "unreviewed" | "reviewed" | "snoozed";
export type ReviewSort = "priority" | "severity" | "recent" | "repository" | "package";
export type RepoFindingIntelMatchKind = "advisory_alias" | "source_url";
export type RiskTrendRange = 7 | 14 | 30;
export type OnboardingStatus = "not_started" | "completed" | "skipped";
export type ExternalSource = "osv" | "github" | "nvd" | "cisa_kev" | "vendor";
export type ExternalIntelSeverity =
  | "malware"
  | "critical"
  | "high"
  | "medium"
  | "low"
  | "unknown";
export type ExternalIntelRelevance = "relevant" | "triage" | "general";
export type ExternalIntelMatchKind = "package_name" | "product" | "vendor" | "ecosystem";

export interface RepoSummary {
  evidenceRevision?:number;
  dependencyRevision?: number;
  id: number;
  name: string;
  path: string;
  sourceType: RepoSourceType;
  sourceKey: string | null;
  remoteUrl: string | null;
  remoteHost: string | null;
  provider: string | null;
  defaultBranch: string | null;
  lastRemoteCommitSha: string | null;
  lastRemoteSyncedAt: string | null;
  remoteFileCount: number | null;
  status: RepoStatus;
  newMedium: number;
  newHigh: number;
  newCritical: number;
  openMedium: number;
  openHigh: number;
  openCritical: number;
  resolvedMedium: number;
  resolvedHigh: number;
  resolvedCritical: number;
  lastScannedAt: string | null;
  lastScanStatus: ScanRunStatus | null;
  lastSuccessfulSourceRevision: string | null;
}

export interface RepoFinding {
  fingerprint: string;
  advisoryId: string;
  packageName: string;
  packageVersion: string | null;
  packageType: string | null;
  packageUrl: string | null;
  severity: Severity;
  fixVersion: string | null;
  title: string | null;
  sourceUrl: string | null;
  state: RepoFindingState;
  intel: RepoFindingIntel | null;
  reviewState: ReviewState;
  reviewedAt: string | null;
  snoozeUntil: string | null;
  reviewNote: string | null;
}

export interface RepoFindingIntel {
  fetchedAt?: string | null;
  canonicalKey: string;
  title: string;
  summary: string;
  url: string;
  severity: ExternalIntelSeverity;
  publishedAt: string | null;
  updatedAt: string | null;
  sources: ExternalSource[];
  aliases: string[];
  matchKind: RepoFindingIntelMatchKind;
  vulnerabilities: AdvisoryPackageImpact[];
  cvssScore: string | null;
  cvssVector: string | null;
  epssPercentage: string | null;
  epssPercentile: string | null;
  cwes: AdvisoryCwe[];
  references: AdvisoryReference[];
  sourceCodeLocation: string | null;
}

export interface AdvisoryPackageImpact {
  ecosystem: string | null;
  packageName: string | null;
  packagePurl: string | null;
  vulnerableVersionRange: string | null;
  firstPatchedVersion: string | null;
  affectedVersionsCount: number;
}

export interface AdvisoryCwe {
  cweId: string;
  name: string;
}

export interface AdvisoryReference {
  url: string;
  source: string | null;
  kind: string | null;
  tags: string[];
}

export interface CisaKevDetails {
  dateAdded: string | null;
  dueDate: string | null;
  requiredAction: string | null;
  knownRansomwareCampaignUse: string | null;
  notes: string | null;
  catalogVersion: string | null;
  catalogReleasedAt: string | null;
}

export interface ExternalIntelSourceDetail {
  source: ExternalSource;
  sourceItemId: string;
  url: string;
  publishedAt: string | null;
  updatedAt: string | null;
  severity: ExternalIntelSeverity;
  cvssScore: string | null;
  epssPercentage: string | null;
  referenceCount: number;
}

export interface ScannedPackageIdentity {
  name: string;
  version: string | null;
  packageType: string | null;
  packageUrl: string | null;
}

export interface RepoPackage extends ScannedPackageIdentity {
  highestSeverity: Severity | null;
  findingCount: number;
  fixAvailable: boolean;
}

export interface RepoScanRun {
  id: number;
  status: ScanRunStatus;
  startedAt: string;
  finishedAt: string | null;
  errorMessage: string | null;
  newMedium: number;
  newHigh: number;
  newCritical: number;
  resolvedMedium: number;
  resolvedHigh: number;
  resolvedCritical: number;
  sourceRevision?: string | null;
  sourceDirty?: boolean | null;
  packageCount?: number | null;
  reopenedCount?: number;
  warnings?: string[];
  evidenceComplete?: boolean;
  openMedium?: number;
  openHigh?: number;
  openCritical?: number;
}

export interface RepoScanComparison {
  run: RepoScanRun;
  previousRunId?: number | null;
  previousPackageCount: number | null;
}

export type RepositoryMonitoringState =
  | "scanning"
  | "failed"
  | "never_scanned"
  | "source_ahead"
  | "stale"
  | "current"
  | "unknown";

export interface RepositoryMonitoringStatus {
  state: RepositoryMonitoringState;
  reason: string;
  lastSuccessfulScanAt: string | null;
  latestAttemptAt: string | null;
}

export interface RemediationOpportunity {
  key: string;
  packageName: string;
  packageType: string | null;
  packageUrl: string | null;
  installedVersion: string | null;
  fixVersion: string | null;
  findingCount: number;
  critical: number;
  high: number;
  medium: number;
  fingerprints: string[];
}

export interface ReviewOpenRequest {
  repositoryId: number;
  resolvedInScanRunId?: number;
  packageIdentity?: string;
  installedPackage?:ReviewPackageIdentity;
  findingFingerprint?: string;
  remediationGroup?: {
    packageName: string;
    packageUrl: string | null;
    packageType: string | null;
    installedVersion: string | null;
    fixVersion: string | null;
    preferredFingerprint?: string;
  };
  origin?: {
    repositoryId: number;
    section: "overview" | "dependencies" | "history" | "chat" | "settings";
    scrollAnchor?: string;
  };
}

export interface FindingChatRequest {
  repositoryId: number;
  fingerprint: string;
  advisoryId: string;
  packageName: string;
  installedVersion: string | null;
  targetVersion: string | null;
  scanRunId: number | null;
  mode: "analysis" | "fix";
}

export interface RepoAsset {
  repoId: number;
  vendor: string;
  product: string;
  version: string | null;
  assetKind: string;
  sourcePath: string | null;
  evidence: string;
  confidence: number;
}

export interface RepoOverview {
  overview: RepoOverviewMetadata;
  summary: RepoSummary;
  latestScan: RepoScanRun | null;
  previousScan: RepoScanRun | null;
  findings: RepoFinding[];
  scanHistory: RepoScanRun[];
}

export type CompanionRuntimePhase =
  | "disabled"
  | "unavailable"
  | "discovering"
  | "starting"
  | "initializing"
  | "signed_out"
  | "ready"
  | "active_turn"
  | "interrupted"
  | "incompatible"
  | "offline";

export interface CompanionRuntimeStatus {
  schemaVersion: number;
  phase: CompanionRuntimePhase;
  codexPath: string | null;
  codexVersion: string | null;
  message: string | null;
  startedAt: string | null;
  activeTurns: number;
}

export type CompanionAuthKind = "chatgpt" | "api_key" | "none";

export interface CompanionAccountStatus {
  schemaVersion: number;
  signedIn: boolean;
  authKind: CompanionAuthKind;
  planType: string | null;
  accountLabel: string | null;
  message: string | null;
}

export type CompanionLoginKind = "chatgpt" | "chatgpt_device_code" | "api_key";

export interface CompanionLoginStart {
  kind: CompanionLoginKind;
  loginId: string | null;
  authUrl: string | null;
  verificationUrl: string | null;
  userCode: string | null;
  status: string;
}

export interface CompanionModel {
  id: string;
  displayName: string;
  description: string | null;
  isDefault: boolean;
  hidden: boolean;
  reasoningEfforts: string[];
  inputModalities: string[];
  contextWindow: number | null;
}

export interface CompanionTokenUsage {
  inputTokens: number | null;
  cachedInputTokens: number | null;
  outputTokens: number | null;
  reasoningTokens: number | null;
  totalTokens: number | null;
  contextWindow: number | null;
  contextUsed: number | null;
}

export interface CompanionRateLimitWindow {
  usedPercent: number;
  windowDurationMinutes: number | null;
  resetsAt: number | null;
}

export interface CompanionUsageStatus {
  thread: CompanionTokenUsage | null;
  primary: CompanionRateLimitWindow | null;
  secondary: CompanionRateLimitWindow | null;
  lifetimeTokens: number | null;
  peakDailyTokens: number | null;
  supported: boolean;
  unavailableReason: string | null;
  fetchedAt: string | null;
}

export type CompanionThreadStatus =
  | "not_loaded"
  | "ready"
  | "active"
  | "interrupted"
  | "history_unavailable";

export interface CompanionThread {
  id: number;
  repositoryId: number;
  provider: string;
  providerThreadId: string;
  title: string;
  status: CompanionThreadStatus;
  createdAt: string;
  updatedAt: string;
  lastOpenedAt: string;
  archivedAt: string | null;
  contextVersion: number;
  lastModelId: string | null;
  providerConnectionId: string;
}

export type CompanionProviderSegmentStatus =
  | "ready"
  | "active"
  | "interrupted"
  | "history_unavailable";

export interface CompanionProviderSegment {
  id: number;
  threadId: number;
  ordinal: number;
  provider: string;
  connectionId: string;
  providerThreadId: string;
  initialModelId: string | null;
  lastModelId: string | null;
  status: CompanionProviderSegmentStatus;
  createdAt: string;
  updatedAt: string;
  endedAt: string | null;
}

export interface CompanionConversationIndex {
  schemaVersion: number;
  threads: CompanionThread[];
}

export interface CompanionComposerSelection {
  repositoryId: number;
  connectionId: string;
  modelId: string | null;
  effort: string | null;
}

export type CompanionTimelineItemKind =
  | "user_message"
  | "assistant_message"
  | "activity"
  | "warning"
  | "error";
export type CompanionTimelineItemStatus =
  | "streaming"
  | "completed"
  | "interrupted"
  | "failed";
export type CompanionActivityKind =
  | "search"
  | "read"
  | "context"
  | "runtime"
  | "reasoning"
  | "provider_transition";

export interface CompanionActivity {
  kind: CompanionActivityKind;
  label: string;
  detail: string | null;
  sourceIds: string[];
}

export type ContextSourceKind =
  | "repository"
  | "scan"
  | "finding"
  | "package"
  | "file"
  | "changes"
  | "advisory";

export interface CompanionCitation {
  sourceId: string;
  kind: ContextSourceKind;
  label: string;
  stale: boolean;
}

export interface CompanionErrorInfo {
  code: string;
  message: string;
  retryable: boolean;
}

export interface CompanionTimelineItem {
  id: string;
  kind: CompanionTimelineItemKind;
  status: CompanionTimelineItemStatus;
  text: string;
  activity: CompanionActivity | null;
  citations: CompanionCitation[];
  error: CompanionErrorInfo | null;
  modelId?: string | null;
  createdAt: string;
}

export interface CompanionThreadSnapshot {
  thread: CompanionThread;
  items: CompanionTimelineItem[];
  usage: CompanionTokenUsage | null;
}

export interface CompanionCachedThreadSnapshot {
  olderCursor?: string | null;
  eventWatermarks?: Record<string, number>;
  streamingItemIds?: string[];
  providerOrdinals?: Record<string, number>;
  schemaVersion: number;
  thread: CompanionThread;
  segments: CompanionProviderSegment[];
  items: CompanionTimelineItem[];
  providerHistoryAvailable: boolean;
}

export type CompanionEventKind =
  | "runtime_status"
  | "account_status"
  | "thread_snapshot"
  | "thread_updated"
  | "turn_started"
  | "item_started"
  | "item_delta"
  | "item_completed"
  | "activity"
  | "usage"
  | "turn_completed"
  | "warning"
  | "error";

export interface CompanionEventEnvelope {
  cacheEpoch?: string | null;
  providerSegmentId?: number | null;
  schemaVersion: number;
  repositoryId: number | null;
  threadId: number | null;
  providerThreadId: string | null;
  connectionId: string | null;
  driver: string | null;
  modelId: string | null;
  turnId: string | null;
  sequence: number;
  kind: CompanionEventKind;
  payload: unknown;
}

export type ContextAttachmentKind =
  | "repository"
  | "scan"
  | "finding"
  | "package"
  | "file"
  | "changes";

export interface ContextAttachment {
  id: string;
  kind: ContextAttachmentKind;
  label: string;
  optional: boolean;
  available: boolean;
  unavailableReason: string | null;
}

export type ContextFreshnessState =
  | "matched"
  | "mismatched"
  | "dirty"
  | "unknown"
  | "newer_scan";

export interface ContextFreshness {
  state: ContextFreshnessState;
  sourceRevision: string | null;
  scanRevision: string | null;
  sourceDirty: boolean | null;
  message: string;
}

export interface ContextRepository {
  id: number;
  name: string;
  sourceType: string;
  sourceMode: string;
  approvedRoot: string | null;
  branch: string | null;
  revision: string | null;
}

export interface ContextScan {
  id: number | null;
  status: string | null;
  scannedAt: string | null;
  revision: string | null;
  evidenceComplete: boolean;
}

export interface RepositoryChatScanStatus {
  repositoryId: number;
  latestCompleted: ContextScan;
  latestAttempt: ContextScan;
}

export interface ContextSource {
  id: string;
  kind: ContextSourceKind;
  label: string;
  repositoryRelativePath: string | null;
  lineStart: number | null;
  lineEnd: number | null;
  scanId: number | null;
  findingFingerprint: string | null;
  packageIdentity: string | null;
  revision: string | null;
  stale: boolean;
}

export interface ContextOmission {
  category: string;
  reason: string;
  count: number;
}

export interface UntrustedEvidence {
  sourceId: string;
  label: string;
  content: string;
}

export interface RepositoryChatContext {
  schemaVersion: number;
  repository: ContextRepository;
  scan: ContextScan;
  freshness: ContextFreshness;
  attachments: ContextAttachment[];
  sources: ContextSource[];
  omissions: ContextOmission[];
  untrustedEvidence: UntrustedEvidence[];
  instructions: string[];
  estimatedBytes: number;
  bounded: boolean;
}

export interface ContextAttachmentRequest {
  kind: ContextAttachmentKind;
  id: string;
}

export interface ContextPreviewRequest {
  repositoryId: number;
  query: string;
  attachments: ContextAttachmentRequest[];
}

export interface CompanionTurnRequest {
  repositoryId: number;
  threadId: number;
  connectionId: string;
  prompt: string;
  model: string | null;
  effort: string | null;
  context: RepositoryChatContext;
  clientTurnId: string;
}

export type ProviderDriverKind = "codex" | "grok" | "open_code";
export type ProviderHealthState =
  | "unchecked"
  | "checking"
  | "ready"
  | "setup_required"
  | "unavailable"
  | "incompatible"
  | "disabled"
  | "stale";
export type InferenceBoundary = "verified_local" | "external" | "unknown";
export type OpenCodeConnectionMode = "managed_local" | "existing_loopback";

export interface CodexConnectionConfig {
  kind: "codex";
  binaryPath?: string | null;
  homePath?: string | null;
  profile?: string | null;
  launchArgs?: string | null;
}

export interface GrokConnectionConfig {
  kind: "grok";
  binaryPath?: string | null;
  customModels?: string[];
}

export interface OpenCodeConnectionConfig {
  kind: "open_code";
  binaryPath?: string | null;
  mode?: OpenCodeConnectionMode;
  serverUrl?: string | null;
}

export type ProviderConnectionConfig = CodexConnectionConfig | GrokConnectionConfig | OpenCodeConnectionConfig;

export interface ProviderConnection {
  schemaVersion: number;
  id: string;
  driver: ProviderDriverKind;
  displayName: string;
  accentColor: string | null;
  enabled: boolean;
  builtIn: boolean;
  config: ProviderConnectionConfig;
  hasServerPassword: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ProviderConnectionDraft {
  id: string;
  driver: ProviderDriverKind;
  displayName: string;
  accentColor: string | null;
  enabled: boolean;
  config: ProviderConnectionConfig;
  serverPassword?: string | null;
}

export interface ProviderCapabilities {
  createThread: boolean;
  resumeThread: boolean;
  interrupt: boolean;
  orderedStreaming: boolean;
  readOnlyPolicy: boolean;
  accountStatus: boolean;
  rateLimits: boolean;
  tokenUsage: boolean;
  reasoningEffort: boolean;
  attachments: boolean;
}

export interface ProviderUpstream {
  id: string;
  displayName: string;
  connected: boolean;
  inferenceBoundary: InferenceBoundary;
}

export interface ProviderConnectionModel extends CompanionModel {
  connectionId: string;
  driver: ProviderDriverKind;
  upstreamProviderId: string | null;
  inferenceBoundary: InferenceBoundary;
}

export interface ProviderErrorInfo {
  code:
    | "not_installed"
    | "incompatible"
    | "authentication_required"
    | "connection_refused"
    | "model_unavailable"
    | "history_unavailable"
    | "policy_denied"
    | "provider_error"
    | "process_exited"
    | "protocol_error"
    | "offline";
  message: string;
  retryable: boolean;
}

export interface ProviderConnectionSnapshot {
  schemaVersion: number;
  connection: ProviderConnection;
  health: ProviderHealthState;
  installedVersion: string | null;
  authKind: CompanionAuthKind;
  accountLabel: string | null;
  capabilities: ProviderCapabilities;
  upstreamProviders: ProviderUpstream[];
  models: ProviderConnectionModel[];
  checkedAt: string | null;
  message: string | null;
  error: ProviderErrorInfo | null;
}

export interface CompanionTurnStart {
  turnId: string;
  providerThreadId: string;
  acceptedClientTurnId: string;
}

export interface CompanionMention {
  id: string;
  kind: ContextAttachmentKind;
  label: string;
  detail: string;
  available: boolean;
  unavailableReason: string | null;
}

export interface CompanionSourceDetail {
  source: ContextSource;
  content: string | null;
  unavailableReason: string | null;
}

export interface PrivacyConsentStatus {
  provider: string;
  disclosureVersion: number;
  acceptedAt: string | null;
  revokedAt: string | null;
  valid: boolean;
}

export type AppNotificationKind = "scan_alert" | "db_update";
export type AppNotificationTone = "warning" | "success" | "neutral";

export interface AppNotification {
  id: number;
  kind: AppNotificationKind;
  tone: AppNotificationTone;
  title: string;
  message: string;
  repoId: number | null;
  repoName: string | null;
  scanRunId?: number;
  createdAt: string;
}

export interface ScanAllSummary {
  startedAt: string;
  finishedAt: string;
  reposAttempted: number;
  reposScanned: number;
  reposFailed: number;
  alertsCreated: number;
}

export interface ScanAllResult {
  summary: ScanAllSummary;
  snapshot: AppSnapshot | null;
}

export interface ReviewRiskBasis {
  severity: Severity;
  fixVersion: string | null;
  knownExploited: boolean;
  ransomwareKnown: boolean;
  scannerState: RepoFindingState;
}

export interface FindingReview {
  repoId: number;
  fingerprint: string;
  state: ReviewState;
  reviewedAt: string | null;
  snoozeUntil: string | null;
  note: string | null;
  riskBasis: ReviewRiskBasis;
  updatedAt: string;
  transitionReason: string | null;
}

export interface ReviewFilters {
  packageIdentity?: ReviewPackageIdentity | null;
  fingerprint?: string;
  reviewStates: ReviewState[];
  scannerStates: RepoFindingState[];
  severities: Severity[];
  repoIds: number[];
  sourceTypes: RepoSourceType[];
  fixAvailable: boolean | null;
  packages: string[];
  ecosystems: string[];
  knownExploited: boolean | null;
  ransomwareKnown: boolean | null;
  changedSince: string | null;
  scanRunIds: number[];
  scanChangeRunIds?: number[];
  scanResolvedRunIds?: number[];
  resolvedInScanRunId?: number;
}

export interface ReviewQuery {
  sorts?: {field: ReviewSort; direction: "asc" | "desc"}[];
  filters: ReviewFilters;
  search: string | null;
  sort: ReviewSort;
  cursor: string | null;
  pageSize: number | null;
}

export interface ReviewFinding {
  advisorySeverity?: Severity | null;
  repoId: number;
  repoName: string;
  repoSourceType: RepoSourceType;
  scanRunId: number | null;
  scannedAt: string | null;
  finding: RepoFinding;
  reviewState: ReviewState;
  reviewedAt: string | null;
  snoozeUntil: string | null;
  reviewNote: string | null;
  knownExploited: boolean;
  ransomwareKnown: boolean;
  materialChange: string | null;
  priorityReasons: string[];
}

export interface ReviewRiskBasis {
  severity: Severity;
  fixVersion: string | null;
  knownExploited: boolean;
  ransomwareKnown: boolean;
  scannerState: RepoFindingState;
}

export interface ReviewActionBasis {
  scanRunId: number | null;
  riskBasis: ReviewRiskBasis;
  reviewState: ReviewState;
  reviewedAt: string | null;
  snoozeUntil: string | null;
  reviewNote: string | null;
}

export type RemediationLocationKind = "manifest" | "lockfile" | "other";
export type RemediationRelationship = "direct" | "transitive" | "unknown";
export type RemediationEvidenceQuality = "current" | "partial" | "unavailable" | "stale";
export type RemediationEvidenceSource = "syft_location" | "syft_relationship" | "supported_manifest" | "none";
export type RemediationVerificationState = "still_detected" | "not_detected" | "reappeared" | "unverifiable";

export interface SourceSnapshot {
  revision: string | null;
  dirty: boolean | null;
}

export interface FindingRemediationProjection {
  schemaVersion: number;
  repoId: number;
  repoName: string;
  fingerprint: string;
  advisoryId: string;
  packageName: string;
  installedVersion: string | null;
  packageType: string | null;
  packageUrl: string | null;
  scannerReportedTarget: string | null;
  evidenceScanRunId: number;
  evidenceScannedAt: string | null;
  sourceSnapshot: SourceSnapshot;
  relativePath: string | null;
  locationKind: RemediationLocationKind | null;
  relationship: RemediationRelationship;
  parentPackage: string | null;
  evidenceQuality: RemediationEvidenceQuality;
  evidenceSource: RemediationEvidenceSource;
  locationCount: number;
  verificationState: RemediationVerificationState;
  verificationScanRunId: number | null;
  verificationScannedAt: string | null;
  canOpenManifest: boolean;
  priorityReasons: string[];
  briefMarkdown: string;
}

export interface ReviewPage {
  validUntil?: string | null;
  items: ReviewFinding[];
  nextCursor: string | null;
  total: number;
  groupCounts: ReviewGroupCounts;
  revision: number;
  reviewSummary?: ReviewSummary | null;
}

export interface ReviewLaneCount {
  id: string;
  label: string;
  count: number;
}

export interface ReviewGroupCounts {
  reviewState: ReviewLaneCount[];
  severity: ReviewLaneCount[];
  repository: ReviewLaneCount[];
  fixAvailability: ReviewLaneCount[];
}

export interface ReviewSummary {
  needsReview: number;
  unreviewedCritical: number;
  unreviewedOther: number;
  dueSnoozed: number;
  openReviewed: number;
  snoozed: number;
  recentlyResolved: number;
  failedScans: number;
}

export interface ReviewUpdate {
  reviewSummary: ReviewSummary;
  revision: number;
}

export type ScanActivityStatus = "running" | "completed" | "partial_failure" | "failed";
export type ScanActivityStage = "queued" | "preparing_source" | "inventorying_packages" | "matching_vulnerabilities" | "enriching_advisories" | "saving_results" | "completed" | "failed";

export interface ScanRepositoryOutcome {
  repoId: number;
  repoName: string;
  scanRunId: number | null;
  succeeded: boolean;
  error: string | null;
  newFindings: number;
  resolvedFindings: number;
  worsenedFindings: number;
}

export interface ScanActivity {
  id: string;
  sequence: number;
  status: ScanActivityStatus;
  stage: ScanActivityStage;
  interrupted?: boolean;
  scopeRepoId: number | null;
  startedAt: string;
  finishedAt: string | null;
  totalRepositories: number;
  completedRepositories: number;
  failedRepositories: number;
  currentRepoId: number | null;
  currentRepoName: string | null;
  outcomes: ScanRepositoryOutcome[];
  warnings: string[];
  newFindings: number;
  resolvedFindings: number;
  worsenedFindings: number;
  terminalError: string | null;
}

export type LocalEntityKind = "repository" | "finding" | "package" | "advisory";
export interface LocalEntityResult {
  id: string;
  kind: LocalEntityKind;
  label: string;
  context: string;
  repoId: number | null;
  fingerprint: string | null;
  advisoryId: number | null;
}

export interface ExternalIntelSummary {
  totalItems: number;
  relevantItems: number;
  triageItems: number;
  lastFetchedAt: string | null;
  sources: ExternalSource[];
  sourceCounts: ExternalSourceCount[];
}

export interface ExternalSourceCount {
  source: ExternalSource;
  count: number;
}

export interface ExternalIntelTopic {
  key: string;
  label: string;
  category: string;
  totalCount: number;
  watched: boolean;
  suggested: boolean;
}

export interface ExternalIntelLocalPackage {
  repoId: number;
  repoName: string;
  ecosystem: string;
  packageName: string;
  package: ScannedPackageIdentity;
  affectedVersionRanges?: string[];
}

export interface ExternalIntelFeed {
  intelRevision: number;
  inventoryRevision: number;
  items: ExternalIntelFeedItem[];
}

export interface ExternalIntelFeedItem {
  id: number;
  canonicalKey: string;
  title: string;
  summary: string;
  ecosystem: string | null;
  packageName: string | null;
  productKeys: string[];
  topicKeys: string[];
  severity: ExternalIntelSeverity;
  publishedAt: string | null;
  withdrawnAt: string | null;
  cvssScore: string | null;
  epssPercentage: string | null;
  isKev: boolean;
  isRansomwareKnown: boolean;
  relevance: ExternalIntelRelevance;
  sources: ExternalSource[];
  aliases: string[];
  matchKind: ExternalIntelMatchKind | null;
  repoCount: number;
  cwes: AdvisoryCwe[];
  kevDates: ExternalIntelKevDates | null;
}

export interface ExternalIntelKevDates {
  dateAdded: string | null;
  dueDate: string | null;
}

export interface ExternalIntelDetail {
  url: string;
  updatedAt: string | null;
  modifiedAt: string | null;
  cvssVector: string | null;
  cvssVersion: string | null;
  epssPercentile: string | null;
  references: AdvisoryReference[];
  affectedPackages: AdvisoryPackageImpact[];
  localPackages: ExternalIntelLocalPackage[];
  kev: CisaKevDetails | null;
  sourceDetails: ExternalIntelSourceDetail[];
}

export interface OverviewSnapshot {
  reposWatched: number;
  newMedium: number;
  newHigh: number;
  newCritical: number;
  openMedium: number;
  openHigh: number;
  openCritical: number;
  reposWithNewFindings: number;
  lastSuccessfulScanAt: string | null;
}

export interface RiskTrendPoint {
  date: string;
  medium: number;
  high: number;
  critical: number;
}

export interface WorkspaceStatus {
  scannerReady: boolean;
  scannerMessage: string | null;
  schedule: ScheduleInterval;
  lastSuccessfulScanAt: string | null;
  nextScheduledScanAt: string | null;
  scanRunning: boolean;
}

export interface VersionInfo {
  app: string;
  syft: string | null;
  grype: string | null;
  grypeDb: GrypeDbStatus | null;
  lastGrypeDbUpdate: GrypeDbUpdateSummary | null;
}

export interface GithubAuthSnapshot {
  checking?:boolean;
  deferred?: boolean;
  configured: boolean;
  clientId?: string | null;
  configRevision?: number;
  sessionRevision?: number;
  canConfigure?: boolean;
  signedIn: boolean;
  login: string | null;
  avatarUrl: string | null;
  profileUrl: string | null;
  expiresAt: string | null;
  rateLimitRemaining: number | null;
  rateLimitLimit: number | null;
  rateLimitResetAt: string | null;
  installUrl: string | null;
  message: string | null;
}

export interface GithubLoginStart {
  loginId: string;
  userCode: string;
  verificationUri: string;
  expiresAt: string;
  intervalSeconds: number;
}

export type GithubLoginPollStatus =
  | "pending"
  | "slow_down"
  | "success"
  | "expired"
  | "denied";

export interface GithubLoginPollResult {
  status: GithubLoginPollStatus;
  message: string | null;
  intervalSeconds: number;
  auth: GithubAuthSnapshot;
}

export interface GithubRepositoryOption {
  owner: string;
  name: string;
  fullName: string;
  private: boolean;
  htmlUrl: string;
  defaultBranch: string;
}

export interface GrypeDbStatus {
  present: boolean;
  valid: boolean;
  path: string;
  schemaVersion: string | null;
  builtAt: string | null;
  sourceUrl: string | null;
  updateAvailable: boolean | null;
  latestSchemaVersion: string | null;
  latestBuiltAt: string | null;
  error: string | null;
  updateCheckError: string | null;
}

export type GrypeDbUpdateOutcome =
  | "downloaded"
  | "updated"
  | "already_current";

export interface GrypeDbUpdateSummary {
  checkedAt: string;
  outcome: GrypeDbUpdateOutcome;
  previousSchemaVersion: string | null;
  previousBuiltAt: string | null;
  currentSchemaVersion: string | null;
  currentBuiltAt: string | null;
}

export interface OnboardingSnapshot {
  status: OnboardingStatus;
  decidedAt: string | null;
  version: string | null;
}

export interface AppSnapshot {
  repos: RepoSummary[];
  notifications: AppNotification[];
  externalIntelSummary: ExternalIntelSummary;
  externalIntelTopics: ExternalIntelTopic[];
  overviewSnapshot: OverviewSnapshot;
  workspaceStatus: WorkspaceStatus;
  discoveryRoot: string;
  versionInfo: VersionInfo;
  githubAuth: GithubAuthSnapshot;
  onboarding: OnboardingSnapshot;
  reviewSummary: ReviewSummary;
  scanActivity: ScanActivity | null;
  dataRevisions: DataRevisions;
}

export interface DataRevisions {
  advisory?: number;
  inventory: number;
  scanHistory: number;
  review: number;
  intel: number;
  scanner: number;
  notifications: number;
  settings: number;
}

export interface RepoOverviewMetadata {
  latestCompletedScan: RepoScanRun | null;
  packageCount: number;
  affectedCount: number;
  openFindingCount: number;
  opportunityCount: number;
  opportunities: RemediationOpportunity[];
}
export interface DependencyQuery {
  search?: string | null;
  exactPackage?: ScannedPackageIdentity;
  ecosystem?: string | null;
  severity?: Severity | null;
  affectedOnly?: boolean | null;
  fixAvailable?: boolean | null;
  sort?: string | null;
  cursor?: string | null;
  pageSize?: number | null;
  expectedScanRunId?: number | null;
}
export interface DependencyPage {
  items: RepoPackage[];
  total: number;
  inventoryCount: number;
  affectedCount: number;
  ecosystems: string[];
  nextCursor: string | null;
  scanRunId: number | null;
  revision: number;
}
export interface ReviewPackageIdentity {
  anyFix?:boolean;
  packageName: string;
  packageUrl: string | null;
  packageType: string | null;
  installedVersion: string | null;
  fixVersion: string | null;
}
export interface FixFinding {
  fingerprint: string;
  advisoryId: string;
  packageName: string;
  version: string | null;
  packageType: string | null;
  packageUrl: string | null;
  fixVersion: string | null;
  severity: string;
}

export interface FixStartRequest {
  connectionId: string;
  connectionUpdatedAt: string;
  prompt: string;
  model: string | null;
  effort: string | null;
  consent: boolean;
}

export interface FixProvider {
  connectionId: string;
  driver: ProviderDriverKind;
  displayName: string;
  model: string | null;
}

export interface FixMessage {
  provider?: FixProvider | null;
  role: string;
  text: string;
}

export interface FixCheck {
  action: string;
  status: string;
  exitCode: number | null;
  output: string;
  digest: string;
  stale: boolean;
  finishedAt: string;
}

export interface FixRescan {
  findings: FixFinding[];
  findingCount: number;
  notDetected: number | null;
  targetDetected?: boolean | null;
  digest: string;
  stale: boolean;
  scannedAt: string;
  scannerVersions: string;
}

export interface FixWorkspace {
  provider?: FixProvider | null;
  schemaVersion: number;
  revision: number;
  archivedAt?: string | null;
  id: string;
  repositoryId: number;
  sourcePath: string;
  worktreePath: string;
  branch: string;
  baseRevision: string;
  baselineScanId: number | null;
  baselineMatched: boolean;
  baselineFindings: FixFinding[];
  findingFingerprint: string | null;
  providerThreadId: string | null;
  turnId: string | null;
  status: string;
  activity: string;
  messages: FixMessage[];
  checks: FixCheck[];
  rescan: FixRescan | null;
  createdAt: string;
  updatedAt: string;
}

export interface FixDiffFile {
  path: string;
  patch: string;
  truncated: boolean;
}

export interface FixDiff {
  files: FixDiffFile[];
  omittedFiles: number;
  truncated: boolean;
  digest: string;
}

export interface FixSnapshot {
  workspace: FixWorkspace;
  diff: FixDiff | null;
  sourceError: string | null;
  verificationWarning?: string | null;
  actions: string[];
}

export interface FixWorkspaceSummary {
  id: string;
  repositoryId: number;
  branch: string;
  title: string;
  findingFingerprint?: string;
  status?: string;
  archivedAt: string;
}

export interface FixArchivePage {
  entries: FixWorkspaceSummary[];
  nextCursor: string | null;
}
