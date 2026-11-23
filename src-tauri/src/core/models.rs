use serde::{Deserialize, Serialize};
use time::Duration;

use super::github_auth::GithubAuthSnapshot;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RepoStatus {
    Unknown,
    Clean,
    Warning,
    Critical,
    Error,
    Scanning,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepoSourceType {
    Local,
    Github,
    Unknown,
}

impl RepoSourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Github => "github",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "local" => Self::Local,
            "github" => Self::Github,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Severity {
    #[serde(rename = "MEDIUM")]
    Medium,
    #[serde(rename = "HIGH")]
    High,
    #[serde(rename = "CRITICAL")]
    Critical,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Medium => "MEDIUM",
            Severity::High => "HIGH",
            Severity::Critical => "CRITICAL",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_uppercase().as_str() {
            "MEDIUM" => Some(Self::Medium),
            "HIGH" => Some(Self::High),
            "CRITICAL" => Some(Self::Critical),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoRecord {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub is_active: bool,
    pub source_type: RepoSourceType,
    pub source_key: Option<String>,
    pub remote_url: Option<String>,
    pub remote_host: Option<String>,
    pub provider: Option<String>,
    pub default_branch: Option<String>,
    pub last_remote_commit_sha: Option<String>,
    pub last_remote_synced_at: Option<String>,
    pub remote_file_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoSummary {
    #[serde(default)]
    pub evidence_revision: u64,
    #[serde(default)]
    pub dependency_revision: u64,
    pub id: i64,
    pub name: String,
    pub path: String,
    pub source_type: RepoSourceType,
    pub source_key: Option<String>,
    pub remote_url: Option<String>,
    pub remote_host: Option<String>,
    pub provider: Option<String>,
    pub default_branch: Option<String>,
    pub last_remote_commit_sha: Option<String>,
    pub last_remote_synced_at: Option<String>,
    pub remote_file_count: Option<u32>,
    pub status: RepoStatus,
    pub new_medium: u32,
    pub new_high: u32,
    pub new_critical: u32,
    pub open_medium: u32,
    pub open_high: u32,
    pub open_critical: u32,
    pub resolved_medium: u32,
    pub resolved_high: u32,
    pub resolved_critical: u32,
    pub last_scanned_at: Option<String>,
    pub last_scan_status: Option<ScanRunStatus>,
    pub last_successful_source_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppNotificationKind {
    ScanAlert,
    DbUpdate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppNotificationTone {
    Warning,
    Success,
    Neutral,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppNotification {
    pub id: i64,
    pub kind: AppNotificationKind,
    pub tone: AppNotificationTone,
    pub title: String,
    pub message: String,
    pub repo_id: Option<i64>,
    pub repo_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_run_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ExternalSource {
    Osv,
    Github,
    Nvd,
    CisaKev,
    Vendor,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ExternalIntelSeverity {
    Malware,
    Critical,
    High,
    Medium,
    Low,
    Unknown,
}

impl ExternalIntelSeverity {
    pub fn rank(&self) -> u8 {
        match self {
            ExternalIntelSeverity::Malware => 5,
            ExternalIntelSeverity::Critical => 4,
            ExternalIntelSeverity::High => 3,
            ExternalIntelSeverity::Medium => 2,
            ExternalIntelSeverity::Low => 1,
            ExternalIntelSeverity::Unknown => 0,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ExternalIntelSeverity::Malware => "malware",
            ExternalIntelSeverity::Critical => "critical",
            ExternalIntelSeverity::High => "high",
            ExternalIntelSeverity::Medium => "medium",
            ExternalIntelSeverity::Low => "low",
            ExternalIntelSeverity::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ExternalIntelRelevance {
    Relevant,
    Triage,
    General,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ExternalIntelMatchKind {
    PackageName,
    Product,
    Vendor,
    Ecosystem,
}

impl ExternalIntelMatchKind {
    pub(super) fn relevance(kind: Option<&Self>) -> ExternalIntelRelevance {
        match kind {
            Some(Self::PackageName) => ExternalIntelRelevance::Relevant,
            Some(_) => ExternalIntelRelevance::Triage,
            None => ExternalIntelRelevance::General,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIntelLocalPackage {
    pub repo_id: i64,
    pub repo_name: String,
    pub ecosystem: String,
    pub package_name: String,
    pub package: ScannedPackageIdentity,
    #[serde(default)]
    pub affected_version_ranges: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIntelFeed {
    pub intel_revision: u64,
    pub inventory_revision: u64,
    pub items: Vec<ExternalIntelFeedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIntelFeedItem {
    pub id: i64,
    pub canonical_key: String,
    pub title: String,
    pub summary: String,
    pub ecosystem: Option<String>,
    pub package_name: Option<String>,
    pub product_keys: Vec<String>,
    pub topic_keys: Vec<String>,
    pub severity: ExternalIntelSeverity,
    pub published_at: Option<String>,
    pub withdrawn_at: Option<String>,
    pub cvss_score: Option<String>,
    pub epss_percentage: Option<String>,
    pub is_kev: bool,
    pub is_ransomware_known: bool,
    pub relevance: ExternalIntelRelevance,
    pub sources: Vec<ExternalSource>,
    pub aliases: Vec<String>,
    pub match_kind: Option<ExternalIntelMatchKind>,
    pub repo_count: u32,
    pub cwes: Vec<AdvisoryCwe>,
    pub kev_dates: Option<ExternalIntelKevDates>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIntelKevDates {
    pub date_added: Option<String>,
    pub due_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIntelDetail {
    pub url: String,
    pub updated_at: Option<String>,
    pub modified_at: Option<String>,
    pub cvss_vector: Option<String>,
    pub cvss_version: Option<String>,
    pub epss_percentile: Option<String>,
    pub references: Vec<AdvisoryReference>,
    pub affected_packages: Vec<AdvisoryPackageImpact>,
    pub local_packages: Vec<ExternalIntelLocalPackage>,
    pub kev: Option<CisaKevDetails>,
    pub source_details: Vec<ExternalIntelSourceDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSourceCount {
    pub source: ExternalSource,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIntelSummary {
    pub total_items: u32,
    pub relevant_items: u32,
    pub triage_items: u32,
    pub last_fetched_at: Option<String>,
    pub sources: Vec<ExternalSource>,
    pub source_counts: Vec<ExternalSourceCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIntelTopic {
    pub key: String,
    pub label: String,
    pub category: String,
    pub total_count: u32,
    pub watched: bool,
    pub suggested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct RepoAsset {
    pub repo_id: i64,
    pub vendor: String,
    pub product: String,
    pub version: Option<String>,
    pub asset_kind: String,
    pub source_path: Option<String>,
    pub evidence: String,
    pub confidence: u8,
}

pub const REMEDIATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemediationLocationKind {
    Manifest,
    Lockfile,
    Other,
}

impl RemediationLocationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manifest => "manifest",
            Self::Lockfile => "lockfile",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "manifest" => Self::Manifest,
            "lockfile" => Self::Lockfile,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemediationRelationship {
    Direct,
    Transitive,
    Unknown,
}

impl RemediationRelationship {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Transitive => "transitive",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "direct" => Self::Direct,
            "transitive" => Self::Transitive,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemediationEvidenceQuality {
    Current,
    Partial,
    Unavailable,
    Stale,
}

impl RemediationEvidenceQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Partial => "partial",
            Self::Unavailable => "unavailable",
            Self::Stale => "stale",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "current" => Self::Current,
            "partial" => Self::Partial,
            "stale" => Self::Stale,
            _ => Self::Unavailable,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemediationEvidenceSource {
    SyftLocation,
    SyftRelationship,
    SupportedManifest,
    None,
}

impl RemediationEvidenceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SyftLocation => "syft_location",
            Self::SyftRelationship => "syft_relationship",
            Self::SupportedManifest => "supported_manifest",
            Self::None => "none",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "syft_location" => Self::SyftLocation,
            "syft_relationship" => Self::SyftRelationship,
            "supported_manifest" => Self::SupportedManifest,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemediationVerificationState {
    StillDetected,
    NotDetected,
    Reappeared,
    Unverifiable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceSnapshot {
    pub revision: Option<String>,
    pub dirty: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRemediationEvidence {
    pub schema_version: u32,
    pub fingerprint: String,
    pub package_name: String,
    pub package_version: Option<String>,
    pub package_type: Option<String>,
    pub package_url: Option<String>,
    pub relative_path: Option<String>,
    pub location_kind: Option<RemediationLocationKind>,
    pub relationship: RemediationRelationship,
    pub parent_package: Option<String>,
    pub source: RemediationEvidenceSource,
    pub quality: RemediationEvidenceQuality,
    pub location_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FindingRemediationProjection {
    pub schema_version: u32,
    pub repo_id: i64,
    pub repo_name: String,
    pub fingerprint: String,
    pub advisory_id: String,
    pub package_name: String,
    pub installed_version: Option<String>,
    pub package_type: Option<String>,
    pub package_url: Option<String>,
    pub scanner_reported_target: Option<String>,
    pub evidence_scan_run_id: i64,
    pub evidence_scanned_at: Option<String>,
    pub source_snapshot: SourceSnapshot,
    pub relative_path: Option<String>,
    pub location_kind: Option<RemediationLocationKind>,
    pub relationship: RemediationRelationship,
    pub parent_package: Option<String>,
    pub evidence_quality: RemediationEvidenceQuality,
    pub evidence_source: RemediationEvidenceSource,
    pub location_count: u32,
    pub verification_state: RemediationVerificationState,
    pub verification_scan_run_id: Option<i64>,
    pub verification_scanned_at: Option<String>,
    pub can_open_manifest: bool,
    pub priority_reasons: Vec<String>,
    pub brief_markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanOutput {
    pub findings: Vec<NormalizedFinding>,
    pub packages: Vec<RepoPackage>,
    pub remediation_evidence: Vec<NormalizedRemediationEvidence>,
    #[serde(skip)]
    pub(crate) evidence: Option<super::scanner::ScanEvidence>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepoFindingState {
    New,
    Open,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepoFindingIntelMatchKind {
    AdvisoryAlias,
    SourceUrl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoFindingIntel {
    #[serde(default)]
    pub fetched_at: Option<String>,
    pub canonical_key: String,
    pub title: String,
    pub summary: String,
    pub url: String,
    pub severity: ExternalIntelSeverity,
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
    pub sources: Vec<ExternalSource>,
    pub aliases: Vec<String>,
    pub match_kind: RepoFindingIntelMatchKind,
    pub vulnerabilities: Vec<AdvisoryPackageImpact>,
    pub cvss_score: Option<String>,
    pub cvss_vector: Option<String>,
    pub epss_percentage: Option<String>,
    pub epss_percentile: Option<String>,
    pub cwes: Vec<AdvisoryCwe>,
    pub references: Vec<AdvisoryReference>,
    pub source_code_location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryDetails {
    #[serde(default)]
    pub fetched_at: Option<String>,
    pub ghsa_id: Option<String>,
    pub cve_id: Option<String>,
    pub summary: String,
    pub description: String,
    pub severity: ExternalIntelSeverity,
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
    pub vulnerabilities: Vec<AdvisoryPackageImpact>,
    pub cvss_score: Option<String>,
    pub cvss_vector: Option<String>,
    pub epss_percentage: Option<String>,
    pub epss_percentile: Option<String>,
    pub cwes: Vec<AdvisoryCwe>,
    pub references: Vec<AdvisoryReference>,
    pub source_code_location: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryPackageImpact {
    pub ecosystem: Option<String>,
    pub package_name: Option<String>,
    pub package_purl: Option<String>,
    pub vulnerable_version_range: Option<String>,
    pub first_patched_version: Option<String>,
    pub affected_versions_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryCwe {
    pub cwe_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryReference {
    pub url: String,
    pub source: Option<String>,
    pub kind: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CisaKevDetails {
    pub date_added: Option<String>,
    pub due_date: Option<String>,
    pub required_action: Option<String>,
    pub known_ransomware_campaign_use: Option<String>,
    pub notes: Option<String>,
    pub catalog_version: Option<String>,
    pub catalog_released_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalIntelSourceDetail {
    pub source: ExternalSource,
    pub source_item_id: String,
    pub url: String,
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
    pub severity: ExternalIntelSeverity,
    pub cvss_score: Option<String>,
    pub epss_percentage: Option<String>,
    pub reference_count: u32,
    #[serde(skip)]
    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoFinding {
    pub fingerprint: String,
    pub advisory_id: String,
    pub package_name: String,
    pub package_version: Option<String>,
    pub package_type: Option<String>,
    pub package_url: Option<String>,
    pub severity: Severity,
    pub fix_version: Option<String>,
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub state: RepoFindingState,
    pub intel: Option<RepoFindingIntel>,
    pub review_state: ReviewState,
    pub reviewed_at: Option<String>,
    pub snooze_until: Option<String>,
    pub review_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoPackage {
    pub name: String,
    pub version: Option<String>,
    pub package_type: Option<String>,
    pub package_url: Option<String>,
    pub highest_severity: Option<Severity>,
    pub finding_count: u32,
    pub fix_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoScanRun {
    pub id: i64,
    pub status: ScanRunStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error_message: Option<String>,
    pub new_medium: u32,
    pub new_high: u32,
    pub new_critical: u32,
    pub resolved_medium: u32,
    pub resolved_high: u32,
    pub resolved_critical: u32,
    pub source_revision: Option<String>,
    pub source_dirty: Option<bool>,
    pub package_count: Option<u32>,
    pub reopened_count: u32,
    pub warnings: Vec<String>,
    pub evidence_complete: bool,
    pub open_medium: u32,
    pub open_high: u32,
    pub open_critical: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoDetail {
    #[serde(default)]
    pub overview: Option<RepoOverviewMetadata>,
    pub summary: RepoSummary,
    pub latest_scan: Option<RepoScanRun>,
    pub previous_scan: Option<RepoScanRun>,
    pub findings: Vec<RepoFinding>,
    pub packages: Vec<RepoPackage>,
    pub scan_history: Vec<RepoScanRun>,
    pub assets: Vec<RepoAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoOverview {
    pub summary: RepoSummary,
    pub latest_scan: Option<RepoScanRun>,
    pub previous_scan: Option<RepoScanRun>,
    pub findings: Vec<RepoFinding>,
    pub scan_history: Vec<RepoScanRun>,
    pub overview: RepoOverviewMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoScanComparison {
    pub run: RepoScanRun,
    pub previous_run_id: Option<i64>,
    pub previous_package_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemediationOpportunity {
    pub key: String,
    pub package_name: String,
    pub package_type: Option<String>,
    pub package_url: Option<String>,
    pub installed_version: Option<String>,
    pub fix_version: Option<String>,
    pub finding_count: u32,
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub fingerprints: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoOverviewMetadata {
    pub latest_completed_scan: Option<RepoScanRun>,
    pub package_count: u32,
    pub affected_count: u32,
    pub open_finding_count: u32,
    pub opportunity_count: u32,
    pub opportunities: Vec<RemediationOpportunity>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DependencyQuery {
    pub search: Option<String>,
    pub exact_package: Option<ScannedPackageIdentity>,
    pub ecosystem: Option<String>,
    pub severity: Option<Severity>,
    pub affected_only: Option<bool>,
    pub fix_available: Option<bool>,
    pub sort: Option<String>,
    pub cursor: Option<String>,
    pub page_size: Option<u32>,
    pub expected_scan_run_id: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct ScannedPackageIdentity {
    pub name: String,
    pub version: Option<String>,
    pub package_type: Option<String>,
    pub package_url: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyPage {
    pub items: Vec<RepoPackage>,
    pub total: u32,
    pub inventory_count: u32,
    pub affected_count: u32,
    pub ecosystems: Vec<String>,
    pub next_cursor: Option<String>,
    pub scan_run_id: Option<i64>,
    pub revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPackageIdentity {
    #[serde(default)]
    pub any_fix: bool,
    pub package_name: String,
    pub package_url: Option<String>,
    pub package_type: Option<String>,
    pub installed_version: Option<String>,
    pub fix_version: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Unreviewed,
    Reviewed,
    Snoozed,
}

impl ReviewState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unreviewed => "unreviewed",
            Self::Reviewed => "reviewed",
            Self::Snoozed => "snoozed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "reviewed" => Self::Reviewed,
            "snoozed" => Self::Snoozed,
            _ => Self::Unreviewed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRiskBasis {
    pub severity: Severity,
    pub fix_version: Option<String>,
    pub known_exploited: bool,
    pub ransomware_known: bool,
    pub scanner_state: RepoFindingState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewActionBasis {
    pub scan_run_id: Option<i64>,
    pub risk_basis: ReviewRiskBasis,
    pub review_state: ReviewState,
    pub reviewed_at: Option<String>,
    pub snooze_until: Option<String>,
    pub review_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FindingReview {
    pub repo_id: i64,
    pub fingerprint: String,
    pub state: ReviewState,
    pub reviewed_at: Option<String>,
    pub snooze_until: Option<String>,
    pub note: Option<String>,
    pub risk_basis: ReviewRiskBasis,
    pub updated_at: String,
    pub transition_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFilters {
    #[serde(default)]
    pub package_identity: Option<ReviewPackageIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub review_states: Vec<ReviewState>,
    pub scanner_states: Vec<RepoFindingState>,
    pub severities: Vec<Severity>,
    pub repo_ids: Vec<i64>,
    pub source_types: Vec<RepoSourceType>,
    pub fix_available: Option<bool>,
    pub packages: Vec<String>,
    pub ecosystems: Vec<String>,
    pub known_exploited: Option<bool>,
    pub ransomware_known: Option<bool>,
    pub changed_since: Option<String>,
    pub scan_run_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scan_change_run_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scan_resolved_run_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_in_scan_run_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSort {
    #[default]
    Priority,
    Severity,
    Recent,
    Repository,
    Package,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDirection {
    Asc,
    Desc,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewOrder {
    pub field: ReviewSort,
    pub direction: ReviewDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewQuery {
    #[serde(default)]
    pub sorts: Vec<ReviewOrder>,
    pub filters: ReviewFilters,
    pub search: Option<String>,
    pub sort: ReviewSort,
    pub cursor: Option<String>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinding {
    #[serde(default)]
    pub advisory_severity: Option<Severity>,
    pub repo_id: i64,
    pub repo_name: String,
    pub repo_source_type: RepoSourceType,
    pub scan_run_id: Option<i64>,
    pub scanned_at: Option<String>,
    pub finding: RepoFinding,
    pub review_state: ReviewState,
    pub reviewed_at: Option<String>,
    pub snooze_until: Option<String>,
    pub review_note: Option<String>,
    pub known_exploited: bool,
    pub ransomware_known: bool,
    pub material_change: Option<String>,
    pub priority_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewLaneCount {
    pub id: String,
    pub label: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewGroupCounts {
    pub review_state: Vec<ReviewLaneCount>,
    pub severity: Vec<ReviewLaneCount>,
    pub repository: Vec<ReviewLaneCount>,
    pub fix_availability: Vec<ReviewLaneCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPage {
    #[serde(default)]
    pub valid_until: Option<String>,
    pub items: Vec<ReviewFinding>,
    pub next_cursor: Option<String>,
    pub total: u32,
    pub group_counts: ReviewGroupCounts,
    pub revision: u64,
    pub review_summary: Option<ReviewSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSummary {
    pub needs_review: u32,
    pub unreviewed_critical: u32,
    pub unreviewed_other: u32,
    pub due_snoozed: u32,
    pub open_reviewed: u32,
    pub snoozed: u32,
    pub recently_resolved: u32,
    pub failed_scans: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUpdate {
    pub review_summary: ReviewSummary,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SnoozeFindingInput {
    pub repo_id: i64,
    pub fingerprint: String,
    pub evidence: ReviewActionBasis,
    pub until: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalEntityKind {
    Repository,
    Finding,
    Package,
    Advisory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalEntityResult {
    pub id: String,
    pub kind: LocalEntityKind,
    pub label: String,
    pub context: String,
    pub repo_id: Option<i64>,
    pub fingerprint: Option<String>,
    pub advisory_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanRunStatus {
    Running,
    Completed,
    Failed,
    Unknown,
}

impl ScanRunStatus {
    pub fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScannerVersions {
    pub syft: String,
    pub grype: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScannerAvailability {
    #[serde(default)]
    pub checked_at: Option<String>,
    #[serde(default)]
    pub checking: bool,
    pub ready: bool,
    pub message: Option<String>,
    pub versions: Option<ScannerVersions>,
    pub grype_db: Option<GrypeDbStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedFinding {
    pub fingerprint: String,
    pub advisory_id: String,
    pub package_name: String,
    pub package_version: Option<String>,
    pub package_type: Option<String>,
    pub severity: Severity,
    pub fix_version: Option<String>,
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub package_url: Option<String>,
    pub advisory_details: Option<AdvisoryDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeltaSummary {
    pub new_findings: Vec<NormalizedFinding>,
    pub resolved_findings: Vec<NormalizedFinding>,
    pub new_medium_count: u32,
    pub new_high_count: u32,
    pub new_critical_count: u32,
    pub resolved_medium_count: u32,
    pub resolved_high_count: u32,
    pub resolved_critical_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanAllSummary {
    pub started_at: String,
    pub finished_at: String,
    pub repos_attempted: usize,
    pub repos_scanned: usize,
    pub repos_failed: usize,
    pub alerts_created: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanAllResult {
    pub summary: ScanAllSummary,
    pub snapshot: Option<AppSnapshot>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanActivityStatus {
    Running,
    Completed,
    PartialFailure,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanActivityStage {
    Queued,
    PreparingSource,
    InventoryingPackages,
    MatchingVulnerabilities,
    EnrichingAdvisories,
    SavingResults,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanRepositoryOutcome {
    pub repo_id: i64,
    pub repo_name: String,
    pub scan_run_id: Option<i64>,
    pub succeeded: bool,
    pub error: Option<String>,
    pub new_findings: u32,
    pub resolved_findings: u32,
    pub worsened_findings: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanActivity {
    pub id: String,
    pub sequence: u64,
    pub status: ScanActivityStatus,
    pub stage: ScanActivityStage,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interrupted: bool,
    pub scope_repo_id: Option<i64>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub total_repositories: u32,
    pub completed_repositories: u32,
    pub failed_repositories: u32,
    pub current_repo_id: Option<i64>,
    pub current_repo_name: Option<String>,
    pub outcomes: Vec<ScanRepositoryOutcome>,
    pub warnings: Vec<String>,
    pub new_findings: u32,
    pub resolved_findings: u32,
    pub worsened_findings: u32,
    pub terminal_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverviewSnapshot {
    pub repos_watched: u32,
    pub new_medium: u32,
    pub new_high: u32,
    pub new_critical: u32,
    pub open_medium: u32,
    pub open_high: u32,
    pub open_critical: u32,
    pub repos_with_new_findings: u32,
    pub last_successful_scan_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RiskTrendPoint {
    pub date: String,
    pub medium: u32,
    pub high: u32,
    pub critical: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatus {
    pub scanner_ready: bool,
    pub scanner_message: Option<String>,
    pub schedule: ScheduleInterval,
    pub last_successful_scan_at: Option<String>,
    pub next_scheduled_scan_at: Option<String>,
    pub scan_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrypeDbStatus {
    pub present: bool,
    pub valid: bool,
    pub path: String,
    pub schema_version: Option<String>,
    pub built_at: Option<String>,
    pub source_url: Option<String>,
    pub update_available: Option<bool>,
    pub latest_schema_version: Option<String>,
    pub latest_built_at: Option<String>,
    pub error: Option<String>,
    pub update_check_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrypeDbUpdateOutcome {
    Downloaded,
    Updated,
    AlreadyCurrent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrypeDbUpdateSummary {
    pub checked_at: String,
    pub outcome: GrypeDbUpdateOutcome,
    pub previous_schema_version: Option<String>,
    pub previous_built_at: Option<String>,
    pub current_schema_version: Option<String>,
    pub current_built_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub app: String,
    pub syft: Option<String>,
    pub grype: Option<String>,
    pub grype_db: Option<GrypeDbStatus>,
    pub last_grype_db_update: Option<GrypeDbUpdateSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStatus {
    NotStarted,
    Completed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingSnapshot {
    pub status: OnboardingStatus,
    pub decided_at: Option<String>,
    pub version: Option<String>,
}

impl Default for OnboardingSnapshot {
    fn default() -> Self {
        Self {
            status: OnboardingStatus::NotStarted,
            decided_at: None,
            version: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub repos: Vec<RepoSummary>,
    pub notifications: Vec<AppNotification>,
    pub external_intel_summary: ExternalIntelSummary,
    pub external_intel_topics: Vec<ExternalIntelTopic>,
    pub overview_snapshot: OverviewSnapshot,
    pub workspace_status: WorkspaceStatus,
    pub discovery_root: String,
    pub version_info: VersionInfo,
    pub github_auth: GithubAuthSnapshot,
    pub onboarding: OnboardingSnapshot,
    pub review_summary: ReviewSummary,
    pub scan_activity: Option<ScanActivity>,
    #[serde(default)]
    pub data_revisions: DataRevisions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataRevisions {
    pub advisory: u64,
    pub inventory: u64,
    #[serde(default)]
    pub scan_history: u64,
    pub review: u64,
    pub intel: u64,
    pub scanner: u64,
    pub notifications: u64,
    pub settings: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScheduleInterval {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "12h")]
    Every12Hours,
}

impl ScheduleInterval {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScheduleInterval::Off => "off",
            ScheduleInterval::Every12Hours => "12h",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "12h" => Some(Self::Every12Hours),
            _ => None,
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        match self {
            ScheduleInterval::Off => None,
            ScheduleInterval::Every12Hours => Some(Duration::hours(12)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayStatus {
    Clean,
    Warning,
    Red,
}

impl TrayStatus {
    pub(crate) fn from_repositories(repos: &[RepoSummary]) -> Self {
        let mut status = Self::Clean;
        for repo in repos {
            match repo.status {
                RepoStatus::Critical => return Self::Red,
                RepoStatus::Warning | RepoStatus::Error => status = Self::Warning,
                _ => {}
            }
        }
        status
    }
}
