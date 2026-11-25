import { FINDING_MATCH_LABEL } from "./findingMatch";
import { isSeverity } from "./types";
import type {
  ArtifactFindings,
  ArtifactScan,
  CompanionAccountStatus,
  CompanionCachedThreadSnapshot,
  CompanionConversationIndex,
  CompanionEventEnvelope,
  CompanionLoginStart,
  CompanionMention,
  CompanionModel,
  CompanionRuntimeStatus,
  CompanionSourceDetail,
  CompanionThread,
  CompanionProviderSegment,
  CompanionThreadSnapshot,
  CompanionTurnStart,
  CompanionUsageStatus,
  DependencyPage,
  FindingRemediationProjection,
  LocalEntityResult,
  PrivacyConsentStatus,
  ProviderConnection,
  ProviderConnectionSnapshot,
  RepoAsset,
  RepoOverview,
  RepoScanComparison,
  RepositoryChatContext,
  RepositoryChatScanStatus,
  ReviewPage,
  ReviewFinding,
  ScanActivity,
} from "./types";

const COMPANION_SCHEMA_VERSION = 1;
const MAX_COMPANION_CONTEXT_BYTES = 32 * 1024;

function object(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`Malformed ${label} payload`);
  return value as Record<string, unknown>;
}

function stringOrNull(value: unknown, label: string): string | null {
  if (value === null) return null;
  if (typeof value !== "string") throw new Error(`Malformed ${label} payload`);
  return value;
}

function isNonNegativeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0;
}

function boundedStringOrNull(value: unknown, label: string, maxLength: number): string | null {
  const parsed = stringOrNull(value, label);
  if (parsed !== null && parsed.length > maxLength) throw new Error(`Malformed ${label} payload`);
  return parsed;
}

export function parseFindingRemediation(value: unknown): FindingRemediationProjection {
  const projection = object(value, "finding remediation");
  if (projection.schemaVersion !== 1) throw new Error("Unsupported finding remediation schema");
  if (!Number.isInteger(projection.repoId) || Number(projection.repoId) <= 0 || !Number.isInteger(projection.evidenceScanRunId) || Number(projection.evidenceScanRunId) <= 0) throw new Error("Malformed finding remediation payload");
  for (const key of ["repoName", "fingerprint", "advisoryId", "packageName"] as const) {
    if (typeof projection[key] !== "string" || projection[key].length === 0 || projection[key].length > 512) throw new Error("Malformed finding remediation payload");
  }
  for (const key of ["installedVersion", "packageType", "packageUrl", "scannerReportedTarget", "evidenceScannedAt", "relativePath", "parentPackage", "verificationScannedAt"] as const) {
    boundedStringOrNull(projection[key], "finding remediation", key === "relativePath" ? 1024 : 512);
  }
  if (!([null, "manifest", "lockfile", "other"] as unknown[]).includes(projection.locationKind)) throw new Error("Malformed finding remediation location");
  if (!["direct", "transitive", "unknown"].includes(String(projection.relationship))) throw new Error("Malformed finding remediation relationship");
  if (!["current", "partial", "unavailable", "stale"].includes(String(projection.evidenceQuality))) throw new Error("Malformed finding remediation evidence quality");
  if (!["syft_location", "syft_relationship", "supported_manifest", "none"].includes(String(projection.evidenceSource))) throw new Error("Malformed finding remediation evidence source");
  if (!["still_detected", "not_detected", "reappeared", "unverifiable"].includes(String(projection.verificationState))) throw new Error("Malformed finding remediation verification state");
  if (!isNonNegativeInteger(projection.locationCount) || Number(projection.locationCount) > 32 || typeof projection.canOpenManifest !== "boolean") throw new Error("Malformed finding remediation payload");
  if (projection.verificationScanRunId !== null && (!Number.isInteger(projection.verificationScanRunId) || Number(projection.verificationScanRunId) <= 0)) throw new Error("Malformed finding remediation payload");
  const source = object(projection.sourceSnapshot, "source snapshot");
  boundedStringOrNull(source.revision, "source snapshot", 64);
  if (source.dirty !== null && typeof source.dirty !== "boolean") throw new Error("Malformed source snapshot payload");
  if (!Array.isArray(projection.priorityReasons) || projection.priorityReasons.length > 5 || !projection.priorityReasons.every((reason) => typeof reason === "string" && reason.length <= 160)) throw new Error("Malformed finding remediation priorities");
  if (typeof projection.briefMarkdown !== "string" || projection.briefMarkdown.length === 0 || projection.briefMarkdown.length > 4096) throw new Error("Malformed finding remediation brief");
  return value as FindingRemediationProjection;
}

export function parseReviewPage(value: unknown): ReviewPage {
  const page = object(value, "Review page");
  if (!Array.isArray(page.items) || typeof page.total !== "number" || !Number.isInteger(page.total) || page.total < 0) throw new Error("Malformed Review page payload");
  stringOrNull(page.nextCursor, "Review cursor");
  const groupCounts = object(page.groupCounts, "Review group counts");
  for (const grouping of ["reviewState", "severity", "repository", "fixAvailability"] as const) {
    const counts = groupCounts[grouping];
    if (!Array.isArray(counts)) throw new Error("Malformed Review group counts payload");
    for (const value of counts) {
      const count = object(value, "Review lane count");
      if (typeof count.id !== "string" || typeof count.label !== "string" || typeof count.count !== "number" || !Number.isInteger(count.count) || count.count < 0) throw new Error("Malformed Review lane count payload");
    }
  }
  page.items.forEach(parseReviewFinding);
  if (!isNonNegativeInteger(page.revision)) throw new Error("Malformed Review revision payload");
  return value as ReviewPage;
}

export function parseReviewFinding(value: unknown): ReviewFinding {
  const item = object(value, "Review finding");
  const finding = object(item.finding, "Review finding");
  if (typeof item.repoId !== "number" || typeof item.repoName !== "string" || typeof finding.fingerprint !== "string" || typeof finding.advisoryId !== "string" || !["unreviewed", "reviewed", "snoozed"].includes(String(item.reviewState)) || !Array.isArray(item.priorityReasons)) throw new Error("Malformed Review finding payload");
  validateFindingMatchEvidence(finding.matchEvidence);
  return value as ReviewFinding;
}

function validateFindingMatchEvidence(value: unknown): void {
  if (value === undefined) return;
  const evidence = object(value, "finding match evidence");
  if (typeof evidence.kind !== "string" || !Object.hasOwn(FINDING_MATCH_LABEL, evidence.kind)) {
    throw new Error("Malformed finding match evidence");
  }
  stringOrNull(evidence.relativePath, "finding match location");
  if (evidence.bundledTool !== undefined) {
    const tool = object(evidence.bundledTool, "bundled tool evidence");
    stringOrNull(tool.packageName, "bundled tool package name");
    stringOrNull(tool.componentFixVersion, "bundled component fix version");
  }
}

export function parseRepoAssets(value: unknown, repoId: number): RepoAsset[] {
  if (!Array.isArray(value)) throw new Error("Malformed repository product signals payload");
  for (const raw of value) {
    const asset = object(raw, "repository product signal");
    if (asset.repoId !== repoId || !isNonNegativeInteger(asset.confidence)) {
      throw new Error("Malformed repository product signal payload");
    }
    for (const key of ["vendor", "product", "assetKind", "evidence"] as const) {
      if (typeof asset[key] !== "string") throw new Error("Malformed repository product signal payload");
    }
    stringOrNull(asset.sourcePath, "repository product signal path");
    stringOrNull(asset.version, "repository product signal version");
  }
  return value as RepoAsset[];
}

export function parseArtifactScan(value: unknown, repoId: number): ArtifactScan {
  const scan = object(value, "artifact scan");
  const summary = object(scan.summary, "artifact scan summary");
  if (scan.repoId !== repoId || !isNonNegativeInteger(scan.id) || scan.id === 0 ||
    !["build_file", "build_folder", "sbom"].includes(String(summary.kind)) ||
    typeof summary.name !== "string" || typeof summary.scannedAt !== "string" ||
    typeof summary.digest !== "string" || !/^[a-f0-9]{64}$/.test(summary.digest) ||
    !isNonNegativeInteger(summary.packageCount) || !isNonNegativeInteger(summary.findingCount) ||
    !Array.isArray(summary.coverageWarnings) || !summary.coverageWarnings.every((warning) => typeof warning === "string")) {
    throw new Error("Malformed artifact scan payload");
  }
  stringOrNull(summary.platform, "artifact scan platform");
  return value as ArtifactScan;
}

export function parseArtifactFindings(value: unknown, repoId: number, scanId: number, offset: number): ArtifactFindings {
  const page = object(value, "artifact findings");
  if (page.repoId !== repoId || page.scanId !== scanId || page.offset !== offset || !Array.isArray(page.findings)) {
    throw new Error("Malformed artifact findings payload");
  }
  for (const raw of page.findings) {
    const finding = object(raw, "artifact finding");
    if (typeof finding.fingerprint !== "string" || typeof finding.advisoryId !== "string" || typeof finding.packageName !== "string" || !isSeverity(finding.severity)) {
      throw new Error("Malformed artifact finding payload");
    }
    for (const key of ["packageVersion", "packageType", "fixVersion", "title", "sourceUrl"] as const) {
      stringOrNull(finding[key], "artifact finding");
    }
    validateFindingMatchEvidence(finding.matchEvidence);
  }
  return value as ArtifactFindings;
}

export function parseRepoOverview(value: unknown): RepoOverview {
  const detail = object(value, "Repository overview");
  const summary = object(detail.summary, "Repository summary");
  if (typeof summary.id !== "number" || !Number.isInteger(summary.id) || summary.id <= 0 || typeof summary.name !== "string" || typeof summary.path !== "string" || !["local", "github", "unknown"].includes(String(summary.sourceType))) {
    throw new Error("Malformed Repository overview payload");
  }
  if (summary.dependencyRevision !== undefined &&
    (typeof summary.dependencyRevision !== "number" || !Number.isSafeInteger(summary.dependencyRevision) || summary.dependencyRevision < 0)) {
    throw new Error("Malformed dependency inventory revision");
  }
  const overview = object(detail.overview, "Repository overview metadata");
  for (const key of ["packageCount", "affectedCount", "openFindingCount", "opportunityCount"] as const) {
    if (!isNonNegativeInteger(overview[key])) throw new Error("Malformed Repository overview counts");
  }
  if (overview.latestCompletedScan !== null) {
    const scan = object(overview.latestCompletedScan, "Latest completed scan");
    parseRepoScan(scan);
    if (scan.status !== "completed") throw new Error("Malformed latest completed scan");
  }
  if (!Array.isArray(overview.opportunities) || !Array.isArray(detail.findings) || !Array.isArray(detail.scanHistory)) {
    throw new Error("Malformed Repository overview collections");
  }
  for (const value of overview.opportunities) {
    const opportunity = object(value, "Remediation opportunity");
    if (typeof opportunity.key !== "string" || typeof opportunity.packageName !== "string" ||
      !Array.isArray(opportunity.fingerprints) || !opportunity.fingerprints.every((fingerprint) => typeof fingerprint === "string")) {
      throw new Error("Malformed Remediation opportunity payload");
    }
    for (const key of ["packageType", "packageUrl", "installedVersion", "fixVersion"] as const) stringOrNull(opportunity[key], "Remediation opportunity");
    for (const key of ["findingCount", "critical", "high", "medium"] as const) {
      if (!isNonNegativeInteger(opportunity[key])) throw new Error("Malformed Remediation opportunity counts");
    }
  }
  for (const value of detail.findings) {
    const finding = object(value, "Repository finding");
    if (typeof finding.fingerprint !== "string" || typeof finding.advisoryId !== "string" || typeof finding.packageName !== "string" || !isSeverity(finding.severity) || !["new", "open"].includes(String(finding.state))) {
      throw new Error("Malformed Repository finding payload");
    }
    validateFindingMatchEvidence(finding.matchEvidence);
  }
  for (const key of ["latestScan", "previousScan"] as const) {
    if (detail[key] !== null) parseRepoScan(detail[key]);
  }
  for (const scan of detail.scanHistory) parseRepoScan(scan);
  if (detail.nugetProjects !== undefined) {
    if (!Array.isArray(detail.nugetProjects)) throw new Error("Malformed .NET projects payload");
    for (const raw of detail.nugetProjects) {
      const project = object(raw, ".NET project");
      if (typeof project.path !== "string" || !Array.isArray(project.frameworks) ||
        !project.frameworks.every((framework) => typeof framework === "string") || !Array.isArray(project.declarations)) {
        throw new Error("Malformed .NET project payload");
      }
      stringOrNull(project.resolutionPath, "NuGet resolution path");
      for (const item of project.declarations) {
        const declaration = object(item, "NuGet declaration");
        if (typeof declaration.name !== "string" || typeof declaration.conditional !== "boolean") {
          throw new Error("Malformed NuGet declaration payload");
        }
        stringOrNull(declaration.requirement, "NuGet version requirement");
        stringOrNull(declaration.requirementPath, "NuGet version requirement path");
      }
    }
  }
  return value as RepoOverview;
}

export function parseDependencyPage(value: unknown): DependencyPage {
  const page = object(value, "Dependency page");
  for (const key of ["total", "inventoryCount", "affectedCount", "revision"] as const) {
    if (!isNonNegativeInteger(page[key])) throw new Error("Malformed dependency page counts");
  }
  if (page.scanRunId !== null && (!isNonNegativeInteger(page.scanRunId) || page.scanRunId === 0)) {
    throw new Error("Malformed dependency scan identity");
  }
  stringOrNull(page.nextCursor, "Dependency cursor");
  if (!Array.isArray(page.ecosystems) || !page.ecosystems.every((kind) => typeof kind === "string") || !Array.isArray(page.items)) {
    throw new Error("Malformed dependency page collections");
  }
  for (const value of page.items) {
    const pkg = object(value, "Repository package");
    if (typeof pkg.name !== "string" || !isNonNegativeInteger(pkg.findingCount) || typeof pkg.fixAvailable !== "boolean" ||
      (pkg.highestSeverity !== null && !isSeverity(pkg.highestSeverity))) {
      throw new Error("Malformed Repository package payload");
    }
    for (const key of ["version", "packageType", "packageUrl"] as const) stringOrNull(pkg[key], "Repository package");
  }
  return value as DependencyPage;
}

export function parseRepositoryChatScanStatus(value: unknown): RepositoryChatScanStatus {
  const status = object(value, "Repository chat scan status");
  const latestCompleted = object(status.latestCompleted, "Completed scan status");
  const latestAttempt = object(status.latestAttempt, "Latest scan status");
  const validId = (id: unknown) => typeof id === "number" && Number.isSafeInteger(id) && id > 0;
  const validScan = (scan: Record<string, unknown>) =>
    (scan.id === null ? scan.status === null : validId(scan.id) && ["running", "completed", "failed", "unknown"].includes(String(scan.status)))
    && (scan.scannedAt === null || typeof scan.scannedAt === "string")
    && (scan.revision === null || typeof scan.revision === "string")
    && typeof scan.evidenceComplete === "boolean";
  if (!validId(status.repositoryId) || !validScan(latestCompleted) || !validScan(latestAttempt)
    || (latestCompleted.id !== null && latestCompleted.status !== "completed")) {
    throw new Error("Malformed Repository chat scan status");
  }
  return value as RepositoryChatScanStatus;
}

function parseRepoScan(value: unknown): void {
  const scan = object(value, "Repository scan");
  if (typeof scan.id !== "number" || !Number.isInteger(scan.id) || typeof scan.startedAt !== "string" || !["running", "completed", "failed", "unknown"].includes(String(scan.status))) {
    throw new Error("Malformed Repository scan payload");
  }
  if (scan.sourceRevision !== undefined) stringOrNull(scan.sourceRevision, "Repository scan");
  if (scan.sourceDirty !== undefined && scan.sourceDirty !== null && typeof scan.sourceDirty !== "boolean") throw new Error("Malformed Repository scan payload");
  if (scan.packageCount !== undefined && scan.packageCount !== null && !isNonNegativeInteger(scan.packageCount)) {
    throw new Error("Malformed Repository scan payload");
  }
  for (const key of ["reopenedCount", "openMedium", "openHigh", "openCritical", "openOther", "newOther", "resolvedOther"] as const) {
    if (scan[key] !== undefined && !isNonNegativeInteger(scan[key])) throw new Error("Malformed Repository scan payload");
  }
  if (scan.evidenceComplete !== undefined && typeof scan.evidenceComplete !== "boolean") throw new Error("Malformed Repository scan payload");
  if (scan.warnings !== undefined && (!Array.isArray(scan.warnings) || !scan.warnings.every((warning) => typeof warning === "string"))) {
    throw new Error("Malformed Repository scan payload");
  }
  if (scan.coverageWarnings !== undefined && (!Array.isArray(scan.coverageWarnings) || !scan.coverageWarnings.every((warning) => typeof warning === "string"))) {
    throw new Error("Malformed Repository scan payload");
  }
}

export function parseRepoScanComparison(value: unknown): RepoScanComparison | null {
  if (value === null) return null;
  const comparison = object(value, "Scan comparison");
  parseRepoScan(comparison.run);
  if (comparison.previousRunId != null && (!isNonNegativeInteger(comparison.previousRunId) || comparison.previousRunId === 0)) {
    throw new Error("Malformed previous scan id");
  }
  if (comparison.previousPackageCount !== null && !isNonNegativeInteger(comparison.previousPackageCount)) {
    throw new Error("Malformed scan comparison payload");
  }
  return value as RepoScanComparison;
}

export function parseScanActivity(value: unknown): ScanActivity | null {
  if (value === null) return null;
  const activity = object(value, "Scan activity");
  const statuses = ["running", "completed", "partial_failure", "failed"];
  const stages = ["queued", "preparing_source", "inventorying_packages", "matching_vulnerabilities", "enriching_advisories", "saving_results", "completed", "failed"];
  if (typeof activity.id !== "string" || !Number.isSafeInteger(activity.sequence) || Number(activity.sequence) <= 0 || !statuses.includes(String(activity.status)) || !stages.includes(String(activity.stage)) || typeof activity.startedAt !== "string" || typeof activity.totalRepositories !== "number" || !Array.isArray(activity.outcomes) || !Array.isArray(activity.warnings)) throw new Error("Malformed Scan activity payload");
  if (activity.interrupted !== undefined && typeof activity.interrupted !== "boolean") throw new Error("Malformed Scan activity payload");
  return value as ScanActivity;
}

export function parseLocalEntityResults(value: unknown): LocalEntityResult[] {
  if (!Array.isArray(value)) throw new Error("Malformed local search payload");
  const kinds = ["repository", "finding", "package", "advisory"];
  value.forEach((entry) => {
    const result = object(entry, "local search");
    if (typeof result.id !== "string" || typeof result.label !== "string" || typeof result.context !== "string" || !kinds.includes(String(result.kind))) throw new Error("Malformed local search payload");
    if (result.repoId !== null && typeof result.repoId !== "number") throw new Error("Malformed local search payload");
    if (result.fingerprint !== null && typeof result.fingerprint !== "string") throw new Error("Malformed local search payload");
    if (result.advisoryId !== null && typeof result.advisoryId !== "number") throw new Error("Malformed local search payload");
  });
  return value as LocalEntityResult[];
}

function requiredString(value: unknown, label: string, maxLength = 1024): string {
  if (typeof value !== "string" || value.length === 0 || value.length > maxLength) throw new Error(`Malformed ${label} payload`);
  return value;
}

function optionalPositiveInteger(value: unknown, label: string): void {
  if (value !== null && (!Number.isInteger(value) || Number(value) <= 0)) throw new Error(`Malformed ${label} payload`);
}

function array(value: unknown, label: string, maxLength: number): unknown[] {
  if (!Array.isArray(value) || value.length > maxLength) throw new Error(`Malformed ${label} payload`);
  return value;
}

export function parseCompanionRuntimeStatus(value: unknown): CompanionRuntimeStatus {
  const status = object(value, "companion runtime");
  const phases = ["disabled", "unavailable", "discovering", "starting", "initializing", "signed_out", "ready", "active_turn", "interrupted", "incompatible", "offline"];
  if (status.schemaVersion !== COMPANION_SCHEMA_VERSION || !phases.includes(String(status.phase)) || !isNonNegativeInteger(status.activeTurns)) throw new Error("Malformed companion runtime payload");
  boundedStringOrNull(status.codexPath, "companion runtime", 4096);
  boundedStringOrNull(status.codexVersion, "companion runtime", 256);
  boundedStringOrNull(status.message, "companion runtime", 2048);
  boundedStringOrNull(status.startedAt, "companion runtime", 128);
  return value as CompanionRuntimeStatus;
}

export function parseCompanionAccountStatus(value: unknown): CompanionAccountStatus {
  const status = object(value, "companion account");
  if (status.schemaVersion !== COMPANION_SCHEMA_VERSION || typeof status.signedIn !== "boolean" || !["chatgpt", "api_key", "none"].includes(String(status.authKind))) throw new Error("Malformed companion account payload");
  boundedStringOrNull(status.planType, "companion account", 128);
  boundedStringOrNull(status.accountLabel, "companion account", 256);
  boundedStringOrNull(status.message, "companion account", 2048);
  return value as CompanionAccountStatus;
}

export function parseCompanionLoginStart(value: unknown): CompanionLoginStart {
  const login = object(value, "companion login");
  if (!["chatgpt", "chatgpt_device_code", "api_key"].includes(String(login.kind))) throw new Error("Malformed companion login payload");
  for (const key of ["loginId", "authUrl", "verificationUrl", "userCode"] as const) boundedStringOrNull(login[key], "companion login", key.endsWith("Url") ? 4096 : 512);
  requiredString(login.status, "companion login", 128);
  return value as CompanionLoginStart;
}

export function parseCompanionModels(value: unknown): CompanionModel[] {
  const models = array(value, "companion models", 256);
  for (const entry of models) {
    const model = object(entry, "companion model");
    requiredString(model.id, "companion model", 256);
    requiredString(model.displayName, "companion model", 256);
    boundedStringOrNull(model.description, "companion model", 2048);
    if (typeof model.isDefault !== "boolean" || typeof model.hidden !== "boolean") throw new Error("Malformed companion model payload");
    if (!array(model.reasoningEfforts, "companion model", 16).every((item) => typeof item === "string" && item.length <= 64)) throw new Error("Malformed companion model payload");
    if (!array(model.inputModalities, "companion model", 16).every((item) => typeof item === "string" && item.length <= 64)) throw new Error("Malformed companion model payload");
    if (model.contextWindow !== null && !isNonNegativeInteger(model.contextWindow)) throw new Error("Malformed companion model payload");
  }
  return value as CompanionModel[];
}

export function parseCompanionUsage(value: unknown): CompanionUsageStatus {
  const usage = object(value, "companion usage");
  if (typeof usage.supported !== "boolean") throw new Error("Malformed companion usage payload");
  boundedStringOrNull(usage.unavailableReason, "companion usage", 1024);
  boundedStringOrNull(usage.fetchedAt, "companion usage", 128);
  for (const key of ["lifetimeTokens", "peakDailyTokens"] as const) if (usage[key] !== null && !isNonNegativeInteger(usage[key])) throw new Error("Malformed companion usage payload");
  for (const key of ["primary", "secondary"] as const) {
    if (usage[key] === null) continue;
    const window = object(usage[key], "companion usage window");
    if (typeof window.usedPercent !== "number" || window.usedPercent < 0 || window.usedPercent > 100) throw new Error("Malformed companion usage payload");
  }
  return value as CompanionUsageStatus;
}

export function parseCompanionThread(value: unknown): CompanionThread {
  const thread = object(value, "companion thread");
  if (!Number.isInteger(thread.id) || Number(thread.id) <= 0 || !Number.isInteger(thread.repositoryId) || Number(thread.repositoryId) <= 0) throw new Error("Malformed companion thread payload");
  for (const key of ["provider", "providerThreadId", "title", "createdAt", "updatedAt", "lastOpenedAt"] as const) requiredString(thread[key], "companion thread", key === "title" ? 256 : 1024);
  if (!["not_loaded", "ready", "active", "interrupted", "history_unavailable"].includes(String(thread.status)) || thread.contextVersion !== 1) throw new Error("Malformed companion thread payload");
  boundedStringOrNull(thread.archivedAt, "companion thread", 128);
  boundedStringOrNull(thread.lastModelId, "companion thread", 256);
  requiredString(thread.providerConnectionId, "companion thread connection", 64);
  return value as CompanionThread;
}

export function parseCompanionThreads(value: unknown): CompanionThread[] {
  return array(value, "companion threads", 500).map(parseCompanionThread);
}

export function parseCompanionProviderSegment(value: unknown): CompanionProviderSegment {
  const segment = object(value, "companion provider segment");
  if (!Number.isInteger(segment.id) || Number(segment.id) <= 0 || !Number.isInteger(segment.threadId) || Number(segment.threadId) <= 0 || !isNonNegativeInteger(segment.ordinal)) throw new Error("Malformed companion provider segment payload");
  for (const key of ["provider", "connectionId", "providerThreadId", "createdAt", "updatedAt"] as const) requiredString(segment[key], "companion provider segment", key === "providerThreadId" ? 512 : 128);
  boundedStringOrNull(segment.initialModelId, "companion provider segment", 256);
  boundedStringOrNull(segment.lastModelId, "companion provider segment", 256);
  boundedStringOrNull(segment.endedAt, "companion provider segment", 128);
  if (!["ready", "active", "interrupted", "history_unavailable"].includes(String(segment.status))) throw new Error("Malformed companion provider segment payload");
  return value as CompanionProviderSegment;
}

export function parseCompanionConversationIndex(value: unknown): CompanionConversationIndex {
  const index = object(value, "companion conversation index");
  if (index.schemaVersion !== COMPANION_SCHEMA_VERSION) throw new Error("Unsupported companion conversation index schema");
  array(index.threads, "companion conversation index", 5000).forEach(parseCompanionThread);
  return value as CompanionConversationIndex;
}

function parseCompanionTimelineItems(value: unknown): void {
  const items = array(value, "companion timeline", 5000);
  for (const value of items) {
    const item = object(value, "companion timeline item");
    requiredString(item.id, "companion timeline item", 512);
    if (item.modelId !== undefined) boundedStringOrNull(item.modelId, "companion timeline item model", 256);
    if (!["user_message", "assistant_message", "activity", "warning", "error"].includes(String(item.kind)) || !["streaming", "completed", "interrupted", "failed"].includes(String(item.status)) || typeof item.text !== "string" || item.text.length > 2_000_000) throw new Error("Malformed companion timeline item payload");
    array(item.citations, "companion citations", 128);
  }
}

export function parseCompanionThreadSnapshot(value: unknown): CompanionThreadSnapshot {
  const snapshot = object(value, "companion thread snapshot");
  parseCompanionThread(snapshot.thread);
  parseCompanionTimelineItems(snapshot.items);
  return value as CompanionThreadSnapshot;
}

export function parseCompanionCachedThreadSnapshot(value: unknown): CompanionCachedThreadSnapshot {
  const snapshot = object(value, "cached companion thread snapshot");
  if (snapshot.schemaVersion !== COMPANION_SCHEMA_VERSION || typeof snapshot.providerHistoryAvailable !== "boolean") throw new Error("Malformed cached companion thread snapshot payload");
  if(snapshot.olderCursor!==undefined)boundedStringOrNull(snapshot.olderCursor,"history cursor",256);
  if(snapshot.eventWatermarks!==undefined){const watermarks=object(snapshot.eventWatermarks,"history watermarks");if(!Object.values(watermarks).every(isNonNegativeInteger))throw new Error("Malformed history watermark payload");}
  if(snapshot.streamingItemIds!==undefined && (!Array.isArray(snapshot.streamingItemIds)||!snapshot.streamingItemIds.every((id)=>typeof id==="string")))throw new Error("Malformed streaming history IDs");
  if(snapshot.providerOrdinals!==undefined){const ordinals=object(snapshot.providerOrdinals,"history provider ordinals");if(!Object.values(ordinals).every(isNonNegativeInteger))throw new Error("Malformed history provider ordinals payload");}
  parseCompanionThread(snapshot.thread);
  array(snapshot.segments, "companion provider segments", 256).forEach(parseCompanionProviderSegment);
  parseCompanionTimelineItems(snapshot.items);
  return value as CompanionCachedThreadSnapshot;
}

export function parseCompanionEvent(value: unknown): CompanionEventEnvelope {
  const event = object(value, "companion event");
  const kinds = ["runtime_status", "account_status", "thread_snapshot", "thread_updated", "turn_started", "item_started", "item_delta", "item_completed", "activity", "usage", "turn_completed", "warning", "error"];
  if (event.schemaVersion !== COMPANION_SCHEMA_VERSION || !isNonNegativeInteger(event.sequence) || !kinds.includes(String(event.kind))) throw new Error("Malformed companion event payload");
  if(event.providerSegmentId!==undefined)optionalPositiveInteger(event.providerSegmentId,"companion event segment");
  optionalPositiveInteger(event.repositoryId, "companion event");
  optionalPositiveInteger(event.threadId, "companion event");
  boundedStringOrNull(event.providerThreadId, "companion event", 512);
  boundedStringOrNull(event.connectionId, "companion event", 64);
  boundedStringOrNull(event.driver, "companion event", 64);
  boundedStringOrNull(event.modelId, "companion event", 256);
  boundedStringOrNull(event.turnId, "companion event", 512);
  return value as CompanionEventEnvelope;
}

export function parseProviderConnection(value: unknown): ProviderConnection {
  const connection = object(value, "AI connection");
  if (connection.schemaVersion !== 1) throw new Error("Unsupported AI connection schema");
  requiredString(connection.id, "AI connection id", 64);
  requiredString(connection.displayName, "AI connection name", 80);
  if (!["codex", "grok", "open_code"].includes(String(connection.driver))) throw new Error("Malformed AI connection driver");
  if (typeof connection.enabled !== "boolean" || typeof connection.builtIn !== "boolean" || typeof connection.hasServerPassword !== "boolean") throw new Error("Malformed AI connection payload");
  boundedStringOrNull(connection.accentColor, "AI connection accent", 7);
  requiredString(connection.createdAt, "AI connection timestamp", 128);
  requiredString(connection.updatedAt, "AI connection timestamp", 128);
  const config = object(connection.config, "AI connection config");
  if (connection.driver === "codex") {
    if (config.kind !== "codex") throw new Error("Malformed Codex connection config");
    for (const key of ["binaryPath", "homePath", "profile", "launchArgs"] as const) {
      if (config[key] !== undefined) boundedStringOrNull(config[key], "Codex connection config", key === "launchArgs" ? 2048 : 4096);
    }
  } else if (connection.driver === "grok") {
    if (config.kind !== "grok") throw new Error("Malformed Grok connection config");
    if (config.binaryPath !== undefined) boundedStringOrNull(config.binaryPath, "Grok connection config", 4096);
    if (config.customModels !== undefined) {
      const models = array(config.customModels, "Grok custom models", 64);
      for (const model of models) requiredString(model, "Grok custom model", 256);
    }
  } else {
    if (config.kind !== "open_code" || (config.mode !== undefined && !["managed_local", "existing_loopback"].includes(String(config.mode)))) throw new Error("Malformed OpenCode connection config");
    if (config.binaryPath !== undefined) boundedStringOrNull(config.binaryPath, "OpenCode connection config", 4096);
    if (config.serverUrl !== undefined) boundedStringOrNull(config.serverUrl, "OpenCode connection config", 4096);
  }
  return value as ProviderConnection;
}

export function parseProviderConnectionSnapshot(value: unknown): ProviderConnectionSnapshot {
  const snapshot = object(value, "AI connection snapshot");
  if (snapshot.schemaVersion !== 1) throw new Error("Unsupported AI connection snapshot schema");
  parseProviderConnection(snapshot.connection);
  if (!["unchecked", "checking", "ready", "setup_required", "unavailable", "incompatible", "disabled", "stale"].includes(String(snapshot.health))) throw new Error("Malformed AI connection health");
  boundedStringOrNull(snapshot.installedVersion, "AI connection version", 256);
  boundedStringOrNull(snapshot.accountLabel, "AI connection account", 256);
  boundedStringOrNull(snapshot.checkedAt, "AI connection timestamp", 128);
  boundedStringOrNull(snapshot.message, "AI connection message", 2048);
  if (!["chatgpt", "api_key", "none"].includes(String(snapshot.authKind))) throw new Error("Malformed AI connection auth");
  const capabilities = object(snapshot.capabilities, "AI connection capabilities");
  for (const value of Object.values(capabilities)) if (typeof value !== "boolean") throw new Error("Malformed AI connection capabilities");
  const upstreams = array(snapshot.upstreamProviders, "AI upstream providers", 128);
  for (const value of upstreams) {
    const upstream = object(value, "AI upstream provider");
    requiredString(upstream.id, "AI upstream provider", 128);
    requiredString(upstream.displayName, "AI upstream provider", 256);
    if (typeof upstream.connected !== "boolean" || !["verified_local", "external", "unknown"].includes(String(upstream.inferenceBoundary))) throw new Error("Malformed AI upstream provider");
  }
  const models = array(snapshot.models, "AI connection models", 2048);
  for (const value of models) {
    const model = object(value, "AI connection model");
    requiredString(model.connectionId, "AI connection model", 64);
    requiredString(model.id, "AI connection model", 256);
    requiredString(model.displayName, "AI connection model", 256);
    if (!["codex", "grok", "open_code"].includes(String(model.driver)) || !["verified_local", "external", "unknown"].includes(String(model.inferenceBoundary))) throw new Error("Malformed AI connection model");
    boundedStringOrNull(model.upstreamProviderId, "AI connection model", 128);
  }
  if (snapshot.error !== null) {
    const error = object(snapshot.error, "AI connection error");
    if (!["not_installed", "incompatible", "authentication_required", "connection_refused", "model_unavailable", "history_unavailable", "policy_denied", "provider_error", "process_exited", "protocol_error", "offline"].includes(String(error.code))) throw new Error("Malformed AI connection error");
    requiredString(error.message, "AI connection error", 2048);
    if (typeof error.retryable !== "boolean") throw new Error("Malformed AI connection error");
  }
  return value as ProviderConnectionSnapshot;
}

export function parseProviderConnectionSnapshots(value: unknown): ProviderConnectionSnapshot[] {
  return array(value, "AI connections", 128).map(parseProviderConnectionSnapshot);
}

export function parseRepositoryChatContext(value: unknown): RepositoryChatContext {
  const context = object(value, "repository chat context");
  if (context.schemaVersion !== COMPANION_SCHEMA_VERSION || typeof context.bounded !== "boolean" || !isNonNegativeInteger(context.estimatedBytes) || Number(context.estimatedBytes) > MAX_COMPANION_CONTEXT_BYTES) throw new Error("Malformed repository chat context payload");
  const repository = object(context.repository, "repository chat repository");
  if (!Number.isInteger(repository.id) || Number(repository.id) <= 0) throw new Error("Malformed repository chat context payload");
  requiredString(repository.name, "repository chat repository", 512);
  requiredString(repository.sourceType, "repository chat repository", 64);
  requiredString(repository.sourceMode, "repository chat repository", 64);
  boundedStringOrNull(repository.approvedRoot, "repository chat repository", 4096);
  const scan = object(context.scan, "repository chat scan");
  optionalPositiveInteger(scan.id, "repository chat scan");
  const freshness = object(context.freshness, "repository chat freshness");
  if (!["matched", "mismatched", "dirty", "unknown", "newer_scan"].includes(String(freshness.state))) throw new Error("Malformed repository chat context payload");
  requiredString(freshness.message, "repository chat freshness", 1024);
  // The repository and latest completed scan can accompany 32 explicit attachments.
  const attachments = array(context.attachments, "repository chat attachments", 34);
  for (const value of attachments) {
    const attachment = object(value, "repository chat attachment");
    requiredString(attachment.id, "repository chat attachment", 1024);
    requiredString(attachment.label, "repository chat attachment", 512);
    if (!["repository", "scan", "finding", "package", "file", "changes"].includes(String(attachment.kind)) || typeof attachment.optional !== "boolean" || typeof attachment.available !== "boolean") throw new Error("Malformed repository chat attachment payload");
  }
  array(context.sources, "repository chat sources", 24);
  array(context.omissions, "repository chat omissions", 48);
  const evidence = array(context.untrustedEvidence, "repository chat evidence", 24);
  for (const value of evidence) {
    const item = object(value, "repository chat evidence");
    requiredString(item.sourceId, "repository chat evidence", 512);
    requiredString(item.label, "repository chat evidence", 512);
    if (typeof item.content !== "string" || item.content.length > 24 * 1024) throw new Error("Malformed repository chat evidence payload");
  }
  if (!array(context.instructions, "repository chat instructions", 16).every((item) => typeof item === "string" && item.length <= 2048)) throw new Error("Malformed repository chat context payload");
  return value as RepositoryChatContext;
}

export function parseCompanionMentions(value: unknown): CompanionMention[] {
  const mentions = array(value, "companion mentions", 100);
  for (const entry of mentions) {
    const mention = object(entry, "companion mention");
    requiredString(mention.id, "companion mention", 1024);
    requiredString(mention.label, "companion mention", 512);
    if (typeof mention.available !== "boolean") throw new Error("Malformed companion mention payload");
  }
  return value as CompanionMention[];
}

export function parseCompanionSourceDetail(value: unknown): CompanionSourceDetail {
  const detail = object(value, "companion source detail");
  object(detail.source, "companion source");
  boundedStringOrNull(detail.content, "companion source detail", 24 * 1024);
  boundedStringOrNull(detail.unavailableReason, "companion source detail", 1024);
  return value as CompanionSourceDetail;
}

export function parseCompanionTurnStart(value: unknown): CompanionTurnStart {
  const turn = object(value, "companion turn");
  requiredString(turn.turnId, "companion turn", 512);
  requiredString(turn.providerThreadId, "companion turn", 512);
  requiredString(turn.acceptedClientTurnId, "companion turn", 512);
  return value as CompanionTurnStart;
}

export function parsePrivacyConsent(value: unknown): PrivacyConsentStatus {
  const consent = object(value, "companion privacy consent");
  requiredString(consent.provider, "companion privacy consent", 64);
  if (consent.disclosureVersion !== 1 || typeof consent.valid !== "boolean") throw new Error("Malformed companion privacy consent payload");
  boundedStringOrNull(consent.acceptedAt, "companion privacy consent", 128);
  boundedStringOrNull(consent.revokedAt, "companion privacy consent", 128);
  return value as PrivacyConsentStatus;
}
