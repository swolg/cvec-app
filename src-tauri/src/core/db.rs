use crate::core::timestamps;
mod chat_cache;
mod chat_drafts;
mod enrichment;
mod fix_workspaces;
mod intel_packages;
mod inventory;
mod review;
mod revisions;

use intel_packages::CachedIntelPackages;
pub(super) use intel_packages::{advisory_package_keys, AdvisoryPackageKey, LocalPackageIndex};
pub(crate) use inventory::{ChatFindingCounts, ChatScan, ChatScanEvidence};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use time::{Duration, OffsetDateTime};

use crate::Error;

use super::companion::{
    CompanionActivity, CompanionActivityKind, CompanionCachedThreadSnapshot,
    CompanionConversationIndex, CompanionErrorInfo, CompanionEventEnvelope, CompanionEventKind,
    CompanionProviderSegment, CompanionProviderSegmentStatus, CompanionThread,
    CompanionThreadSnapshot, CompanionThreadStatus, CompanionTimelineItem,
    CompanionTimelineItemKind, CompanionTimelineItemStatus, PrivacyConsentStatus,
    COMPANION_DISCLOSURE_VERSION, COMPANION_SCHEMA_VERSION, REPOSITORY_CHAT_CONTEXT_VERSION,
};
use super::delta::{same_scan_package, scan_finding_changes, DetectedFindings};
use super::external_topics::{
    default_topic_keys, normalize_topic_key, product_key_to_topic, topic_definitions,
};
use super::github::{GithubRemoteFile, GithubRepositoryInfo};
use super::github_auth::GithubAuthSnapshot;
use super::models::{
    AdvisoryCwe, AdvisoryPackageImpact, AdvisoryReference, AppNotification, AppNotificationKind,
    AppNotificationTone, AppSnapshot, CisaKevDetails, DeltaSummary, ExternalIntelDetail,
    ExternalIntelFeed, ExternalIntelFeedItem, ExternalIntelKevDates, ExternalIntelMatchKind,
    ExternalIntelRelevance, ExternalIntelSeverity, ExternalIntelSourceDetail, ExternalIntelSummary,
    ExternalIntelTopic, ExternalSource, ExternalSourceCount, FindingRemediationProjection,
    FindingReview, GrypeDbUpdateSummary, LocalEntityKind, LocalEntityResult, NormalizedFinding,
    NormalizedRemediationEvidence, OnboardingSnapshot, OnboardingStatus, OverviewSnapshot,
    RemediationEvidenceQuality, RemediationEvidenceSource, RemediationLocationKind,
    RemediationRelationship, RemediationVerificationState, RepoAsset, RepoDetail, RepoFinding,
    RepoFindingIntel, RepoFindingIntelMatchKind, RepoFindingState, RepoPackage, RepoRecord,
    RepoScanRun, RepoSourceType, RepoStatus, RepoSummary, ReviewActionBasis, ReviewFinding,
    ReviewGroupCounts, ReviewLaneCount, ReviewPage, ReviewQuery, ReviewRiskBasis, ReviewSort,
    ReviewState, ReviewSummary, RiskTrendPoint, ScanActivity, ScanActivityStage,
    ScanActivityStatus, ScanOutput, ScanRepositoryOutcome, ScanRunStatus, ScannerAvailability,
    ScannerVersions, ScheduleInterval, Severity, SourceSnapshot, TrayStatus, VersionInfo,
    WorkspaceStatus, REMEDIATION_SCHEMA_VERSION,
};
use super::provider_connections::{
    config_from_json, config_to_json, now_timestamp, ProviderConnection, ProviderConnectionDraft,
    ProviderConnectionId, ProviderDriverKind, DEFAULT_CODEX_CONNECTION_ID,
    DEFAULT_GROK_CONNECTION_ID, DEFAULT_OPENCODE_CONNECTION_ID, PROVIDER_CONNECTION_SCHEMA_VERSION,
};
use super::repo_discovery::DiscoveredRepo;
use super::scanner::package_identity;

const EXTERNAL_INTEL_TOPICS_SETTING: &str = "external_intel_topics";
const ONBOARDING_SETTING: &str = "onboarding";
const SCHEMA_VERSION: i64 = 11;
const INTERRUPTED_SCAN_ERROR: &str = "The application exited before this scan completed.";

#[derive(Debug, Clone)]
pub struct Database {
    path: PathBuf,
    review_counts: Arc<Mutex<Option<review::CachedReviewCounts>>>,
    intel_packages: Arc<Mutex<Option<CachedIntelPackages>>>,
    dependency_counts: Arc<Mutex<Option<inventory::CachedDependencyCounts>>>,
}

pub struct ScanCompletion<'a> {
    pub repo_id: i64,
    pub run_id: i64,
    pub findings: &'a [NormalizedFinding],
    pub packages: &'a [RepoPackage],
    pub remediation_evidence: &'a [NormalizedRemediationEvidence],
    pub source_snapshot: &'a SourceSnapshot,
    pub delta: &'a DeltaSummary,
}

#[derive(Debug)]
pub(crate) struct ScanReviewChanges {
    pub new_findings: u32,
    pub worsened_findings: u32,
    pub resolved_findings: u32,
    pub alerts: u32,
    pub review_warning: Option<String>,
}

struct RemediationVerification {
    state: RemediationVerificationState,
    run_id: Option<i64>,
    scanned_at: Option<String>,
    exact_match: bool,
}

#[derive(Debug, Clone)]
pub struct PersistedExternalIntelItem {
    pub canonical_key: String,
    pub primary_source: ExternalSource,
    pub source_item_id: String,
    pub title: String,
    pub summary: String,
    pub url: String,
    pub ecosystem: Option<String>,
    pub package_name: Option<String>,
    pub product_keys: Vec<String>,
    pub topic_keys: Vec<String>,
    pub severity: ExternalIntelSeverity,
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
    pub modified_at: Option<String>,
    pub withdrawn_at: Option<String>,
    pub cvss_score: Option<String>,
    pub cvss_vector: Option<String>,
    pub cvss_version: Option<String>,
    pub epss_percentage: Option<String>,
    pub epss_percentile: Option<String>,
    pub is_kev: bool,
    pub is_ransomware_known: bool,
    pub fetched_at: String,
    pub raw_json: String,
    pub sources: Vec<ExternalSource>,
    pub source_details: Vec<ExternalIntelSourceDetail>,
    pub aliases: Vec<String>,
    pub affected_packages: Vec<AdvisoryPackageImpact>,
    pub references: Vec<AdvisoryReference>,
    pub cwes: Vec<AdvisoryCwe>,
    pub kev: Option<CisaKevDetails>,
    pub match_kind: Option<ExternalIntelMatchKind>,
    pub repo_count: u32,
    pub relevance: ExternalIntelRelevance,
}

const LATEST_COMPLETED_SCAN_FOR_REPO: &str = "SELECT id FROM scan_runs
    WHERE repo_id = r.id AND status = 'completed'
    ORDER BY started_at DESC, id DESC LIMIT 1";

impl Database {
    pub fn new(path: PathBuf) -> Result<Self, Error> {
        let database = Self {
            path,
            review_counts: Default::default(),
            intel_packages: Default::default(),
            dependency_counts: Default::default(),
        };
        database.init()?;
        Ok(database)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(any(debug_assertions, test))]
    pub fn seed_native_remediation_qa(&self, fixture_root: &Path) -> Result<(), Error> {
        const SEED_KEY: &str = "native_remediation_qa_seed";
        const SEED_VERSION: &str = "4";
        if self.get_setting(SEED_KEY)?.as_deref() == Some(SEED_VERSION) {
            return Ok(());
        }

        std::fs::create_dir_all(fixture_root)?;
        let local_slugs = [
            "exact",
            "no-fix",
            "resolved",
            "reappeared",
            "partial",
            "unavailable",
            "stale",
            "failed",
            "incomplete",
            "moved",
            "old-data",
        ];
        for slug in local_slugs {
            let root = fixture_root.join(slug);
            std::fs::create_dir_all(&root)?;
            if slug != "moved" {
                std::fs::write(root.join("package.json"), "{\"name\":\"cvec-native-qa\"}\n")?;
            }
            if slug == "partial" {
                std::fs::create_dir_all(root.join("apps/web"))?;
                std::fs::write(
                    root.join("apps/web/package-lock.json"),
                    "{\"lockfileVersion\":3}\n",
                )?;
            }
        }

        let discovery_root = fixture_root.join("discovery-root");
        std::fs::create_dir_all(&discovery_root)?;
        self.set_setting("discovery_root", discovery_root.display().to_string())?;

        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM repos
             WHERE source_key LIKE 'qa-remediation:%' OR path LIKE ?1",
            [format!(
                "{}{}%",
                fixture_root.display(),
                std::path::MAIN_SEPARATOR
            )],
        )?;
        let base = OffsetDateTime::now_utc() - Duration::hours(4);
        let mut next_offset = 0_i64;

        let exact = qa_insert_repo(
            &transaction,
            fixture_root,
            "exact",
            "QA Exact target",
            false,
            &base,
        )?;
        let exact_run = qa_insert_scan(
            &transaction,
            exact,
            "completed",
            true,
            "1111111111111111111111111111111111111111",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            exact,
            exact_run,
            NativeQaFinding::exact(
                "qa-exact",
                "CVE-2026-1000",
                "react-router",
                "7.6.1",
                Some("7.6.3"),
                "CRITICAL",
                Some(NativeQaEvidence::direct("package.json")),
            ),
        )?;

        let no_fix = qa_insert_repo(
            &transaction,
            fixture_root,
            "no-fix",
            "QA No target",
            false,
            &base,
        )?;
        let no_fix_run = qa_insert_scan(
            &transaction,
            no_fix,
            "completed",
            true,
            "2222222222222222222222222222222222222222",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            no_fix,
            no_fix_run,
            NativeQaFinding::exact(
                "qa-no-fix",
                "CVE-2026-1001",
                "minimist",
                "1.2.5",
                None,
                "HIGH",
                Some(NativeQaEvidence::unavailable()),
            ),
        )?;

        let resolved = qa_insert_repo(
            &transaction,
            fixture_root,
            "resolved",
            "QA Resolved",
            false,
            &base,
        )?;
        let resolved_evidence = qa_insert_scan(
            &transaction,
            resolved,
            "completed",
            true,
            "3333333333333333333333333333333333333333",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            resolved,
            resolved_evidence,
            NativeQaFinding::exact(
                "qa-resolved",
                "CVE-2026-1002",
                "semver",
                "7.5.0",
                Some("7.6.3"),
                "HIGH",
                Some(NativeQaEvidence::direct("package.json")),
            ),
        )?;
        qa_insert_scan(
            &transaction,
            resolved,
            "completed",
            true,
            "3333333333333333333333333333333333333333",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;

        let reappeared = qa_insert_repo(
            &transaction,
            fixture_root,
            "reappeared",
            "QA Reappeared",
            false,
            &base,
        )?;
        let first_reappeared = qa_insert_scan(
            &transaction,
            reappeared,
            "completed",
            true,
            "4444444444444444444444444444444444444444",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            reappeared,
            first_reappeared,
            NativeQaFinding::exact(
                "qa-reappeared",
                "CVE-2026-1003",
                "yaml",
                "2.4.0",
                Some("2.6.1"),
                "MEDIUM",
                Some(NativeQaEvidence::direct("package.json")),
            ),
        )?;
        qa_insert_scan(
            &transaction,
            reappeared,
            "completed",
            true,
            "4444444444444444444444444444444444444444",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        let latest_reappeared = qa_insert_scan(
            &transaction,
            reappeared,
            "completed",
            true,
            "4444444444444444444444444444444444444444",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            reappeared,
            latest_reappeared,
            NativeQaFinding::exact(
                "qa-reappeared",
                "CVE-2026-1003",
                "yaml",
                "2.4.0",
                Some("2.6.1"),
                "MEDIUM",
                Some(NativeQaEvidence::direct("package.json")),
            ),
        )?;

        let partial = qa_insert_repo(
            &transaction,
            fixture_root,
            "partial",
            "QA Partial transitive",
            false,
            &base,
        )?;
        let partial_run = qa_insert_scan(
            &transaction,
            partial,
            "completed",
            true,
            "5555555555555555555555555555555555555555",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            partial,
            partial_run,
            NativeQaFinding::exact(
                "qa-partial",
                "GHSA-abcd-1234-5678",
                "lodash",
                "4.17.20",
                Some("4.17.21"),
                "HIGH",
                Some(NativeQaEvidence::transitive(
                    "apps/web/package-lock.json",
                    3,
                )),
            ),
        )?;

        let unavailable = qa_insert_repo(
            &transaction,
            fixture_root,
            "unavailable",
            "QA Evidence unavailable",
            false,
            &base,
        )?;
        let unavailable_run = qa_insert_scan(
            &transaction,
            unavailable,
            "completed",
            true,
            "6666666666666666666666666666666666666666",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            unavailable,
            unavailable_run,
            NativeQaFinding::exact(
                "qa-unavailable",
                "CVE-2026-1004",
                "cookie",
                "0.6.0",
                Some("0.7.2"),
                "MEDIUM",
                Some(NativeQaEvidence::unavailable()),
            ),
        )?;

        let stale = qa_insert_repo(
            &transaction,
            fixture_root,
            "stale",
            "QA Stale resolved",
            false,
            &base,
        )?;
        let stale_evidence = qa_insert_scan(
            &transaction,
            stale,
            "completed",
            true,
            "7777777777777777777777777777777777777777",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            stale,
            stale_evidence,
            NativeQaFinding::exact(
                "qa-stale",
                "CVE-2026-1005",
                "vite",
                "6.0.0",
                Some("6.1.2"),
                "HIGH",
                Some(NativeQaEvidence::direct("package.json")),
            ),
        )?;
        qa_insert_scan(
            &transaction,
            stale,
            "completed",
            true,
            "8888888888888888888888888888888888888888",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;

        let failed = qa_insert_repo(
            &transaction,
            fixture_root,
            "failed",
            "QA Failed verification",
            false,
            &base,
        )?;
        let failed_evidence = qa_insert_scan(
            &transaction,
            failed,
            "completed",
            true,
            "9999999999999999999999999999999999999999",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            failed,
            failed_evidence,
            NativeQaFinding::exact(
                "qa-failed",
                "CVE-2026-1006",
                "axios",
                "1.6.0",
                Some("1.7.9"),
                "HIGH",
                Some(NativeQaEvidence::direct("package.json")),
            ),
        )?;
        qa_insert_scan(
            &transaction,
            failed,
            "failed",
            false,
            "9999999999999999999999999999999999999999",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;

        let incomplete = qa_insert_repo(
            &transaction,
            fixture_root,
            "incomplete",
            "QA Incomplete verification",
            false,
            &base,
        )?;
        let incomplete_evidence = qa_insert_scan(
            &transaction,
            incomplete,
            "completed",
            true,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            incomplete,
            incomplete_evidence,
            NativeQaFinding::exact(
                "qa-incomplete",
                "CVE-2026-1007",
                "router",
                "1.0.0",
                Some("1.0.1"),
                "HIGH",
                Some(NativeQaEvidence::direct("package.json")),
            ),
        )?;
        let incomplete_latest = qa_insert_scan(
            &transaction,
            incomplete,
            "completed",
            false,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            incomplete,
            incomplete_latest,
            NativeQaFinding::exact(
                "qa-incomplete",
                "CVE-2026-1007",
                "router",
                "1.0.0",
                Some("1.0.1"),
                "HIGH",
                None,
            ),
        )?;

        let moved = qa_insert_repo(
            &transaction,
            fixture_root,
            "moved",
            "QA Manifest moved",
            false,
            &base,
        )?;
        let moved_run = qa_insert_scan(
            &transaction,
            moved,
            "completed",
            true,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            moved,
            moved_run,
            NativeQaFinding::exact(
                "qa-moved",
                "CVE-2026-1008",
                "express",
                "4.18.0",
                Some("4.21.0"),
                "HIGH",
                Some(NativeQaEvidence::direct("package.json")),
            ),
        )?;

        let old_data = qa_insert_repo(
            &transaction,
            fixture_root,
            "old-data",
            "QA Old data",
            false,
            &base,
        )?;
        let old_data_run = qa_insert_scan(
            &transaction,
            old_data,
            "completed",
            true,
            "cccccccccccccccccccccccccccccccccccccccc",
            base + Duration::minutes(next_offset),
        )?;
        next_offset += 1;
        qa_insert_finding(
            &transaction,
            old_data,
            old_data_run,
            NativeQaFinding::exact(
                "qa-old-data",
                "CVE-2026-1009",
                "legacy-package",
                "0.9.0",
                Some("1.0.0"),
                "MEDIUM",
                None,
            ),
        )?;

        let remote = qa_insert_repo(
            &transaction,
            fixture_root,
            "remote",
            "QA Linked GitHub",
            true,
            &base,
        )?;
        let remote_run = qa_insert_scan(
            &transaction,
            remote,
            "completed",
            true,
            "dddddddddddddddddddddddddddddddddddddddd",
            base + Duration::minutes(next_offset),
        )?;
        qa_insert_finding(
            &transaction,
            remote,
            remote_run,
            NativeQaFinding::exact(
                "qa-remote",
                "GHSA-remote-1234-5678",
                "remote-package",
                "3.0.0",
                Some("3.0.1"),
                "HIGH",
                Some(NativeQaEvidence::direct("package.json")),
            ),
        )?;

        transaction.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![SEED_KEY, SEED_VERSION],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, Error> {
        Self::setting_on(&self.connect()?, key)
    }

    fn setting_on(connection: &Connection, key: &str) -> Result<Option<String>, Error> {
        Ok(connection
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: String) -> Result<(), Error> {
        let connection = self.connect()?;
        connection.execute(
            r#"
            INSERT INTO app_settings (key, value)
            VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
            params![key, value],
        )?;
        Ok(())
    }

    pub(crate) fn save_schedule(
        &self,
        interval: ScheduleInterval,
        next: Option<&str>,
    ) -> Result<(), Error> {
        self.connect()?.execute(
            "INSERT INTO app_settings (key, value)
             VALUES ('schedule_interval', ?1), ('next_scheduled_scan_at', ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![interval.as_str(), next.unwrap_or_default()],
        )?;
        Ok(())
    }

    pub fn list_ai_connections(&self) -> Result<Vec<ProviderConnection>, Error> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            r#"
            SELECT id, driver, display_name, accent_color, enabled, built_in,
                   config_version, config_json, has_server_password, created_at, updated_at
            FROM ai_connections
            ORDER BY built_in DESC, lower(display_name), id
            "#,
        )?;
        let values = statement
            .query_map([], provider_connection_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        values.into_iter().map(validate_stored_connection).collect()
    }

    pub fn ai_connection(&self, id: &str) -> Result<ProviderConnection, Error> {
        let connection = self.connect()?;
        let value = connection
            .query_row(
                r#"
                SELECT id, driver, display_name, accent_color, enabled, built_in,
                       config_version, config_json, has_server_password, created_at, updated_at
                FROM ai_connections WHERE id = ?1
                "#,
                [id],
                provider_connection_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::App("AI connection not found".into()))?;
        validate_stored_connection(value)
    }

    pub fn save_ai_connection(
        &self,
        draft: &ProviderConnectionDraft,
    ) -> Result<ProviderConnection, Error> {
        let id = ProviderConnectionId::parse(draft.id.trim().to_string())?;
        if draft.driver != draft.config.driver() {
            return Err(Error::App("connection driver/config mismatch".into()));
        }
        draft.config.validate()?;
        let display_name = draft
            .display_name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let timestamp = now_timestamp();
        let current = self.ai_connection(id.as_ref()).ok();
        let built_in = current.as_ref().is_some_and(|value| value.built_in);
        if built_in
            && current
                .as_ref()
                .is_some_and(|value| value.driver != draft.driver)
        {
            return Err(Error::App(
                "a built-in AI connection cannot change driver".into(),
            ));
        }
        let candidate = ProviderConnection {
            schema_version: PROVIDER_CONNECTION_SCHEMA_VERSION,
            id: id.clone(),
            driver: draft.driver,
            display_name: display_name.clone(),
            accent_color: draft.accent_color.clone(),
            enabled: draft.enabled,
            built_in,
            config: draft.config.clone(),
            has_server_password: draft
                .server_password
                .as_deref()
                .is_some_and(|value| !value.is_empty())
                || current
                    .as_ref()
                    .is_some_and(|value| value.has_server_password),
            created_at: current
                .as_ref()
                .map(|value| value.created_at.clone())
                .unwrap_or_else(|| timestamp.clone()),
            updated_at: timestamp.clone(),
        };
        candidate.validate()?;
        let connection = self.connect()?;
        let duplicate: Option<String> = connection
            .query_row(
                "SELECT id FROM ai_connections WHERE lower(display_name) = lower(?1) AND id <> ?2",
                params![display_name, id.as_ref()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(duplicate) = duplicate {
            return Err(Error::App(format!(
                "connection name conflicts with '{duplicate}'"
            )));
        }
        connection.execute(
            r#"
            INSERT INTO ai_connections (
              id, driver, display_name, accent_color, enabled, built_in,
              config_version, config_json, has_server_password, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO UPDATE SET
              driver = excluded.driver,
              display_name = excluded.display_name,
              accent_color = excluded.accent_color,
              enabled = excluded.enabled,
              config_version = excluded.config_version,
              config_json = excluded.config_json,
              has_server_password = excluded.has_server_password,
              updated_at = excluded.updated_at
            "#,
            params![
                candidate.id.as_ref(),
                candidate.driver.as_str(),
                candidate.display_name,
                candidate.accent_color,
                candidate.enabled,
                candidate.built_in,
                PROVIDER_CONNECTION_SCHEMA_VERSION,
                config_to_json(&candidate.config)?,
                candidate.has_server_password,
                candidate.created_at,
                candidate.updated_at,
            ],
        )?;
        self.ai_connection(id.as_ref())
    }

    pub fn set_ai_connection_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> Result<ProviderConnection, Error> {
        let connection = self.connect()?;
        let changed = connection.execute(
            "UPDATE ai_connections SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, enabled, now_timestamp()],
        )?;
        if changed == 0 {
            return Err(Error::App("AI connection not found".into()));
        }
        self.ai_connection(id)
    }

    pub fn set_ai_connection_secret_presence(
        &self,
        id: &str,
        present: bool,
    ) -> Result<ProviderConnection, Error> {
        let connection = self.connect()?;
        let changed = connection.execute(
            "UPDATE ai_connections SET has_server_password = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, present, now_timestamp()],
        )?;
        if changed == 0 {
            return Err(Error::App("AI connection not found".into()));
        }
        self.ai_connection(id)
    }

    pub fn reset_ai_connection(&self, id: &str) -> Result<ProviderConnection, Error> {
        let current = self.ai_connection(id)?;
        if !current.built_in {
            return Err(Error::App(
                "only built-in AI connections can be reset".into(),
            ));
        }
        let timestamp = now_timestamp();
        let default = match id {
            DEFAULT_CODEX_CONNECTION_ID => ProviderConnection::default_codex(timestamp.clone()),
            DEFAULT_GROK_CONNECTION_ID => ProviderConnection::default_grok(timestamp.clone()),
            DEFAULT_OPENCODE_CONNECTION_ID => {
                ProviderConnection::default_opencode(timestamp.clone())
            }
            _ => return Err(Error::App("unknown built-in AI connection".into())),
        };
        let connection = self.connect()?;
        let display_name = available_builtin_display(&connection, id, &default.display_name)?;
        connection.execute(
            r#"
            UPDATE ai_connections
            SET driver = ?2, display_name = ?3, accent_color = NULL,
                enabled = 1, config_version = ?4, config_json = ?5,
                has_server_password = 0, updated_at = ?6
            WHERE id = ?1 AND built_in = 1
            "#,
            params![
                id,
                default.driver.as_str(),
                display_name,
                PROVIDER_CONNECTION_SCHEMA_VERSION,
                config_to_json(&default.config)?,
                timestamp,
            ],
        )?;
        self.ai_connection(id)
    }

    pub fn remove_ai_connection(&self, id: &str) -> Result<u32, Error> {
        let connection = self.connect()?;
        let built_in: Option<bool> = connection
            .query_row(
                "SELECT built_in FROM ai_connections WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        match built_in {
            None => return Err(Error::App("AI connection not found".into())),
            Some(true) => {
                return Err(Error::App(
                    "built-in AI connections can be disabled or reset, not removed".into(),
                ))
            }
            Some(false) => {}
        }
        let bound_threads: u32 = connection.query_row(
            "SELECT count(*) FROM companion_threads WHERE provider_connection_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        connection.execute("DELETE FROM ai_connections WHERE id = ?1", [id])?;
        connection.execute(
            "DELETE FROM ai_connection_consents WHERE connection_id = ?1",
            [id],
        )?;
        Ok(bound_threads)
    }

    pub fn ai_connection_bound_thread_count(&self, id: &str) -> Result<u32, Error> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT count(*) FROM companion_threads WHERE provider_connection_id = ?1",
                [id],
                |row| row.get(0),
            )
            .map_err(Error::from)
    }

    pub fn connection_privacy_consent(
        &self,
        connection_id: &str,
        fingerprint: &str,
        disclosure_version: u32,
    ) -> Result<PrivacyConsentStatus, Error> {
        let connection = self.connect()?;
        let row = connection
            .query_row(
                r#"
                SELECT disclosure_version, accepted_at, revoked_at
                FROM ai_connection_consents
                WHERE connection_id = ?1 AND boundary_fingerprint = ?2
                "#,
                params![connection_id, fingerprint],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let (stored_version, accepted_at, revoked_at) =
            row.unwrap_or((disclosure_version, None, None));
        Ok(PrivacyConsentStatus {
            provider: fingerprint.into(),
            disclosure_version: stored_version,
            valid: stored_version == disclosure_version
                && accepted_at.is_some()
                && revoked_at.is_none(),
            accepted_at,
            revoked_at,
        })
    }

    pub fn set_connection_privacy_consent(
        &self,
        connection_id: &str,
        fingerprint: &str,
        disclosure_version: u32,
        accepted: bool,
    ) -> Result<PrivacyConsentStatus, Error> {
        let connection = self.connect()?;
        let timestamp = now_timestamp();
        let (accepted_at, revoked_at) = if accepted {
            (Some(timestamp.as_str()), None)
        } else {
            (None, Some(timestamp.as_str()))
        };
        connection.execute(
            r#"
            INSERT INTO ai_connection_consents (
              connection_id, boundary_fingerprint, disclosure_version, accepted_at, revoked_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(connection_id, boundary_fingerprint) DO UPDATE SET
              disclosure_version = excluded.disclosure_version,
              accepted_at = excluded.accepted_at,
              revoked_at = excluded.revoked_at
            "#,
            params![
                connection_id,
                fingerprint,
                disclosure_version,
                accepted_at,
                revoked_at
            ],
        )?;
        self.connection_privacy_consent(connection_id, fingerprint, disclosure_version)
    }

    pub fn list_companion_conversation_index(&self) -> Result<CompanionConversationIndex, Error> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            r#"
            SELECT id, repository_id, provider_driver, provider_thread_id, title,
                   created_at, updated_at, last_opened_at, archived_at,
                   context_version, last_model_id, provider_connection_id
            FROM companion_threads
            ORDER BY (archived_at IS NOT NULL), repository_id, last_opened_at DESC, id DESC
            LIMIT 5000
            "#,
        )?;
        let threads = statement
            .query_map([], companion_thread_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CompanionConversationIndex {
            schema_version: COMPANION_SCHEMA_VERSION,
            threads,
        })
    }

    pub fn list_companion_threads(
        &self,
        repository_id: i64,
        include_archived: bool,
    ) -> Result<Vec<CompanionThread>, Error> {
        let connection = self.connect()?;
        ensure_repository_exists(&connection, repository_id)?;
        let mut statement = connection.prepare(
            r#"
            SELECT id, repository_id, provider_driver, provider_thread_id, title,
                   created_at, updated_at, last_opened_at, archived_at,
                   context_version, last_model_id, provider_connection_id
            FROM companion_threads
            WHERE repository_id = ?1
              AND (?2 = 1 OR archived_at IS NULL)
            ORDER BY (archived_at IS NOT NULL), last_opened_at DESC, id DESC
            LIMIT 500
            "#,
        )?;
        let rows = statement
            .query_map(
                params![repository_id, include_archived],
                companion_thread_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn companion_provider_segments(
        &self,
        thread_id: i64,
    ) -> Result<Vec<CompanionProviderSegment>, Error> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            r#"
            SELECT id, thread_id, ordinal, provider, connection_id, provider_thread_id,
                   initial_model_id, last_model_id, status, created_at, updated_at, ended_at
            FROM companion_provider_segments
            WHERE thread_id = ?1
            ORDER BY ordinal, id
            "#,
        )?;
        let segments = statement
            .query_map([thread_id], companion_provider_segment_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(segments)
    }

    pub fn current_companion_provider_segment(
        &self,
        thread_id: i64,
    ) -> Result<CompanionProviderSegment, Error> {
        let connection = self.connect()?;
        connection
            .query_row(
                r#"
                SELECT id, thread_id, ordinal, provider, connection_id, provider_thread_id,
                       initial_model_id, last_model_id, status, created_at, updated_at, ended_at
                FROM companion_provider_segments
                WHERE thread_id = ?1
                ORDER BY ordinal DESC, id DESC
                LIMIT 1
                "#,
                [thread_id],
                companion_provider_segment_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::App("companion provider segment not found".into()))
    }

    pub fn create_companion_provider_segment(
        &self,
        thread_id: i64,
        connection_id: &str,
        provider: &str,
        provider_thread_id: &str,
        model_id: Option<&str>,
    ) -> Result<CompanionProviderSegment, Error> {
        validate_companion_provider_identity(provider, provider_thread_id, model_id)?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let configured_driver: Option<String> = transaction
            .query_row(
                "SELECT driver FROM ai_connections WHERE id = ?1 AND enabled = 1",
                [connection_id],
                |row| row.get(0),
            )
            .optional()?;
        if configured_driver.as_deref() != Some(provider) {
            return Err(Error::App(
                "AI connection is unavailable or has a different driver".into(),
            ));
        }
        let ordinal: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM companion_provider_segments WHERE thread_id = ?1",
            [thread_id],
            |row| row.get(0),
        )?;
        let timestamp = now();
        transaction.execute(
            "UPDATE companion_provider_segments SET ended_at = COALESCE(ended_at, ?2), updated_at = ?2 WHERE thread_id = ?1 AND ended_at IS NULL",
            params![thread_id, timestamp],
        )?;
        transaction.execute(
            r#"
            INSERT INTO companion_provider_segments (
              thread_id, ordinal, provider, connection_id, provider_thread_id,
              initial_model_id, last_model_id, status, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 'ready', ?7, ?7)
            "#,
            params![
                thread_id,
                ordinal,
                provider,
                connection_id,
                provider_thread_id,
                model_id,
                timestamp,
            ],
        )?;
        let segment_id = transaction.last_insert_rowid();
        let changed = transaction.execute(
            r#"
            UPDATE companion_threads
            SET provider = ?2, provider_driver = ?2, provider_thread_id = ?3,
                provider_connection_id = ?4, provider_turn_started = 0,
                last_model_id = ?5, updated_at = ?6, last_opened_at = ?6
            WHERE id = ?1
            "#,
            params![
                thread_id,
                provider,
                provider_thread_id,
                connection_id,
                model_id,
                timestamp,
            ],
        )?;
        if changed == 0 {
            return Err(Error::App("companion thread not found".into()));
        }
        transaction.commit()?;
        self.companion_provider_segment(segment_id)
    }

    pub fn rollback_companion_provider_segment(
        &self,
        thread_id: i64,
        segment_id: i64,
        previous: &CompanionProviderSegment,
        previous_turn_started: bool,
    ) -> Result<CompanionThread, Error> {
        if previous.thread_id != thread_id || previous.id == segment_id {
            return Err(Error::App(
                "invalid companion provider segment rollback".into(),
            ));
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let current_id = transaction
            .query_row(
                "SELECT id FROM companion_provider_segments WHERE thread_id = ?1 ORDER BY ordinal DESC, id DESC LIMIT 1",
                [thread_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if current_id != Some(segment_id) {
            return Err(Error::App(
                "only the latest companion provider segment can be rolled back".into(),
            ));
        }
        transaction.execute(
            "DELETE FROM companion_provider_segments WHERE id = ?1 AND thread_id = ?2",
            params![segment_id, thread_id],
        )?;
        let timestamp = now();
        transaction.execute(
            "UPDATE companion_provider_segments SET ended_at = NULL, updated_at = ?2 WHERE id = ?1",
            params![previous.id, timestamp],
        )?;
        transaction.execute(
            r#"
            UPDATE companion_threads
            SET provider = ?2, provider_driver = ?2, provider_thread_id = ?3,
                provider_connection_id = ?4, provider_turn_started = ?5,
                last_model_id = ?6, updated_at = ?7, last_opened_at = ?7
            WHERE id = ?1
            "#,
            params![
                thread_id,
                previous.provider,
                previous.provider_thread_id,
                previous.connection_id,
                previous_turn_started,
                previous.last_model_id,
                timestamp,
            ],
        )?;
        transaction.commit()?;
        self.companion_thread(thread_id)
    }

    pub fn delete_companion_timeline_item(
        &self,
        thread_id: i64,
        segment_id: i64,
        item_key: &str,
    ) -> Result<(), Error> {
        self.connect()?.execute(
            "DELETE FROM companion_timeline_items WHERE thread_id = ?1 AND provider_segment_id = ?2 AND item_key = ?3",
            params![thread_id, segment_id, item_key],
        )?;
        Ok(())
    }

    pub fn cache_companion_thread_snapshot(
        &self,
        thread_id: i64,
        snapshot: &CompanionThreadSnapshot,
    ) -> Result<(), Error> {
        if snapshot.thread.id != thread_id {
            return Err(Error::App(
                "companion snapshot belongs to a different thread".into(),
            ));
        }
        let mut connection = self.connect()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let segment = self
            .companion_provider_segment_for_event_on(
                &tx,
                thread_id,
                Some(&snapshot.thread.provider_connection_id),
                Some(&snapshot.thread.provider_thread_id),
            )?
            .ok_or_else(|| {
                Error::App("Provider snapshot no longer matches its conversation segment".into())
            })?;
        let mut streaming = false;
        for item in &snapshot.items {
            let previous =
                self.cached_companion_timeline_item_on(&tx, thread_id, segment.id, &item.id)?;
            let current = previous.as_ref().filter(|previous| {
                previous.status == CompanionTimelineItemStatus::Streaming
                    && previous.text.len() > item.text.len()
                    && previous.text.starts_with(&item.text)
            });
            let item = current.unwrap_or(item);
            streaming |= item.status == CompanionTimelineItemStatus::Streaming;
            self.upsert_companion_timeline_item_on(&tx, thread_id, segment.id, item)?;
        }
        self.set_companion_provider_segment_status_on(
            &tx,
            segment.id,
            if streaming {
                CompanionProviderSegmentStatus::Active
            } else {
                CompanionProviderSegmentStatus::Ready
            },
            snapshot.thread.last_model_id.as_deref(),
        )?;
        tx.commit()?;
        Ok(())
    }

    fn companion_provider_segment_for_event_on(
        &self,
        connection: &Connection,
        thread_id: i64,
        connection_id: Option<&str>,
        provider_thread_id: Option<&str>,
    ) -> Result<Option<CompanionProviderSegment>, Error> {
        let mut statement = connection.prepare(
            r#"
            SELECT id, thread_id, ordinal, provider, connection_id, provider_thread_id,
                   initial_model_id, last_model_id, status, created_at, updated_at, ended_at
            FROM companion_provider_segments
            WHERE thread_id = ?1
              AND (?2 IS NULL OR connection_id = ?2)
              AND (?3 IS NULL OR provider_thread_id = ?3)
            ORDER BY ordinal DESC, id DESC
            LIMIT 1
            "#,
        )?;
        statement
            .query_row(
                params![thread_id, connection_id, provider_thread_id],
                companion_provider_segment_from_row,
            )
            .optional()
            .map_err(Error::from)
    }

    #[cfg(test)]
    fn cached_companion_timeline_item(
        &self,
        thread_id: i64,
        segment_id: i64,
        item_key: &str,
    ) -> Result<Option<CompanionTimelineItem>, Error> {
        self.cached_companion_timeline_item_on(&self.connect()?, thread_id, segment_id, item_key)
    }
    fn cached_companion_timeline_item_on(
        &self,
        connection: &Connection,
        thread_id: i64,
        segment_id: i64,
        item_key: &str,
    ) -> Result<Option<CompanionTimelineItem>, Error> {
        let value: Option<String> = connection
            .query_row(
                "SELECT item_json FROM companion_timeline_items WHERE thread_id = ?1 AND provider_segment_id = ?2 AND item_key = ?3",
                params![thread_id, segment_id, item_key],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Error::from))
            .transpose()
    }

    fn finish_streaming_companion_items_on(
        &self,
        connection: &Connection,
        thread_id: i64,
        segment_id: i64,
        interrupted: bool,
    ) -> Result<(), Error> {
        let mut statement = connection.prepare(
            "SELECT item_key, item_json FROM companion_timeline_items WHERE thread_id = ?1 AND provider_segment_id = ?2 AND json_extract(item_json,'$.status')='streaming'",
        )?;
        let items = statement
            .query_map(params![thread_id, segment_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for (item_key, value) in items {
            let mut item = serde_json::from_str::<CompanionTimelineItem>(&value)?;
            if item.status == CompanionTimelineItemStatus::Streaming {
                item.status = if interrupted {
                    CompanionTimelineItemStatus::Interrupted
                } else {
                    CompanionTimelineItemStatus::Completed
                };
                self.upsert_companion_timeline_item_on(
                    connection,
                    thread_id,
                    segment_id,
                    &CompanionTimelineItem {
                        id: item_key,
                        ..item
                    },
                )?;
            }
        }
        Ok(())
    }

    fn upsert_companion_timeline_item_on(
        &self,
        connection: &Connection,
        thread_id: i64,
        segment_id: i64,
        item: &CompanionTimelineItem,
    ) -> Result<bool, Error> {
        if matches!(
            item.activity.as_ref().map(|activity| &activity.kind),
            Some(CompanionActivityKind::Context)
        ) {
            return Ok(false);
        }
        if item.id.trim().is_empty() || item.id.len() > 512 {
            return Err(Error::App("invalid companion timeline item id".into()));
        }
        if item
            .model_id
            .as_deref()
            .is_some_and(|value| value.len() > 256)
        {
            return Err(Error::App(
                "invalid companion timeline item model id".into(),
            ));
        }
        let segment_owner: Option<i64> = connection
            .query_row(
                "SELECT thread_id FROM companion_provider_segments WHERE id = ?1",
                [segment_id],
                |row| row.get(0),
            )
            .optional()?;
        if segment_owner != Some(thread_id) {
            return Err(Error::App(
                "companion provider segment belongs to a different thread".into(),
            ));
        }
        let previous: Option<String> = connection.query_row(
            "SELECT item_json FROM companion_timeline_items WHERE thread_id = ?1 AND provider_segment_id = ?2 AND item_key = ?3",
            params![thread_id, segment_id, item.id],
            |row| row.get(0),
        ).optional()?;
        let mut normalized = item.clone();
        if let Some(previous) = &previous {
            let cached = serde_json::from_str(previous)?;
            normalized.preserve_cached_citations(&cached);
            normalized.preserve_cached_model_id(&cached);
        }
        let timestamp = now();
        if previous.is_none()
            && item.kind == CompanionTimelineItemKind::UserMessage
            && !item.id.starts_with("local:")
        {
            let optimistic_key = {
                let mut statement = connection.prepare(
                    "SELECT item_key, item_json FROM companion_timeline_items WHERE thread_id = ?1 AND provider_segment_id = ?2 AND item_key LIKE 'local:%' ORDER BY sequence LIMIT 32",
                )?;
                let candidates = statement.query_map(params![thread_id, segment_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                let mut matched = None;
                for candidate in candidates {
                    let (key, value) = candidate?;
                    let local: CompanionTimelineItem = serde_json::from_str(&value)?;
                    if local.kind == CompanionTimelineItemKind::UserMessage
                        && local.text == item.text
                    {
                        matched = Some((key, local));
                        break;
                    }
                }
                matched
            };
            if let Some((key, local)) = optimistic_key {
                // Reconcile one local echo by changing its identity in place.
                // Equal text on two provider messages denotes two turns.
                normalized.preserve_cached_model_id(&local);
                let item_json = serde_json::to_string(&normalized)?;
                if item_json.len() > 2_097_152 {
                    return Err(Error::App("companion timeline item is too large".into()));
                }
                connection.execute(
                    "UPDATE companion_timeline_items SET item_key = ?4, item_json = ?5, updated_at = ?6 WHERE thread_id = ?1 AND provider_segment_id = ?2 AND item_key = ?3",
                    params![thread_id, segment_id, key, item.id, item_json, timestamp],
                )?;
                return Ok(true);
            }
        }
        let item_json = serde_json::to_string(&normalized)?;
        if item_json.len() > 2_097_152 {
            return Err(Error::App("companion timeline item is too large".into()));
        }
        connection.execute(
            r#"
            INSERT INTO companion_timeline_items (
              thread_id, provider_segment_id, item_key, sequence, item_json, created_at, updated_at
            ) VALUES (
              ?1, ?2, ?3,
              COALESCE((SELECT MAX(sequence) + 1 FROM companion_timeline_items WHERE thread_id = ?1), 1),
              ?4, ?5, ?5
            )
            ON CONFLICT(thread_id, provider_segment_id, item_key) DO UPDATE SET
              item_json = excluded.item_json,
              updated_at = excluded.updated_at
            "#,
            params![thread_id, segment_id, item.id, item_json, timestamp],
        )?;
        Ok(true)
    }
    fn set_companion_provider_segment_status_on(
        &self,
        connection: &Connection,
        segment_id: i64,
        status: CompanionProviderSegmentStatus,
        model_id: Option<&str>,
    ) -> Result<(), Error> {
        if model_id.is_some_and(|value| value.len() > 256) {
            return Err(Error::App("invalid companion model id".into()));
        }
        let timestamp = now();
        let changed = connection.execute(
            r#"
            UPDATE companion_provider_segments
            SET status = ?2, last_model_id = COALESCE(?3, last_model_id), updated_at = ?4
            WHERE id = ?1
            "#,
            params![segment_id, status.as_str(), model_id, timestamp],
        )?;
        if changed == 0 {
            return Err(Error::App("companion provider segment not found".into()));
        }
        Ok(())
    }
    pub fn set_companion_provider_segment_status(
        &self,
        segment_id: i64,
        status: CompanionProviderSegmentStatus,
        model_id: Option<&str>,
    ) -> Result<CompanionProviderSegment, Error> {
        self.set_companion_provider_segment_status_on(
            &self.connect()?,
            segment_id,
            status,
            model_id,
        )?;
        self.companion_provider_segment(segment_id)
    }

    fn claim_companion_cached_event_on(
        &self,
        connection: &Connection,
        segment_id: i64,
        event_key: &str,
    ) -> Result<bool, Error> {
        let inserted = connection.execute(
            "INSERT OR IGNORE INTO companion_cached_events (provider_segment_id, event_key, created_at) VALUES (?1, ?2, ?3)",
            params![segment_id, event_key, now()],
        )?;
        Ok(inserted > 0)
    }
    fn cache_companion_event_on(
        &self,
        connection: &Connection,
        event: &CompanionEventEnvelope,
    ) -> Result<bool, Error> {
        let Some(thread_id) = event.thread_id else {
            return Ok(true);
        };
        let Some(segment) = self.companion_provider_segment_for_event_on(
            connection,
            thread_id,
            event.connection_id.as_deref(),
            event.provider_thread_id.as_deref(),
        )?
        else {
            return Ok(true);
        };
        let event_key = companion_event_cache_key(event)?;
        if !self.claim_companion_cached_event_on(connection, segment.id, &event_key)? {
            return Ok(false);
        }
        let result = (|| -> Result<(), Error> {
            match &event.kind {
                CompanionEventKind::TurnStarted => {
                    self.set_companion_provider_segment_status_on(
                        connection,
                        segment.id,
                        CompanionProviderSegmentStatus::Active,
                        event.model_id.as_deref(),
                    )?;
                }
                CompanionEventKind::ItemStarted | CompanionEventKind::ItemCompleted => {
                    if let Ok(mut item) =
                        serde_json::from_value::<CompanionTimelineItem>(event.payload.clone())
                    {
                        if item.model_id.is_none() {
                            item.model_id.clone_from(&event.model_id);
                        }
                        self.upsert_companion_timeline_item_on(
                            connection, thread_id, segment.id, &item,
                        )?;
                    }
                }
                CompanionEventKind::ItemDelta => {
                    let item_key = event
                        .payload
                        .get("itemId")
                        .and_then(serde_json::Value::as_str)
                        .filter(|value| !value.is_empty() && value.len() <= 512)
                        .unwrap_or("assistant");
                    let delta = event
                        .payload
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let mut item = self
                        .cached_companion_timeline_item_on(
                            connection, thread_id, segment.id, item_key,
                        )?
                        .unwrap_or_else(|| CompanionTimelineItem {
                            id: item_key.into(),
                            kind: CompanionTimelineItemKind::AssistantMessage,
                            status: CompanionTimelineItemStatus::Streaming,
                            text: String::new(),
                            activity: None,
                            citations: Vec::new(),
                            error: None,
                            model_id: event.model_id.clone(),
                            created_at: now(),
                        });
                    if item.model_id.is_none() {
                        item.model_id.clone_from(&event.model_id);
                    }
                    if item.text.len().saturating_add(delta.len()) <= 2_000_000 {
                        item.text.push_str(delta);
                        item.status = CompanionTimelineItemStatus::Streaming;
                        self.upsert_companion_timeline_item_on(
                            connection, thread_id, segment.id, &item,
                        )?;
                    }
                }
                CompanionEventKind::Activity => {
                    if let Ok(activity) =
                        serde_json::from_value::<CompanionActivity>(event.payload.clone())
                    {
                        let item = CompanionTimelineItem {
                            id: super::companion::generated_event_item_id(event, "activity"),
                            kind: CompanionTimelineItemKind::Activity,
                            status: CompanionTimelineItemStatus::Completed,
                            text: String::new(),
                            activity: Some(activity),
                            citations: Vec::new(),
                            error: None,
                            model_id: None,
                            created_at: now(),
                        };
                        self.upsert_companion_timeline_item_on(
                            connection, thread_id, segment.id, &item,
                        )?;
                    }
                }
                CompanionEventKind::Error | CompanionEventKind::Warning => {
                    if let Ok(error) =
                        serde_json::from_value::<CompanionErrorInfo>(event.payload.clone())
                    {
                        let warning = event.kind == CompanionEventKind::Warning
                            || matches!(error.code.as_str(), "codex_warning" | "action_denied");
                        let item = CompanionTimelineItem {
                            id: if warning {
                                super::companion::generated_event_item_id(event, "warning")
                            } else {
                                super::companion::generated_event_item_id(event, "error")
                            },
                            kind: if warning {
                                CompanionTimelineItemKind::Warning
                            } else {
                                CompanionTimelineItemKind::Error
                            },
                            status: if warning {
                                CompanionTimelineItemStatus::Completed
                            } else {
                                CompanionTimelineItemStatus::Failed
                            },
                            text: String::new(),
                            activity: None,
                            citations: Vec::new(),
                            error: Some(error),
                            model_id: None,
                            created_at: now(),
                        };
                        self.upsert_companion_timeline_item_on(
                            connection, thread_id, segment.id, &item,
                        )?;
                        if warning {
                            return Ok(());
                        }
                        self.finish_streaming_companion_items_on(
                            connection, thread_id, segment.id, true,
                        )?;
                        self.set_companion_provider_segment_status_on(
                            connection,
                            segment.id,
                            CompanionProviderSegmentStatus::Interrupted,
                            event.model_id.as_deref(),
                        )?;
                    }
                }
                CompanionEventKind::TurnCompleted => {
                    let interrupted = event
                        .payload
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        == Some("interrupted");
                    self.finish_streaming_companion_items_on(
                        connection,
                        thread_id,
                        segment.id,
                        interrupted,
                    )?;
                    self.set_companion_provider_segment_status_on(
                        connection,
                        segment.id,
                        if interrupted {
                            CompanionProviderSegmentStatus::Interrupted
                        } else {
                            CompanionProviderSegmentStatus::Ready
                        },
                        event.model_id.as_deref(),
                    )?;
                }
                _ => {}
            }
            Ok(())
        })();
        result.map(|()| true)
    }
    pub fn upsert_companion_timeline_item(
        &self,
        thread_id: i64,
        segment_id: i64,
        item: &CompanionTimelineItem,
    ) -> Result<bool, Error> {
        let mut connection = self.connect()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = self.upsert_companion_timeline_item_on(&tx, thread_id, segment_id, item)?;
        tx.commit()?;
        Ok(changed)
    }
    pub fn cache_companion_event(&self, event: &CompanionEventEnvelope) -> Result<(), Error> {
        self.cache_companion_events(std::slice::from_ref(event))
            .map(|_| ())
    }

    pub fn cached_companion_thread_snapshot(
        &self,
        thread_id: i64,
    ) -> Result<CompanionCachedThreadSnapshot, Error> {
        let thread = self.companion_thread(thread_id)?;
        let segments = self.companion_provider_segments(thread_id)?;
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT item_json FROM companion_timeline_items WHERE thread_id = ?1 ORDER BY sequence, id",
        )?;
        let mut items = statement
            .query_map([thread_id], |row| row.get::<_, String>(0))?
            .map(|value| {
                let value = value?;
                serde_json::from_str::<CompanionTimelineItem>(&value).map_err(|cause| {
                    rusqlite::Error::FromSqlConversionFailure(
                        value.len(),
                        rusqlite::types::Type::Text,
                        Box::new(cause),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for item in &mut items {
            if item.kind == CompanionTimelineItemKind::Error
                && item.error.as_ref().is_some_and(|error| {
                    matches!(error.code.as_str(), "codex_warning" | "action_denied")
                })
            {
                item.kind = CompanionTimelineItemKind::Warning;
                item.status = CompanionTimelineItemStatus::Completed;
            }
            if item.status == CompanionTimelineItemStatus::Streaming {
                item.status = CompanionTimelineItemStatus::Interrupted;
            }
        }
        let provider_history_available = segments.last().is_none_or(|segment| {
            segment.status != CompanionProviderSegmentStatus::HistoryUnavailable
        });
        Ok(CompanionCachedThreadSnapshot {
            older_cursor: None,
            event_watermarks: Default::default(),
            streaming_item_ids: Vec::new(),
            provider_ordinals: Default::default(),
            schema_version: COMPANION_SCHEMA_VERSION,
            thread,
            segments,
            items,
            provider_history_available,
        })
    }

    fn companion_provider_segment(
        &self,
        segment_id: i64,
    ) -> Result<CompanionProviderSegment, Error> {
        let connection = self.connect()?;
        connection
            .query_row(
                r#"
                SELECT id, thread_id, ordinal, provider, connection_id, provider_thread_id,
                       initial_model_id, last_model_id, status, created_at, updated_at, ended_at
                FROM companion_provider_segments WHERE id = ?1
                "#,
                [segment_id],
                companion_provider_segment_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::App("companion provider segment not found".into()))
    }

    pub fn companion_thread(&self, thread_id: i64) -> Result<CompanionThread, Error> {
        let connection = self.connect()?;
        connection
            .query_row(
                r#"
                SELECT id, repository_id, provider_driver, provider_thread_id, title,
                       created_at, updated_at, last_opened_at, archived_at,
                       context_version, last_model_id, provider_connection_id
                FROM companion_threads WHERE id = ?1
                "#,
                [thread_id],
                companion_thread_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::App("companion thread not found".into()))
    }

    pub fn companion_thread_by_provider_id(
        &self,
        provider_thread_id: &str,
    ) -> Result<Option<CompanionThread>, Error> {
        let connection = self.connect()?;
        connection
            .query_row(
                r#"
                SELECT id, repository_id, provider_driver, provider_thread_id, title,
                       created_at, updated_at, last_opened_at, archived_at,
                       context_version, last_model_id, provider_connection_id
                FROM companion_threads
                WHERE provider_thread_id = ?1
                "#,
                [provider_thread_id],
                companion_thread_from_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn create_companion_thread(
        &self,
        repository_id: i64,
        provider_thread_id: &str,
        title: &str,
        last_model_id: Option<&str>,
    ) -> Result<CompanionThread, Error> {
        self.create_companion_thread_for_connection(
            repository_id,
            DEFAULT_CODEX_CONNECTION_ID,
            "codex",
            provider_thread_id,
            title,
            last_model_id,
        )
    }

    pub fn create_companion_thread_for_connection(
        &self,
        repository_id: i64,
        connection_id: &str,
        provider: &str,
        provider_thread_id: &str,
        title: &str,
        last_model_id: Option<&str>,
    ) -> Result<CompanionThread, Error> {
        validate_companion_provider_identity(provider, provider_thread_id, last_model_id)?;
        let mut connection = self.connect()?;
        ensure_repository_exists(&connection, repository_id)?;
        let configured_driver: Option<String> = connection
            .query_row(
                "SELECT driver FROM ai_connections WHERE id = ?1 AND enabled = 1",
                [connection_id],
                |row| row.get(0),
            )
            .optional()?;
        if configured_driver.as_deref() != Some(provider) {
            return Err(Error::App(
                "AI connection is unavailable or has a different driver".into(),
            ));
        }
        let timestamp = now();
        let title = normalize_companion_title(title);
        let transaction = connection.transaction()?;
        transaction.execute(
            r#"
            INSERT INTO companion_threads (
              repository_id, provider, provider_driver, provider_thread_id, title, created_at,
              updated_at, last_opened_at, context_version, last_model_id,
              provider_turn_started, provider_connection_id
            ) VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?5, ?5, ?6, ?7, 0, ?8)
            "#,
            params![
                repository_id,
                provider,
                provider_thread_id,
                title,
                timestamp,
                REPOSITORY_CHAT_CONTEXT_VERSION,
                last_model_id,
                connection_id,
            ],
        )?;
        let thread_id = transaction.last_insert_rowid();
        transaction.execute(
            r#"
            INSERT INTO companion_provider_segments (
              thread_id, ordinal, provider, connection_id, provider_thread_id,
              initial_model_id, last_model_id, status, created_at, updated_at
            ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?5, 'ready', ?6, ?6)
            "#,
            params![
                thread_id,
                provider,
                connection_id,
                provider_thread_id,
                last_model_id,
                timestamp,
            ],
        )?;
        transaction.commit()?;
        self.companion_thread(thread_id)
    }

    pub fn rename_companion_thread(
        &self,
        thread_id: i64,
        title: &str,
    ) -> Result<CompanionThread, Error> {
        let connection = self.connect()?;
        let timestamp = now();
        let title = normalize_companion_title(title);
        let changed = connection.execute(
            "UPDATE companion_threads SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![thread_id, title, timestamp],
        )?;
        if changed == 0 {
            return Err(Error::App("companion thread not found".into()));
        }
        self.companion_thread(thread_id)
    }

    pub fn touch_companion_thread(
        &self,
        thread_id: i64,
        last_model_id: Option<&str>,
    ) -> Result<CompanionThread, Error> {
        if last_model_id.is_some_and(|value| value.len() > 256) {
            return Err(Error::App("invalid companion model id".into()));
        }
        let connection = self.connect()?;
        let timestamp = now();
        let changed = connection.execute(
            r#"
            UPDATE companion_threads
            SET updated_at = ?2, last_opened_at = ?2,
                last_model_id = COALESCE(?3, last_model_id)
            WHERE id = ?1
            "#,
            params![thread_id, timestamp, last_model_id],
        )?;
        if changed == 0 {
            return Err(Error::App("companion thread not found".into()));
        }
        self.companion_thread(thread_id)
    }

    pub fn companion_thread_has_provider_turn(&self, thread_id: i64) -> Result<bool, Error> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT provider_turn_started FROM companion_threads WHERE id = ?1",
                [thread_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .ok_or_else(|| Error::App("companion thread not found".into()))
    }

    pub fn rebind_empty_companion_thread(
        &self,
        thread_id: i64,
        provider_thread_id: &str,
    ) -> Result<CompanionThread, Error> {
        if provider_thread_id.trim().is_empty() || provider_thread_id.len() > 512 {
            return Err(Error::App("invalid provider thread id".into()));
        }
        let mut connection = self.connect()?;
        let timestamp = now();
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            r#"
            UPDATE companion_threads
            SET provider_thread_id = ?2, provider_turn_started = 0,
                updated_at = ?3, last_opened_at = ?3
            WHERE id = ?1 AND provider_turn_started = 0
            "#,
            params![thread_id, provider_thread_id, timestamp],
        )?;
        if changed == 0 {
            return Err(Error::App(
                "only an empty companion thread can be refreshed".into(),
            ));
        }
        transaction.execute(
            r#"
            UPDATE companion_provider_segments
            SET provider_thread_id = ?2, updated_at = ?3
            WHERE thread_id = ?1 AND ordinal = (
              SELECT MAX(ordinal) FROM companion_provider_segments WHERE thread_id = ?1
            )
            "#,
            params![thread_id, provider_thread_id, timestamp],
        )?;
        transaction.commit()?;
        self.companion_thread(thread_id)
    }

    pub fn mark_companion_thread_turn_started(
        &self,
        thread_id: i64,
        last_model_id: Option<&str>,
    ) -> Result<CompanionThread, Error> {
        if last_model_id.is_some_and(|value| value.len() > 256) {
            return Err(Error::App("invalid companion model id".into()));
        }
        let mut connection = self.connect()?;
        let timestamp = now();
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            r#"
            UPDATE companion_threads
            SET provider_turn_started = 1, updated_at = ?2, last_opened_at = ?2,
                last_model_id = COALESCE(?3, last_model_id)
            WHERE id = ?1
            "#,
            params![thread_id, timestamp, last_model_id],
        )?;
        if changed == 0 {
            return Err(Error::App("companion thread not found".into()));
        }
        transaction.execute(
            r#"
            UPDATE companion_provider_segments
            SET status = 'active', last_model_id = COALESCE(?2, last_model_id), updated_at = ?3
            WHERE thread_id = ?1 AND ordinal = (
              SELECT MAX(ordinal) FROM companion_provider_segments WHERE thread_id = ?1
            )
            "#,
            params![thread_id, last_model_id, timestamp],
        )?;
        transaction.commit()?;
        self.companion_thread(thread_id)
    }

    pub fn set_companion_thread_archived(
        &self,
        thread_id: i64,
        archived: bool,
    ) -> Result<CompanionThread, Error> {
        let connection = self.connect()?;
        let timestamp = now();
        let archived_at = archived.then_some(timestamp.as_str());
        let changed = connection.execute(
            r#"
            UPDATE companion_threads
            SET archived_at = ?2, updated_at = ?3, last_opened_at = ?3
            WHERE id = ?1
            "#,
            params![thread_id, archived_at, timestamp],
        )?;
        if changed == 0 {
            return Err(Error::App("companion thread not found".into()));
        }
        self.companion_thread(thread_id)
    }

    pub fn companion_privacy_consent(&self) -> Result<PrivacyConsentStatus, Error> {
        let connection = self.connect()?;
        let row = connection
            .query_row(
                r#"
                SELECT disclosure_version, accepted_at, revoked_at
                FROM companion_privacy_consents WHERE provider = 'openai_codex'
                "#,
                [],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let (disclosure_version, accepted_at, revoked_at) =
            row.unwrap_or((COMPANION_DISCLOSURE_VERSION, None, None));
        let valid = disclosure_version == COMPANION_DISCLOSURE_VERSION
            && accepted_at.is_some()
            && revoked_at.is_none();
        Ok(PrivacyConsentStatus {
            provider: "openai_codex".into(),
            disclosure_version,
            accepted_at,
            revoked_at,
            valid,
        })
    }

    pub fn set_companion_privacy_consent(
        &self,
        accepted: bool,
    ) -> Result<PrivacyConsentStatus, Error> {
        let connection = self.connect()?;
        let timestamp = now();
        let (accepted_at, revoked_at) = if accepted {
            (Some(timestamp.as_str()), None)
        } else {
            (None, Some(timestamp.as_str()))
        };
        connection.execute(
            r#"
            INSERT INTO companion_privacy_consents (
              provider, disclosure_version, accepted_at, revoked_at
            ) VALUES ('openai_codex', ?1, ?2, ?3)
            ON CONFLICT(provider) DO UPDATE SET
              disclosure_version = excluded.disclosure_version,
              accepted_at = excluded.accepted_at,
              revoked_at = excluded.revoked_at
            "#,
            params![COMPANION_DISCLOSURE_VERSION, accepted_at, revoked_at],
        )?;
        self.companion_privacy_consent()
    }

    pub fn set_onboarding_status(
        &self,
        status: OnboardingStatus,
        app_version: String,
    ) -> Result<(), Error> {
        let snapshot = OnboardingSnapshot {
            status,
            decided_at: Some(now()),
            version: Some(app_version),
        };
        self.set_setting(ONBOARDING_SETTING, serde_json::to_string(&snapshot)?)
    }

    pub fn upsert_github_repository(&self, info: &GithubRepositoryInfo) -> Result<i64, Error> {
        let connection = self.connect()?;
        let timestamp = now();
        let source_key = info.source_key();

        if let Some(id) = connection
            .query_row(
                "SELECT id FROM repos WHERE source_key = ?1",
                [&source_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        {
            connection.execute(
                r#"
                UPDATE repos
                SET name = ?2,
                    path = ?3,
                    is_active = 1,
                    source_type = 'github',
                    remote_url = ?3,
                    remote_host = 'github.com',
                    provider = 'github',
                    default_branch = ?4,
                    updated_at = ?5
                WHERE id = ?1
                "#,
                params![id, info.name, info.html_url, info.default_branch, timestamp],
            )?;
            return Ok(id);
        }

        connection.execute(
            r#"
            INSERT INTO repos (
              name, path, is_active, source_type, source_key, last_scan_status,
              remote_url, remote_host, provider, default_branch, created_at, updated_at
            )
            VALUES (?1, ?2, 1, 'github', ?3, 'unknown', ?2, 'github.com', 'github', ?4, ?5, ?5)
            "#,
            params![
                info.name,
                info.html_url,
                source_key,
                info.default_branch,
                timestamp,
            ],
        )?;
        Ok(connection.last_insert_rowid())
    }

    fn update_github_sync_metadata(
        connection: &Connection,
        repo_id: i64,
        info: &GithubRepositoryInfo,
        files: &[GithubRemoteFile],
        timestamp: &str,
    ) -> Result<(), Error> {
        let updated = connection.execute(
            r#"
            UPDATE repos
            SET name = ?2,
                path = ?3,
                remote_url = ?3,
                remote_host = 'github.com',
                provider = 'github',
                default_branch = ?4,
                last_remote_commit_sha = ?5,
                last_remote_synced_at = ?6,
                remote_file_count = ?7,
                updated_at = ?6
            WHERE id = ?1 AND source_type = 'github'
            "#,
            params![
                repo_id,
                info.name,
                info.html_url,
                info.default_branch,
                info.commit_sha,
                timestamp,
                files.len() as u32,
            ],
        )?;
        if updated != 1 {
            return Err(Error::App(format!(
                "GitHub repository {repo_id} was not found"
            )));
        }

        connection.execute(
            "DELETE FROM repo_remote_files WHERE repo_id = ?1",
            [repo_id],
        )?;
        let mut insert = connection.prepare_cached(
            r#"
            INSERT INTO repo_remote_files (repo_id, path, blob_sha, size, seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )?;
        for file in files {
            insert.execute(params![
                repo_id,
                file.path,
                file.blob_sha,
                file.size as i64,
                timestamp
            ])?;
        }
        Ok(())
    }

    pub fn remove_remote_repository(&self, repo_id: i64) -> Result<(), Error> {
        let connection = self.connect()?;
        let source_type = connection
            .query_row(
                "SELECT source_type FROM repos WHERE id = ?1",
                [repo_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| Error::App(format!("repository {repo_id} was not found")))?;

        if source_type != "github" {
            return Err(Error::App(
                "only linked GitHub repositories can be removed from cvec".into(),
            ));
        }

        connection.execute("DELETE FROM repos WHERE id = ?1", [repo_id])?;
        Ok(())
    }

    pub fn sync_repositories(&self, discovered_repos: &[DiscoveredRepo]) -> Result<(), Error> {
        self.sync_repositories_with_completeness(discovered_repos, true)
    }

    pub fn sync_repositories_with_completeness(
        &self,
        discovered_repos: &[DiscoveredRepo],
        discovery_complete: bool,
    ) -> Result<(), Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let timestamp = now();
        let mut discovered_paths = std::collections::HashSet::with_capacity(discovered_repos.len());

        {
            let mut upsert = transaction.prepare(
                r#"
            INSERT INTO repos (
              name, path, is_active, source_type, source_key, last_scan_status,
              remote_url, remote_host, provider, created_at, updated_at
            )
            VALUES (?1, ?2, 1, 'local', NULL, 'unknown', ?3, ?4, ?5, ?6, ?6)
            ON CONFLICT(path) DO UPDATE SET
              name = excluded.name, is_active = 1, source_type = 'local',
              source_key = NULL, remote_url = excluded.remote_url,
              remote_host = excluded.remote_host, provider = excluded.provider,
              updated_at = excluded.updated_at
            WHERE repos.name IS NOT excluded.name OR repos.is_active IS NOT 1
               OR repos.source_type IS NOT 'local' OR repos.source_key IS NOT NULL
               OR repos.remote_url IS NOT excluded.remote_url
               OR repos.remote_host IS NOT excluded.remote_host
               OR repos.provider IS NOT excluded.provider
            "#,
            )?;
            for repo in discovered_repos {
                let path = repo.path.to_string_lossy();
                upsert.execute(params![
                    repo.name,
                    path,
                    repo.remote_url,
                    repo.remote_host,
                    repo.provider,
                    timestamp
                ])?;
                if discovery_complete {
                    discovered_paths.insert(path);
                }
            }
        }

        if discovery_complete {
            let active_paths = {
                let mut statement = transaction.prepare(
                    "SELECT path FROM repos
                     WHERE is_active = 1 AND source_type = 'local' AND source_key IS NULL",
                )?;
                let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let mut deactivate = transaction
                .prepare("UPDATE repos SET is_active = 0, updated_at = ?1 WHERE path = ?2")?;
            for path in active_paths {
                if !discovered_paths.contains(path.as_str()) {
                    deactivate.execute(params![timestamp, path])?;
                }
            }
        }

        transaction.commit()?;
        Ok(())
    }

    pub fn list_active_repositories(&self) -> Result<Vec<RepoRecord>, Error> {
        let connection = self.connect()?;
        let mut statement =
            connection.prepare(&format!("{ACTIVE_REPOSITORIES} ORDER BY name ASC"))?;
        let rows = statement.query_map([], repo_record_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn active_repository(&self, repo_id: i64) -> Result<RepoRecord, Error> {
        self.connect()?
            .query_row(
                &format!("{ACTIVE_REPOSITORIES} AND id=?1"),
                [repo_id],
                repo_record_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::App(format!("repository {repo_id} was not found")))
    }

    pub(crate) fn require_active_repository(&self, repository_id: i64) -> Result<(), Error> {
        ensure_repository_exists(&self.connect()?, repository_id)
    }

    pub fn list_repo_summaries(&self) -> Result<Vec<RepoSummary>, Error> {
        let connection = self.connect()?;
        Self::repo_summaries_on(&connection, None)
    }

    fn repo_summaries_on(
        connection: &Connection,
        repo_id: Option<i64>,
    ) -> Result<Vec<RepoSummary>, Error> {
        // Keep the primary-key lookup for repository detail and use the same
        // projection when the overview needs all active repositories.
        let scope = if repo_id.is_some() {
            "AND r.id = ?1"
        } else {
            ""
        };
        let mut statement = connection.prepare(&format!(
            r#"
            SELECT r.id, r.name, r.path, r.source_type, r.source_key, r.remote_url,
                   r.remote_host, r.provider, r.default_branch, r.last_remote_commit_sha,
                   r.last_remote_synced_at, r.remote_file_count,
                   COALESCE(rv.revision, 0) AS evidence_revision,
                   COALESCE(rv.dependency_revision, 0) AS dependency_revision, latest.status AS last_scan_status,
                   completed.id AS completed_run_id, completed.finished_at, completed.source_revision,
                   COALESCE(completed.new_medium_count, 0) AS new_medium,
                   COALESCE(completed.new_high_count, 0) AS new_high,
                   COALESCE(completed.new_critical_count, 0) AS new_critical,
                   COALESCE(completed.resolved_medium_count, 0) AS resolved_medium,
                   COALESCE(completed.resolved_high_count, 0) AS resolved_high,
                   COALESCE(completed.resolved_critical_count, 0) AS resolved_critical,
                   (SELECT COUNT(*) FROM vulnerabilities WHERE scan_run_id = completed.id AND severity = 'MEDIUM') AS open_medium,
                   (SELECT COUNT(*) FROM vulnerabilities WHERE scan_run_id = completed.id AND severity = 'HIGH') AS open_high,
                   (SELECT COUNT(*) FROM vulnerabilities WHERE scan_run_id = completed.id AND severity = 'CRITICAL') AS open_critical
            FROM repos r
            LEFT JOIN repository_data_revisions rv ON rv.repo_id = r.id
            LEFT JOIN scan_runs latest ON latest.id = (
                SELECT id FROM scan_runs WHERE repo_id = r.id ORDER BY started_at DESC, id DESC LIMIT 1
            )
            LEFT JOIN scan_runs completed ON completed.id = (
                SELECT id FROM scan_runs WHERE repo_id = r.id AND status = 'completed'
                ORDER BY finished_at DESC, id DESC LIMIT 1
            )
            WHERE r.is_active = 1 {scope}
            ORDER BY open_critical DESC, open_high DESC, open_medium DESC,
                     new_critical DESC, new_high DESC, new_medium DESC, completed.finished_at DESC, r.name, r.id
            "#,
        ))?;
        let rows = statement.query_map(rusqlite::params_from_iter(repo_id), |row| {
            let scan_status: Option<String> = row.get("last_scan_status")?;
            let last_scan_status = scan_status.as_deref().map(ScanRunStatus::parse);
            let completed: Option<i64> = row.get("completed_run_id")?;
            let new_critical: u32 = row.get("new_critical")?;
            let open_medium: u32 = row.get("open_medium")?;
            let open_high: u32 = row.get("open_high")?;
            let open_critical: u32 = row.get("open_critical")?;
            let status = match last_scan_status {
                Some(ScanRunStatus::Running) => RepoStatus::Scanning,
                Some(ScanRunStatus::Failed) => RepoStatus::Error,
                _ if new_critical > 0 => RepoStatus::Critical,
                _ if open_medium > 0 || open_high > 0 || open_critical > 0 => RepoStatus::Warning,
                _ if completed.is_some() => RepoStatus::Clean,
                _ => RepoStatus::Unknown,
            };

            Ok(RepoSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                source_type: RepoSourceType::parse(row.get_ref(3)?.as_str()?),
                source_key: row.get(4)?,
                remote_url: row.get(5)?,
                remote_host: row.get(6)?,
                provider: row.get(7)?,
                default_branch: row.get(8)?,
                last_remote_commit_sha: row.get(9)?,
                last_remote_synced_at: row.get(10)?,
                remote_file_count: row.get(11)?,
                evidence_revision: row.get::<_, i64>("evidence_revision")?.max(0) as u64,
                dependency_revision: row.get::<_, i64>("dependency_revision")?.max(0) as u64,
                status,
                new_medium: row.get("new_medium")?,
                new_high: row.get("new_high")?,
                new_critical,
                open_medium,
                open_high,
                open_critical,
                resolved_medium: row.get("resolved_medium")?,
                resolved_high: row.get("resolved_high")?,
                resolved_critical: row.get("resolved_critical")?,
                last_scanned_at: if completed.is_some() {
                    Some(row.get("finished_at")?)
                } else {
                    None
                },
                last_scan_status,
                last_successful_source_revision: row.get("source_revision")?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn active_repo_summary(
        &self,
        connection: &Connection,
        repo_id: i64,
    ) -> Result<RepoSummary, Error> {
        Self::repo_summaries_on(connection, Some(repo_id))?
            .pop()
            .ok_or_else(|| Error::App(format!("repository {repo_id} was not found")))
    }

    pub fn repo_detail(&self, repo_id: i64) -> Result<RepoDetail, Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let summary = self.active_repo_summary(&transaction, repo_id)?;
        let scan_history = self.repo_scan_history(&transaction, repo_id, 12)?;
        let latest_scan = scan_history.first().cloned();
        let completed_scans = self.repo_scan_history_filtered(&transaction, repo_id, 2, true)?;
        let latest_completed = completed_scans.first().cloned();
        let previous_scan = completed_scans.get(1).cloned();

        let latest_findings = match latest_completed.as_ref() {
            Some(run) => self.findings_for_run(&transaction, repo_id, run.id)?,
            None => Vec::new(),
        };
        let previous_findings = match previous_scan.as_ref() {
            Some(run) => self.findings_for_run(&transaction, repo_id, run.id)?,
            None => Vec::new(),
        };

        // Repository detail is an inventory projection. Review state and external
        // intelligence are loaded by their canonical workspaces; joining them here
        // caused one query per finding and made repository switches noticeably slow.
        let findings = build_repo_findings(
            &latest_findings,
            &previous_findings,
            previous_scan
                .as_ref()
                .is_some_and(|scan| scan.evidence_complete)
                || !previous_findings.is_empty(),
        );
        let packages = match latest_completed.as_ref() {
            Some(run) => {
                let packages = self.packages_for_run(&transaction, repo_id, run.id)?;
                aggregate_repo_packages(packages, &findings)
            }
            None => Vec::new(),
        };
        let assets = self.repo_assets(&transaction, repo_id)?;
        transaction.commit()?;

        Ok(RepoDetail {
            overview: None,
            summary,
            latest_scan,
            previous_scan,
            findings,
            packages,
            scan_history,
            assets,
        })
    }

    pub fn repo_scan_comparison(
        &self,
        repo_id: i64,
        run_id: i64,
    ) -> Result<Option<super::models::RepoScanComparison>, Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let run = transaction
            .query_row(
                &format!(
                    "{SCAN_RUN_DETAILS} WHERE repo_id=?1 AND id=?2
                     AND EXISTS(SELECT 1 FROM repos WHERE id=?1 AND is_active=1)"
                ),
                params![repo_id, run_id],
                scan_run_from_row,
            )
            .optional()?;
        let Some(run) = run else {
            return Ok(None);
        };
        let previous_run_id = if run.status == ScanRunStatus::Completed {
            Self::previous_completed_scan(&transaction, repo_id, run_id)?
        } else {
            None
        };
        let previous_package_count = match previous_run_id {
            Some(previous_id) => transaction.query_row(
                "SELECT package_count FROM scan_runs WHERE id=?1",
                [previous_id],
                |row| row.get(0),
            )?,
            None => None,
        };
        transaction.commit()?;
        Ok(Some(super::models::RepoScanComparison {
            run,
            previous_run_id,
            previous_package_count,
        }))
    }

    pub fn mark_finding_reviewed(
        &self,
        repo_id: i64,
        fingerprint: &str,
    ) -> Result<FindingReview, Error> {
        self.mark_finding_reviewed_with_evidence(repo_id, fingerprint, None)
    }

    pub(crate) fn mark_finding_reviewed_with_evidence(
        &self,
        repo_id: i64,
        fingerprint: &str,
        evidence: Option<&ReviewActionBasis>,
    ) -> Result<FindingReview, Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = Self::review_action_finding(&transaction, repo_id, fingerprint, evidence)?;
        if current.finding.state == RepoFindingState::Resolved {
            return Err(Error::App(
                "resolved findings cannot be marked reviewed".into(),
            ));
        }
        let basis = Self::review_risk_basis(&current);
        let updated_at = now();
        transaction.execute(
            r#"
            INSERT INTO finding_reviews (
              repo_id, fingerprint, state, reviewed_at, snooze_until, note,
              basis_severity, basis_fix_version, basis_known_exploited,
              basis_ransomware_known, basis_scanner_state, updated_at, transition_reason
            ) VALUES (?1, ?2, 'reviewed', ?3, NULL, NULL, ?4, ?5, ?6, ?7, ?8, ?3, 'reviewed')
            ON CONFLICT(repo_id, fingerprint) DO UPDATE SET
              state = 'reviewed', reviewed_at = excluded.reviewed_at,
              snooze_until = NULL, note = NULL,
              basis_severity = excluded.basis_severity,
              basis_fix_version = excluded.basis_fix_version,
              basis_known_exploited = excluded.basis_known_exploited,
              basis_ransomware_known = excluded.basis_ransomware_known,
              basis_scanner_state = excluded.basis_scanner_state,
              updated_at = excluded.updated_at, transition_reason = 'reviewed'
            "#,
            params![
                repo_id,
                fingerprint,
                updated_at,
                basis.severity.as_str(),
                basis.fix_version,
                basis.known_exploited,
                basis.ransomware_known,
                finding_state_as_str(&basis.scanner_state),
            ],
        )?;
        let review = self
            .finding_review(&transaction, repo_id, fingerprint)?
            .ok_or_else(|| Error::App("review state was not saved".into()))?;
        transaction.commit()?;
        Ok(review)
    }

    pub fn snooze_finding(
        &self,
        repo_id: i64,
        fingerprint: &str,
        until: &str,
        note: Option<String>,
    ) -> Result<FindingReview, Error> {
        self.snooze_finding_with_evidence(repo_id, fingerprint, until, note, None)
    }

    pub(crate) fn snooze_finding_with_evidence(
        &self,
        repo_id: i64,
        fingerprint: &str,
        until: &str,
        note: Option<String>,
        evidence: Option<&ReviewActionBasis>,
    ) -> Result<FindingReview, Error> {
        let until_value = timestamps::parse(until)
            .map_err(|_| Error::App("snooze time must be an RFC3339 timestamp".into()))?;
        if until_value <= OffsetDateTime::now_utc() {
            return Err(Error::App("snooze time must be in the future".into()));
        }
        let note = note
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if note
            .as_ref()
            .is_some_and(|value| value.chars().count() > 500)
        {
            return Err(Error::App(
                "snooze note must be 500 characters or fewer".into(),
            ));
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = Self::review_action_finding(&transaction, repo_id, fingerprint, evidence)?;
        if current.finding.state == RepoFindingState::Resolved {
            return Err(Error::App("resolved findings cannot be snoozed".into()));
        }
        let basis = Self::review_risk_basis(&current);
        let updated_at = now();
        transaction.execute(
            r#"
            INSERT INTO finding_reviews (
              repo_id, fingerprint, state, reviewed_at, snooze_until, note,
              basis_severity, basis_fix_version, basis_known_exploited,
              basis_ransomware_known, basis_scanner_state, updated_at, transition_reason
            ) VALUES (?1, ?2, 'snoozed', NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'snoozed')
            ON CONFLICT(repo_id, fingerprint) DO UPDATE SET
              state = 'snoozed', reviewed_at = NULL,
              snooze_until = excluded.snooze_until, note = excluded.note,
              basis_severity = excluded.basis_severity,
              basis_fix_version = excluded.basis_fix_version,
              basis_known_exploited = excluded.basis_known_exploited,
              basis_ransomware_known = excluded.basis_ransomware_known,
              basis_scanner_state = excluded.basis_scanner_state,
              updated_at = excluded.updated_at, transition_reason = 'snoozed'
            "#,
            params![
                repo_id,
                fingerprint,
                timestamps::format(&until_value),
                note,
                basis.severity.as_str(),
                basis.fix_version,
                basis.known_exploited,
                basis.ransomware_known,
                finding_state_as_str(&basis.scanner_state),
                updated_at,
            ],
        )?;
        let review = self
            .finding_review(&transaction, repo_id, fingerprint)?
            .ok_or_else(|| Error::App("snooze state was not saved".into()))?;
        transaction.commit()?;
        Ok(review)
    }

    pub fn clear_finding_review(&self, repo_id: i64, fingerprint: &str) -> Result<(), Error> {
        self.clear_finding_review_with_evidence(repo_id, fingerprint, None)
    }

    pub(crate) fn clear_finding_review_with_evidence(
        &self,
        repo_id: i64,
        fingerprint: &str,
        evidence: Option<&ReviewActionBasis>,
    ) -> Result<(), Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::review_action_finding(&transaction, repo_id, fingerprint, evidence)?;
        transaction.execute(
            "DELETE FROM finding_reviews WHERE repo_id = ?1 AND fingerprint = ?2",
            params![repo_id, fingerprint],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn review_action_finding(
        transaction: &Transaction<'_>,
        repo_id: i64,
        fingerprint: &str,
        evidence: Option<&ReviewActionBasis>,
    ) -> Result<ReviewFinding, Error> {
        let Some(expected) = evidence else {
            return Self::current_review_finding_on(transaction, repo_id, fingerprint);
        };
        let stale = || {
            Error::App(
                "REVIEW_EVIDENCE_STALE: The finding changed before the review action was saved."
                    .into(),
            )
        };
        let current = Self::find_current_review_finding_on(transaction, repo_id, fingerprint)?
            .ok_or_else(stale)?;
        let matches = current.scan_run_id == expected.scan_run_id
            && Self::review_risk_basis(&current) == expected.risk_basis
            && current.review_state == expected.review_state
            && current.reviewed_at == expected.reviewed_at
            && current.snooze_until == expected.snooze_until
            && current.review_note == expected.review_note;
        if !matches {
            return Err(stale());
        }
        Ok(current)
    }

    fn reopen_due_snoozes(connection: &Connection) -> Result<u32, Error> {
        Ok(connection.execute(
            "DELETE FROM finding_reviews WHERE state = 'snoozed' AND snooze_until <= ?1",
            [now()],
        )? as u32)
    }

    #[cfg(test)]
    fn all_review_findings(&self) -> Result<Vec<ReviewFinding>, Error> {
        let mut items = Vec::new();
        let connection = self.connect()?;
        for repo in self.list_active_repositories()? {
            let detail = self.repo_detail(repo.id)?;
            for finding in detail.findings.clone() {
                items.push(self.review_finding_from_parts(&connection, &repo, &detail, finding)?);
            }
        }
        Ok(items)
    }

    pub fn finding_remediation(
        &self,
        repo_id: i64,
        fingerprint: &str,
        evidence_scan_run_id: i64,
    ) -> Result<FindingRemediationProjection, Error> {
        if fingerprint.is_empty() || fingerprint.len() > 512 || evidence_scan_run_id <= 0 {
            return Err(Error::App("invalid remediation request".into()));
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let row = transaction
            .query_row(
                r#"
                SELECT r.name, r.source_type, r.is_active,
                       COALESCE(
                         (SELECT source_revision FROM scan_runs latest
                          WHERE latest.repo_id = r.id AND latest.status = 'completed' AND latest.source_revision IS NOT NULL
                          ORDER BY latest.id DESC LIMIT 1),
                         r.last_remote_commit_sha
                       ),
                       v.advisory_id, v.package_name, v.package_version, v.package_type,
                       v.package_url, v.fix_version, v.severity,
                       s.finished_at, s.source_revision, s.source_dirty
                FROM vulnerabilities v
                INNER JOIN repos r ON r.id = v.repo_id
                INNER JOIN scan_runs s ON s.id = v.scan_run_id AND s.repo_id = v.repo_id
                WHERE v.repo_id = ?1 AND v.fingerprint = ?2 AND v.scan_run_id = ?3
                "#,
                params![repo_id, fingerprint, evidence_scan_run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, Option<bool>>(13)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| Error::App("finding evidence is no longer available".into()))?;
        let (
            repo_name,
            source_type,
            repo_active,
            latest_known_revision,
            advisory_id,
            package_name,
            installed_version,
            package_type,
            package_url,
            scanner_reported_target,
            severity,
            evidence_scanned_at,
            evidence_revision,
            evidence_dirty,
        ) = row;

        let evidence = transaction
            .query_row(
                r#"
                SELECT schema_version, relative_path, location_kind, relationship,
                       parent_package, evidence_source, evidence_quality, location_count
                FROM vulnerability_remediation_evidence
                WHERE repo_id = ?1 AND fingerprint = ?2 AND scan_run_id = ?3
                "#,
                params![repo_id, fingerprint, evidence_scan_run_id],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, u32>(7)?,
                    ))
                },
            )
            .optional()?;
        let (
            evidence_schema_version,
            relative_path,
            location_kind,
            relationship,
            parent_package,
            evidence_source,
            evidence_quality,
            location_count,
        ) = evidence
            .filter(|(schema_version, _, _, _, _, _, _, _)| {
                *schema_version == REMEDIATION_SCHEMA_VERSION
            })
            .unwrap_or_else(|| {
                (
                    REMEDIATION_SCHEMA_VERSION,
                    None,
                    None,
                    "unknown".into(),
                    None,
                    "none".into(),
                    "unavailable".into(),
                    0,
                )
            });
        let relationship = RemediationRelationship::parse(&relationship);
        let evidence_source = RemediationEvidenceSource::parse(&evidence_source);
        let mut evidence_quality = RemediationEvidenceQuality::parse(&evidence_quality);
        let location_kind = location_kind.as_deref().map(RemediationLocationKind::parse);
        let verification = Self::remediation_verification(
            &transaction,
            repo_id,
            fingerprint,
            evidence_scan_run_id,
        )?;
        let revision_changed = evidence_revision.is_some()
            && latest_known_revision.is_some()
            && evidence_revision != latest_known_revision;
        let finding_replaced = matches!(
            verification.state,
            RemediationVerificationState::StillDetected | RemediationVerificationState::Reappeared
        ) && !verification.exact_match;
        if revision_changed || finding_replaced {
            evidence_quality = RemediationEvidenceQuality::Stale;
        }

        let priority_reasons = Self::current_review_finding_on(&transaction, repo_id, fingerprint)
            .map(|item| item.priority_reasons)
            .unwrap_or_else(|_| {
                let mut reasons = vec![format!("{} severity", severity.to_ascii_uppercase())];
                if scanner_reported_target.is_some() {
                    reasons.push("Fix available".into());
                }
                reasons
            });
        let source_snapshot = SourceSnapshot {
            revision: evidence_revision,
            dirty: evidence_dirty,
        };
        let can_open_manifest = repo_active
            && RepoSourceType::parse(&source_type) == RepoSourceType::Local
            && relative_path.is_some()
            && self
                .finding_manifest_target(repo_id, fingerprint, evidence_scan_run_id)
                .is_ok();
        let mut projection = FindingRemediationProjection {
            schema_version: evidence_schema_version,
            repo_id,
            repo_name,
            fingerprint: fingerprint.to_string(),
            advisory_id,
            package_name,
            installed_version,
            package_type,
            package_url,
            scanner_reported_target,
            evidence_scan_run_id,
            evidence_scanned_at,
            source_snapshot,
            relative_path,
            location_kind,
            relationship,
            parent_package,
            evidence_quality,
            evidence_source,
            location_count: location_count.min(32),
            verification_state: verification.state,
            verification_scan_run_id: verification.run_id,
            verification_scanned_at: verification.scanned_at,
            can_open_manifest,
            priority_reasons: priority_reasons.into_iter().take(5).collect(),
            brief_markdown: String::new(),
        };
        projection.brief_markdown = format_fix_brief(&projection);
        Ok(projection)
    }

    fn remediation_verification(
        connection: &Connection,
        repo_id: i64,
        fingerprint: &str,
        evidence_scan_run_id: i64,
    ) -> Result<RemediationVerification, Error> {
        use RemediationVerificationState::{NotDetected, Reappeared, StillDetected, Unverifiable};
        let latest = connection
            .query_row(
                "SELECT id, status, finished_at, evidence_complete FROM scan_runs
                 WHERE repo_id=?1 ORDER BY started_at DESC,id DESC LIMIT 1",
                [repo_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, status, finished_at, complete)) = latest else {
            return Ok(RemediationVerification {
                state: Unverifiable,
                run_id: None,
                scanned_at: None,
                exact_match: false,
            });
        };
        let (state, exact_match) = if status == "completed" && complete {
            let detected = scan_has_finding("detected", "?4", "basis");
            let previous = Self::previous_completed_scan(connection, repo_id, id)?;
            let history = finding_reappeared("basis", "?5", &detected);
            connection.query_row(
                &format!(
                    "SELECT ({detected}), ({history}),
                            (latest.started_at,latest.id) > (recorded.started_at,recorded.id),
                            EXISTS(SELECT 1 FROM vulnerabilities exact
                                   WHERE exact.scan_run_id=?4 AND exact.fingerprint=?2)
                     FROM vulnerabilities basis JOIN scan_runs recorded ON recorded.id=basis.scan_run_id
                     JOIN scan_runs latest ON latest.id=?4
                     WHERE basis.repo_id=?1 AND basis.fingerprint=?2 AND basis.scan_run_id=?3"
                ),
                params![repo_id, fingerprint, evidence_scan_run_id, id, previous],
                |row| {
                    let state = match (row.get::<_, bool>(0)?, row.get::<_, bool>(1)?, row.get::<_, bool>(2)?) {
                        (true, true, _) => Reappeared,
                        (true, false, _) => StillDetected,
                        (false, _, true) => NotDetected,
                        _ => Unverifiable,
                    };
                    Ok((state, row.get::<_, bool>(3)?))
                },
            )?
        } else {
            (Unverifiable, false)
        };
        Ok(RemediationVerification {
            state,
            run_id: Some(id),
            scanned_at: finished_at,
            exact_match,
        })
    }

    pub fn finding_manifest_target(
        &self,
        repo_id: i64,
        fingerprint: &str,
        evidence_scan_run_id: i64,
    ) -> Result<PathBuf, Error> {
        let connection = self.connect()?;
        let (repo_path, source_type, active, relative_path) = connection
            .query_row(
                r#"
                SELECT r.path, r.source_type, r.is_active, e.relative_path
                FROM vulnerability_remediation_evidence e
                INNER JOIN repos r ON r.id = e.repo_id
                WHERE e.repo_id = ?1 AND e.fingerprint = ?2 AND e.scan_run_id = ?3
                "#,
                params![repo_id, fingerprint, evidence_scan_run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| Error::App("manifest evidence is not available".into()))?;
        if !active || RepoSourceType::parse(&source_type) != RepoSourceType::Local {
            return Err(Error::App(
                "manifest opening is available for active local repositories only".into(),
            ));
        }
        let relative_path = relative_path
            .filter(|path| is_safe_stored_relative_path(path))
            .ok_or_else(|| Error::App("manifest evidence is not available".into()))?;
        let root = PathBuf::from(repo_path)
            .canonicalize()
            .map_err(|_| Error::App("repository is no longer available".into()))?;
        let target = root
            .join(relative_path)
            .canonicalize()
            .map_err(|_| Error::App("manifest is no longer available".into()))?;
        if !target.starts_with(&root) || !target.is_file() {
            return Err(Error::App("manifest is no longer available".into()));
        }
        Ok(target)
    }

    #[cfg(test)]
    fn review_finding_from_parts(
        &self,
        connection: &Connection,
        repo: &RepoRecord,
        detail: &RepoDetail,
        finding: RepoFinding,
    ) -> Result<ReviewFinding, Error> {
        let evidence_scan = connection
            .query_row(
                r#"
                SELECT s.id, s.finished_at
                FROM vulnerabilities v
                INNER JOIN scan_runs s ON s.id = v.scan_run_id
                WHERE v.repo_id = ?1 AND v.fingerprint = ?2 AND s.status = 'completed'
                ORDER BY s.id DESC
                LIMIT 1
                "#,
                params![repo.id, finding.fingerprint],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let review = self.finding_review(connection, repo.id, &finding.fingerprint)?;
        let (known_exploited, ransomware_known) =
            self.finding_exploitation_flags(connection, &finding)?;
        let is_reappeared = if finding.state == RepoFindingState::New {
            connection
                .query_row(
                    &format!(
                        "SELECT {reappeared} FROM vulnerabilities v
                     WHERE v.repo_id=?1 AND v.fingerprint=?2 AND v.scan_run_id=?3",
                        reappeared = finding_reappeared("v", "?4", "1"),
                    ),
                    params![
                        repo.id,
                        finding.fingerprint,
                        detail
                            .scan_history
                            .iter()
                            .find(|run| run.status == ScanRunStatus::Completed)
                            .map(|run| run.id),
                        detail.previous_scan.as_ref().map(|run| run.id)
                    ],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false)
        } else {
            false
        };
        let material_change = if is_reappeared {
            Some("reappeared".to_string())
        } else if finding.state == RepoFindingState::New {
            Some("new".to_string())
        } else {
            None
        };
        let mut priority_reasons = Vec::new();
        if ransomware_known {
            priority_reasons.push("Known ransomware use".into());
        } else if known_exploited {
            priority_reasons.push("Known exploited vulnerability".into());
        }
        priority_reasons.push(format!("{} severity", finding.severity.as_str()));
        if let Some(change) = &material_change {
            priority_reasons.push(match change.as_str() {
                "reappeared" => "Reappeared after resolution".into(),
                _ => "New since previous scan".into(),
            });
        }
        if finding.fix_version.is_some() {
            priority_reasons.push("Fix available".into());
        }
        Ok(ReviewFinding {
            advisory_severity: None,
            repo_id: repo.id,
            repo_name: repo.name.clone(),
            repo_source_type: repo.source_type,
            scan_run_id: evidence_scan.as_ref().map(|(id, _)| *id),
            scanned_at: evidence_scan.and_then(|(_, finished_at)| finished_at),
            review_state: review
                .as_ref()
                .map(|value| value.state)
                .unwrap_or(ReviewState::Unreviewed),
            reviewed_at: review.as_ref().and_then(|value| value.reviewed_at.clone()),
            snooze_until: review.as_ref().and_then(|value| value.snooze_until.clone()),
            review_note: review.and_then(|value| value.note),
            known_exploited,
            ransomware_known,
            material_change,
            priority_reasons,
            finding,
        })
    }

    fn review_risk_basis(item: &ReviewFinding) -> ReviewRiskBasis {
        ReviewRiskBasis {
            severity: item
                .advisory_severity
                .filter(|severity| severity_rank(severity) > severity_rank(&item.finding.severity))
                .unwrap_or(item.finding.severity),
            fix_version: item.finding.fix_version.clone(),
            known_exploited: item.known_exploited,
            ransomware_known: item.ransomware_known,
            scanner_state: item.finding.state,
        }
    }

    fn finding_review(
        &self,
        connection: &Connection,
        repo_id: i64,
        fingerprint: &str,
    ) -> Result<Option<FindingReview>, Error> {
        connection
            .query_row(
                r#"
                SELECT state, reviewed_at, snooze_until, note, basis_severity,
                       basis_fix_version, basis_known_exploited, basis_ransomware_known,
                       basis_scanner_state, updated_at, transition_reason
                FROM finding_reviews WHERE repo_id = ?1 AND fingerprint = ?2
                "#,
                params![repo_id, fingerprint],
                |row| {
                    Ok(FindingReview {
                        repo_id,
                        fingerprint: fingerprint.to_string(),
                        state: ReviewState::parse(&row.get::<_, String>(0)?),
                        reviewed_at: row.get(1)?,
                        snooze_until: row.get(2)?,
                        note: row.get(3)?,
                        risk_basis: ReviewRiskBasis {
                            severity: Severity::parse(&row.get::<_, String>(4)?)
                                .unwrap_or(Severity::Medium),
                            fix_version: row.get(5)?,
                            known_exploited: row.get(6)?,
                            ransomware_known: row.get(7)?,
                            scanner_state: parse_finding_state(&row.get::<_, String>(8)?),
                        },
                        updated_at: row.get(9)?,
                        transition_reason: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    #[cfg(test)]
    fn finding_exploitation_flags(
        &self,
        connection: &Connection,
        finding: &RepoFinding,
    ) -> Result<(bool, bool), Error> {
        let result = connection
            .query_row(
                r#"
                SELECT ea.is_kev, ea.is_ransomware_known
                FROM external_advisories ea
                LEFT JOIN external_advisory_aliases alias ON alias.advisory_id = ea.id
                WHERE lower(ea.canonical_key) = lower(?1)
                   OR lower(ea.source_item_id) = lower(?1)
                   OR lower(alias.alias) = lower(?1)
                ORDER BY ea.is_ransomware_known DESC, ea.is_kev DESC
                LIMIT 1
                "#,
                [&finding.advisory_id],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?;
        Ok(result.unwrap_or((false, false)))
    }

    pub fn list_risk_trend(&self, days: u32) -> Result<Vec<RiskTrendPoint>, Error> {
        let days = match days {
            14 => 14,
            30 => 30,
            _ => 7,
        };
        let connection = self.connect()?;
        let today = OffsetDateTime::now_utc().date();
        let first_day = today - Duration::days(i64::from(days - 1));
        // The same scan can supply many daily points. Materialize its counts once.
        // Scan timestamps are stored in UTC. A next-midnight cutoff includes
        // fractional seconds on the chart day.
        let mut statement = connection.prepare(
            r#"
            WITH RECURSIVE dates(day) AS (
                SELECT ?1
                UNION ALL
                SELECT date(day, '+1 day') FROM dates WHERE day < ?2
            ), selected AS MATERIALIZED (
                SELECT day, (
                    SELECT id FROM scan_runs
                    WHERE repo_id = r.id AND status = 'completed'
                      AND finished_at < (date(day, '+1 day') || 'T00:00:00')
                    ORDER BY finished_at DESC, id DESC LIMIT 1
                ) AS run_id
                FROM dates CROSS JOIN repos r WHERE r.is_active = 1
            ), counts AS MATERIALIZED (
                SELECT run_id,
                    (SELECT COUNT(*) FROM vulnerabilities WHERE scan_run_id = run_id AND severity = 'MEDIUM') AS medium,
                    (SELECT COUNT(*) FROM vulnerabilities WHERE scan_run_id = run_id AND severity = 'HIGH') AS high,
                    (SELECT COUNT(*) FROM vulnerabilities WHERE scan_run_id = run_id AND severity = 'CRITICAL') AS critical
                FROM (SELECT DISTINCT run_id FROM selected WHERE run_id IS NOT NULL)
            )
            SELECT dates.day, COALESCE(SUM(medium), 0), COALESCE(SUM(high), 0), COALESCE(SUM(critical), 0)
            FROM dates LEFT JOIN selected USING (day) LEFT JOIN counts USING (run_id)
            GROUP BY dates.day ORDER BY dates.day
            "#,
        )?;
        let rows =
            statement.query_map(params![first_day.to_string(), today.to_string()], |row| {
                Ok(RiskTrendPoint {
                    date: row.get(0)?,
                    medium: row.get(1)?,
                    high: row.get(2)?,
                    critical: row.get(3)?,
                })
            })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn list_notifications_on(
        connection: &Connection,
        limit: usize,
    ) -> Result<Vec<AppNotification>, Error> {
        let mut statement = connection.prepare(
            r#"
            SELECT id, kind, tone, title, message, repo_id, repo_name, created_at, scan_run_id
            FROM notifications
            ORDER BY created_at DESC, id DESC
            LIMIT ?1
            "#,
        )?;

        let rows = statement.query_map([limit as i64], |row| {
            Ok(AppNotification {
                id: row.get(0)?,
                kind: parse_notification_kind(&row.get::<_, String>(1)?),
                tone: parse_notification_tone(&row.get::<_, String>(2)?),
                title: row.get(3)?,
                message: row.get(4)?,
                repo_id: row.get(5)?,
                repo_name: row.get(6)?,
                created_at: row.get(7)?,
                scan_run_id: row.get(8)?,
            })
        })?;

        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn insert_notification(
        &self,
        kind: AppNotificationKind,
        tone: AppNotificationTone,
        title: &str,
        message: &str,
        repository: Option<(i64, &str)>,
        scan_run_id: Option<i64>,
    ) -> Result<(), Error> {
        let connection = self.connect()?;
        connection.execute(
            r#"
            INSERT INTO notifications (kind, tone, title, message, repo_id, repo_name, created_at, scan_run_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                notification_kind_as_str(&kind),
                notification_tone_as_str(&tone),
                title,
                message,
                repository.map(|(id, _)| id),
                repository.map(|(_, name)| name),
                now(),
                scan_run_id,
            ],
        )?;
        Ok(())
    }

    pub fn clear_notification(&self, notification_id: i64) -> Result<(), Error> {
        let connection = self.connect()?;
        connection.execute("DELETE FROM notifications WHERE id = ?1", [notification_id])?;
        Ok(())
    }

    pub fn clear_all_notifications(&self) -> Result<(), Error> {
        let connection = self.connect()?;
        connection.execute("DELETE FROM notifications", [])?;
        Ok(())
    }

    pub fn list_repository_assets(&self, repo_id: i64) -> Result<Vec<RepoAsset>, Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        ensure_repository_exists(&transaction, repo_id)?;
        let assets = self.repo_assets(&transaction, repo_id)?;
        transaction.commit()?;
        Ok(assets)
    }

    pub fn replace_repo_assets(&self, repo_id: i64, assets: &[RepoAsset]) -> Result<(), Error> {
        let mut connection = self.connect()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        self.replace_repo_assets_on(&transaction, repo_id, assets)?;
        transaction.commit()?;
        Ok(())
    }

    fn replace_repo_assets_on(
        &self,
        connection: &Connection,
        repo_id: i64,
        assets: &[RepoAsset],
    ) -> Result<(), Error> {
        let mut previous = self.repo_assets(connection, repo_id)?;
        if previous.len() == assets.len() {
            // Compare the evidence, including duplicates, independent of
            // the order in which detection and SQLite returned the rows.
            let mut incoming = assets.to_vec();
            for asset in &mut incoming {
                asset.repo_id = repo_id;
            }
            previous.sort_unstable();
            incoming.sort_unstable();
            if previous == incoming {
                return Ok(());
            }
        }
        connection.execute("DELETE FROM repo_assets WHERE repo_id = ?1", [repo_id])?;

        let mut insert = connection.prepare_cached(
            r#"
            INSERT INTO repo_assets (
              repo_id, vendor, product, version, asset_kind, source_path, evidence, confidence
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )?;
        for asset in assets {
            insert.execute(params![
                repo_id,
                asset.vendor,
                asset.product,
                asset.version,
                asset.asset_kind,
                asset.source_path,
                asset.evidence,
                asset.confidence,
            ])?;
        }

        Ok(())
    }

    pub fn replace_external_intel(
        &self,
        items: &[PersistedExternalIntelItem],
        fetched_at: &str,
    ) -> Result<(), Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;

        transaction.execute("DELETE FROM external_advisory_matches", [])?;
        transaction.execute("DELETE FROM external_advisory_products", [])?;
        transaction.execute("DELETE FROM external_advisory_kev", [])?;
        transaction.execute("DELETE FROM external_advisory_cwes", [])?;
        transaction.execute("DELETE FROM external_advisory_references", [])?;
        transaction.execute("DELETE FROM external_advisory_affected_packages", [])?;
        transaction.execute("DELETE FROM external_advisory_aliases", [])?;
        transaction.execute("DELETE FROM external_advisory_sources", [])?;
        transaction.execute("DELETE FROM external_advisories", [])?;

        for item in items {
            transaction.execute(
                r#"
                INSERT INTO external_advisories (
                  canonical_key, title, summary, url, ecosystem, package_name,
                  severity, published_at, updated_at, modified_at, withdrawn_at,
                  cvss_score, cvss_vector, cvss_version, epss_percentage, epss_percentile,
                  is_kev, is_ransomware_known, primary_source, source_item_id, raw_primary_json,
                  product_keys_json, topic_keys_json, relevance, fetched_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)
                "#,
                params![
                    item.canonical_key,
                    item.title,
                    item.summary,
                    item.url,
                    item.ecosystem,
                    item.package_name,
                    item.severity.as_str(),
                    item.published_at,
                    item.updated_at,
                    item.modified_at,
                    item.withdrawn_at,
                    item.cvss_score,
                    item.cvss_vector,
                    item.cvss_version,
                    item.epss_percentage,
                    item.epss_percentile,
                    item.is_kev,
                    item.is_ransomware_known,
                    external_source_as_str(&item.primary_source),
                    item.source_item_id,
                    item.raw_json,
                    serde_json::to_string(&item.product_keys)?,
                    serde_json::to_string(&item.topic_keys)?,
                    external_relevance_as_str(&item.relevance),
                    item.fetched_at,
                ],
            )?;
            let advisory_id = transaction.last_insert_rowid();

            for detail in &item.source_details {
                transaction.execute(
                    r#"
                    INSERT INTO external_advisory_sources (
                      advisory_id, source, source_item_id, url, published_at, updated_at,
                      severity, cvss_score, epss_percentage, reference_count, raw_json
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    "#,
                    params![
                        advisory_id,
                        external_source_as_str(&detail.source),
                        detail.source_item_id,
                        detail.url,
                        detail.published_at,
                        detail.updated_at,
                        detail.severity.as_str(),
                        detail.cvss_score,
                        detail.epss_percentage,
                        detail.reference_count,
                        detail.raw_json,
                    ],
                )?;
            }

            for alias in &item.aliases {
                transaction.execute(
                    "INSERT INTO external_advisory_aliases (advisory_id, alias) VALUES (?1, ?2)",
                    params![advisory_id, alias],
                )?;
            }

            for affected in &item.affected_packages {
                transaction.execute(
                    r#"
                    INSERT INTO external_advisory_affected_packages (
                      advisory_id, ecosystem, package_name, package_purl, vulnerable_range,
                      first_patched_version, affected_versions_count
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    "#,
                    params![
                        advisory_id,
                        affected.ecosystem,
                        affected.package_name,
                        affected.package_purl,
                        affected.vulnerable_version_range,
                        affected.first_patched_version,
                        affected.affected_versions_count,
                    ],
                )?;
            }

            for reference in &item.references {
                transaction.execute(
                    r#"
                    INSERT INTO external_advisory_references (
                      advisory_id, url, source, kind, tags_json
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5)
                    "#,
                    params![
                        advisory_id,
                        reference.url,
                        reference.source,
                        reference.kind,
                        serde_json::to_string(&reference.tags)?,
                    ],
                )?;
            }

            for cwe in &item.cwes {
                transaction.execute(
                    "INSERT INTO external_advisory_cwes (advisory_id, cwe_id, name) VALUES (?1, ?2, ?3)",
                    params![advisory_id, cwe.cwe_id, cwe.name],
                )?;
            }

            if let Some(kev) = &item.kev {
                transaction.execute(
                    r#"
                    INSERT INTO external_advisory_kev (
                      advisory_id, date_added, due_date, required_action,
                      known_ransomware_campaign_use, notes, catalog_version, catalog_released_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    "#,
                    params![
                        advisory_id,
                        kev.date_added,
                        kev.due_date,
                        kev.required_action,
                        kev.known_ransomware_campaign_use,
                        kev.notes,
                        kev.catalog_version,
                        kev.catalog_released_at,
                    ],
                )?;
            }

            for product_key_value in &item.product_keys {
                let (vendor, product) = split_product_key(product_key_value);
                let topic_key = product_key_to_topic(product_key_value);
                transaction.execute(
                    r#"
                    INSERT INTO external_advisory_products (
                      advisory_id, vendor, product, product_key, topic_key
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5)
                    "#,
                    params![advisory_id, vendor, product, product_key_value, topic_key],
                )?;
            }

            if let Some(match_kind) = &item.match_kind {
                transaction.execute(
                    r#"
                    INSERT INTO external_advisory_matches (advisory_id, match_kind, repo_count)
                    VALUES (?1, ?2, ?3)
                    "#,
                    params![
                        advisory_id,
                        external_match_kind_as_str(match_kind),
                        item.repo_count
                    ],
                )?;
            }
        }

        transaction.execute(
            r#"
            INSERT INTO app_settings (key, value)
            VALUES ('external_intel_last_fetched_at', ?1)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
            [fetched_at],
        )?;

        transaction.commit()?;
        Ok(())
    }

    pub fn watched_external_intel_topic_keys(&self) -> Result<Vec<String>, Error> {
        let connection = self.connect()?;
        Self::watched_topic_keys(&connection, None)
    }

    fn watched_topic_keys(
        connection: &Connection,
        suggested: Option<&[String]>,
    ) -> Result<Vec<String>, Error> {
        if let Some(value) = Self::setting_on(connection, EXTERNAL_INTEL_TOPICS_SETTING)? {
            let parsed = serde_json::from_str::<Vec<String>>(&value).unwrap_or_default();
            return Ok(normalize_topic_keys(parsed));
        }

        let local_topics = match suggested {
            Some(keys) => keys.to_vec(),
            None => Self::suggested_external_intel_topic_keys(connection)?,
        };
        if local_topics.is_empty() {
            Ok(default_topic_keys())
        } else {
            Ok(local_topics)
        }
    }

    fn external_intel_topics_on(connection: &Connection) -> Result<Vec<ExternalIntelTopic>, Error> {
        let suggested = Self::suggested_external_intel_topic_keys(connection)?;
        let watched = Self::watched_topic_keys(connection, Some(&suggested))?
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        let suggested = suggested
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();

        Ok(topic_definitions()
            .iter()
            .map(|topic| ExternalIntelTopic {
                key: topic.key.to_string(),
                label: topic.label.to_string(),
                category: topic.category.to_string(),
                total_count: topic.total_count,
                watched: watched.contains(topic.key),
                suggested: suggested.contains(topic.key),
            })
            .collect())
    }

    pub fn set_external_intel_topics(&self, topic_keys: Vec<String>) -> Result<(), Error> {
        let normalized = normalize_topic_keys(topic_keys);
        self.set_setting(
            EXTERNAL_INTEL_TOPICS_SETTING,
            serde_json::to_string(&normalized)?,
        )
    }

    pub fn external_intel_feed(&self) -> Result<ExternalIntelFeed, Error> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let revisions = Self::read_data_revisions(&tx)?;
        let mut statement = tx.prepare(
            r#"
            SELECT id, canonical_key, title, summary, ecosystem, package_name, severity,
                   published_at, withdrawn_at, cvss_score, epss_percentage, is_kev,
                   is_ransomware_known, product_keys_json, topic_keys_json
            FROM external_advisories
            ORDER BY id
            "#,
        )?;
        let mut items = statement
            .query_map([], external_intel_feed_item_from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        if !items.is_empty() {
            let mut packages = tx.prepare(
                "SELECT advisory_id, ecosystem, package_name, package_purl
                 FROM external_advisory_affected_packages",
            )?;
            let mut keys_by_advisory: HashMap<i64, HashSet<AdvisoryPackageKey>> = HashMap::new();
            let mut rows = packages.query([])?;
            while let Some(row) = rows.next()? {
                if let Some(key) = AdvisoryPackageKey::new(
                    row.get::<_, Option<String>>(1)?.as_deref(),
                    row.get::<_, Option<String>>(2)?.as_deref(),
                    row.get::<_, Option<String>>(3)?.as_deref(),
                ) {
                    keys_by_advisory.entry(row.get(0)?).or_default().insert(key);
                }
            }
            for item in &items {
                keys_by_advisory.entry(item.id).or_insert_with(|| {
                    advisory_package_keys(
                        item.ecosystem.as_deref(),
                        item.package_name.as_deref(),
                        &[],
                    )
                });
            }
            let local_index = self.intel_feed_packages(&tx, &revisions, &keys_by_advisory)?;
            let indexes: HashMap<i64, usize> = items
                .iter()
                .enumerate()
                .map(|(index, item)| (item.id, index))
                .collect();
            let mut aliases = tx.prepare(
                "SELECT advisory_id, alias FROM external_advisory_aliases ORDER BY alias",
            )?;
            for row in aliases.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get(1)?)))? {
                let (id, alias) = row?;
                if let Some(&index) = indexes.get(&id) {
                    items[index].aliases.push(alias);
                }
            }

            let mut sources =
                tx.prepare("SELECT DISTINCT advisory_id, source FROM external_advisory_sources")?;
            for row in sources.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    parse_external_source(row.get_ref(1)?.as_str()?),
                ))
            })? {
                let (id, source) = row?;
                if let Some(&index) = indexes.get(&id) {
                    items[index].sources.push(source);
                }
            }

            let mut cwes = tx.prepare(
                "SELECT advisory_id, cwe_id, name FROM external_advisory_cwes ORDER BY cwe_id",
            )?;
            for row in cwes.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    AdvisoryCwe {
                        cwe_id: row.get(1)?,
                        name: row.get(2)?,
                    },
                ))
            })? {
                let (id, cwe) = row?;
                if let Some(&index) = indexes.get(&id) {
                    items[index].cwes.push(cwe);
                }
            }

            let mut kev = tx.prepare(
                "SELECT advisory_id, date_added, due_date FROM external_advisory_kev ORDER BY id",
            )?;
            for row in kev.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    ExternalIntelKevDates {
                        date_added: row.get(1)?,
                        due_date: row.get(2)?,
                    },
                ))
            })? {
                let (id, dates) = row?;
                if let Some(&index) = indexes.get(&id) {
                    items[index].kev_dates.get_or_insert(dates);
                }
            }

            for item in &mut items {
                let (kind, count) = local_index.match_advisory(
                    &keys_by_advisory[&item.id],
                    &item.product_keys,
                    item.ecosystem.as_deref(),
                );
                item.relevance = ExternalIntelMatchKind::relevance(kind.as_ref());
                item.match_kind = kind;
                item.repo_count = count as u32;
                item.sources.sort_unstable();
                item.sources.dedup();
            }

            items.sort_by_cached_key(|item| {
                use std::cmp::Reverse;
                (
                    Reverse(external_published_sort_key(item.published_at.as_deref())),
                    Reverse(relevance_rank(&item.relevance)),
                    Reverse(item.severity.rank()),
                    item.title.clone(),
                )
            });
        }

        Ok(ExternalIntelFeed {
            intel_revision: revisions.intel,
            inventory_revision: revisions.inventory,
            items,
        })
    }

    pub fn external_intel_detail(
        &self,
        canonical_key: &str,
        intel_revision: u64,
        inventory_revision: u64,
    ) -> Result<Option<ExternalIntelDetail>, Error> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let revisions = Self::read_data_revisions(&tx)?;
        if revisions.intel != intel_revision || revisions.inventory != inventory_revision {
            return Ok(None);
        }

        let row = tx
            .query_row(
                r#"
            SELECT id, url, updated_at, modified_at, cvss_vector, cvss_version, epss_percentile,
                   ecosystem, package_name, withdrawn_at IS NOT NULL
            FROM external_advisories WHERE canonical_key = ?1
            "#,
                [canonical_key],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, bool>(9)?,
                        ExternalIntelDetail {
                            url: row.get(1)?,
                            updated_at: row.get(2)?,
                            modified_at: row.get(3)?,
                            cvss_vector: row.get(4)?,
                            cvss_version: row.get(5)?,
                            epss_percentile: row.get(6)?,
                            references: Vec::new(),
                            affected_packages: Vec::new(),
                            local_packages: Vec::new(),
                            kev: None,
                            source_details: Vec::new(),
                        },
                    ))
                },
            )
            .optional()?;
        let Some((id, ecosystem, package, withdrawn, mut detail)) = row else {
            return Ok(None);
        };
        detail.source_details = self.list_external_source_details(&tx, id)?;
        detail.affected_packages = self.list_external_affected_packages(&tx, id)?;
        if !withdrawn {
            let keys = advisory_package_keys(
                ecosystem.as_deref(),
                package.as_deref(),
                &detail.affected_packages,
            );
            if !keys.is_empty() {
                let index = match self.cached_intel_packages(&revisions) {
                    Some(index) => index,
                    None => Arc::new(LocalPackageIndex::load(&tx, &keys)?),
                };
                detail.local_packages = index.local_packages(&keys, &detail.affected_packages);
            }
        }
        detail.references = self.list_external_references(&tx, id)?;
        detail.kev = self.get_external_kev(&tx, id)?;
        Ok(Some(detail))
    }

    pub fn external_intel_summary(&self) -> Result<ExternalIntelSummary, Error> {
        Self::external_intel_summary_on(&self.connect()?)
    }

    fn external_intel_summary_on(connection: &Connection) -> Result<ExternalIntelSummary, Error> {
        let (total_items, relevant_items, triage_items) = connection.query_row(
            "SELECT COUNT(*), COUNT(*) FILTER (WHERE relevance = 'relevant'),
                    COUNT(*) FILTER (WHERE relevance = 'triage') FROM external_advisories",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;

        let last_fetched_at = Self::setting_on(connection, "external_intel_last_fetched_at")?;
        let mut statement = connection
            .prepare("SELECT source, COUNT(*) FROM external_advisory_sources GROUP BY source")?;
        let rows = statement.query_map([], |row| {
            Ok((
                parse_external_source(&row.get::<_, String>(0)?),
                row.get::<_, u32>(1)?,
            ))
        })?;
        let mut sources = std::collections::BTreeSet::new();
        let mut source_counts = std::collections::BTreeMap::<ExternalSource, u32>::new();
        for row in rows {
            let (source, count) = row?;
            sources.insert(source.clone());
            *source_counts.entry(source).or_insert(0) += count;
        }

        Ok(ExternalIntelSummary {
            total_items,
            relevant_items,
            triage_items,
            last_fetched_at,
            sources: sources.into_iter().collect(),
            source_counts: source_counts
                .into_iter()
                .map(|(source, count)| ExternalSourceCount { source, count })
                .collect(),
        })
    }

    fn suggested_external_intel_topic_keys(connection: &Connection) -> Result<Vec<String>, Error> {
        let mut statement = connection.prepare(&format!(
            r#"
            WITH active_runs AS MATERIALIZED (
                SELECT r.id, ({LATEST_COMPLETED_SCAN_FOR_REPO}) scan_id
                FROM repos r WHERE r.is_active = 1
            )
            SELECT p.package_type, NULL AS product_key
            FROM active_runs r
            CROSS JOIN scan_packages p ON p.repo_id = r.id AND p.scan_run_id = r.scan_id
            WHERE p.package_type IS NOT NULL
            UNION
            SELECT v.package_type, NULL AS product_key
            FROM active_runs r
            CROSS JOIN vulnerabilities v ON v.repo_id = r.id AND v.scan_run_id = r.scan_id
            WHERE v.package_type IS NOT NULL
            UNION
            SELECT NULL, a.vendor || ':' || a.product
            FROM repo_assets a
            JOIN repos r ON r.id = a.repo_id
            WHERE r.is_active = 1
            "#
        ))?;
        let rows = statement.query_map([], |row| {
            if let Some(package_type) = row.get::<_, Option<String>>(0)? {
                Ok(normalize_local_package_type(&package_type).and_then(normalize_topic_key))
            } else {
                Ok(product_key_to_topic(&row.get::<_, String>(1)?))
            }
        })?;
        let mut topics = std::collections::BTreeSet::new();
        for topic in rows {
            topics.extend(topic?);
        }
        Ok(topics.into_iter().collect())
    }

    pub fn insert_scan_start(
        &self,
        repo_id: i64,
        versions: &ScannerVersions,
    ) -> Result<i64, Error> {
        let connection = self.connect()?;
        let versions_json = serde_json::to_string(versions)?;
        let inserted = connection.execute(
            r#"
            INSERT INTO scan_runs (repo_id, status, started_at, scanner_versions)
            SELECT id, 'running', ?2, ?3 FROM repos WHERE id=?1 AND is_active=1
            "#,
            params![repo_id, now(), versions_json],
        )?;
        if inserted == 0 {
            return Err(Error::App(format!("repository {repo_id} was not found")));
        }
        Ok(connection.last_insert_rowid())
    }

    pub fn complete_scan(
        &self,
        repo_id: i64,
        run_id: i64,
        findings: &[NormalizedFinding],
        packages: &[RepoPackage],
        delta: &DeltaSummary,
    ) -> Result<(), Error> {
        let source_revision = self
            .connect()?
            .query_row(
                "SELECT last_remote_commit_sha FROM repos WHERE id = ?1",
                [repo_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        self.complete_scan_with_remediation(ScanCompletion {
            repo_id,
            run_id,
            findings,
            packages,
            remediation_evidence: &[],
            source_snapshot: &SourceSnapshot {
                revision: source_revision,
                dirty: None,
            },
            delta,
        })
    }

    pub fn complete_scan_with_remediation(&self, input: ScanCompletion<'_>) -> Result<(), Error> {
        let mut connection = self.connect()?;
        let transaction =
            Self::begin_scan_completion(&mut connection, input.repo_id, input.run_id)?;
        let previous = Self::previous_completed_scan(&transaction, input.repo_id, input.run_id)?;
        Self::persist_scan(&transaction, input, &now(), previous, None)?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn complete_repository_scan(
        &self,
        repo_id: i64,
        run_id: i64,
        output: &ScanOutput,
        source: &SourceSnapshot,
        assets: &[RepoAsset],
        remote: Option<(&GithubRepositoryInfo, &[GithubRemoteFile])>,
    ) -> Result<ScanReviewChanges, Error> {
        let mut connection = self.connect()?;
        let mut transaction = Self::begin_scan_completion(&mut connection, repo_id, run_id)?;
        let previous = Self::previous_completed_scan(&transaction, repo_id, run_id)?;
        let severities = Self::finding_severities(&transaction, previous)?;
        if let (Some(previous), Some(evidence)) = (previous, output.evidence.as_ref()) {
            if !Self::scan_scope_matches_on(&transaction, repo_id, previous, evidence)? {
                let detected: HashSet<_> = output
                    .findings
                    .iter()
                    .map(|finding| finding.fingerprint.as_str())
                    .collect();
                if severities
                    .iter()
                    .any(|(fingerprint, _)| !detected.contains(fingerprint.as_str()))
                {
                    return Err(Error::Scanner("Incomplete scan evidence: The scan scope changed and some previous findings could not be verified. Previous results were retained. Retry with the prior scope.".into()));
                }
            }
        }
        let changes = scan_finding_changes(severities, &output.findings);
        let finished_at = now();
        if let Some((info, files)) = remote {
            if source.revision.as_deref() != Some(info.commit_sha.as_str()) {
                return Err(Error::App(
                    "GitHub scan source revision does not match the downloaded commit".into(),
                ));
            }
            Self::update_github_sync_metadata(&transaction, repo_id, info, files, &finished_at)?;
        }
        self.replace_repo_assets_on(&transaction, repo_id, assets)?;
        let scope = output
            .evidence
            .as_ref()
            .map(|evidence| serde_json::to_string(&evidence.scope))
            .transpose()?;
        let resolved_findings = Self::persist_scan(
            &transaction,
            ScanCompletion {
                repo_id,
                run_id,
                findings: &output.findings,
                packages: &output.packages,
                remediation_evidence: &output.remediation_evidence,
                source_snapshot: source,
                delta: &changes.delta,
            },
            &finished_at,
            previous,
            scope.as_deref(),
        )?;
        let (reopened, review_warning) =
            Self::record_scan_review_result(&mut transaction, repo_id, run_id, &changes)?;
        transaction.commit()?;
        Ok(ScanReviewChanges {
            new_findings: changes.new.len() as u32,
            worsened_findings: changes.worsened.len() as u32 + reopened,
            resolved_findings,
            alerts: changes.delta.new_high_count + changes.delta.new_critical_count,
            review_warning,
        })
    }

    pub(crate) fn scan_scope_matches(
        &self,
        repo_id: i64,
        run_id: i64,
        evidence: &super::scanner::ScanEvidence,
    ) -> Result<bool, Error> {
        Self::scan_scope_matches_on(&self.connect()?, repo_id, run_id, evidence)
    }

    fn scan_scope_matches_on(
        connection: &Connection,
        repo_id: i64,
        run_id: i64,
        evidence: &super::scanner::ScanEvidence,
    ) -> Result<bool, Error> {
        let previous_scope = connection.query_row(
            "SELECT scan_scope FROM scan_runs WHERE repo_id = ?1 AND id = ?2 AND status = 'completed'",
            params![repo_id, run_id],
            |row| row.get::<_, Option<String>>(0),
        ).optional()?;
        Ok(match previous_scope {
            Some(Some(scope)) => evidence.scope.covers(&scope),
            // Older scans lack a scope fingerprint. Compare them only to a
            // scan using Syft's full, default cataloging scope.
            Some(None) => evidence.default_scope,
            None => false,
        })
    }

    fn record_scan_review_result(
        transaction: &mut Transaction<'_>,
        repo_id: i64,
        run_id: i64,
        changes: &super::delta::ScanFindingChanges<'_>,
    ) -> Result<(u32, Option<String>), Error> {
        Self::record_scan_review_changes(transaction, run_id, changes)?;
        // Keep scanner evidence usable even if the review reconciliation fails.
        // Readers cannot consume a successful review transition before it is attributed.
        let mut review = transaction.savepoint()?;
        let (reopened, warning) = match Self::reopen_scan_reviews(&review, repo_id, run_id, None) {
            Ok(reopened) => {
                review.commit()?;
                (reopened, None)
            }
            Err(cause) => {
                review.rollback()?;
                review.commit()?;
                let warning = format!("Scanner results were saved, but Review could not reconcile existing decisions: {cause}");
                Self::append_scan_warnings_on(transaction, run_id, std::slice::from_ref(&warning))?;
                (0, Some(warning))
            }
        };
        transaction.execute(
            "UPDATE scan_runs SET reopened_count=?2 WHERE id=?1 AND reopened_count<>?2",
            params![run_id, changes.worsened.len() as u32 + reopened],
        )?;
        Ok((reopened, warning))
    }

    fn begin_scan_completion(
        connection: &mut Connection,
        repo_id: i64,
        run_id: i64,
    ) -> Result<Transaction<'_>, Error> {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let running: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM scan_runs WHERE id=?1 AND repo_id=?2 AND status='running')",
            params![run_id, repo_id],
            |row| row.get(0),
        )?;
        if !running {
            return Err(scan_run_not_running(repo_id, run_id));
        }
        // Readers see either the preceding evidence/revision or this entire
        // completed scan. Older writers remain covered by the normal triggers.
        transaction.execute(
            "INSERT INTO performance_write_scope(repo_id) VALUES(?1)",
            [repo_id],
        )?;
        Ok(transaction)
    }

    fn persist_scan(
        transaction: &Transaction<'_>,
        input: ScanCompletion<'_>,
        finished_at: &str,
        previous: Option<i64>,
        scope: Option<&str>,
    ) -> Result<u32, Error> {
        let ScanCompletion {
            repo_id,
            run_id,
            findings,
            packages,
            remediation_evidence,
            source_snapshot,
            delta,
        } = input;
        for finding in findings {
            transaction.prepare_cached(
                r#"
	                INSERT INTO vulnerabilities
	                (repo_id, scan_run_id, fingerprint, advisory_id, package_name, package_version, package_type,
	                 package_url, severity, fix_version, title, source_url, advisory_details_json)
	                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                "#,
            )?.execute(params![
                    repo_id,
                    run_id,
                    finding.fingerprint,
                    finding.advisory_id,
                    finding.package_name,
                    finding.package_version,
                    finding.package_type,
                    finding.package_url,
	                    finding.severity.as_str(),
	                    finding.fix_version,
	                    finding.title,
	                    finding.source_url,
	                    finding
	                        .advisory_details
	                        .as_ref()
	                        .map(serde_json::to_string)
	                        .transpose()?
	                ],
	            )?;
        }

        let finding_keys: std::collections::HashSet<&str> = findings
            .iter()
            .map(|finding| finding.fingerprint.as_str())
            .collect();
        for evidence in remediation_evidence
            .iter()
            .filter(|evidence| finding_keys.contains(evidence.fingerprint.as_str()))
        {
            transaction
                .prepare_cached(
                    r#"
                INSERT OR REPLACE INTO vulnerability_remediation_evidence (
                  repo_id, scan_run_id, fingerprint, schema_version, relative_path,
                  location_kind, relationship, parent_package, evidence_source,
                  evidence_quality, location_count
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                "#,
                )?
                .execute(params![
                    repo_id,
                    run_id,
                    evidence.fingerprint,
                    evidence.schema_version,
                    evidence.relative_path,
                    evidence.location_kind.map(|kind| kind.as_str()),
                    evidence.relationship.as_str(),
                    evidence.parent_package,
                    evidence.source.as_str(),
                    match evidence.quality {
                        RemediationEvidenceQuality::Stale => "partial",
                        quality => quality.as_str(),
                    },
                    evidence.location_count.min(32),
                ])?;
        }

        transaction.execute("DELETE FROM scan_packages WHERE repo_id = ?1", [repo_id])?;
        transaction.execute(
            "DELETE FROM scan_package_snapshots WHERE scan_run_id = ?1",
            [run_id],
        )?;
        for package in packages {
            transaction
                .prepare_cached(
                    r#"
                INSERT INTO scan_packages
                (repo_id, scan_run_id, name, version, package_type, package_url)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                )?
                .execute(params![
                    repo_id,
                    run_id,
                    package.name,
                    package.version,
                    package.package_type,
                    package.package_url
                ])?;
            transaction
                .prepare_cached(
                    r#"
                INSERT INTO scan_package_snapshots
                (repo_id, scan_run_id, name, version, package_type, package_url)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                )?
                .execute(params![
                    repo_id,
                    run_id,
                    package.name,
                    package.version,
                    package.package_type,
                    package.package_url
                ])?;
        }

        let (resolved_medium, resolved_high, resolved_critical) =
            Self::scan_resolution_counts(transaction, previous, run_id)?;
        transaction.execute(
            r#"
            UPDATE scan_runs
            SET status = 'completed',
                finished_at = ?2,
                new_high_count = ?3,
                new_critical_count = ?4,
                resolved_high_count = ?5,
                resolved_critical_count = ?6,
                new_medium_count = ?7,
                resolved_medium_count = ?8,
                source_revision = ?9,
                source_dirty = ?10,
                package_count = ?11,
                scan_scope = ?12,
                warnings_json = '[]',
                evidence_complete = 1
            WHERE id = ?1
            "#,
            params![
                run_id,
                finished_at,
                delta.new_high_count,
                delta.new_critical_count,
                resolved_high,
                resolved_critical,
                delta.new_medium_count,
                resolved_medium,
                source_snapshot.revision,
                source_snapshot.dirty,
                packages.len() as i64,
                scope,
            ],
        )?;

        // Package snapshots are intentionally bounded: the latest two successful
        // inventories are sufficient for current/previous comparisons.
        transaction.execute(
            r#"
            DELETE FROM scan_package_snapshots
            WHERE repo_id = ?1
              AND scan_run_id NOT IN (
                SELECT id FROM scan_runs
                WHERE repo_id = ?1 AND status = 'completed'
                ORDER BY finished_at DESC, id DESC
                LIMIT 2
              )
            "#,
            [repo_id],
        )?;

        transaction.execute(
            "UPDATE repos SET last_scan_status = 'completed', last_scanned_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![finished_at, repo_id],
        )?;
        transaction.execute(
            "DELETE FROM performance_write_scope WHERE repo_id=?1",
            [repo_id],
        )?;
        transaction.execute(
            "UPDATE data_revisions SET revision=revision+1 WHERE domain IN ('inventory','scan_history','review','risk')",
            [],
        )?;
        transaction.execute(
            "INSERT INTO repository_data_revisions(repo_id,revision,dependency_revision) VALUES(?1,1,1)
             ON CONFLICT(repo_id) DO UPDATE SET revision=revision+1, dependency_revision=dependency_revision+1",
            [repo_id],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO review_dirty_repos(repo_id) VALUES(?1)",
            [repo_id],
        )?;

        Ok(resolved_medium + resolved_high + resolved_critical)
    }

    pub fn record_scan_warnings(&self, run_id: i64, warnings: &[String]) -> Result<(), Error> {
        Self::append_scan_warnings_on(&self.connect()?, run_id, warnings)
    }

    fn append_scan_warnings_on(
        connection: &Connection,
        run_id: i64,
        warnings: &[String],
    ) -> Result<(), Error> {
        connection.execute(
            "UPDATE scan_runs SET warnings_json=(SELECT json_group_array(value) FROM (
                SELECT value FROM json_each(CASE WHEN json_valid(warnings_json) THEN warnings_json ELSE '[]' END)
                UNION ALL SELECT value FROM json_each(?2)
             )) WHERE id=?1",
            params![run_id, serde_json::to_string(warnings)?],
        )?;
        Ok(())
    }

    pub fn fail_scan(
        &self,
        repo_id: i64,
        run_id: i64,
        error_message: &str,
        source: Option<&SourceSnapshot>,
    ) -> Result<(), Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let timestamp = now();
        let updated = transaction.execute(
            r#"UPDATE scan_runs
               SET status = 'failed', finished_at = ?2, error_message = ?3,
                   source_revision = ?5, source_dirty = ?6,
                   package_count = NULL, warnings_json = '[]', evidence_complete = 0
               WHERE id = ?1 AND repo_id = ?4 AND status = 'running'"#,
            params![
                run_id,
                timestamp,
                error_message,
                repo_id,
                source.and_then(|source| source.revision.as_deref()),
                source.and_then(|source| source.dirty),
            ],
        )?;
        if updated != 1 {
            return Err(scan_run_not_running(repo_id, run_id));
        }
        transaction.execute(
            "UPDATE repos SET last_scan_status = 'failed', updated_at = ?1 WHERE id = ?2",
            params![timestamp, repo_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn latest_completed_findings(&self, repo_id: i64) -> Result<Vec<NormalizedFinding>, Error> {
        let connection = self.connect()?;
        let run_id: Option<i64> = connection
            .query_row(
                "SELECT id FROM scan_runs WHERE repo_id = ?1 AND status = 'completed' ORDER BY finished_at DESC, id DESC LIMIT 1",
                [repo_id],
                |row| row.get(0),
            )
            .optional()?;

        let Some(run_id) = run_id else {
            return Ok(Vec::new());
        };

        let mut statement = connection.prepare(
            r#"
            SELECT fingerprint, advisory_id, package_name, package_version, package_type, package_url, severity, fix_version, title, source_url, advisory_details_json
            FROM vulnerabilities
            WHERE repo_id = ?1 AND scan_run_id = ?2
            ORDER BY advisory_id, package_name
            "#,
        )?;

        let rows = statement.query_map(params![repo_id, run_id], |row| {
            let severity: String = row.get(6)?;
            Ok(NormalizedFinding {
                fingerprint: row.get(0)?,
                advisory_id: row.get(1)?,
                package_name: row.get(2)?,
                package_version: row.get(3)?,
                package_type: row.get(4)?,
                severity: Severity::parse(&severity).unwrap_or(Severity::High),
                fix_version: row.get(7)?,
                title: row.get(8)?,
                source_url: row.get(9)?,
                package_url: row.get(5)?,
                advisory_details: row
                    .get::<_, Option<String>>(10)?
                    .and_then(|value| serde_json::from_str(&value).ok()),
            })
        })?;

        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn previous_completed_scan(
        connection: &Connection,
        repo_id: i64,
        run_id: i64,
    ) -> Result<Option<i64>, Error> {
        Ok(connection
            .query_row(
                "SELECT id FROM scan_runs
                 WHERE repo_id=?1 AND status='completed'
                   AND (started_at,id) < (SELECT started_at,id FROM scan_runs WHERE id=?2)
                 ORDER BY started_at DESC,id DESC LIMIT 1",
                params![repo_id, run_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn finding_severities(
        connection: &Connection,
        run_id: Option<i64>,
    ) -> Result<Vec<(String, Severity)>, Error> {
        let Some(run_id) = run_id else {
            return Ok(Vec::new());
        };
        let mut statement = connection
            .prepare("SELECT fingerprint, severity FROM vulnerabilities WHERE scan_run_id=?1")?;
        let rows = statement.query_map([run_id], |row| {
            let severity: String = row.get(1)?;
            Ok((
                row.get(0)?,
                Severity::parse(&severity).unwrap_or(Severity::High),
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn scan_resolution_counts(
        connection: &Connection,
        previous: Option<i64>,
        current: i64,
    ) -> Result<(u32, u32, u32), Error> {
        let Some(previous) = previous else {
            return Ok((0, 0, 0));
        };
        let detected = scan_has_finding("current", "?2", "v");
        Ok(connection.query_row(
            &format!(
                "SELECT COUNT(*) FILTER (WHERE severity='MEDIUM' COLLATE NOCASE),
                        COUNT(*) FILTER (WHERE severity COLLATE NOCASE NOT IN ('MEDIUM','CRITICAL')),
                        COUNT(*) FILTER (WHERE severity='CRITICAL' COLLATE NOCASE)
                 FROM vulnerabilities v WHERE v.scan_run_id=?1 AND NOT {detected}"
            ),
            params![previous, current],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?)
    }

    pub fn overall_tray_status(&self) -> Result<TrayStatus, Error> {
        Ok(TrayStatus::from_repositories(&self.list_repo_summaries()?))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn snapshot(
        &self,
        schedule: ScheduleInterval,
        scanner: ScannerAvailability,
        discovery_root: String,
        scan_running: bool,
        app_version: String,
        github_auth: GithubAuthSnapshot,
        scan_activity: Option<ScanActivity>,
        next_scheduled_scan_at: Option<String>,
    ) -> Result<AppSnapshot, Error> {
        self.read_reconciled_review(|connection, due, _| {
            let repos = Self::repo_summaries_on(connection, None)?;
            let notifications = Self::list_notifications_on(connection, 50)?;
            let external_intel_summary = Self::external_intel_summary_on(connection)?;
            let external_intel_topics = Self::external_intel_topics_on(connection)?;
            let last_successful_scan_at = repos
                .iter()
                .filter_map(|repo| repo.last_scanned_at.as_ref())
                .max()
                .cloned();
            let scanner_versions = scanner.versions.clone();
            let last_grype_db_update = Self::last_grype_db_update_on(connection)?;
            let onboarding = Self::onboarding_snapshot_on(connection)?;

            Ok(AppSnapshot {
                overview_snapshot: self.overview_snapshot(&repos, &last_successful_scan_at),
                workspace_status: WorkspaceStatus {
                    scanner_ready: scanner.ready,
                    scanner_message: scanner.message.clone(),
                    schedule,
                    last_successful_scan_at: last_successful_scan_at.clone(),
                    next_scheduled_scan_at: next_scheduled_scan_at.clone(),
                    scan_running,
                },
                repos,
                notifications,
                external_intel_summary,
                external_intel_topics,
                discovery_root: discovery_root.clone(),
                version_info: VersionInfo {
                    app: app_version.clone(),
                    syft: scanner_versions
                        .as_ref()
                        .map(|versions| versions.syft.clone()),
                    grype: scanner_versions.map(|versions| versions.grype),
                    grype_db: scanner.grype_db.clone(),
                    last_grype_db_update,
                },
                github_auth: github_auth.clone(),
                onboarding,
                review_summary: self.review_summary_on(connection, due)?,
                scan_activity: scan_activity.clone(),
                data_revisions: Self::read_data_revisions(connection)?,
            })
        })
    }

    fn init(&self) -> Result<(), Error> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut connection = self.connect()?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        restrict_database_file_permissions(&self.path)?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let schema_version: i64 =
            connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if schema_version > SCHEMA_VERSION {
            return Err(Error::App(format!(
                "database schema version {schema_version} is newer than supported version {SCHEMA_VERSION}"
            )));
        }
        connection.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS app_settings (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS companion_privacy_consents (
              provider TEXT PRIMARY KEY,
              disclosure_version INTEGER NOT NULL CHECK(disclosure_version >= 1),
              accepted_at TEXT,
              revoked_at TEXT
            );

            CREATE TABLE IF NOT EXISTS ai_connections (
              id TEXT PRIMARY KEY,
              driver TEXT NOT NULL CHECK(driver IN ('codex', 'grok', 'opencode')),
              display_name TEXT NOT NULL COLLATE NOCASE UNIQUE,
              accent_color TEXT,
              enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
              built_in INTEGER NOT NULL CHECK(built_in IN (0, 1)),
              config_version INTEGER NOT NULL CHECK(config_version >= 1),
              config_json TEXT NOT NULL,
              has_server_password INTEGER NOT NULL DEFAULT 0 CHECK(has_server_password IN (0, 1)),
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS ai_connection_consents (
              connection_id TEXT NOT NULL,
              boundary_fingerprint TEXT NOT NULL,
              disclosure_version INTEGER NOT NULL CHECK(disclosure_version >= 1),
              accepted_at TEXT,
              revoked_at TEXT,
              PRIMARY KEY(connection_id, boundary_fingerprint)
            );

            CREATE TABLE IF NOT EXISTS repos (
              id INTEGER PRIMARY KEY,
              name TEXT NOT NULL,
              path TEXT NOT NULL UNIQUE,
              is_active INTEGER NOT NULL DEFAULT 1,
              source_type TEXT NOT NULL DEFAULT 'local',
              source_key TEXT,
              last_scan_status TEXT NOT NULL DEFAULT 'unknown',
              last_scanned_at TEXT,
              remote_url TEXT,
              remote_host TEXT,
              provider TEXT,
              default_branch TEXT,
              last_remote_commit_sha TEXT,
              last_remote_synced_at TEXT,
              remote_file_count INTEGER,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS companion_drafts (
              repository_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
              scope TEXT NOT NULL CHECK(length(scope) BETWEEN 1 AND 1024),
              text TEXT NOT NULL CHECK(length(text) <= 65536),
              PRIMARY KEY(repository_id, scope)
            );

            CREATE TABLE IF NOT EXISTS companion_threads (
              id INTEGER PRIMARY KEY,
              repository_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
              provider TEXT NOT NULL CHECK(provider IN ('codex', 'grok', 'opencode')),
              provider_driver TEXT NOT NULL DEFAULT 'codex',
              provider_thread_id TEXT NOT NULL,
              title TEXT NOT NULL,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              last_opened_at TEXT NOT NULL,
              archived_at TEXT,
              context_version INTEGER NOT NULL CHECK(context_version >= 1),
              last_model_id TEXT,
              provider_connection_id TEXT NOT NULL DEFAULT 'codex_default',
              provider_turn_started INTEGER NOT NULL DEFAULT 0 CHECK(provider_turn_started IN (0, 1)),
              UNIQUE(provider, provider_thread_id)
            );

            CREATE TABLE IF NOT EXISTS companion_provider_segments (
              id INTEGER PRIMARY KEY,
              thread_id INTEGER NOT NULL REFERENCES companion_threads(id) ON DELETE CASCADE,
              ordinal INTEGER NOT NULL CHECK(ordinal >= 1),
              provider TEXT NOT NULL CHECK(provider IN ('codex', 'grok', 'opencode')),
              connection_id TEXT NOT NULL,
              provider_thread_id TEXT NOT NULL,
              initial_model_id TEXT,
              last_model_id TEXT,
              status TEXT NOT NULL DEFAULT 'ready'
                CHECK(status IN ('ready', 'active', 'interrupted', 'history_unavailable')),
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              ended_at TEXT,
              UNIQUE(thread_id, ordinal),
              UNIQUE(provider, provider_thread_id)
            );

            CREATE TABLE IF NOT EXISTS companion_timeline_items (
              id INTEGER PRIMARY KEY,
              thread_id INTEGER NOT NULL REFERENCES companion_threads(id) ON DELETE CASCADE,
              provider_segment_id INTEGER NOT NULL REFERENCES companion_provider_segments(id) ON DELETE CASCADE,
              item_key TEXT NOT NULL CHECK(length(item_key) BETWEEN 1 AND 512),
              sequence INTEGER NOT NULL CHECK(sequence >= 1),
              item_json TEXT NOT NULL CHECK(length(item_json) <= 2097152),
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              UNIQUE(thread_id, provider_segment_id, item_key)
            );

            CREATE TABLE IF NOT EXISTS companion_cached_events (
              id INTEGER PRIMARY KEY,
              provider_segment_id INTEGER NOT NULL REFERENCES companion_provider_segments(id) ON DELETE CASCADE,
              event_key TEXT NOT NULL CHECK(length(event_key) BETWEEN 1 AND 128),
              created_at TEXT NOT NULL,
              UNIQUE(provider_segment_id, event_key)
            );

            CREATE TABLE IF NOT EXISTS scan_runs (
              id INTEGER PRIMARY KEY,
              repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
              status TEXT NOT NULL,
              started_at TEXT NOT NULL,
              finished_at TEXT,
              scanner_versions TEXT,
              new_medium_count INTEGER NOT NULL DEFAULT 0,
              new_high_count INTEGER NOT NULL DEFAULT 0,
              new_critical_count INTEGER NOT NULL DEFAULT 0,
              resolved_medium_count INTEGER NOT NULL DEFAULT 0,
              resolved_high_count INTEGER NOT NULL DEFAULT 0,
              resolved_critical_count INTEGER NOT NULL DEFAULT 0,
              error_message TEXT,
              source_revision TEXT,
              source_dirty INTEGER CHECK(source_dirty IN (0, 1) OR source_dirty IS NULL),
              package_count INTEGER,
              reopened_count INTEGER NOT NULL DEFAULT 0,
              warnings_json TEXT NOT NULL DEFAULT '[]',
              scan_scope TEXT,
              evidence_complete INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS vulnerabilities (
              id INTEGER PRIMARY KEY,
              repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
              scan_run_id INTEGER NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
              fingerprint TEXT NOT NULL,
              advisory_id TEXT NOT NULL,
              package_name TEXT NOT NULL,
              package_version TEXT,
              package_type TEXT,
              package_url TEXT,
              severity TEXT NOT NULL,
	              fix_version TEXT,
	              title TEXT,
	              source_url TEXT,
	              advisory_details_json TEXT,
	              UNIQUE(scan_run_id, fingerprint)
	            );

            CREATE TABLE IF NOT EXISTS vulnerability_remediation_evidence (
              repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
              scan_run_id INTEGER NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
              fingerprint TEXT NOT NULL,
              schema_version INTEGER NOT NULL CHECK(schema_version >= 1),
              relative_path TEXT,
              location_kind TEXT CHECK(location_kind IN ('manifest', 'lockfile', 'other') OR location_kind IS NULL),
              relationship TEXT NOT NULL CHECK(relationship IN ('direct', 'transitive', 'unknown')),
              parent_package TEXT,
              evidence_source TEXT NOT NULL CHECK(evidence_source IN ('syft_location', 'syft_relationship', 'supported_manifest', 'none')),
              evidence_quality TEXT NOT NULL CHECK(evidence_quality IN ('current', 'partial', 'unavailable')),
              location_count INTEGER NOT NULL DEFAULT 0 CHECK(location_count >= 0 AND location_count <= 32),
              PRIMARY KEY (repo_id, scan_run_id, fingerprint),
              FOREIGN KEY (scan_run_id, fingerprint) REFERENCES vulnerabilities(scan_run_id, fingerprint) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS scan_packages (
              id INTEGER PRIMARY KEY,
              repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
              scan_run_id INTEGER NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
              name TEXT NOT NULL,
              version TEXT,
              package_type TEXT,
              package_url TEXT,
              created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
              UNIQUE(repo_id, name, version, package_type, package_url)
            );

            CREATE TABLE IF NOT EXISTS scan_package_snapshots (
              id INTEGER PRIMARY KEY,
              repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
              scan_run_id INTEGER NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
              name TEXT NOT NULL,
              version TEXT,
              package_type TEXT,
              package_url TEXT,
              created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS notifications (
              id INTEGER PRIMARY KEY,
              kind TEXT NOT NULL,
              tone TEXT NOT NULL,
              title TEXT NOT NULL,
              message TEXT NOT NULL,
              repo_id INTEGER REFERENCES repos(id) ON DELETE SET NULL,
              repo_name TEXT,
              scan_run_id INTEGER REFERENCES scan_runs(id) ON DELETE SET NULL,
              created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS finding_reviews (
              repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
              fingerprint TEXT NOT NULL,
              state TEXT NOT NULL CHECK(state IN ('reviewed', 'snoozed')),
              reviewed_at TEXT,
              snooze_until TEXT,
              note TEXT,
              basis_severity TEXT NOT NULL,
              basis_fix_version TEXT,
              basis_known_exploited INTEGER NOT NULL DEFAULT 0,
              basis_ransomware_known INTEGER NOT NULL DEFAULT 0,
              basis_scanner_state TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              transition_reason TEXT,
              PRIMARY KEY(repo_id, fingerprint)
            );

            CREATE TABLE IF NOT EXISTS scan_review_changes (
              scan_run_id INTEGER NOT NULL,
              fingerprint TEXT NOT NULL,
              kind TEXT NOT NULL CHECK(kind IN ('new', 'severity_increased', 'reopened')),
              PRIMARY KEY (scan_run_id, fingerprint),
              FOREIGN KEY (scan_run_id, fingerprint)
                REFERENCES vulnerabilities(scan_run_id, fingerprint) ON DELETE CASCADE
            );

            DROP TABLE IF EXISTS external_intel_matches;
            DROP TABLE IF EXISTS external_intel_aliases;
            DROP TABLE IF EXISTS external_intel_items;

            CREATE TABLE IF NOT EXISTS external_advisories (
              id INTEGER PRIMARY KEY,
              canonical_key TEXT NOT NULL UNIQUE,
              title TEXT NOT NULL,
              summary TEXT NOT NULL,
              url TEXT NOT NULL,
              ecosystem TEXT,
              package_name TEXT,
              severity TEXT NOT NULL,
              published_at TEXT,
              updated_at TEXT,
              modified_at TEXT,
              withdrawn_at TEXT,
              cvss_score TEXT,
              cvss_vector TEXT,
              cvss_version TEXT,
              epss_percentage TEXT,
              epss_percentile TEXT,
              is_kev INTEGER NOT NULL DEFAULT 0,
              is_ransomware_known INTEGER NOT NULL DEFAULT 0,
              primary_source TEXT NOT NULL,
              source_item_id TEXT NOT NULL,
              raw_primary_json TEXT NOT NULL,
              product_keys_json TEXT NOT NULL DEFAULT '[]',
              topic_keys_json TEXT NOT NULL DEFAULT '[]',
              relevance TEXT NOT NULL DEFAULT 'general',
              fetched_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS external_advisory_sources (
              id INTEGER PRIMARY KEY,
              advisory_id INTEGER NOT NULL REFERENCES external_advisories(id) ON DELETE CASCADE,
              source TEXT NOT NULL,
              source_item_id TEXT NOT NULL,
              url TEXT NOT NULL,
              published_at TEXT,
              updated_at TEXT,
              severity TEXT NOT NULL,
              cvss_score TEXT,
              epss_percentage TEXT,
              reference_count INTEGER NOT NULL DEFAULT 0,
              raw_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS repo_assets (
              id INTEGER PRIMARY KEY,
              repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
              vendor TEXT NOT NULL,
              product TEXT NOT NULL,
              version TEXT,
              asset_kind TEXT NOT NULL,
              source_path TEXT,
              evidence TEXT NOT NULL,
              confidence INTEGER NOT NULL,
              created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS repo_remote_files (
              id INTEGER PRIMARY KEY,
              repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
              path TEXT NOT NULL,
              blob_sha TEXT NOT NULL,
              size INTEGER NOT NULL,
              seen_at TEXT NOT NULL,
              UNIQUE(repo_id, path)
            );

            CREATE TABLE IF NOT EXISTS external_advisory_aliases (
              id INTEGER PRIMARY KEY,
              advisory_id INTEGER NOT NULL REFERENCES external_advisories(id) ON DELETE CASCADE,
              alias TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS external_advisory_affected_packages (
              id INTEGER PRIMARY KEY,
              advisory_id INTEGER NOT NULL REFERENCES external_advisories(id) ON DELETE CASCADE,
              ecosystem TEXT,
              package_name TEXT,
              package_purl TEXT,
              vulnerable_range TEXT,
              first_patched_version TEXT,
              affected_versions_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS external_advisory_references (
              id INTEGER PRIMARY KEY,
              advisory_id INTEGER NOT NULL REFERENCES external_advisories(id) ON DELETE CASCADE,
              url TEXT NOT NULL,
              source TEXT,
              kind TEXT,
              tags_json TEXT NOT NULL DEFAULT '[]'
            );

            CREATE TABLE IF NOT EXISTS external_advisory_cwes (
              id INTEGER PRIMARY KEY,
              advisory_id INTEGER NOT NULL REFERENCES external_advisories(id) ON DELETE CASCADE,
              cwe_id TEXT NOT NULL,
              name TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS external_advisory_kev (
              id INTEGER PRIMARY KEY,
              advisory_id INTEGER NOT NULL REFERENCES external_advisories(id) ON DELETE CASCADE,
              date_added TEXT,
              due_date TEXT,
              required_action TEXT,
              known_ransomware_campaign_use TEXT,
              notes TEXT,
              catalog_version TEXT,
              catalog_released_at TEXT
            );

            CREATE TABLE IF NOT EXISTS external_advisory_products (
              id INTEGER PRIMARY KEY,
              advisory_id INTEGER NOT NULL REFERENCES external_advisories(id) ON DELETE CASCADE,
              vendor TEXT NOT NULL,
              product TEXT NOT NULL,
              product_key TEXT NOT NULL,
              topic_key TEXT
            );

            CREATE TABLE IF NOT EXISTS external_advisory_matches (
              id INTEGER PRIMARY KEY,
              advisory_id INTEGER NOT NULL REFERENCES external_advisories(id) ON DELETE CASCADE,
              match_kind TEXT NOT NULL,
              repo_count INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_scan_runs_repo_started_at ON scan_runs(repo_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_companion_threads_repo_opened ON companion_threads(repository_id, archived_at, last_opened_at DESC);
            CREATE INDEX IF NOT EXISTS idx_companion_segments_thread_ordinal ON companion_provider_segments(thread_id, ordinal);
            CREATE INDEX IF NOT EXISTS idx_companion_timeline_thread_sequence ON companion_timeline_items(thread_id, sequence);
            CREATE INDEX IF NOT EXISTS idx_vulns_repo_scan ON vulnerabilities(repo_id, scan_run_id);
            CREATE INDEX IF NOT EXISTS idx_vulns_repo_fp ON vulnerabilities(repo_id, fingerprint);
            CREATE INDEX IF NOT EXISTS idx_vulns_run_advisory_package
                ON vulnerabilities(scan_run_id, advisory_id COLLATE NOCASE, package_name);
            CREATE INDEX IF NOT EXISTS idx_remediation_evidence_lookup ON vulnerability_remediation_evidence(repo_id, fingerprint, scan_run_id);
            CREATE INDEX IF NOT EXISTS idx_scan_packages_repo ON scan_packages(repo_id);
            CREATE INDEX IF NOT EXISTS idx_scan_packages_repo_purl ON scan_packages(repo_id, package_url);
            CREATE INDEX IF NOT EXISTS idx_scan_package_snapshots_repo_run ON scan_package_snapshots(repo_id, scan_run_id);
            CREATE INDEX IF NOT EXISTS idx_notifications_created_at ON notifications(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_finding_reviews_state_due ON finding_reviews(state, snooze_until);
            CREATE INDEX IF NOT EXISTS idx_external_advisories_published_at ON external_advisories(published_at DESC);
            CREATE INDEX IF NOT EXISTS idx_external_advisory_aliases_alias ON external_advisory_aliases(alias);
            CREATE INDEX IF NOT EXISTS idx_external_advisory_aliases_advisory ON external_advisory_aliases(advisory_id);
            CREATE INDEX IF NOT EXISTS idx_external_advisory_sources_source ON external_advisory_sources(source);
            CREATE INDEX IF NOT EXISTS idx_external_advisory_products_key ON external_advisory_products(product_key);
            CREATE INDEX IF NOT EXISTS idx_external_advisory_matches_kind ON external_advisory_matches(match_kind);
            CREATE INDEX IF NOT EXISTS idx_repo_assets_repo ON repo_assets(repo_id);
            CREATE INDEX IF NOT EXISTS idx_repo_assets_product ON repo_assets(vendor, product);
            CREATE INDEX IF NOT EXISTS idx_repo_remote_files_repo ON repo_remote_files(repo_id);
            "#,
        )?;
        self.ensure_repo_columns(&connection)?;
        self.ensure_scan_run_columns(&connection)?;
        self.ensure_notification_columns(&connection)?;
        self.ensure_vulnerability_columns(&connection)?;
        self.ensure_external_intel_columns(&connection)?;
        self.ensure_companion_thread_columns(&connection)?;
        self.ensure_provider_driver_constraints(&mut connection)?;
        self.ensure_ai_connections(&mut connection)?;
        self.ensure_companion_conversation_cache(&mut connection)?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        self.recover_interrupted_scans(&mut connection)?;
        self.init_data_revisions(&connection)?;
        Self::reconcile_scan_resolution_counts(&mut connection)?;
        restrict_database_file_permissions(&self.path)?;
        Ok(())
    }

    fn reconcile_scan_resolution_counts(connection: &mut Connection) -> Result<(), Error> {
        const KEY: &str = "scan_resolution_counts_by_package_v1";
        let done = |connection: &Connection| {
            connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM performance_metadata WHERE key=?1)",
                [KEY],
                |row| row.get::<_, bool>(0),
            )
        };
        if done(connection)? {
            return Ok(());
        }

        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !done(&transaction)? {
            // Recompute derived counts from the saved findings of complete scans.
            let detected = scan_has_finding("current", "s.id", "v");
            transaction.execute_batch(&format!(
                "WITH resolved AS MATERIALIZED (
                     SELECT s.id, COUNT(*) FILTER (WHERE v.severity='MEDIUM' COLLATE NOCASE) medium,
                                  COUNT(*) FILTER (WHERE v.severity COLLATE NOCASE NOT IN ('MEDIUM','CRITICAL')) high,
                                  COUNT(*) FILTER (WHERE v.severity='CRITICAL' COLLATE NOCASE) critical
                     FROM scan_runs s JOIN vulnerabilities v ON v.scan_run_id=(
                         SELECT p.id FROM scan_runs p WHERE p.repo_id=s.repo_id AND p.status='completed'
                           AND (p.started_at,p.id)<(s.started_at,s.id)
                         ORDER BY p.started_at DESC,p.id DESC LIMIT 1
                     )
                     WHERE s.status='completed' AND s.evidence_complete=1 AND NOT {detected}
                     GROUP BY s.id
                 )
                 UPDATE scan_runs AS s SET
                     resolved_medium_count=COALESCE(r.medium,0),
                     resolved_high_count=COALESCE(r.high,0),
                     resolved_critical_count=COALESCE(r.critical,0)
                 FROM (SELECT id FROM scan_runs WHERE status='completed' AND evidence_complete=1) eligible
                 LEFT JOIN resolved r USING(id)
                 WHERE s.id=eligible.id AND (s.resolved_medium_count,s.resolved_high_count,s.resolved_critical_count)
                     != (COALESCE(r.medium,0),COALESCE(r.high,0),COALESCE(r.critical,0))",
            ))?;
            transaction.execute(
                "INSERT INTO performance_metadata(key,value) VALUES(?1,1)",
                [KEY],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn recover_interrupted_scans(&self, connection: &mut Connection) -> Result<(), Error> {
        let transaction = connection.transaction()?;
        let timestamp = now();
        transaction.execute(
            r#"UPDATE repos SET last_scan_status = 'failed', updated_at = ?1
               WHERE id IN (SELECT repo_id FROM scan_runs WHERE status = 'running')"#,
            [&timestamp],
        )?;
        transaction.execute(
            r#"UPDATE scan_runs SET status = 'failed', finished_at = ?1, error_message = ?2
               WHERE status = 'running'"#,
            params![timestamp, INTERRUPTED_SCAN_ERROR],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn interrupted_scan_activity(&self) -> Result<Option<ScanActivity>, Error> {
        let connection = self.connect()?;
        // Only use the latest attempt. A later successful run supersedes the
        // interrupted activity, while the failure stays in its scan history.
        let mut statement = connection.prepare(
            r#"SELECT r.id, r.name, s.id, s.started_at, s.finished_at
               FROM repos r
               JOIN scan_runs s ON s.id=(
                   SELECT id FROM scan_runs WHERE repo_id=r.id ORDER BY started_at DESC,id DESC LIMIT 1
               )
               WHERE r.is_active=1 AND s.status='failed' AND s.error_message=?1
                 AND s.finished_at IS NOT NULL
               ORDER BY s.finished_at DESC,s.started_at DESC,s.id DESC"#,
        )?;
        let mut rows = statement.query([INTERRUPTED_SCAN_ERROR])?;
        let mut outcomes = Vec::new();
        let mut started_at = String::new();
        let mut finished_at = String::new();
        while let Some(row) = rows.next()? {
            let started: String = row.get(3)?;
            let finished: String = row.get(4)?;
            if outcomes.is_empty() {
                finished_at = finished;
            } else if finished != finished_at {
                break;
            }
            started_at = started;
            outcomes.push(ScanRepositoryOutcome {
                repo_id: row.get(0)?,
                repo_name: row.get(1)?,
                scan_run_id: Some(row.get(2)?),
                succeeded: false,
                error: Some(INTERRUPTED_SCAN_ERROR.into()),
                new_findings: 0,
                resolved_findings: 0,
                worsened_findings: 0,
            });
        }
        let Some(ScanRepositoryOutcome {
            repo_id,
            repo_name,
            scan_run_id: Some(latest_run_id),
            ..
        }) = outcomes.first()
        else {
            return Ok(None);
        };
        let scope_repo_id = (outcomes.len() == 1).then_some(*repo_id);
        Ok(Some(ScanActivity {
            id: format!("interrupted-{latest_run_id}"),
            sequence: 1,
            status: ScanActivityStatus::Failed,
            stage: ScanActivityStage::Failed,
            interrupted: true,
            scope_repo_id,
            started_at,
            finished_at: Some(finished_at),
            total_repositories: outcomes.len() as u32,
            completed_repositories: 0,
            failed_repositories: outcomes.len() as u32,
            current_repo_id: scope_repo_id,
            current_repo_name: scope_repo_id.map(|_| repo_name.clone()),
            outcomes,
            warnings: Vec::new(),
            new_findings: 0,
            resolved_findings: 0,
            worsened_findings: 0,
            terminal_error: None,
        }))
    }

    fn ensure_repo_columns(&self, connection: &Connection) -> Result<(), Error> {
        let mut statement = connection.prepare("PRAGMA table_info(repos)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;

        for (column, definition) in [
            ("remote_url", "TEXT"),
            ("remote_host", "TEXT"),
            ("provider", "TEXT"),
            ("last_scanned_at", "TEXT"),
            ("source_type", "TEXT NOT NULL DEFAULT 'local'"),
            ("source_key", "TEXT"),
            ("default_branch", "TEXT"),
            ("last_remote_commit_sha", "TEXT"),
            ("last_remote_synced_at", "TEXT"),
            ("remote_file_count", "INTEGER"),
        ] {
            if columns.iter().any(|existing| existing == column) {
                continue;
            }

            connection.execute(
                &format!("ALTER TABLE repos ADD COLUMN {column} {definition}"),
                [],
            )?;
        }

        connection.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_repos_source_key ON repos(source_key) WHERE source_key IS NOT NULL",
            [],
        )?;

        Ok(())
    }

    fn ensure_companion_thread_columns(&self, connection: &Connection) -> Result<(), Error> {
        let mut statement = connection.prepare("PRAGMA table_info(companion_threads)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        if !columns
            .iter()
            .any(|column| column == "provider_turn_started")
        {
            connection.execute(
                "ALTER TABLE companion_threads ADD COLUMN provider_turn_started INTEGER NOT NULL DEFAULT 1 CHECK(provider_turn_started IN (0, 1))",
                [],
            )?;
            connection.execute(
                "UPDATE companion_threads SET provider_turn_started = 0 WHERE updated_at = created_at",
                [],
            )?;
        }
        if !columns
            .iter()
            .any(|column| column == "provider_connection_id")
        {
            connection.execute(
                "ALTER TABLE companion_threads ADD COLUMN provider_connection_id TEXT NOT NULL DEFAULT 'codex_default'",
                [],
            )?;
        }
        if !columns.iter().any(|column| column == "provider_driver") {
            connection.execute(
                "ALTER TABLE companion_threads ADD COLUMN provider_driver TEXT NOT NULL DEFAULT 'codex'",
                [],
            )?;
        }
        Ok(())
    }

    fn ensure_companion_conversation_cache(
        &self,
        connection: &mut Connection,
    ) -> Result<(), Error> {
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS companion_provider_segments (
              id INTEGER PRIMARY KEY,
              thread_id INTEGER NOT NULL REFERENCES companion_threads(id) ON DELETE CASCADE,
              ordinal INTEGER NOT NULL CHECK(ordinal >= 1),
              provider TEXT NOT NULL CHECK(provider IN ('codex', 'grok', 'opencode')),
              connection_id TEXT NOT NULL,
              provider_thread_id TEXT NOT NULL,
              initial_model_id TEXT,
              last_model_id TEXT,
              status TEXT NOT NULL DEFAULT 'ready'
                CHECK(status IN ('ready', 'active', 'interrupted', 'history_unavailable')),
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              ended_at TEXT,
              UNIQUE(thread_id, ordinal),
              UNIQUE(provider, provider_thread_id)
            );
            CREATE TABLE IF NOT EXISTS companion_timeline_items (
              id INTEGER PRIMARY KEY,
              thread_id INTEGER NOT NULL REFERENCES companion_threads(id) ON DELETE CASCADE,
              provider_segment_id INTEGER NOT NULL REFERENCES companion_provider_segments(id) ON DELETE CASCADE,
              item_key TEXT NOT NULL CHECK(length(item_key) BETWEEN 1 AND 512),
              sequence INTEGER NOT NULL CHECK(sequence >= 1),
              item_json TEXT NOT NULL CHECK(length(item_json) <= 2097152),
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              UNIQUE(thread_id, provider_segment_id, item_key)
            );
            CREATE TABLE IF NOT EXISTS companion_cached_events (
              id INTEGER PRIMARY KEY,
              provider_segment_id INTEGER NOT NULL REFERENCES companion_provider_segments(id) ON DELETE CASCADE,
              event_key TEXT NOT NULL CHECK(length(event_key) BETWEEN 1 AND 128),
              created_at TEXT NOT NULL,
              UNIQUE(provider_segment_id, event_key)
            );
            CREATE INDEX IF NOT EXISTS idx_companion_segments_thread_ordinal
              ON companion_provider_segments(thread_id, ordinal);
            CREATE INDEX IF NOT EXISTS idx_companion_timeline_thread_sequence
              ON companion_timeline_items(thread_id, sequence);
            CREATE INDEX IF NOT EXISTS idx_companion_cached_events_segment
              ON companion_cached_events(provider_segment_id, id);
            "#,
        )?;
        transaction.execute(
            r#"
            INSERT INTO companion_provider_segments (
              thread_id, ordinal, provider, connection_id, provider_thread_id,
              initial_model_id, last_model_id, status, created_at, updated_at
            )
            SELECT id, 1, provider_driver, provider_connection_id, provider_thread_id,
                   last_model_id, last_model_id,
                   CASE WHEN provider_turn_started = 1 THEN 'ready' ELSE 'ready' END,
                   created_at, updated_at
            FROM companion_threads
            WHERE NOT EXISTS (
              SELECT 1 FROM companion_provider_segments segment WHERE segment.thread_id = companion_threads.id
            )
            "#,
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn ensure_ai_connections(&self, connection: &mut Connection) -> Result<(), Error> {
        let transaction = connection.transaction()?;
        let timestamp = now_timestamp();
        for mut default in ProviderConnection::builtin_defaults(timestamp.clone()) {
            let exists = transaction
                .query_row(
                    "SELECT 1 FROM ai_connections WHERE id = ?1",
                    [default.id.as_ref()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if exists {
                continue;
            }
            default.display_name = available_builtin_display(
                &transaction,
                default.id.as_ref(),
                &default.display_name,
            )?;
            transaction.execute(
                r#"
                INSERT INTO ai_connections (
                  id, driver, display_name, accent_color, enabled, built_in,
                  config_version, config_json, has_server_password, created_at, updated_at
                ) VALUES (?1, ?2, ?3, NULL, 1, 1, ?4, ?5, 0, ?6, ?6)
                "#,
                params![
                    default.id.as_ref(),
                    default.driver.as_str(),
                    default.display_name,
                    PROVIDER_CONNECTION_SCHEMA_VERSION,
                    config_to_json(&default.config)?,
                    timestamp,
                ],
            )?;
        }
        transaction.execute(
            "UPDATE companion_threads SET provider_connection_id = ?1 WHERE provider_connection_id IS NULL OR trim(provider_connection_id) = ''",
            [DEFAULT_CODEX_CONNECTION_ID],
        )?;
        transaction.execute(
            r#"
            INSERT INTO ai_connection_consents (
              connection_id, boundary_fingerprint, disclosure_version, accepted_at, revoked_at
            )
            SELECT ?1, 'v1:codex_default:codex:openai:external',
                   disclosure_version, accepted_at, revoked_at
            FROM companion_privacy_consents
            WHERE provider = 'openai_codex'
            ON CONFLICT(connection_id, boundary_fingerprint) DO NOTHING
            "#,
            [DEFAULT_CODEX_CONNECTION_ID],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn ensure_provider_driver_constraints(&self, connection: &mut Connection) -> Result<(), Error> {
        let ai_sql: String = connection.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'ai_connections'",
            [],
            |row| row.get(0),
        )?;
        if !ai_sql.contains("'grok'") {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE ai_connections_next (
                  id TEXT PRIMARY KEY,
                  driver TEXT NOT NULL CHECK(driver IN ('codex', 'grok', 'opencode')),
                  display_name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                  accent_color TEXT,
                  enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
                  built_in INTEGER NOT NULL CHECK(built_in IN (0, 1)),
                  config_version INTEGER NOT NULL CHECK(config_version >= 1),
                  config_json TEXT NOT NULL,
                  has_server_password INTEGER NOT NULL DEFAULT 0 CHECK(has_server_password IN (0, 1)),
                  created_at TEXT NOT NULL,
                  updated_at TEXT NOT NULL
                );
                INSERT INTO ai_connections_next
                SELECT id, driver, display_name, accent_color, enabled, built_in,
                       config_version, config_json, has_server_password, created_at, updated_at
                FROM ai_connections;
                DROP TABLE ai_connections;
                ALTER TABLE ai_connections_next RENAME TO ai_connections;
                "#,
            )?;
            transaction.commit()?;
        }

        let thread_sql: String = connection.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'companion_threads'",
            [],
            |row| row.get(0),
        )?;
        if !thread_sql.contains("'grok'") || !thread_sql.contains("'opencode'") {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                r#"
                CREATE TABLE companion_threads_next (
                  id INTEGER PRIMARY KEY,
                  repository_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
                  provider TEXT NOT NULL CHECK(provider IN ('codex', 'grok', 'opencode')),
                  provider_driver TEXT NOT NULL DEFAULT 'codex',
                  provider_thread_id TEXT NOT NULL,
                  title TEXT NOT NULL,
                  created_at TEXT NOT NULL,
                  updated_at TEXT NOT NULL,
                  last_opened_at TEXT NOT NULL,
                  archived_at TEXT,
                  context_version INTEGER NOT NULL CHECK(context_version >= 1),
                  last_model_id TEXT,
                  provider_connection_id TEXT NOT NULL DEFAULT 'codex_default',
                  provider_turn_started INTEGER NOT NULL DEFAULT 0 CHECK(provider_turn_started IN (0, 1)),
                  UNIQUE(provider, provider_thread_id)
                );
                INSERT INTO companion_threads_next (
                  id, repository_id, provider, provider_driver, provider_thread_id, title,
                  created_at, updated_at, last_opened_at, archived_at, context_version,
                  last_model_id, provider_connection_id, provider_turn_started
                )
                SELECT id, repository_id, provider, provider_driver, provider_thread_id, title,
                       created_at, updated_at, last_opened_at, archived_at, context_version,
                       last_model_id, provider_connection_id, provider_turn_started
                FROM companion_threads;
                DROP TABLE companion_threads;
                ALTER TABLE companion_threads_next RENAME TO companion_threads;
                CREATE INDEX IF NOT EXISTS idx_companion_threads_repo_opened
                  ON companion_threads(repository_id, archived_at, last_opened_at DESC);
                "#,
            )?;
            transaction.commit()?;
        }
        Ok(())
    }

    fn ensure_notification_columns(&self, connection: &Connection) -> Result<(), Error> {
        let has_scan_run_id: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('notifications') WHERE name='scan_run_id')",
            [],
            |row| row.get(0),
        )?;
        if !has_scan_run_id {
            connection.execute_batch(
                "ALTER TABLE notifications ADD COLUMN scan_run_id INTEGER REFERENCES scan_runs(id) ON DELETE SET NULL;",
            )?;
        }
        Ok(())
    }

    fn ensure_scan_run_columns(&self, connection: &Connection) -> Result<(), Error> {
        let mut statement = connection.prepare("PRAGMA table_info(scan_runs)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;

        for (column, definition) in [
            ("new_medium_count", "INTEGER NOT NULL DEFAULT 0"),
            ("resolved_medium_count", "INTEGER NOT NULL DEFAULT 0"),
            ("source_revision", "TEXT"),
            ("source_dirty", "INTEGER"),
            ("package_count", "INTEGER"),
            ("reopened_count", "INTEGER NOT NULL DEFAULT 0"),
            ("warnings_json", "TEXT NOT NULL DEFAULT '[]'"),
            ("scan_scope", "TEXT"),
            ("evidence_complete", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            if columns.iter().any(|existing| existing == column) {
                continue;
            }

            connection.execute(
                &format!("ALTER TABLE scan_runs ADD COLUMN {column} {definition}"),
                [],
            )?;
        }

        Ok(())
    }

    fn ensure_vulnerability_columns(&self, connection: &Connection) -> Result<(), Error> {
        let mut statement = connection.prepare("PRAGMA table_info(vulnerabilities)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;

        for (column, definition) in [("package_url", "TEXT"), ("advisory_details_json", "TEXT")] {
            if columns.iter().any(|existing| existing == column) {
                continue;
            }
            connection.execute(
                &format!("ALTER TABLE vulnerabilities ADD COLUMN {column} {definition}"),
                [],
            )?;
        }

        Ok(())
    }

    fn ensure_external_intel_columns(&self, connection: &Connection) -> Result<(), Error> {
        connection.execute("DROP TABLE IF EXISTS external_intel_matches", [])?;
        connection.execute("DROP TABLE IF EXISTS external_intel_aliases", [])?;
        connection.execute("DROP TABLE IF EXISTS external_intel_items", [])?;
        Ok(())
    }

    fn connect(&self) -> Result<Connection, Error> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(StdDuration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.create_scalar_function(
            "cvec_package_identity",
            4,
            rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                Ok(package_identity(
                    context.get_raw(0).as_str_or_null()?,
                    context.get_raw(1).as_str()?,
                    context.get_raw(2).as_str_or_null()?,
                    context.get_raw(3).as_str_or_null()?,
                ))
            },
        )?;
        connection.create_scalar_function(
            "cvec_same_scan_package",
            4,
            rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                Ok(same_scan_package(
                    context.get_raw(0).as_str_or_null()?,
                    context.get_raw(1).as_str_or_null()?,
                    context.get_raw(2).as_str_or_null()?,
                    context.get_raw(3).as_str_or_null()?,
                ))
            },
        )?;
        connection.create_scalar_function(
            "cvec_contains",
            2,
            rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                let haystack: String = context.get(0)?;
                let needle: String = context.get(1)?;
                Ok(haystack.to_lowercase().contains(&needle.to_lowercase()))
            },
        )?;
        connection.create_scalar_function(
            "cvec_text_score",
            2,
            rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                let read_text = |idx| {
                    let value = context.get_raw(idx);
                    value.as_str().map_err(|_| {
                        rusqlite::Error::InvalidFunctionParameterType(idx, value.data_type())
                    })
                };
                // Local entity search normalizes its query once, before SQL.
                Ok(text_score(read_text(0)?, read_text(1)?))
            },
        )?;
        Ok(connection)
    }

    fn list_external_source_details(
        &self,
        connection: &Connection,
        advisory_id: i64,
    ) -> Result<Vec<ExternalIntelSourceDetail>, Error> {
        let mut statement = connection.prepare(
            r#"
            SELECT source, source_item_id, url, published_at, updated_at, severity,
                   cvss_score, epss_percentage, reference_count
            FROM external_advisory_sources
            WHERE advisory_id = ?1
            ORDER BY source ASC
            "#,
        )?;
        let rows = statement.query_map([advisory_id], |row| {
            Ok(ExternalIntelSourceDetail {
                source: parse_external_source(&row.get::<_, String>(0)?),
                source_item_id: row.get(1)?,
                url: row.get(2)?,
                published_at: row.get(3)?,
                updated_at: row.get(4)?,
                severity: parse_external_severity(&row.get::<_, String>(5)?),
                cvss_score: row.get(6)?,
                epss_percentage: row.get(7)?,
                reference_count: row.get(8)?,
                raw_json: String::new(),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn list_external_affected_packages(
        &self,
        connection: &Connection,
        advisory_id: i64,
    ) -> Result<Vec<AdvisoryPackageImpact>, Error> {
        let mut statement = connection.prepare(
            r#"
            SELECT ecosystem, package_name, package_purl, vulnerable_range,
                   first_patched_version, affected_versions_count
            FROM external_advisory_affected_packages
            WHERE advisory_id = ?1
            ORDER BY ecosystem, package_name
            "#,
        )?;
        let rows = statement.query_map([advisory_id], |row| {
            Ok(AdvisoryPackageImpact {
                ecosystem: row.get(0)?,
                package_name: row.get(1)?,
                package_purl: row.get(2)?,
                vulnerable_version_range: row.get(3)?,
                first_patched_version: row.get(4)?,
                affected_versions_count: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn list_external_references(
        &self,
        connection: &Connection,
        advisory_id: i64,
    ) -> Result<Vec<AdvisoryReference>, Error> {
        let mut statement = connection.prepare(
            r#"
            SELECT url, source, kind, tags_json
            FROM external_advisory_references
            WHERE advisory_id = ?1
            ORDER BY url ASC
            "#,
        )?;
        let rows = statement.query_map([advisory_id], |row| {
            let tags_json: String = row.get(3)?;
            Ok(AdvisoryReference {
                url: row.get(0)?,
                source: row.get(1)?,
                kind: row.get(2)?,
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn get_external_kev(
        &self,
        connection: &Connection,
        advisory_id: i64,
    ) -> Result<Option<CisaKevDetails>, Error> {
        connection
            .query_row(
                r#"
                SELECT date_added, due_date, required_action, known_ransomware_campaign_use,
                       notes, catalog_version, catalog_released_at
                FROM external_advisory_kev
                WHERE advisory_id = ?1
                ORDER BY id
                LIMIT 1
                "#,
                [advisory_id],
                |row| {
                    Ok(CisaKevDetails {
                        date_added: row.get(0)?,
                        due_date: row.get(1)?,
                        required_action: row.get(2)?,
                        known_ransomware_campaign_use: row.get(3)?,
                        notes: row.get(4)?,
                        catalog_version: row.get(5)?,
                        catalog_released_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Error::from)
    }

    fn repo_scan_history(
        &self,
        connection: &Connection,
        repo_id: i64,
        limit: usize,
    ) -> Result<Vec<RepoScanRun>, Error> {
        self.repo_scan_history_filtered(connection, repo_id, limit, false)
    }

    fn repo_scan_history_filtered(
        &self,
        connection: &Connection,
        repo_id: i64,
        limit: usize,
        completed_only: bool,
    ) -> Result<Vec<RepoScanRun>, Error> {
        let mut statement = connection.prepare(&format!(
            "{SCAN_RUN_DETAILS}
             WHERE repo_id = ?1 AND (?3=0 OR status='completed')
             ORDER BY started_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = statement.query_map(
            params![repo_id, limit as i64, completed_only],
            scan_run_from_row,
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn findings_for_run(
        &self,
        connection: &Connection,
        repo_id: i64,
        run_id: i64,
    ) -> Result<Vec<NormalizedFinding>, Error> {
        let mut statement = connection.prepare(
            r#"
	            SELECT fingerprint, advisory_id, package_name, package_version, package_type, package_url,
	                   severity, fix_version, title, source_url, advisory_details_json
            FROM vulnerabilities
            WHERE repo_id = ?1 AND scan_run_id = ?2
            ORDER BY advisory_id, package_name
            "#,
        )?;
        let rows = statement.query_map(params![repo_id, run_id], |row| {
            let severity: String = row.get(6)?;
            Ok(NormalizedFinding {
                fingerprint: row.get(0)?,
                advisory_id: row.get(1)?,
                package_name: row.get(2)?,
                package_version: row.get(3)?,
                package_type: row.get(4)?,
                package_url: row.get(5)?,
                severity: Severity::parse(&severity).unwrap_or(Severity::High),
                fix_version: row.get(7)?,
                title: row.get(8)?,
                source_url: row.get(9)?,
                advisory_details: row
                    .get::<_, Option<String>>(10)?
                    .and_then(|value| serde_json::from_str(&value).ok()),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn packages_for_run(
        &self,
        connection: &Connection,
        repo_id: i64,
        run_id: i64,
    ) -> Result<Vec<RepoPackage>, Error> {
        let mut statement = connection.prepare(
            r#"
            SELECT name, version, package_type, package_url
            FROM scan_packages
            WHERE repo_id = ?1 AND scan_run_id = ?2
            ORDER BY name, version
            "#,
        )?;
        let rows = statement.query_map(params![repo_id, run_id], |row| {
            Ok(RepoPackage {
                name: row.get(0)?,
                version: row.get(1)?,
                package_type: row.get(2)?,
                package_url: row.get(3)?,
                highest_severity: None,
                finding_count: 0,
                fix_available: false,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn repo_assets(&self, connection: &Connection, repo_id: i64) -> Result<Vec<RepoAsset>, Error> {
        let mut statement = connection.prepare(
            r#"
            SELECT repo_id, vendor, product, version, asset_kind, source_path, evidence, confidence
            FROM repo_assets
            WHERE repo_id = ?1
            ORDER BY confidence DESC, vendor, product
            "#,
        )?;
        let rows = statement.query_map([repo_id], |row| {
            Ok(RepoAsset {
                repo_id: row.get(0)?,
                vendor: row.get(1)?,
                product: row.get(2)?,
                version: row.get(3)?,
                asset_kind: row.get(4)?,
                source_path: row.get(5)?,
                evidence: row.get(6)?,
                confidence: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn overview_snapshot(
        &self,
        repos: &[RepoSummary],
        last_successful_scan_at: &Option<String>,
    ) -> OverviewSnapshot {
        OverviewSnapshot {
            repos_watched: repos.len() as u32,
            new_medium: repos.iter().map(|repo| repo.new_medium).sum(),
            new_high: repos.iter().map(|repo| repo.new_high).sum(),
            new_critical: repos.iter().map(|repo| repo.new_critical).sum(),
            open_medium: repos.iter().map(|repo| repo.open_medium).sum(),
            open_high: repos.iter().map(|repo| repo.open_high).sum(),
            open_critical: repos.iter().map(|repo| repo.open_critical).sum(),
            repos_with_new_findings: repos
                .iter()
                .filter(|repo| repo.new_medium > 0 || repo.new_high > 0 || repo.new_critical > 0)
                .count() as u32,
            last_successful_scan_at: last_successful_scan_at.clone(),
        }
    }

    fn last_grype_db_update_on(
        connection: &Connection,
    ) -> Result<Option<GrypeDbUpdateSummary>, Error> {
        let Some(value) = Self::setting_on(connection, "last_grype_db_update")? else {
            return Ok(None);
        };

        let summary = serde_json::from_str::<GrypeDbUpdateSummary>(&value)?;
        Ok(Some(summary))
    }

    fn onboarding_snapshot_on(connection: &Connection) -> Result<OnboardingSnapshot, Error> {
        let Some(value) = Self::setting_on(connection, ONBOARDING_SETTING)? else {
            return Ok(OnboardingSnapshot::default());
        };

        let snapshot = serde_json::from_str::<OnboardingSnapshot>(&value)
            .unwrap_or_else(|_| OnboardingSnapshot::default());
        Ok(snapshot)
    }
}

fn finding_state_as_str(state: &RepoFindingState) -> &'static str {
    match state {
        RepoFindingState::New => "new",
        RepoFindingState::Open => "open",
        RepoFindingState::Resolved => "resolved",
    }
}

fn parse_finding_state(value: &str) -> RepoFindingState {
    match value {
        "new" => RepoFindingState::New,
        "resolved" => RepoFindingState::Resolved,
        _ => RepoFindingState::Open,
    }
}

fn parse_datetime(value: &str) -> Option<OffsetDateTime> {
    timestamps::parse(value).ok()
}

#[cfg(test)]
fn material_risk_increased(previous: &ReviewRiskBasis, current: &ReviewRiskBasis) -> bool {
    severity_rank(&current.severity) > severity_rank(&previous.severity)
        || (!previous.known_exploited && current.known_exploited)
        || (!previous.ransomware_known && current.ransomware_known)
        || (current.fix_version.is_some() && current.fix_version != previous.fix_version)
        || (previous.scanner_state == RepoFindingState::Resolved
            && current.scanner_state != RepoFindingState::Resolved)
        || (previous.scanner_state != RepoFindingState::New
            && current.scanner_state == RepoFindingState::New)
}

fn validate_review_query(query: &ReviewQuery) -> Result<(), Error> {
    if query
        .page_size
        .is_some_and(|value| value == 0 || value > 200)
    {
        return Err(Error::App(
            "review page size must be between 1 and 200".into(),
        ));
    }
    if query
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.len() > 16_384)
    {
        return Err(Error::App("review cursor is invalid".into()));
    }
    let filters = &query.filters;
    if filters
        .fingerprint
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 16_384)
    {
        return Err(Error::App("invalid review fingerprint".into()));
    }
    if [
        filters.repo_ids.len(),
        filters.scan_run_ids.len(),
        filters.packages.len(),
        filters.ecosystems.len(),
        filters.review_states.len(),
        filters.scanner_states.len(),
        filters.severities.len(),
        filters.source_types.len(),
    ]
    .iter()
    .any(|count| *count > 200)
        || filters
            .packages
            .iter()
            .chain(&filters.ecosystems)
            .any(|value| value.len() > 512)
    {
        return Err(Error::App(
            "Review filters exceed the supported size".into(),
        ));
    }
    // One global scan can include all repositories under the discovery limit.
    for run_ids in [&filters.scan_change_run_ids, &filters.scan_resolved_run_ids] {
        if run_ids.len() > 20_000 || run_ids.iter().any(|id| *id <= 0) {
            return Err(Error::App("Invalid scan result filter".into()));
        }
    }
    if filters.resolved_in_scan_run_id.is_some_and(|id| id <= 0) {
        return Err(Error::App("Invalid scan result filter".into()));
    }
    if [
        !filters.scan_resolved_run_ids.is_empty(),
        !filters.scan_change_run_ids.is_empty(),
        filters.resolved_in_scan_run_id.is_some(),
    ]
    .into_iter()
    .filter(|active| *active)
    .count()
        > 1
    {
        return Err(Error::App("Choose one scan result filter".into()));
    }
    if let Some(value) = &query.filters.changed_since {
        if parse_datetime(value).is_none() {
            return Err(Error::App(
                "changedSince must be an RFC3339 timestamp".into(),
            ));
        }
    }
    if query
        .search
        .as_ref()
        .is_some_and(|value| value.chars().count() > 200)
    {
        return Err(Error::App(
            "review search must be 200 characters or fewer".into(),
        ));
    }
    Ok(())
}

fn review_group_counts(items: &[ReviewFinding]) -> ReviewGroupCounts {
    let mut review_state = [0_u32; 3];
    let mut severity = [0_u32; 3];
    let mut fix_availability = [0_u32; 2];
    let mut repositories = std::collections::BTreeMap::<i64, (String, u32)>::new();

    for item in items {
        match item.review_state {
            ReviewState::Unreviewed => review_state[0] += 1,
            ReviewState::Reviewed => review_state[1] += 1,
            ReviewState::Snoozed => review_state[2] += 1,
        }
        match item.finding.severity {
            Severity::Critical => severity[0] += 1,
            Severity::High => severity[1] += 1,
            Severity::Medium => severity[2] += 1,
        }
        fix_availability[usize::from(item.finding.fix_version.is_none())] += 1;
        let repository = repositories
            .entry(item.repo_id)
            .or_insert_with(|| (item.repo_name.clone(), 0));
        repository.1 += 1;
    }

    let mut repository = repositories
        .into_iter()
        .map(|(id, (label, count))| ReviewLaneCount {
            id: format!("repository:{id}"),
            label,
            count,
        })
        .collect::<Vec<_>>();
    repository.sort_by(|left, right| {
        left.label
            .to_lowercase()
            .cmp(&right.label.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });

    ReviewGroupCounts {
        review_state: [
            ("unreviewed", "Needs review"),
            ("reviewed", "Reviewed"),
            ("snoozed", "Snoozed"),
        ]
        .into_iter()
        .zip(review_state)
        .map(|((id, label), count)| ReviewLaneCount {
            id: id.to_string(),
            label: label.to_string(),
            count,
        })
        .collect(),
        severity: [
            ("CRITICAL", "Critical"),
            ("HIGH", "High"),
            ("MEDIUM", "Medium"),
        ]
        .into_iter()
        .zip(severity)
        .map(|((id, label), count)| ReviewLaneCount {
            id: id.to_string(),
            label: label.to_string(),
            count,
        })
        .collect(),
        repository,
        fix_availability: [
            ("available", "Fix available"),
            ("unavailable", "No fix listed"),
        ]
        .into_iter()
        .zip(fix_availability)
        .map(|((id, label), count)| ReviewLaneCount {
            id: id.to_string(),
            label: label.to_string(),
            count,
        })
        .collect(),
    }
}

#[cfg(test)]
fn review_item_matches(item: &ReviewFinding, query: &ReviewQuery) -> bool {
    let filters = &query.filters;
    if filters
        .fingerprint
        .as_ref()
        .is_some_and(|value| *value != item.finding.fingerprint)
    {
        return false;
    }
    if !filters.review_states.is_empty() && !filters.review_states.contains(&item.review_state) {
        return false;
    }
    if !filters.scanner_states.is_empty() && !filters.scanner_states.contains(&item.finding.state) {
        return false;
    }
    if !filters.severities.is_empty() && !filters.severities.contains(&item.finding.severity) {
        return false;
    }
    if !filters.repo_ids.is_empty() && !filters.repo_ids.contains(&item.repo_id) {
        return false;
    }
    if !filters.source_types.is_empty() && !filters.source_types.contains(&item.repo_source_type) {
        return false;
    }
    if let Some(expected) = filters.fix_available {
        if item.finding.fix_version.is_some() != expected {
            return false;
        }
    }
    if !filters.packages.is_empty()
        && !filters
            .packages
            .iter()
            .any(|value| value.eq_ignore_ascii_case(&item.finding.package_name))
    {
        return false;
    }
    if !filters.ecosystems.is_empty()
        && !item.finding.package_type.as_ref().is_some_and(|kind| {
            filters
                .ecosystems
                .iter()
                .any(|value| value.eq_ignore_ascii_case(kind))
        })
    {
        return false;
    }
    if filters
        .known_exploited
        .is_some_and(|expected| item.known_exploited != expected)
    {
        return false;
    }
    if filters
        .ransomware_known
        .is_some_and(|expected| item.ransomware_known != expected)
    {
        return false;
    }
    if !filters.scan_run_ids.is_empty()
        && !item
            .scan_run_id
            .is_some_and(|value| filters.scan_run_ids.contains(&value))
    {
        return false;
    }
    if let Some(changed_since) = filters.changed_since.as_deref().and_then(parse_datetime) {
        if !item
            .scanned_at
            .as_deref()
            .and_then(parse_datetime)
            .is_some_and(|value| value >= changed_since)
        {
            return false;
        }
    }
    if let Some(search) = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let candidate = format!(
            "{} {} {} {} {}",
            item.repo_name,
            item.finding.advisory_id,
            item.finding.package_name,
            item.finding.package_version.as_deref().unwrap_or_default(),
            item.finding.title.as_deref().unwrap_or_default()
        );
        if !candidate.to_lowercase().contains(&search.to_lowercase()) {
            return false;
        }
    }
    true
}

#[cfg(test)]
fn sort_review_items(items: &mut [ReviewFinding], sort: ReviewSort) {
    items.sort_by(|left, right| match sort {
        ReviewSort::Priority => review_priority_tuple(right)
            .cmp(&review_priority_tuple(left))
            .then_with(|| left.repo_name.cmp(&right.repo_name))
            .then_with(|| left.finding.advisory_id.cmp(&right.finding.advisory_id))
            .then_with(|| left.finding.fingerprint.cmp(&right.finding.fingerprint)),
        ReviewSort::Severity => severity_rank(&right.finding.severity)
            .cmp(&severity_rank(&left.finding.severity))
            .then_with(|| left.repo_name.cmp(&right.repo_name)),
        ReviewSort::Recent => right
            .scanned_at
            .cmp(&left.scanned_at)
            .then_with(|| left.repo_name.cmp(&right.repo_name)),
        ReviewSort::Repository => left.repo_name.cmp(&right.repo_name).then_with(|| {
            severity_rank(&right.finding.severity).cmp(&severity_rank(&left.finding.severity))
        }),
        ReviewSort::Package => left
            .finding
            .package_name
            .cmp(&right.finding.package_name)
            .then_with(|| left.finding.advisory_id.cmp(&right.finding.advisory_id)),
    });
}

#[cfg(test)]
fn review_priority_tuple(item: &ReviewFinding) -> (u8, u8, u8, u8, String) {
    let exploitation = if item.ransomware_known {
        2
    } else if item.known_exploited {
        1
    } else {
        0
    };
    let change = match item.material_change.as_deref() {
        Some("reappeared") => 3,
        Some("worsened") => 2,
        Some("new") => 1,
        _ => 0,
    };
    (
        exploitation,
        severity_rank(&item.finding.severity),
        change,
        u8::from(item.finding.fix_version.is_some()),
        item.scanned_at.clone().unwrap_or_default(),
    )
}

fn text_score(candidate: &str, needle: &str) -> Option<u8> {
    let lowercase;
    let candidate =
        if candidate.is_ascii() && !candidate.bytes().any(|byte| byte.is_ascii_uppercase()) {
            candidate
        } else {
            lowercase = candidate.to_lowercase();
            &lowercase
        };
    if candidate == needle {
        Some(0)
    } else if candidate.starts_with(needle) {
        Some(1)
    } else if candidate.contains(needle) {
        // Prefixes of tokens are a subset of substring matches. Avoid
        // scanning Unicode word boundaries for candidates that cannot match.
        if candidate
            .split(|character: char| !character.is_alphanumeric())
            .any(|token| token.starts_with(needle))
        {
            Some(2)
        } else {
            Some(3)
        }
    } else {
        None
    }
}

fn is_safe_stored_relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1_024
        && !value.as_bytes().contains(&0)
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn brief_value(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

fn format_fix_brief(projection: &FindingRemediationProjection) -> String {
    let installed = projection
        .installed_version
        .as_deref()
        .map(|value| brief_value(value, 120))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "version not listed".into());
    let mut lines = vec![
        "## CVEC fix brief".to_string(),
        String::new(),
        format!("- Repository: {}", brief_value(&projection.repo_name, 160)),
        format!("- Finding: {}", brief_value(&projection.advisory_id, 160)),
        format!(
            "- Package: {} {}",
            brief_value(&projection.package_name, 160),
            installed
        ),
        format!(
            "- Scanner-reported target: {}",
            projection
                .scanner_reported_target
                .as_deref()
                .map(|value| brief_value(value, 120))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "No fixed version listed".into())
        ),
        format!(
            "- Location: {}",
            projection
                .relative_path
                .as_deref()
                .filter(|path| is_safe_stored_relative_path(path))
                .map(|value| brief_value(value, 1_024))
                .unwrap_or_else(|| "Not available from this scan".into())
        ),
        format!(
            "- Relationship: {}",
            match projection.relationship {
                RemediationRelationship::Direct => "Direct",
                RemediationRelationship::Transitive => "Transitive",
                RemediationRelationship::Unknown => "Not proven",
            }
        ),
    ];
    if !projection.priority_reasons.is_empty() {
        lines.push(format!(
            "- Why: {}",
            projection
                .priority_reasons
                .iter()
                .take(5)
                .map(|value| brief_value(value, 100))
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let mut evidence = format!("scan {}", projection.evidence_scan_run_id);
    if let Some(scanned_at) = projection.evidence_scanned_at.as_deref() {
        evidence.push_str(" at ");
        evidence.push_str(&brief_value(scanned_at, 80));
    }
    if let Some(revision) = projection.source_snapshot.revision.as_deref() {
        evidence.push_str(", revision ");
        evidence.push_str(&brief_value(revision, 64));
        if projection.source_snapshot.dirty == Some(true) {
            evidence.push_str(" + local changes");
        }
    }
    lines.push(format!("- Evidence: {evidence}"));
    lines.push(String::new());
    lines.push(
        "Next: update the dependency using this repository's normal workflow, then run a new CVEC scan to verify whether the finding is still detected."
            .into(),
    );
    lines.join("\n").chars().take(4_096).collect()
}

fn now() -> String {
    timestamps::now()
}

fn scan_run_not_running(repo_id: i64, run_id: i64) -> Error {
    Error::App(format!(
        "Scan #{run_id} is no longer running for repository {repo_id}"
    ))
}

fn scan_has_finding(candidate: &str, run_id: &str, basis: &str) -> String {
    format!(
        "(EXISTS(SELECT 1 FROM vulnerabilities {candidate}
           WHERE {candidate}.scan_run_id={run_id} AND {candidate}.fingerprint={basis}.fingerprint)
          OR EXISTS(SELECT 1 FROM vulnerabilities {candidate}
           WHERE {candidate}.scan_run_id={run_id}
             AND {candidate}.advisory_id={basis}.advisory_id COLLATE NOCASE
             AND {candidate}.package_name={basis}.package_name
             AND cvec_same_scan_package({candidate}.package_type,{candidate}.package_url,
                                        {basis}.package_type,{basis}.package_url)))"
    )
}

fn finding_reappeared(basis: &str, previous_id: &str, eligible: &str) -> String {
    // A reappearance needs a complete intervening scan with no match.
    // Skip failed and incomplete absences, which cannot establish resolution.
    format!(
        "CASE WHEN ({eligible}) THEN EXISTS(
           SELECT 1 FROM scan_runs gap
           WHERE gap.id={previous_id} AND gap.evidence_complete=1
             AND NOT {gap_detected}
             AND (
                EXISTS(SELECT 1 FROM vulnerabilities old
                       JOIN scan_runs history ON history.id=old.scan_run_id
                       WHERE old.repo_id=gap.repo_id AND old.fingerprint={basis}.fingerprint
                         AND history.status='completed'
                         AND (history.started_at,history.id)<(gap.started_at,gap.id))
                OR EXISTS(SELECT 1 FROM vulnerabilities old
                          JOIN scan_runs history ON history.id=old.scan_run_id
                          WHERE old.advisory_id={basis}.advisory_id COLLATE NOCASE
                            AND old.repo_id=gap.repo_id AND old.package_name={basis}.package_name
                            AND history.status='completed'
                            AND (history.started_at,history.id)<(gap.started_at,gap.id)
                            AND cvec_same_scan_package(old.package_type,old.package_url,
                                                       {basis}.package_type,{basis}.package_url))
             )
         ) ELSE 0 END",
        gap_detected = scan_has_finding("absent", "gap.id", basis),
    )
}

fn build_repo_findings(
    latest_findings: &[NormalizedFinding],
    previous_findings: &[NormalizedFinding],
    has_previous_evidence: bool,
) -> Vec<RepoFinding> {
    let previous_keys = previous_findings
        .iter()
        .map(|finding| finding.fingerprint.clone())
        .collect::<std::collections::BTreeSet<_>>();

    let mut findings = Vec::new();
    for finding in latest_findings {
        let state = if !has_previous_evidence || previous_keys.contains(&finding.fingerprint) {
            RepoFindingState::Open
        } else {
            RepoFindingState::New
        };
        findings.push(repo_finding_from_normalized(finding, state));
    }

    if !previous_findings.is_empty() {
        let detected = DetectedFindings::new(latest_findings);
        for finding in previous_findings {
            if !detected.contains(
                &finding.fingerprint,
                &finding.advisory_id,
                &finding.package_name,
                finding.package_type.as_deref(),
                finding.package_url.as_deref(),
            ) {
                findings.push(repo_finding_from_normalized(
                    finding,
                    RepoFindingState::Resolved,
                ));
            }
        }
    }

    findings.sort_by(|left, right| {
        finding_state_rank(&right.state)
            .cmp(&finding_state_rank(&left.state))
            .then_with(|| severity_rank(&right.severity).cmp(&severity_rank(&left.severity)))
            .then_with(|| left.package_name.cmp(&right.package_name))
            .then_with(|| left.advisory_id.cmp(&right.advisory_id))
    });
    findings
}

fn repo_finding_from_normalized(
    finding: &NormalizedFinding,
    state: RepoFindingState,
) -> RepoFinding {
    RepoFinding {
        fingerprint: finding.fingerprint.clone(),
        advisory_id: finding.advisory_id.clone(),
        package_name: finding.package_name.clone(),
        package_version: finding.package_version.clone(),
        package_type: finding.package_type.clone(),
        package_url: finding.package_url.clone(),
        severity: finding.severity,
        fix_version: finding.fix_version.clone(),
        title: finding.title.clone(),
        source_url: finding.source_url.clone(),
        state,
        intel: finding
            .advisory_details
            .as_ref()
            .map(repo_finding_intel_from_advisory_details),
        review_state: ReviewState::Unreviewed,
        reviewed_at: None,
        snooze_until: None,
        review_note: None,
    }
}

fn repo_finding_intel_from_advisory_details(
    details: &super::models::AdvisoryDetails,
) -> RepoFindingIntel {
    let aliases = [details.ghsa_id.clone(), details.cve_id.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    RepoFindingIntel {
        fetched_at: details.fetched_at.clone(),
        canonical_key: details
            .ghsa_id
            .clone()
            .or_else(|| details.cve_id.clone())
            .unwrap_or_else(|| details.url.clone()),
        title: details.summary.clone(),
        summary: if details.description.trim().is_empty() {
            details.summary.clone()
        } else {
            details.description.clone()
        },
        url: details.url.clone(),
        severity: details.severity,
        published_at: details.published_at.clone(),
        updated_at: details.updated_at.clone(),
        sources: vec![ExternalSource::Github],
        aliases,
        match_kind: RepoFindingIntelMatchKind::SourceUrl,
        vulnerabilities: details.vulnerabilities.clone(),
        cvss_score: details.cvss_score.clone(),
        cvss_vector: details.cvss_vector.clone(),
        epss_percentage: details.epss_percentage.clone(),
        epss_percentile: details.epss_percentile.clone(),
        cwes: details.cwes.clone(),
        references: details.references.clone(),
        source_code_location: details.source_code_location.clone(),
    }
}

fn aggregate_repo_packages(
    mut packages: Vec<RepoPackage>,
    findings: &[RepoFinding],
) -> Vec<RepoPackage> {
    let mut package_indexes = std::collections::BTreeMap::new();
    for (index, package) in packages.iter().enumerate() {
        package_indexes.insert(
            package_identity(
                package.package_url.as_deref(),
                &package.name,
                package.version.as_deref(),
                package.package_type.as_deref(),
            ),
            index,
        );
    }

    for finding in findings
        .iter()
        .filter(|finding| finding.state != RepoFindingState::Resolved)
    {
        let identity = package_identity(
            finding.package_url.as_deref(),
            &finding.package_name,
            finding.package_version.as_deref(),
            finding.package_type.as_deref(),
        );
        if !package_indexes.contains_key(&identity) {
            let index = packages.len();
            packages.push(RepoPackage {
                name: finding.package_name.clone(),
                version: finding.package_version.clone(),
                package_type: finding.package_type.clone(),
                package_url: finding.package_url.clone(),
                highest_severity: None,
                finding_count: 0,
                fix_available: false,
            });
            package_indexes.insert(identity.clone(), index);
        }

        if let Some(package) = package_indexes
            .get(&identity)
            .and_then(|index| packages.get_mut(*index))
        {
            package.finding_count += 1;
            package.fix_available = package.fix_available || finding.fix_version.is_some();
            if package
                .highest_severity
                .as_ref()
                .map(|severity| severity_rank(&finding.severity) > severity_rank(severity))
                .unwrap_or(true)
            {
                package.highest_severity = Some(finding.severity);
            }
        }
    }

    packages.sort_by(|left, right| {
        severity_rank_opt(right.highest_severity.as_ref())
            .cmp(&severity_rank_opt(left.highest_severity.as_ref()))
            .then_with(|| right.finding_count.cmp(&left.finding_count))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.version.cmp(&right.version))
    });
    packages
}

fn finding_state_rank(state: &RepoFindingState) -> u8 {
    match state {
        RepoFindingState::New => 2,
        RepoFindingState::Open => 1,
        RepoFindingState::Resolved => 0,
    }
}

fn severity_rank_opt(severity: Option<&Severity>) -> u8 {
    severity.map(severity_rank).unwrap_or(0)
}

fn severity_rank(severity: &Severity) -> u8 {
    match severity {
        Severity::Critical => 3,
        Severity::High => 2,
        Severity::Medium => 1,
    }
}

fn relevance_rank(value: &ExternalIntelRelevance) -> u8 {
    match value {
        ExternalIntelRelevance::Relevant => 2,
        ExternalIntelRelevance::Triage => 1,
        ExternalIntelRelevance::General => 0,
    }
}

fn external_published_sort_key(value: Option<&str>) -> Option<OffsetDateTime> {
    value.and_then(|entry| timestamps::parse(entry).ok())
}

fn notification_kind_as_str(kind: &AppNotificationKind) -> &'static str {
    match kind {
        AppNotificationKind::ScanAlert => "scan_alert",
        AppNotificationKind::DbUpdate => "db_update",
    }
}

fn parse_notification_kind(value: &str) -> AppNotificationKind {
    match value {
        "db_update" => AppNotificationKind::DbUpdate,
        _ => AppNotificationKind::ScanAlert,
    }
}

fn notification_tone_as_str(tone: &AppNotificationTone) -> &'static str {
    match tone {
        AppNotificationTone::Warning => "warning",
        AppNotificationTone::Success => "success",
        AppNotificationTone::Neutral => "neutral",
    }
}

fn parse_notification_tone(value: &str) -> AppNotificationTone {
    match value {
        "success" => AppNotificationTone::Success,
        "neutral" => AppNotificationTone::Neutral,
        _ => AppNotificationTone::Warning,
    }
}

fn external_source_as_str(source: &ExternalSource) -> &'static str {
    match source {
        ExternalSource::Osv => "osv",
        ExternalSource::Github => "github",
        ExternalSource::Nvd => "nvd",
        ExternalSource::CisaKev => "cisa_kev",
        ExternalSource::Vendor => "vendor",
    }
}

fn parse_external_source(value: &str) -> ExternalSource {
    match value {
        "github" => ExternalSource::Github,
        "nvd" => ExternalSource::Nvd,
        "cisa_kev" => ExternalSource::CisaKev,
        "vendor" => ExternalSource::Vendor,
        _ => ExternalSource::Osv,
    }
}

fn external_relevance_as_str(relevance: &ExternalIntelRelevance) -> &'static str {
    match relevance {
        ExternalIntelRelevance::Relevant => "relevant",
        ExternalIntelRelevance::Triage => "triage",
        ExternalIntelRelevance::General => "general",
    }
}

fn parse_external_severity(value: &str) -> ExternalIntelSeverity {
    match value {
        "malware" => ExternalIntelSeverity::Malware,
        "critical" => ExternalIntelSeverity::Critical,
        "high" => ExternalIntelSeverity::High,
        "medium" => ExternalIntelSeverity::Medium,
        "low" => ExternalIntelSeverity::Low,
        _ => ExternalIntelSeverity::Unknown,
    }
}

fn external_match_kind_as_str(value: &ExternalIntelMatchKind) -> &'static str {
    match value {
        ExternalIntelMatchKind::PackageName => "package_name",
        ExternalIntelMatchKind::Product => "product",
        ExternalIntelMatchKind::Vendor => "vendor",
        ExternalIntelMatchKind::Ecosystem => "ecosystem",
    }
}

fn external_intel_feed_item_from_row(row: &Row<'_>) -> rusqlite::Result<ExternalIntelFeedItem> {
    let product_keys_json: String = row.get(13)?;
    let topic_keys_json: String = row.get(14)?;
    Ok(ExternalIntelFeedItem {
        id: row.get(0)?,
        canonical_key: row.get(1)?,
        title: row.get(2)?,
        summary: row.get(3)?,
        ecosystem: row.get(4)?,
        package_name: row.get(5)?,
        product_keys: serde_json::from_str(&product_keys_json).unwrap_or_default(),
        topic_keys: serde_json::from_str(&topic_keys_json).unwrap_or_default(),
        severity: parse_external_severity(&row.get::<_, String>(6)?),
        published_at: row.get(7)?,
        withdrawn_at: row.get(8)?,
        cvss_score: row.get(9)?,
        epss_percentage: row.get(10)?,
        is_kev: row.get::<_, i64>(11)? != 0,
        is_ransomware_known: row.get::<_, i64>(12)? != 0,
        relevance: ExternalIntelRelevance::General,
        sources: Vec::new(),
        aliases: Vec::new(),
        match_kind: None,
        repo_count: 0,
        cwes: Vec::new(),
        kev_dates: None,
    })
}

fn split_product_key(product_key: &str) -> (String, String) {
    product_key
        .split_once(':')
        .map(|(vendor, product)| (vendor.to_string(), product.to_string()))
        .unwrap_or_else(|| ("unknown".to_string(), product_key.to_string()))
}

fn companion_thread_from_row(row: &Row<'_>) -> rusqlite::Result<CompanionThread> {
    Ok(CompanionThread {
        id: row.get(0)?,
        repository_id: row.get(1)?,
        provider: row.get(2)?,
        provider_thread_id: row.get(3)?,
        title: row.get(4)?,
        status: CompanionThreadStatus::NotLoaded,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        last_opened_at: row.get(7)?,
        archived_at: row.get(8)?,
        context_version: row.get(9)?,
        last_model_id: row.get(10)?,
        provider_connection_id: row.get(11)?,
    })
}

fn companion_event_cache_key(event: &CompanionEventEnvelope) -> Result<String, Error> {
    // Stable FNV-1a over the normalized envelope. Only the digest is persisted;
    // provider protocol payloads and prepared repository context never enter the cache table.
    let bytes = if event.provider_segment_id.is_some() {
        let mut canonical = event.clone();
        canonical.provider_segment_id = None;
        serde_json::to_vec(&canonical)?
    } else {
        serde_json::to_vec(event)?
    };
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Ok(format!("{hash:016x}"))
}

fn companion_provider_segment_from_row(
    row: &Row<'_>,
) -> rusqlite::Result<CompanionProviderSegment> {
    let ordinal = row.get::<_, i64>(2)?;
    Ok(CompanionProviderSegment {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        ordinal: u32::try_from(ordinal).unwrap_or_default(),
        provider: row.get(3)?,
        connection_id: row.get(4)?,
        provider_thread_id: row.get(5)?,
        initial_model_id: row.get(6)?,
        last_model_id: row.get(7)?,
        status: CompanionProviderSegmentStatus::from_db(&row.get::<_, String>(8)?),
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        ended_at: row.get(11)?,
    })
}

fn validate_companion_provider_identity(
    provider: &str,
    provider_thread_id: &str,
    model_id: Option<&str>,
) -> Result<(), Error> {
    if !matches!(provider, "codex" | "grok" | "opencode") {
        return Err(Error::App("invalid companion provider driver".into()));
    }
    if provider_thread_id.trim().is_empty() || provider_thread_id.len() > 512 {
        return Err(Error::App("invalid provider thread id".into()));
    }
    if model_id.is_some_and(|value| value.len() > 256) {
        return Err(Error::App("invalid companion model id".into()));
    }
    Ok(())
}

fn available_builtin_display(
    connection: &Connection,
    id: &str,
    preferred: &str,
) -> Result<String, Error> {
    let conflicts = |candidate: &str| -> Result<bool, Error> {
        Ok(connection
            .query_row(
                "SELECT 1 FROM ai_connections WHERE lower(display_name) = lower(?1) AND id <> ?2",
                params![candidate, id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    };
    if !conflicts(preferred)? {
        return Ok(preferred.into());
    }
    let fallback = format!("{preferred} · Built-in");
    if !conflicts(&fallback)? {
        return Ok(fallback);
    }
    Ok(format!("{fallback} ({id})"))
}

fn provider_connection_from_row(row: &Row<'_>) -> rusqlite::Result<ProviderConnection> {
    let driver_value: String = row.get(1)?;
    let driver = ProviderDriverKind::parse(&driver_value).map_err(|cause| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(cause))
    })?;
    let config_json: String = row.get(7)?;
    let config = config_from_json(driver, &config_json).map_err(|cause| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(cause))
    })?;
    Ok(ProviderConnection {
        schema_version: row.get(6)?,
        id: ProviderConnectionId(row.get(0)?),
        driver,
        display_name: row.get(2)?,
        accent_color: row.get(3)?,
        enabled: row.get(4)?,
        built_in: row.get(5)?,
        config,
        has_server_password: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn validate_stored_connection(value: ProviderConnection) -> Result<ProviderConnection, Error> {
    if value.schema_version != PROVIDER_CONNECTION_SCHEMA_VERSION {
        return Err(Error::App(format!(
            "unsupported AI connection schema {}",
            value.schema_version
        )));
    }
    value.validate()?;
    Ok(value)
}

const ACTIVE_REPOSITORIES: &str =
    "SELECT id,name,path,is_active,source_type,source_key,remote_url,remote_host,provider,
            default_branch,last_remote_commit_sha,last_remote_synced_at,remote_file_count
     FROM repos WHERE is_active=1";

const SCAN_RUN_DETAILS: &str = r#"
    SELECT id, status, started_at, finished_at, error_message,
           new_medium_count, new_high_count, new_critical_count,
           resolved_medium_count, resolved_high_count, resolved_critical_count,
           source_revision, source_dirty, package_count, reopened_count, warnings_json,
           evidence_complete,
           (SELECT COUNT(*) FROM vulnerabilities v WHERE v.scan_run_id=scan_runs.id AND v.severity='MEDIUM') AS open_medium,
           (SELECT COUNT(*) FROM vulnerabilities v WHERE v.scan_run_id=scan_runs.id AND v.severity='HIGH') AS open_high,
           (SELECT COUNT(*) FROM vulnerabilities v WHERE v.scan_run_id=scan_runs.id AND v.severity='CRITICAL') AS open_critical
    FROM scan_runs
"#;

fn scan_run_from_row(row: &Row<'_>) -> rusqlite::Result<RepoScanRun> {
    Ok(RepoScanRun {
        id: row.get(0)?,
        status: ScanRunStatus::parse(row.get_ref(1)?.as_str()?),
        started_at: row.get(2)?,
        finished_at: row.get(3)?,
        error_message: row.get(4)?,
        new_medium: row.get(5)?,
        new_high: row.get(6)?,
        new_critical: row.get(7)?,
        resolved_medium: row.get(8)?,
        resolved_high: row.get(9)?,
        resolved_critical: row.get(10)?,
        source_revision: row.get(11)?,
        source_dirty: row.get(12)?,
        package_count: row.get(13)?,
        reopened_count: row.get(14)?,
        warnings: serde_json::from_str(row.get_ref(15)?.as_str()?).unwrap_or_default(),
        evidence_complete: row.get(16)?,
        open_medium: row.get(17)?,
        open_high: row.get(18)?,
        open_critical: row.get(19)?,
    })
}

fn repo_record_from_row(row: &Row<'_>) -> rusqlite::Result<RepoRecord> {
    Ok(RepoRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        is_active: row.get(3)?,
        source_type: RepoSourceType::parse(row.get_ref(4)?.as_str()?),
        source_key: row.get(5)?,
        remote_url: row.get(6)?,
        remote_host: row.get(7)?,
        provider: row.get(8)?,
        default_branch: row.get(9)?,
        last_remote_commit_sha: row.get(10)?,
        last_remote_synced_at: row.get(11)?,
        remote_file_count: row.get(12)?,
    })
}

fn ensure_repository_exists(connection: &Connection, repository_id: i64) -> Result<(), Error> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM repos WHERE id = ?1 AND is_active = 1",
            [repository_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(Error::App("repository not found".into()))
    }
}

fn normalize_companion_title(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let source = if normalized.is_empty() {
        "New repository chat"
    } else {
        normalized.as_str()
    };
    source.chars().take(80).collect()
}

#[cfg(any(debug_assertions, test))]
#[derive(Clone, Copy)]
struct NativeQaEvidence<'a> {
    relative_path: Option<&'a str>,
    location_kind: Option<&'a str>,
    relationship: &'a str,
    parent_package: Option<&'a str>,
    source: &'a str,
    quality: &'a str,
    location_count: u32,
}

#[cfg(any(debug_assertions, test))]
impl<'a> NativeQaEvidence<'a> {
    fn direct(path: &'a str) -> Self {
        Self {
            relative_path: Some(path),
            location_kind: Some("manifest"),
            relationship: "direct",
            parent_package: None,
            source: "supported_manifest",
            quality: "current",
            location_count: 1,
        }
    }

    fn transitive(path: &'a str, location_count: u32) -> Self {
        Self {
            relative_path: Some(path),
            location_kind: Some("lockfile"),
            relationship: "transitive",
            parent_package: Some("qa-workspace"),
            source: "syft_relationship",
            quality: "partial",
            location_count,
        }
    }

    fn unavailable() -> Self {
        Self {
            relative_path: None,
            location_kind: None,
            relationship: "unknown",
            parent_package: None,
            source: "none",
            quality: "unavailable",
            location_count: 0,
        }
    }
}

#[cfg(any(debug_assertions, test))]
struct NativeQaFinding<'a> {
    fingerprint: &'a str,
    advisory_id: &'a str,
    package_name: &'a str,
    package_version: &'a str,
    fix_version: Option<&'a str>,
    severity: &'a str,
    evidence: Option<NativeQaEvidence<'a>>,
}

#[cfg(any(debug_assertions, test))]
impl<'a> NativeQaFinding<'a> {
    #[allow(clippy::too_many_arguments)]
    fn exact(
        fingerprint: &'a str,
        advisory_id: &'a str,
        package_name: &'a str,
        package_version: &'a str,
        fix_version: Option<&'a str>,
        severity: &'a str,
        evidence: Option<NativeQaEvidence<'a>>,
    ) -> Self {
        Self {
            fingerprint,
            advisory_id,
            package_name,
            package_version,
            fix_version,
            severity,
            evidence,
        }
    }
}

#[cfg(any(debug_assertions, test))]
fn qa_insert_repo(
    transaction: &rusqlite::Transaction<'_>,
    fixture_root: &Path,
    slug: &str,
    name: &str,
    remote: bool,
    timestamp: &OffsetDateTime,
) -> Result<i64, Error> {
    let path = if remote {
        format!("https://github.com/cvec-fixtures/{slug}")
    } else {
        fixture_root.join(slug).display().to_string()
    };
    let source_type = if remote { "github" } else { "local" };
    let remote_url = remote.then_some(path.as_str());
    transaction.execute(
        r#"
        INSERT INTO repos (
          name, path, is_active, source_type, source_key, last_scan_status,
          remote_url, remote_host, provider, default_branch, last_remote_commit_sha,
          created_at, updated_at
        ) VALUES (?1, ?2, 1, ?3, ?4, 'unknown', ?5, ?6, ?7, ?8, ?9, ?10, ?10)
        "#,
        params![
            name,
            path,
            source_type,
            format!("qa-remediation:{slug}"),
            remote_url,
            remote.then_some("github.com"),
            remote.then_some("github"),
            remote.then_some("main"),
            remote.then_some("dddddddddddddddddddddddddddddddddddddddd"),
            timestamps::format(timestamp),
        ],
    )?;
    Ok(transaction.last_insert_rowid())
}

#[cfg(any(debug_assertions, test))]
fn qa_insert_scan(
    transaction: &rusqlite::Transaction<'_>,
    repo_id: i64,
    status: &str,
    evidence_complete: bool,
    revision: &str,
    timestamp: OffsetDateTime,
) -> Result<i64, Error> {
    let timestamp = timestamps::format(&timestamp);
    transaction.execute(
        r#"
        INSERT INTO scan_runs (
          repo_id, status, started_at, finished_at, scanner_versions, error_message,
          source_revision, source_dirty, package_count, warnings_json, evidence_complete
        ) VALUES (?1, ?2, ?3, ?3, '{"syft":"qa","grype":"qa"}', ?4, ?5, 0, 1, '[]', ?6)
        "#,
        params![
            repo_id,
            status,
            timestamp,
            (status == "failed").then_some("Synthetic scanner failure"),
            revision,
            evidence_complete,
        ],
    )?;
    let run_id = transaction.last_insert_rowid();
    transaction.execute(
        "UPDATE repos SET last_scan_status = ?1, last_scanned_at = ?2, updated_at = ?2 WHERE id = ?3",
        params![status, timestamp, repo_id],
    )?;
    Ok(run_id)
}

#[cfg(any(debug_assertions, test))]
fn qa_insert_finding(
    transaction: &rusqlite::Transaction<'_>,
    repo_id: i64,
    scan_run_id: i64,
    finding: NativeQaFinding<'_>,
) -> Result<(), Error> {
    transaction.execute(
        r#"
        INSERT INTO vulnerabilities (
          repo_id, scan_run_id, fingerprint, advisory_id, package_name, package_version,
          package_type, package_url, severity, fix_version, title, source_url
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'npm', ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            repo_id,
            scan_run_id,
            finding.fingerprint,
            finding.advisory_id,
            finding.package_name,
            finding.package_version,
            format!(
                "pkg:npm/{}@{}",
                finding.package_name, finding.package_version
            ),
            finding.severity,
            finding.fix_version,
            format!("{} affects {}", finding.advisory_id, finding.package_name),
            format!("https://example.invalid/advisories/{}", finding.advisory_id),
        ],
    )?;
    if let Some(evidence) = finding.evidence {
        transaction.execute(
            r#"
            INSERT INTO vulnerability_remediation_evidence (
              repo_id, scan_run_id, fingerprint, schema_version, relative_path,
              location_kind, relationship, parent_package, evidence_source,
              evidence_quality, location_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                repo_id,
                scan_run_id,
                finding.fingerprint,
                REMEDIATION_SCHEMA_VERSION,
                evidence.relative_path,
                evidence.location_kind,
                evidence.relationship,
                evidence.parent_package,
                evidence.source,
                evidence.quality,
                evidence.location_count.min(32),
            ],
        )?;
    }
    Ok(())
}

fn normalize_topic_keys(topic_keys: Vec<String>) -> Vec<String> {
    topic_keys
        .into_iter()
        .filter_map(|value| normalize_topic_key(&value))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_local_package_type(value: &str) -> Option<&'static str> {
    let normalized = match value {
        "npm" => "npm",
        "python" | "python-package" | "pypi" => "pip",
        "gem" | "rubygems" => "rubygems",
        "go-module" | "golang" => "go",
        "rust-crate" | "cargo" => "rust",
        "maven" | "java-archive" => "maven",
        "nuget" => "nuget",
        "composer" => "composer",
        "github-action" | "github-actions" | "actions" => "actions",
        "swift" => "swift",
        "dart-pub" | "pub" => "pub",
        _ => return None,
    };

    Some(normalized)
}

fn restrict_database_file_permissions(path: &std::path::Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for candidate in [
            path.to_path_buf(),
            sqlite_sidecar_path(path, "-wal"),
            sqlite_sidecar_path(path, "-shm"),
        ] {
            if let Ok(metadata) = std::fs::symlink_metadata(&candidate) {
                if metadata.file_type().is_file() {
                    std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))?;
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(unix)]
fn sqlite_sidecar_path(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    std::path::PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::provider_connections::ProviderConnectionConfig;
    use tempfile::tempdir;

    #[test]
    fn intel_topic_suggestions_use_latest_successful_inventory_and_active_repo_assets() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("topics.sqlite")).unwrap();
        assert_eq!(
            db.watched_external_intel_topic_keys().unwrap(),
            default_topic_keys()
        );
        seed_review_finding(&db, "npm-finding", "HIGH", None);
        seed_companion_repository(&db, 2, "inactive-source");
        let connection = db.connect().unwrap();
        insert_completed_run(&connection, 2, 2, "2026-10-13T12:00:00Z".into());
        insert_vulnerability(&connection, 2, 2, "inactive-finding", "HIGH");
        connection
            .execute("UPDATE repos SET is_active=0 WHERE id=2", [])
            .unwrap();
        connection
            .execute(
                "UPDATE vulnerabilities SET package_type='go-module' WHERE repo_id=2",
                [],
            )
            .unwrap();
        connection.execute(
            "INSERT INTO repo_assets (repo_id, vendor, product, asset_kind, evidence, confidence) VALUES (1,'jenkins','jenkins','service','fixture',100),(2,'docker','docker','service','fixture',100)",
            [],
        ).unwrap();
        let flags = |topics: &[ExternalIntelTopic], key: &str| {
            let topic = topics.iter().find(|topic| topic.key == key).unwrap();
            (topic.watched, topic.suggested)
        };
        let defaults = Database::external_intel_topics_on(&connection).unwrap();
        assert_eq!(flags(&defaults, "npm"), (true, true));
        assert_eq!(flags(&defaults, "jenkins"), (true, true));
        assert_eq!(flags(&defaults, "go"), (false, false));
        assert_eq!(flags(&defaults, "docker"), (false, false));

        insert_completed_run(
            &connection,
            3,
            1,
            timestamps::format(&(OffsetDateTime::now_utc() + Duration::minutes(1))),
        );
        insert_vulnerability(&connection, 1, 3, "python-finding", "HIGH");
        connection
            .execute(
                "UPDATE vulnerabilities SET package_type='python-package' WHERE scan_run_id=3",
                [],
            )
            .unwrap();
        connection.execute(
            "INSERT INTO scan_packages(repo_id,scan_run_id,name,version,package_type,package_url)
             VALUES (1,3,'github.com/example/demo','1.0.0','golang','pkg:golang/github.com/example/demo@1.0.0'),
                    (1,1,'old-package','1.0.0','cargo','pkg:cargo/old-package@1.0.0'),
                    (2,2,'inactive','1.0.0','gem','pkg:gem/inactive@1.0.0')",
            [],
        ).unwrap();
        assert_eq!(
            db.watched_external_intel_topic_keys().unwrap(),
            ["go", "jenkins", "pip"]
        );
        db.set_external_intel_topics(vec![]).unwrap();
        let saved_empty = Database::external_intel_topics_on(&connection).unwrap();
        assert_eq!(flags(&saved_empty, "npm"), (false, false));
        assert_eq!(flags(&saved_empty, "pip"), (false, true));
        assert_eq!(flags(&saved_empty, "go"), (false, true));
        assert_eq!(flags(&saved_empty, "rust"), (false, false));
        assert_eq!(flags(&saved_empty, "rubygems"), (false, false));
        assert!(db.watched_external_intel_topic_keys().unwrap().is_empty());
    }

    #[test]
    fn local_chat_drafts_survive_restart_and_keep_profiles_and_workspaces_separate() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("drafts.sqlite");
        let database = Database::new(path.clone()).unwrap();
        seed_companion_repository(&database, 1, "draft-one");
        seed_companion_repository(&database, 2, "draft-two");
        let text = "  Unsent\n\nGrüße 👋\n";
        database.companion_write_draft(1, "analysis", text).unwrap();
        database
            .companion_write_draft(2, "analysis", "other repo")
            .unwrap();
        database
            .companion_write_draft(1, "fix:a", "worktree A")
            .unwrap();
        database
            .companion_write_draft(1, "fix:b", "worktree B")
            .unwrap();
        let reopened = Database::new(path).unwrap();
        assert_eq!(
            reopened
                .companion_read_draft(1, "analysis")
                .unwrap()
                .as_deref(),
            Some(text)
        );
        assert_eq!(
            reopened
                .companion_read_draft(2, "analysis")
                .unwrap()
                .as_deref(),
            Some("other repo")
        );
        assert_eq!(
            reopened
                .companion_read_draft(1, "fix:a")
                .unwrap()
                .as_deref(),
            Some("worktree A")
        );
        assert_eq!(
            reopened
                .companion_read_draft(1, "fix:b")
                .unwrap()
                .as_deref(),
            Some("worktree B")
        );
        let other_profile = Database::new(temp.path().join("other-profile.sqlite")).unwrap();
        seed_companion_repository(&other_profile, 1, "draft-one");
        assert_eq!(
            other_profile.companion_read_draft(1, "analysis").unwrap(),
            None
        );
        reopened.companion_write_draft(1, "fix:a", "").unwrap();
        assert_eq!(
            reopened
                .companion_read_draft(1, "fix:a")
                .unwrap()
                .as_deref(),
            Some("")
        );
        assert!(reopened
            .companion_write_draft(999, "analysis", "wrong repo")
            .is_err());
        assert!(reopened
            .companion_write_draft(1, "", "invalid scope")
            .is_err());
        assert!(reopened
            .companion_write_draft(1, "analysis", &"x".repeat(65_537))
            .is_err());
        assert_eq!(
            reopened
                .companion_read_draft(1, "analysis")
                .unwrap()
                .as_deref(),
            Some(text)
        );
        reopened
            .connect()
            .unwrap()
            .execute("DELETE FROM repos WHERE id = 1", [])
            .unwrap();
        assert_eq!(reopened.companion_read_draft(1, "analysis").unwrap(), None);
    }

    #[test]
    fn companion_history_retains_repeated_questions_and_reconciles_each_echo_once() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("repeated-questions.sqlite");
        let database = Database::new(path.clone()).unwrap();
        seed_companion_repository(&database, 1, "repeated-questions");
        let thread = database
            .create_companion_thread(1, "provider-thread", "Repeated questions", None)
            .unwrap();
        let segment = database
            .current_companion_provider_segment(thread.id)
            .unwrap();
        let question = |id: &str| CompanionTimelineItem {
            id: id.into(),
            kind: CompanionTimelineItemKind::UserMessage,
            status: CompanionTimelineItemStatus::Completed,
            text: "Any critical findings?".into(),
            activity: None,
            citations: vec![],
            error: None,
            model_id: None,
            created_at: now(),
        };
        for (id, model_id) in [("local:turn-1", "model-a"), ("local:turn-2", "model-b")] {
            let mut local = question(id);
            local.model_id = Some(model_id.into());
            database
                .upsert_companion_timeline_item(thread.id, segment.id, &local)
                .unwrap();
        }
        for (sequence, kind) in [
            (1, CompanionEventKind::ItemStarted),
            (2, CompanionEventKind::ItemCompleted),
        ] {
            database
                .cache_companion_event(&CompanionEventEnvelope {
                    cache_epoch: None,
                    provider_segment_id: None,
                    schema_version: COMPANION_SCHEMA_VERSION,
                    repository_id: Some(1),
                    thread_id: Some(thread.id),
                    provider_thread_id: Some(thread.provider_thread_id.clone()),
                    connection_id: Some(DEFAULT_CODEX_CONNECTION_ID.into()),
                    driver: Some("codex".into()),
                    model_id: Some("provider-reported-model".into()),
                    turn_id: Some("turn-1".into()),
                    sequence,
                    kind,
                    payload: serde_json::to_value(question("user-1")).unwrap(),
                })
                .unwrap();
        }
        let snapshot = CompanionThreadSnapshot {
            thread: thread.clone(),
            items: vec![question("user-1"), question("user-2")],
            usage: None,
        };
        database
            .cache_companion_thread_snapshot(thread.id, &snapshot)
            .unwrap();
        database
            .cache_companion_thread_snapshot(thread.id, &snapshot)
            .unwrap();
        drop(database);
        let reopened = Database::new(path).unwrap();
        let cached = reopened
            .cached_companion_thread_snapshot(thread.id)
            .unwrap();
        assert_eq!(
            cached
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["user-1", "user-2"]
        );
        assert_eq!(
            cached
                .items
                .iter()
                .map(|item| item.model_id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("model-a"), Some("model-b")]
        );
    }

    #[test]
    fn companion_snapshot_preserves_citation_metadata_but_accepts_explicit_staleness() {
        use crate::core::companion::{CompanionCitation, ContextSourceKind};
        let temp = tempdir().unwrap();
        let path = temp.path().join("citation-cache.sqlite");
        let database = Database::new(path.clone()).unwrap();
        seed_companion_repository(&database, 1, "citations");
        let thread = database
            .create_companion_thread(1, "provider-thread", "Citations", None)
            .unwrap();
        let segment = database
            .current_companion_provider_segment(thread.id)
            .unwrap();
        let mut answer = CompanionTimelineItem {
            id: "answer".into(),
            kind: CompanionTimelineItemKind::AssistantMessage,
            status: CompanionTimelineItemStatus::Completed,
            text: "See [source:scan:10]".into(),
            activity: None,
            citations: vec![CompanionCitation {
                source_id: "scan:10".into(),
                kind: ContextSourceKind::Scan,
                label: "Scan #10".into(),
                stale: false,
            }],
            error: None,
            model_id: None,
            created_at: now(),
        };
        database
            .upsert_companion_timeline_item(thread.id, segment.id, &answer)
            .unwrap();
        answer.citations[0].label = "scan:10".into();
        answer.citations[0].stale = true;
        database
            .cache_companion_thread_snapshot(
                thread.id,
                &CompanionThreadSnapshot {
                    thread: thread.clone(),
                    items: vec![answer.clone()],
                    usage: None,
                },
            )
            .unwrap();
        let reopened = Database::new(path).unwrap();
        let cached = reopened
            .cached_companion_thread_snapshot(thread.id)
            .unwrap();
        assert_eq!(cached.items[0].citations[0].label, "Scan #10");
        assert!(!cached.items[0].citations[0].stale);
        answer.citations[0].label = "Scan #10".into();
        database
            .upsert_companion_timeline_item(thread.id, segment.id, &answer)
            .unwrap();
        assert!(
            database
                .cached_companion_thread_snapshot(thread.id)
                .unwrap()
                .items[0]
                .citations[0]
                .stale
        );
    }

    fn seed_review_finding(
        database: &Database,
        fingerprint: &str,
        severity: &str,
        fix: Option<&str>,
    ) {
        let connection = database.connect().expect("connect");
        let timestamp = timestamps::now();
        connection.execute("INSERT INTO repos (id, name, path, is_active, created_at, updated_at) VALUES (1, 'review-repo', '/tmp/review-repo', 1, ?1, ?1)", [&timestamp]).expect("repo");
        insert_completed_run(&connection, 1, 1, timestamp);
        connection.execute(
            r#"INSERT INTO vulnerabilities (repo_id, scan_run_id, fingerprint, advisory_id, package_name, package_version, package_type, severity, fix_version)
               VALUES (1, 1, ?1, 'CVE-2026-1000', 'review-package', '1.0.0', 'npm', ?2, ?3)"#,
            params![fingerprint, severity, fix],
        ).expect("finding");
    }

    fn seed_companion_repository(database: &Database, id: i64, name: &str) {
        let connection = database.connect().expect("connect");
        let timestamp = timestamps::now();
        connection
            .execute(
                "INSERT INTO repos (id, name, path, is_active, created_at, updated_at) VALUES (?1, ?2, ?3, 1, ?4, ?4)",
                params![id, name, format!("/tmp/{name}"), timestamp],
            )
            .expect("repo");
    }

    #[test]
    fn companion_schema_and_thread_routing_are_additive_bounded_and_reversible() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("cvec.sqlite");
        let database = Database::new(path.clone()).expect("init");
        seed_companion_repository(&database, 1, "first");
        seed_companion_repository(&database, 2, "second");
        database
            .set_setting("review_views", "preserve-me".into())
            .expect("setting");
        {
            let connection = database.connect().expect("connect evidence");
            let timestamp = timestamps::now();
            insert_completed_run(&connection, 1, 1, timestamp);
            connection
                .execute(
                    r#"INSERT INTO vulnerabilities (
                      repo_id, scan_run_id, fingerprint, advisory_id, package_name,
                      package_version, package_type, severity, fix_version
                    ) VALUES (1, 1, 'companion-preserved', 'CVE-2026-4242',
                              'preserved-package', '1.0.0', 'npm', 'HIGH', '1.1.0')"#,
                    [],
                )
                .expect("finding evidence");
        }
        database
            .mark_finding_reviewed(1, "companion-preserved")
            .expect("decision evidence");

        let first = database
            .create_companion_thread(1, "thr_first", "  Explain   CVE  ", Some("model-a"))
            .expect("create");
        assert_eq!(first.title, "Explain CVE");
        assert_eq!(first.repository_id, 1);
        let second = database
            .create_companion_thread(1, "thr_second", "Second", None)
            .expect("create second");
        assert!(second.id > first.id);
        assert!(database
            .create_companion_thread(2, "thr_first", "duplicate provider", None)
            .is_err());
        assert!(!database
            .companion_thread_has_provider_turn(first.id)
            .expect("empty draft state"));
        let rebound = database
            .rebind_empty_companion_thread(first.id, "thr_first_after_restart")
            .expect("refresh empty provider handle");
        assert_eq!(rebound.provider_thread_id, "thr_first_after_restart");
        database
            .mark_companion_thread_turn_started(first.id, Some("model-a"))
            .expect("mark first provider turn");
        assert!(database
            .companion_thread_has_provider_turn(first.id)
            .expect("started state"));
        assert!(database
            .rebind_empty_companion_thread(first.id, "must-not-rebind")
            .is_err());

        let renamed = database
            .rename_companion_thread(first.id, &"x".repeat(200))
            .expect("rename");
        assert_eq!(renamed.title.chars().count(), 80);
        let touched = database
            .touch_companion_thread(first.id, Some("model-b"))
            .expect("touch");
        assert_eq!(touched.last_model_id.as_deref(), Some("model-b"));
        let archived = database
            .set_companion_thread_archived(first.id, true)
            .expect("archive");
        assert!(archived.archived_at.is_some());
        let index = database
            .list_companion_conversation_index()
            .expect("archived metadata");
        assert_eq!(index.threads.len(), 2);
        assert_eq!(index.threads[0].id, second.id);
        assert!(index
            .threads
            .iter()
            .any(|thread| thread.id == first.id && thread.archived_at.is_some()));
        assert_eq!(
            database
                .list_companion_threads(1, false)
                .expect("active")
                .len(),
            1
        );
        assert_eq!(
            database.list_companion_threads(1, true).expect("all").len(),
            2
        );
        let restored = database
            .set_companion_thread_archived(first.id, false)
            .expect("unarchive");
        assert!(restored.archived_at.is_none());

        let initial = database
            .companion_privacy_consent()
            .expect("initial consent");
        assert!(!initial.valid);
        assert!(
            database
                .set_companion_privacy_consent(true)
                .expect("accept")
                .valid
        );
        assert!(
            !database
                .set_companion_privacy_consent(false)
                .expect("revoke")
                .valid
        );

        let connection = database.connect().expect("connect");
        let columns = table_columns(&connection, "companion_threads");
        assert!(columns
            .iter()
            .any(|column| column == "provider_turn_started"));
        assert!(!columns.iter().any(|column| {
            matches!(
                column.as_str(),
                "transcript" | "api_key" | "access_token" | "credential"
            )
        }));
        connection
            .execute_batch(
                "DROP TABLE companion_route_sequences; DROP TABLE companion_cached_events; DROP TABLE companion_timeline_items; DROP TABLE companion_provider_segments; DROP TABLE companion_threads; DROP TABLE companion_privacy_consents; PRAGMA user_version = 4;",
            )
            .expect("simulate pre-companion schema");
        drop(connection);

        let upgraded = Database::new(path).expect("upgrade");
        assert_eq!(
            upgraded
                .get_setting("review_views")
                .expect("setting")
                .as_deref(),
            Some("preserve-me")
        );
        assert_eq!(upgraded.list_repo_summaries().expect("repos").len(), 2);
        let preserved = upgraded.repo_detail(1).expect("preserved detail");
        assert_eq!(preserved.latest_scan.as_ref().map(|scan| scan.id), Some(1));
        let finding = preserved
            .findings
            .iter()
            .find(|finding| finding.fingerprint == "companion-preserved")
            .expect("preserved finding");
        assert_eq!(finding.advisory_id, "CVE-2026-4242");
        let connection = upgraded.connect().expect("connect");
        let decision: String = connection
            .query_row(
                "SELECT state FROM finding_reviews WHERE repo_id = 1 AND fingerprint = 'companion-preserved'",
                [],
                |row| row.get(0),
            )
            .expect("preserved decision");
        assert_eq!(decision, "reviewed");
        assert!(!table_columns(&connection, "companion_threads").is_empty());
        assert!(!table_columns(&connection, "companion_privacy_consents").is_empty());
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn companion_conversation_cache_is_segmented_idempotent_and_repository_scoped() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("conversation-cache.sqlite");
        let database = Database::new(path.clone()).expect("init");
        seed_companion_repository(&database, 1, "first");
        seed_companion_repository(&database, 2, "second");
        let first = database
            .create_companion_thread(1, "codex-thread", "First", Some("model-a"))
            .expect("first thread");
        let second = database
            .create_companion_thread(2, "codex-thread-2", "Second", Some("model-a"))
            .expect("second thread");

        let index = database
            .list_companion_conversation_index()
            .expect("conversation index");
        assert_eq!(index.schema_version, COMPANION_SCHEMA_VERSION);
        assert_eq!(index.threads.len(), 2);
        assert_eq!(
            database
                .companion_provider_segments(first.id)
                .expect("legacy segment")
                .len(),
            1
        );
        let first_segment = database
            .current_companion_provider_segment(first.id)
            .expect("current segment");
        let mut user_item = CompanionTimelineItem {
            id: "local:turn-1".into(),
            kind: super::super::companion::CompanionTimelineItemKind::UserMessage,
            status: CompanionTimelineItemStatus::Completed,
            text: "Explain the finding".into(),
            activity: None,
            citations: vec![],
            error: None,
            model_id: None,
            created_at: now(),
        };
        assert!(database
            .upsert_companion_timeline_item(first.id, first_segment.id, &user_item)
            .expect("insert visible item"));
        user_item.text = "Explain the highest-risk finding".into();
        assert!(database
            .upsert_companion_timeline_item(first.id, first_segment.id, &user_item)
            .expect("update visible item"));
        let context_item = CompanionTimelineItem {
            id: "context:1".into(),
            kind: super::super::companion::CompanionTimelineItemKind::Activity,
            status: CompanionTimelineItemStatus::Completed,
            text: String::new(),
            activity: Some(super::super::companion::CompanionActivity {
                kind: CompanionActivityKind::Context,
                label: "Prepared raw context".into(),
                detail: Some("must not persist".into()),
                source_ids: vec![],
            }),
            citations: vec![],
            error: None,
            model_id: None,
            created_at: now(),
        };
        assert!(!database
            .upsert_companion_timeline_item(first.id, first_segment.id, &context_item)
            .expect("filter context item"));
        assert!(database
            .upsert_companion_timeline_item(second.id, first_segment.id, &user_item)
            .is_err());

        let next = database
            .create_companion_provider_segment(
                first.id,
                DEFAULT_GROK_CONNECTION_ID,
                "grok",
                "grok-session-1",
                Some("grok-model"),
            )
            .expect("new provider segment");
        assert_eq!(next.ordinal, 2);
        assert_eq!(
            database
                .companion_thread(first.id)
                .expect("mirrored thread")
                .provider,
            "grok"
        );
        let cached = database
            .cached_companion_thread_snapshot(first.id)
            .expect("cached snapshot");
        assert_eq!(cached.items.len(), 1);
        assert_eq!(cached.items[0].text, "Explain the highest-risk finding");
        assert_eq!(cached.segments.len(), 2);
        assert!(cached.provider_history_available);

        let connection = database.connect().expect("connect");
        let item_json: String = connection
            .query_row(
                "SELECT item_json FROM companion_timeline_items WHERE thread_id = ?1",
                [first.id],
                |row| row.get(0),
            )
            .expect("cached item json");
        let lower = item_json.to_lowercase();
        assert!(!lower.contains("api_key"));
        assert!(!lower.contains("credential"));
        assert!(!lower.contains("prepared raw context"));
        drop(connection);

        Database::new(path.clone()).expect("repeat initialization");
        assert_eq!(
            database
                .companion_provider_segments(first.id)
                .expect("segments after repeat")
                .len(),
            2
        );

        let legacy_connection = database.connect().expect("legacy connect");
        legacy_connection
            .execute(
                "DELETE FROM companion_provider_segments WHERE thread_id = ?1",
                [second.id],
            )
            .expect("simulate legacy row");
        drop(legacy_connection);
        Database::new(path).expect("legacy backfill");
        assert_eq!(
            database
                .companion_provider_segments(second.id)
                .expect("backfilled segment")
                .len(),
            1
        );
    }

    #[test]
    fn companion_warnings_do_not_interrupt_cached_responses() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("warnings.sqlite")).expect("init");
        seed_companion_repository(&database, 1, "warnings");
        let thread = database
            .create_companion_thread(1, "provider-warnings", "Warnings", None)
            .expect("thread");
        let event = |sequence, kind, payload| CompanionEventEnvelope {
            cache_epoch: None,
            provider_segment_id: None,
            schema_version: COMPANION_SCHEMA_VERSION,
            repository_id: Some(1),
            thread_id: Some(thread.id),
            provider_thread_id: Some(thread.provider_thread_id.clone()),
            connection_id: Some(DEFAULT_CODEX_CONNECTION_ID.into()),
            driver: Some("codex".into()),
            model_id: None,
            turn_id: Some("turn-warning".into()),
            sequence,
            kind,
            payload,
        };
        for (sequence, kind, payload) in [
            (1, CompanionEventKind::TurnStarted, serde_json::json!({})),
            (
                2,
                CompanionEventKind::ItemDelta,
                serde_json::json!({ "itemId": "answer", "text": "Hey!" }),
            ),
            (
                3,
                CompanionEventKind::Warning,
                serde_json::json!({ "code": "codex_warning", "message": "chronicle", "retryable": false }),
            ),
            (
                4,
                CompanionEventKind::Error,
                serde_json::json!({ "code": "codex_warning", "message": "legacy warning", "retryable": false }),
            ),
            (
                5,
                CompanionEventKind::Warning,
                serde_json::json!({ "code": "action_denied", "message": "Read-only denial", "retryable": false }),
            ),
            (
                6,
                CompanionEventKind::Error,
                serde_json::json!({ "code": "action_denied", "message": "Legacy denial", "retryable": false }),
            ),
        ] {
            database
                .cache_companion_event(&event(sequence, kind, payload))
                .expect("cache event");
        }
        let segment = database
            .companion_provider_segments(thread.id)
            .expect("segments")
            .remove(0);
        assert_eq!(segment.status, CompanionProviderSegmentStatus::Active);
        let answer = database
            .cached_companion_timeline_item(thread.id, segment.id, "answer")
            .expect("cached answer")
            .expect("answer");
        assert_eq!(answer.status, CompanionTimelineItemStatus::Streaming);

        // Existing caches must also render old warning-as-error entries correctly.
        let legacy = CompanionTimelineItem {
            id: "old-warning".into(),
            kind: CompanionTimelineItemKind::Error,
            status: CompanionTimelineItemStatus::Failed,
            text: String::new(),
            activity: None,
            citations: Vec::new(),
            model_id: None,
            created_at: now(),
            error: Some(CompanionErrorInfo {
                code: "codex_warning".into(),
                message: "old chronicle warning".into(),
                retryable: false,
            }),
        };
        database
            .upsert_companion_timeline_item(thread.id, segment.id, &legacy)
            .expect("old cache");
        database
            .cache_companion_event(&event(
                7,
                CompanionEventKind::TurnCompleted,
                serde_json::json!({ "status": "completed" }),
            ))
            .expect("complete");
        let cached = database
            .cached_companion_thread_snapshot(thread.id)
            .expect("snapshot");
        assert_eq!(
            cached.segments[0].status,
            CompanionProviderSegmentStatus::Ready
        );
        assert_eq!(
            cached.items[0].status,
            CompanionTimelineItemStatus::Completed
        );
        assert!(cached.items[1..]
            .iter()
            .all(|item| item.kind == CompanionTimelineItemKind::Warning
                && item.status == CompanionTimelineItemStatus::Completed));
        database.cache_companion_event(&event(8, CompanionEventKind::Error, serde_json::json!({ "code": "offline", "message": "Connection ended", "retryable": true }))).expect("real error");
        let cached = database
            .cached_companion_thread_snapshot(thread.id)
            .expect("snapshot");
        assert_eq!(
            cached.segments[0].status,
            CompanionProviderSegmentStatus::Interrupted
        );
        assert_eq!(
            cached.items.last().unwrap().kind,
            CompanionTimelineItemKind::Error
        );
    }

    #[test]
    fn companion_event_cache_deduplicates_streams_and_excludes_context_payloads() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("event-cache.sqlite")).expect("init");
        seed_companion_repository(&database, 1, "events");
        let thread = database
            .create_companion_thread(1, "provider-events", "Events", Some("model-a"))
            .expect("thread");
        let event = |sequence, kind, payload| CompanionEventEnvelope {
            cache_epoch: None,
            provider_segment_id: None,
            schema_version: COMPANION_SCHEMA_VERSION,
            repository_id: Some(1),
            thread_id: Some(thread.id),
            provider_thread_id: Some(thread.provider_thread_id.clone()),
            connection_id: Some(DEFAULT_CODEX_CONNECTION_ID.into()),
            driver: Some("codex".into()),
            model_id: Some("model-a".into()),
            turn_id: Some("turn-1".into()),
            sequence,
            kind,
            payload,
        };
        let first_delta = event(
            1,
            CompanionEventKind::ItemDelta,
            serde_json::json!({ "itemId": "assistant-1", "text": "partial " }),
        );
        database
            .cache_companion_event(&first_delta)
            .expect("first delta");
        database
            .cache_companion_event(&first_delta)
            .expect("duplicate delta");
        database
            .cache_companion_event(&event(
                2,
                CompanionEventKind::ItemDelta,
                serde_json::json!({ "itemId": "assistant-1", "text": "answer" }),
            ))
            .expect("second delta");
        database
            .cache_companion_event(&event(
                3,
                CompanionEventKind::Activity,
                serde_json::json!({ "kind": "context", "label": "Prepared secret context", "detail": "must not persist", "sourceIds": [] }),
            ))
            .expect("filtered context activity");
        database
            .cache_companion_event(&event(
                4,
                CompanionEventKind::TurnCompleted,
                serde_json::json!({ "status": "completed" }),
            ))
            .expect("complete");

        let cached = database
            .cached_companion_thread_snapshot(thread.id)
            .expect("cached");
        assert_eq!(cached.items.len(), 1);
        assert_eq!(cached.items[0].text, "partial answer");
        assert_eq!(
            cached.items[0].status,
            CompanionTimelineItemStatus::Completed
        );
        assert!(!serde_json::to_string(&cached)
            .expect("serialize cached timeline")
            .contains("Prepared secret context"));

        database
            .cache_companion_event(&event(
                5,
                CompanionEventKind::Error,
                serde_json::json!({ "code": "offline", "message": "Connection ended", "retryable": true }),
            ))
            .expect("visible error");
        let interrupted = database
            .cached_companion_thread_snapshot(thread.id)
            .expect("interrupted cache");
        assert_eq!(interrupted.items.len(), 2);
        assert_eq!(
            interrupted.segments[0].status,
            CompanionProviderSegmentStatus::Interrupted
        );
        assert_eq!(
            interrupted.items[1].error.as_ref().expect("error").code,
            "offline"
        );

        let connection = database.connect().expect("connect");
        let event_keys = connection
            .prepare("SELECT event_key FROM companion_cached_events ORDER BY id")
            .expect("event keys")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query event keys")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect event keys");
        assert_eq!(event_keys.len(), 5);
        assert!(event_keys.iter().all(|key| key.len() == 16));
        assert!(!event_keys.join("").contains("partial"));
        drop(connection);

        let previous = database
            .current_companion_provider_segment(thread.id)
            .expect("previous segment");
        let staged = database
            .create_companion_provider_segment(
                thread.id,
                DEFAULT_GROK_CONNECTION_ID,
                "grok",
                "staged-grok-session",
                Some("grok-model"),
            )
            .expect("staged transition");
        database
            .upsert_companion_timeline_item(
                thread.id,
                staged.id,
                &CompanionTimelineItem {
                    id: "provider-transition:staged".into(),
                    kind: CompanionTimelineItemKind::Activity,
                    status: CompanionTimelineItemStatus::Completed,
                    text: String::new(),
                    activity: Some(CompanionActivity {
                        kind: CompanionActivityKind::ProviderTransition,
                        label: "Continued with Grok".into(),
                        detail: Some("grok-model".into()),
                        source_ids: Vec::new(),
                    }),
                    citations: Vec::new(),
                    error: None,
                    model_id: None,
                    created_at: now(),
                },
            )
            .expect("transition marker");
        let restored = database
            .rollback_companion_provider_segment(thread.id, staged.id, &previous, false)
            .expect("rollback staged transition");
        assert_eq!(restored.provider, "codex");
        assert_eq!(restored.provider_thread_id, "provider-events");
        assert_eq!(
            database
                .companion_provider_segments(thread.id)
                .expect("restored segments")
                .len(),
            1
        );
        assert!(!database
            .cached_companion_thread_snapshot(thread.id)
            .expect("restored cache")
            .items
            .iter()
            .any(|item| item.id == "provider-transition:staged"));
    }

    #[test]
    fn ai_connection_migration_crud_and_orphaned_threads_are_additive() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("connections.sqlite");
        let database = Database::new(path.clone()).expect("fresh database");
        seed_companion_repository(&database, 1, "providers");

        let default = database
            .ai_connection(DEFAULT_CODEX_CONNECTION_ID)
            .expect("default Codex connection");
        assert!(default.built_in);
        assert_eq!(default.driver, ProviderDriverKind::Codex);
        let default_grok = database
            .ai_connection(DEFAULT_GROK_CONNECTION_ID)
            .expect("default Grok connection");
        assert!(default_grok.built_in);
        assert_eq!(default_grok.driver, ProviderDriverKind::Grok);
        let default_opencode = database
            .ai_connection(DEFAULT_OPENCODE_CONNECTION_ID)
            .expect("default OpenCode connection");
        assert!(default_opencode.built_in);
        assert_eq!(default_opencode.driver, ProviderDriverKind::OpenCode);
        assert_eq!(database.list_ai_connections().expect("defaults").len(), 3);
        assert!(database
            .remove_ai_connection(DEFAULT_CODEX_CONNECTION_ID)
            .is_err());
        assert!(database
            .remove_ai_connection(DEFAULT_GROK_CONNECTION_ID)
            .is_err());
        assert!(database
            .remove_ai_connection(DEFAULT_OPENCODE_CONNECTION_ID)
            .is_err());

        database
            .save_ai_connection(&ProviderConnectionDraft {
                id: DEFAULT_GROK_CONNECTION_ID.into(),
                driver: ProviderDriverKind::Grok,
                display_name: "Customized Grok".into(),
                accent_color: Some("#112233".into()),
                enabled: false,
                config: ProviderConnectionConfig::Grok(
                    super::super::provider_connections::GrokConnectionConfig {
                        binary_path: Some("/custom/grok".into()),
                        custom_models: vec!["grok-custom".into()],
                    },
                ),
                server_password: None,
            })
            .expect("customize built-in Grok");
        let reset_grok = database
            .reset_ai_connection(DEFAULT_GROK_CONNECTION_ID)
            .expect("reset built-in Grok");
        assert_eq!(reset_grok.display_name, "Grok");
        assert!(reset_grok.enabled);
        assert_eq!(
            reset_grok.config,
            ProviderConnectionConfig::Grok(Default::default())
        );

        let draft = ProviderConnectionDraft {
            id: "opencode_local".into(),
            driver: ProviderDriverKind::OpenCode,
            display_name: "  Local   models  ".into(),
            accent_color: None,
            enabled: true,
            config: ProviderConnectionConfig::OpenCode(
                super::super::provider_connections::OpenCodeConnectionConfig::default(),
            ),
            server_password: None,
        };
        let saved = database
            .save_ai_connection(&draft)
            .expect("save connection");
        assert_eq!(saved.display_name, "Local models");
        assert!(database
            .save_ai_connection(&ProviderConnectionDraft {
                id: "duplicate_name".into(),
                display_name: "local MODELS".into(),
                ..draft.clone()
            })
            .is_err());
        assert!(
            !database
                .set_ai_connection_enabled(saved.id.as_ref(), false)
                .expect("disable")
                .enabled
        );
        database
            .set_ai_connection_enabled(saved.id.as_ref(), true)
            .expect("enable");

        let grok = database
            .save_ai_connection(&ProviderConnectionDraft {
                id: "grok_work".into(),
                driver: ProviderDriverKind::Grok,
                display_name: "Grok Work".into(),
                accent_color: None,
                enabled: true,
                config: ProviderConnectionConfig::Grok(
                    super::super::provider_connections::GrokConnectionConfig::default(),
                ),
                server_password: None,
            })
            .expect("save direct Grok connection");
        let grok_thread = database
            .create_companion_thread_for_connection(
                1,
                grok.id.as_ref(),
                "grok",
                "draft:grok:provider",
                "Grok chat",
                Some("grok-build"),
            )
            .expect("Grok connection-bound thread");
        assert_eq!(grok_thread.provider, "grok");

        let thread = database
            .create_companion_thread_for_connection(
                1,
                saved.id.as_ref(),
                "opencode",
                "draft:provider",
                "Local chat",
                Some("ollama/qwen3"),
            )
            .expect("connection-bound thread");
        assert_eq!(thread.provider, "opencode");
        assert_eq!(thread.provider_connection_id, "opencode_local");
        assert_eq!(
            database
                .remove_ai_connection("opencode_local")
                .expect("remove"),
            1
        );
        let orphan = database
            .companion_thread(thread.id)
            .expect("orphan preserved");
        assert_eq!(orphan.provider_connection_id, "opencode_local");

        let restored = database
            .save_ai_connection(&draft)
            .expect("restore stable id");
        assert_eq!(restored.id.as_ref(), "opencode_local");
        let connection = database.connect().expect("simulate pre-default upgrade");
        connection
            .execute(
                "DELETE FROM ai_connections WHERE id IN (?1, ?2)",
                params![DEFAULT_GROK_CONNECTION_ID, DEFAULT_OPENCODE_CONNECTION_ID],
            )
            .expect("remove newer defaults");
        connection
            .execute(
                "UPDATE ai_connections SET display_name = 'Grok' WHERE id = 'grok_work'",
                [],
            )
            .expect("simulate legacy custom Grok name");
        connection
            .execute("PRAGMA user_version = 8", [])
            .expect("old version");
        drop(connection);
        let reopened = Database::new(path).expect("repeat migration");
        assert_eq!(
            reopened.list_ai_connections().expect("connections").len(),
            5
        );
        let migrated_grok = reopened
            .ai_connection(DEFAULT_GROK_CONNECTION_ID)
            .expect("migrated Grok default");
        assert!(migrated_grok.built_in);
        assert_eq!(migrated_grok.display_name, "Grok · Built-in");
        assert_eq!(
            reopened
                .ai_connection("grok_work")
                .expect("legacy Grok preserved")
                .display_name,
            "Grok"
        );
        assert_eq!(
            reopened
                .reset_ai_connection(DEFAULT_GROK_CONNECTION_ID)
                .expect("reset conflict-safe Grok default")
                .display_name,
            "Grok · Built-in"
        );
        assert!(
            reopened
                .ai_connection(DEFAULT_OPENCODE_CONNECTION_ID)
                .expect("migrated OpenCode default")
                .built_in
        );
        let connection = reopened.connect().expect("inspect schema");
        let columns = table_columns(&connection, "companion_threads");
        assert!(columns.contains(&"provider".to_string()));
        assert!(columns.contains(&"provider_thread_id".to_string()));
        assert!(columns.contains(&"provider_connection_id".to_string()));
        assert!(columns.contains(&"provider_driver".to_string()));
        assert!(!columns.iter().any(|column| column.contains("secret")));
    }

    #[test]
    fn review_state_migration_is_additive_and_idempotent() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("cvec.sqlite");
        let database = Database::new(path.clone()).expect("init");
        seed_review_finding(&database, "review-fp", "HIGH", None);
        let second = Database::new(path).expect("reopen");
        let connection = second.connect().expect("connect");
        assert!(
            table_columns(&connection, "finding_reviews").contains(&"basis_severity".to_string())
        );
        assert_eq!(second.list_repo_summaries().expect("summaries").len(), 1);
        assert_eq!(second.repo_detail(1).expect("detail").findings.len(), 1);
    }

    #[test]
    fn completed_scan_records_exact_review_changes_before_other_readers_see_it() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("scan-changes.db")).unwrap();
        seed_companion_repository(&db, 1, "first");
        seed_companion_repository(&db, 2, "second");
        let writer = db.connect().unwrap();
        insert_completed_run(&writer, 1, 1, "2026-08-17T12:00:00Z".into());
        insert_completed_run(&writer, 2, 2, "2026-08-17T12:00:00Z".into());
        for fingerprint in ["worse", "new-fix", "unchanged", "due", "resolved"] {
            insert_vulnerability(&writer, 1, 1, fingerprint, "HIGH");
            if fingerprint == "due" {
                db.snooze_finding(
                    1,
                    fingerprint,
                    &timestamps::format(&(OffsetDateTime::now_utc() + Duration::hours(1))),
                    None,
                )
                .unwrap();
            } else if fingerprint != "resolved" {
                db.mark_finding_reviewed(1, fingerprint).unwrap();
            }
        }
        insert_vulnerability(&writer, 2, 2, "other", "HIGH");
        db.mark_finding_reviewed(2, "other").unwrap();
        writer
            .execute(
                "UPDATE vulnerabilities SET severity='CRITICAL' WHERE repo_id=2",
                [],
            )
            .unwrap();
        writer.execute("UPDATE finding_reviews SET snooze_until='2000-01-01T00:00:00+00:00' WHERE fingerprint='due'", []).unwrap();

        let versions = ScannerVersions {
            syft: "fixture".into(),
            grype: "fixture".into(),
        };
        let findings = ["worse", "new-fix", "unchanged", "due", "introduced"]
            .into_iter()
            .map(|fingerprint| NormalizedFinding {
                fingerprint: fingerprint.into(),
                advisory_id: fingerprint.into(),
                package_name: "demo-package".into(),
                package_version: None,
                package_type: None,
                package_url: None,
                severity: if fingerprint == "worse" {
                    Severity::Critical
                } else {
                    Severity::High
                },
                fix_version: (fingerprint == "new-fix" || fingerprint == "due")
                    .then(|| "2.0".into()),
                title: None,
                source_url: None,
                advisory_details: None,
            })
            .collect();
        let output = ScanOutput {
            evidence: None,
            findings,
            packages: vec![],
            remediation_evidence: vec![],
        };
        let source = SourceSnapshot {
            revision: None,
            dirty: None,
        };
        writer.execute_batch("CREATE TRIGGER reject_scan_review_changes BEFORE DELETE ON finding_reviews BEGIN SELECT RAISE(ABORT, 'fixture review write failure'); END;").unwrap();
        let failed_review_run = db.insert_scan_start(1, &versions).unwrap();
        let warning = db
            .complete_repository_scan(1, failed_review_run, &output, &source, &[], None)
            .unwrap()
            .review_warning
            .unwrap();
        assert!(warning.contains("fixture review write failure"));
        let successful = db.repo_detail(1).unwrap().latest_scan.unwrap();
        assert_eq!(successful.status, ScanRunStatus::Completed);
        assert_eq!(successful.warnings, vec![warning]);
        assert!(db.finding_review(&writer, 1, "worse").unwrap().is_some());
        // Restore the fixture baseline so the normal scan has the same comparison.
        writer
            .execute_batch("DROP TRIGGER reject_scan_review_changes;")
            .unwrap();
        writer
            .execute("DELETE FROM scan_runs WHERE id=?1", [failed_review_run])
            .unwrap();

        let run_id = db.insert_scan_start(1, &versions).unwrap();
        let saved = db
            .complete_repository_scan(1, run_id, &output, &source, &[], None)
            .unwrap();
        assert!(saved.review_warning.is_none());
        assert_eq!(
            (
                saved.new_findings,
                saved.worsened_findings,
                saved.resolved_findings
            ),
            (1, 2, 1)
        );
        assert_eq!(
            db.repo_detail(1)
                .unwrap()
                .latest_scan
                .unwrap()
                .reopened_count,
            2
        );
        for fingerprint in ["worse", "new-fix"] {
            assert!(db
                .finding_review(&writer, 1, fingerprint)
                .unwrap()
                .is_none());
        }
        for (repo, fingerprint) in [(1, "unchanged"), (1, "due"), (2, "other")] {
            assert!(db
                .finding_review(&writer, repo, fingerprint)
                .unwrap()
                .is_some());
        }
        let mut query = ReviewQuery::default();
        query.filters.scan_change_run_ids = vec![run_id];
        query.page_size = Some(2);
        let page = db.list_review_findings(&query).unwrap();
        assert_eq!(page.total, 3);
        query.cursor = page.next_cursor.clone();
        assert!(query.cursor.is_some());
        let next = db.list_review_findings(&query).unwrap();
        assert_eq!(next.total, 3);
        assert_eq!(
            page.items
                .into_iter()
                .chain(next.items)
                .map(|item| (item.finding.fingerprint, item.material_change.unwrap()))
                .collect::<std::collections::BTreeMap<_, _>>(),
            std::collections::BTreeMap::from([
                ("introduced".into(), "new".into()),
                ("new-fix".into(), "reopened".into()),
                ("worse".into(), "severity_increased".into())
            ])
        );
        assert!(db.finding_review(&writer, 1, "due").unwrap().is_none());
        assert!(db.finding_review(&writer, 2, "other").unwrap().is_none());
    }

    #[test]
    fn finding_reappears_after_a_completed_scan_with_no_findings() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("scan-changes.db")).unwrap();
        seed_companion_repository(&db, 1, "reappearance");
        let connection = db.connect().unwrap();
        insert_completed_run(&connection, 1, 1, "2026-08-17T12:00:00Z".into());
        insert_completed_run(&connection, 2, 1, "2026-08-18T12:00:00Z".into());
        connection
            .execute("UPDATE scan_runs SET evidence_complete=1 WHERE id=1", [])
            .unwrap();
        insert_vulnerability(&connection, 1, 2, "returned", "HIGH");
        let original = db.latest_completed_findings(1).unwrap();
        let reviewed = db.mark_finding_reviewed(1, "returned").unwrap();
        assert_eq!(reviewed.risk_basis.scanner_state, RepoFindingState::New);
        let versions = ScannerVersions {
            syft: "fixture".into(),
            grype: "fixture".into(),
        };
        let source = SourceSnapshot {
            revision: None,
            dirty: None,
        };
        let scan = |findings| {
            let run_id = db.insert_scan_start(1, &versions).unwrap();
            let result = db
                .complete_repository_scan(
                    1,
                    run_id,
                    &ScanOutput {
                        evidence: None,
                        findings,
                        packages: vec![],
                        remediation_evidence: vec![],
                    },
                    &source,
                    &[],
                    None,
                )
                .unwrap();
            assert!(result.review_warning.is_none());
            (run_id, result)
        };
        assert_eq!(scan(vec![]).1.resolved_findings, 1);
        assert_eq!(
            db.current_review_finding(1, "returned")
                .unwrap()
                .review_state,
            ReviewState::Reviewed
        );
        let (run_id, returned) = scan(original);
        assert_eq!((returned.new_findings, returned.worsened_findings), (1, 0));
        let mut query = ReviewQuery::default();
        query.filters.scan_change_run_ids = vec![run_id];
        let page = db.list_review_findings(&query).unwrap();
        assert_eq!(page.items[0].finding.state, RepoFindingState::New);
        assert_eq!(page.items[0].review_state, ReviewState::Unreviewed);
        assert_eq!(page.items[0].material_change.as_deref(), Some("reappeared"));
        db.mark_finding_reviewed(1, "returned").unwrap();
        db.reconcile_finding_reviews().unwrap();
        assert_eq!(
            db.current_review_finding(1, "returned")
                .unwrap()
                .review_state,
            ReviewState::Reviewed
        );
    }

    #[test]
    fn exact_late_advisory_reopens_once_without_changing_scanner_truth() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("advisory.db")).unwrap();
        seed_review_finding(&db, "ghsa-finding", "HIGH", Some("2.0.0"));
        db.connect().unwrap().execute("UPDATE vulnerabilities SET source_url='https://github.com/advisories/GHSA-abcd-1234-abcd'",[]).unwrap();
        db.mark_finding_reviewed(1, "ghsa-finding").unwrap();
        let before = db.data_revisions().unwrap();
        let mut details = super::super::models::AdvisoryDetails {
            fetched_at: Some(now()),
            ghsa_id: Some("GHSA-ABCD-1234-ABCD".into()),
            cve_id: Some("CVE-2026-1000".into()),
            summary: "Exact upstream advisory".into(),
            description: String::new(),
            severity: ExternalIntelSeverity::Critical,
            published_at: None,
            updated_at: None,
            vulnerabilities: Vec::new(),
            cvss_score: None,
            cvss_vector: None,
            epss_percentage: None,
            epss_percentile: None,
            cwes: Vec::new(),
            references: Vec::new(),
            source_code_location: None,
            url: "https://github.com/advisories/GHSA-abcd-1234-abcd".into(),
        };
        db.cache_advisory("GHSA-ABCD-1234-ABCD", &details, Some("etag"))
            .unwrap();
        assert!(db
            .cached_advisory("GHSA-ABCD-1234-ABCD")
            .unwrap()
            .unwrap()
            .is_fresh());
        let map =
            std::collections::BTreeMap::from([("GHSA-ABCD-1234-ABCD".into(), details.clone())]);
        assert_eq!(db.apply_scan_advisories(1, 1, &map).unwrap(), (1, 1));
        assert_eq!(db.data_revisions().unwrap().inventory, before.inventory);
        assert!(db.data_revisions().unwrap().advisory > before.advisory);
        assert_eq!(db.reconcile_finding_reviews().unwrap(), 0);
        let mut query = ReviewQuery::default();
        query.filters.scan_change_run_ids = vec![1];
        let page = db.list_review_findings(&query).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].material_change.as_deref(), Some("reopened"));
        assert_eq!(
            db.repo_detail(1)
                .unwrap()
                .latest_scan
                .unwrap()
                .reopened_count,
            1
        );
        assert_eq!(page.items[0].finding.severity, Severity::High);
        assert_eq!(page.items[0].finding.fix_version.as_deref(), Some("2.0.0"));
        assert_eq!(page.items[0].advisory_severity, Some(Severity::Critical));
        db.mark_finding_reviewed(1, "ghsa-finding").unwrap();
        assert_eq!(db.apply_scan_advisories(1, 1, &map).unwrap(), (0, 0));
        assert_eq!(db.reconcile_finding_reviews().unwrap(), 0);
        details.ghsa_id = Some("GHSA-FFFF-FFFF-FFFF".into());
        assert!(db
            .cache_advisory("GHSA-ABCD-1234-ABCD", &details, None)
            .is_err());
        assert_eq!(db.apply_scan_advisories(99, 1, &map).unwrap(), (0, 0));
    }

    #[test]
    fn installed_package_handoff_keeps_version_and_ecosystem_exact() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("identity.db")).unwrap();
        seed_review_finding(&db, "one", "HIGH", Some("2.0.0"));
        let connection = db.connect().unwrap();
        connection.execute("INSERT INTO vulnerabilities(repo_id,scan_run_id,fingerprint,advisory_id,package_name,package_version,package_type,severity,fix_version) VALUES(1,1,'two','CVE-2','review-package','2.0.0','npm','HIGH','3.0.0'),(1,1,'three','CVE-3','review-package','1.0.0','gem','HIGH',NULL)",[]).unwrap();
        let query = ReviewQuery {
            filters: super::super::models::ReviewFilters {
                package_identity: Some(super::super::models::ReviewPackageIdentity {
                    any_fix: true,
                    package_name: "review-package".into(),
                    package_url: None,
                    package_type: Some("npm".into()),
                    installed_version: Some("1.0.0".into()),
                    fix_version: None,
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let page = db.list_review_findings(&query).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].finding.fingerprint, "one");
    }

    #[test]
    fn server_sort_direction_and_secondary_fields_span_all_pages() {
        use super::super::models::{ReviewDirection, ReviewOrder};
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("sort.db")).unwrap();
        seed_review_finding(&db, "first", "HIGH", None);
        let connection = db.connect().unwrap();
        for index in 0..120 {
            connection.execute("INSERT INTO vulnerabilities(repo_id,scan_run_id,fingerprint,advisory_id,package_name,severity) VALUES(1,1,?1,?2,?3,?4)",params![format!("fp-{index:03}"),format!("CVE-{index}"),format!("package-{:03}",index%20),if index%3==0{"CRITICAL"}else{"HIGH"}]).unwrap();
        }
        let mut query = ReviewQuery {
            page_size: Some(17),
            sort: ReviewSort::Package,
            sorts: vec![
                ReviewOrder {
                    field: ReviewSort::Package,
                    direction: ReviewDirection::Desc,
                },
                ReviewOrder {
                    field: ReviewSort::Severity,
                    direction: ReviewDirection::Asc,
                },
            ],
            ..Default::default()
        };
        let mut values = Vec::new();
        loop {
            let page = db.list_review_findings(&query).unwrap();
            values.extend(page.items);
            query.cursor = page.next_cursor;
            if query.cursor.is_none() {
                break;
            }
        }
        assert_eq!(values.len(), 121);
        assert_eq!(
            values
                .iter()
                .map(|item| item.finding.fingerprint.clone())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            121
        );
        assert!(values.windows(2).all(|pair| pair[0].finding.package_name
            > pair[1].finding.package_name
            || (pair[0].finding.package_name == pair[1].finding.package_name
                && severity_rank(&pair[0].finding.severity)
                    <= severity_rank(&pair[1].finding.severity))));
    }

    #[test]
    fn review_and_snooze_never_change_scanner_truth() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init");
        seed_review_finding(&database, "review-fp", "CRITICAL", Some("2.0.0"));
        let reviewed = database
            .mark_finding_reviewed(1, "review-fp")
            .expect("review");
        assert_eq!(reviewed.state, ReviewState::Reviewed);
        assert_eq!(
            database.repo_detail(1).expect("detail").findings[0].state,
            RepoFindingState::Open
        );
        // Repository inventory remains decoupled from the canonical Review workflow.
        assert_eq!(
            database.repo_detail(1).expect("detail").findings[0].review_state,
            ReviewState::Unreviewed
        );
        let until = timestamps::format(&(OffsetDateTime::now_utc() + Duration::hours(1)));
        let snoozed = database
            .snooze_finding(1, "review-fp", &until, Some("waiting".into()))
            .expect("snooze");
        assert_eq!(snoozed.state, ReviewState::Snoozed);
        assert_eq!(snoozed.note.as_deref(), Some("waiting"));
        assert!(database
            .snooze_finding(1, "review-fp", &timestamps::now(), None)
            .is_err());
    }

    #[test]
    fn review_actions_require_the_displayed_scan_risk_and_review_state() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("review-actions.sqlite")).unwrap();
        let fingerprint = "decision-evidence";
        seed_review_finding(&db, fingerprint, "HIGH", None);
        let displayed = || {
            let item = db.current_review_finding(1, fingerprint).unwrap();
            ReviewActionBasis {
                risk_basis: Database::review_risk_basis(&item),
                scan_run_id: item.scan_run_id,
                review_state: item.review_state,
                reviewed_at: item.reviewed_at,
                snooze_until: item.snooze_until,
                review_note: item.review_note,
            }
        };
        let assert_stale = |result: Error| {
            assert!(result.to_string().starts_with("REVIEW_EVIDENCE_STALE:"));
        };

        let first = displayed();
        let next = db
            .insert_scan_start(
                1,
                &ScannerVersions {
                    syft: "fixture".into(),
                    grype: "fixture".into(),
                },
            )
            .unwrap();
        let findings = db.latest_completed_findings(1).unwrap();
        db.complete_scan(1, next, &findings, &[], &DeltaSummary::default())
            .unwrap();
        assert_stale(
            db.mark_finding_reviewed_with_evidence(1, fingerprint, Some(&first))
                .unwrap_err(),
        );

        let before_enrichment = displayed();
        db.connect().unwrap().execute(
            "UPDATE vulnerabilities SET advisory_severity='CRITICAL' WHERE scan_run_id=?1 AND fingerprint=?2",
            params![next, fingerprint],
        ).unwrap();
        let until = timestamps::format(&(OffsetDateTime::now_utc() + Duration::hours(1)));
        let revision = db.data_revisions().unwrap().review;
        assert_stale(
            db.snooze_finding_with_evidence(1, fingerprint, &until, None, Some(&before_enrichment))
                .unwrap_err(),
        );
        assert_eq!(db.data_revisions().unwrap().review, revision);
        assert_eq!(
            db.current_review_finding(1, fingerprint)
                .unwrap()
                .review_state,
            ReviewState::Unreviewed,
        );

        let current = displayed();
        let review = db
            .mark_finding_reviewed_with_evidence(1, fingerprint, Some(&current))
            .unwrap();
        assert_eq!(review.risk_basis.severity, Severity::Critical);
        assert_stale(
            db.clear_finding_review_with_evidence(1, fingerprint, Some(&current))
                .unwrap_err(),
        );
        assert_eq!(
            db.current_review_finding(1, fingerprint)
                .unwrap()
                .review_state,
            ReviewState::Reviewed,
        );
        db.clear_finding_review_with_evidence(1, fingerprint, Some(&displayed()))
            .unwrap();
        db.snooze_finding_with_evidence(
            1,
            fingerprint,
            &until,
            Some("follow up".into()),
            Some(&displayed()),
        )
        .unwrap();
        assert_eq!(
            db.current_review_finding(1, fingerprint)
                .unwrap()
                .review_note
                .as_deref(),
            Some("follow up"),
        );
    }

    #[test]
    fn dock_review_count_tracks_review_snooze_and_return_without_resolving_finding() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init");
        seed_review_finding(&database, "dock-review-fp", "HIGH", None);
        let read_update = || {
            database
                .review_summary_with_revision()
                .expect("review update")
        };
        let initial = read_update();
        assert_eq!(initial.review_summary.needs_review, 1);
        database
            .mark_finding_reviewed(1, "dock-review-fp")
            .expect("review");
        let reviewed = read_update();
        assert!(reviewed.revision > initial.revision);
        assert_eq!(reviewed.review_summary.needs_review, 0);
        assert_eq!(reviewed.review_summary.open_reviewed, 1);
        database
            .clear_finding_review(1, "dock-review-fp")
            .expect("return");
        let returned = read_update();
        assert!(returned.revision > reviewed.revision);
        assert_eq!(returned.review_summary.needs_review, 1);
        let until = timestamps::format(&(OffsetDateTime::now_utc() + Duration::hours(1)));
        database
            .snooze_finding(1, "dock-review-fp", &until, None)
            .expect("snooze");
        let snoozed = read_update();
        assert!(snoozed.revision > returned.revision);
        assert_eq!(snoozed.review_summary.needs_review, 0);
        assert_eq!(snoozed.review_summary.snoozed, 1);
        let connection = database.connect().expect("connect");
        connection
            .execute(
                "UPDATE finding_reviews SET snooze_until = ?1 WHERE fingerprint = 'dock-review-fp'",
                [timestamps::format(
                    &(OffsetDateTime::now_utc() - Duration::minutes(1)),
                )],
            )
            .expect("expire snooze");
        drop(connection);
        let reopened = read_update();
        assert!(reopened.revision > snoozed.revision);
        assert_eq!(reopened.review_summary.needs_review, 1);
        assert_eq!(reopened.review_summary.due_snoozed, 1);
        assert_eq!(read_update().revision, reopened.revision);
    }

    #[test]
    fn snapshot_reads_one_database_version_while_a_writer_commits() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("snapshot.db")).unwrap();
        seed_review_finding(&db, "initial", "HIGH", None);
        db.reconcile_finding_reviews().unwrap();
        let previous = db.data_revisions().unwrap();
        let writer = db.connect().unwrap();

        let (repo, review, revisions) = db.read_reconciled_review(|connection, due, _| {
            let repo = Database::repo_summaries_on(connection, None)?.remove(0);
            writer.execute("UPDATE vulnerabilities SET severity = 'CRITICAL' WHERE fingerprint = 'initial'", [])?;
            Ok((repo, db.review_summary_on(connection, due)?, Database::read_data_revisions(connection)?))
        }).unwrap();
        assert_eq!(repo.open_high, 1);
        assert_eq!(repo.open_critical, 0);
        assert_eq!(review.unreviewed_critical, 0);
        assert_eq!(revisions, previous);
        assert!(db.data_revisions().unwrap().inventory > previous.inventory);
        assert_eq!(db.review_summary().unwrap().unreviewed_critical, 1);
    }

    #[test]
    fn exact_advisory_flags_combine_sources_and_reopen_material_risk() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("flags.db")).unwrap();
        seed_companion_repository(&db, 1, "risk-repository");
        let conn = db.connect().unwrap();
        insert_completed_run(&conn, 1, 1, now());
        insert_vulnerability_with_package(
            &conn,
            1,
            1,
            "risk-fp",
            "CVE-2026-9999",
            "package",
            "HIGH",
            "pkg:npm/package@1",
        );
        insert_external_intel_item(
            &conn,
            1,
            "cve-2026-9999",
            "cisa_kev",
            "CVE-2026-9999",
            "Risk",
            "Risk",
            "https://example.test/ransomware",
            "HIGH",
            &[],
            &["CVE-2026-9999"],
        );
        conn.execute(
            "UPDATE external_advisories SET is_ransomware_known=1 WHERE id=1",
            [],
        )
        .unwrap();
        db.mark_finding_reviewed(1, "risk-fp").unwrap();
        assert_eq!(db.review_summary().unwrap().open_reviewed, 1);
        insert_external_intel_item(
            &conn,
            2,
            "kev",
            "github_advisory",
            "kev-source",
            "Risk",
            "Risk",
            "https://example.test/kev",
            "HIGH",
            &[],
            &["CVE-2026-9999"],
        );
        conn.execute("UPDATE external_advisories SET is_kev=1 WHERE id=2", [])
            .unwrap();
        let finding = db.current_review_finding(1, "risk-fp").unwrap();
        assert!(finding.known_exploited);
        assert!(finding.ransomware_known);
        let mut query = ReviewQuery::default();
        query.filters.known_exploited = Some(true);
        query.filters.ransomware_known = Some(true);
        let page = db.list_review_findings(&query).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].review_state, ReviewState::Unreviewed);
        assert_eq!(db.review_summary().unwrap().needs_review, 1);
    }

    #[test]
    fn review_sql_matches_canonical_projection_filters_sorts_and_keyset_pages() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("parity.db")).unwrap();
        for (id, name) in [(1, "Äther"), (2, "beta"), (3, "empty-previous")] {
            seed_companion_repository(&db, id, name);
        }
        let conn = db.connect().unwrap();
        conn.execute("UPDATE repos SET source_type='github' WHERE id=2", [])
            .unwrap();
        for (id, repo, age) in [
            (1, 1, 3),
            (2, 1, 2),
            (3, 1, 1),
            (4, 2, 2),
            (5, 2, 1),
            (6, 3, 2),
            (7, 3, 1),
        ] {
            insert_completed_run(
                &conn,
                id,
                repo,
                timestamps::format(&(OffsetDateTime::now_utc() - Duration::hours(age))),
            );
        }
        conn.execute("UPDATE scan_runs SET evidence_complete=1 WHERE id=2", [])
            .unwrap();
        for (repo, run, fp, severity) in [
            (1, 1, "returned", "HIGH"),
            (1, 2, "open", "HIGH"),
            (1, 2, "resolved", "MEDIUM"),
            (1, 3, "open", "HIGH"),
            (1, 3, "new", "CRITICAL"),
            (1, 3, "returned", "HIGH"),
            (2, 4, "other", "HIGH"),
            (2, 5, "other", "HIGH"),
            (2, 5, "fresh", "HIGH"),
            (3, 7, "empty-open", "MEDIUM"),
        ] {
            insert_vulnerability_with_package(
                &conn,
                repo,
                run,
                fp,
                fp,
                "Äther-package",
                severity,
                "pkg:npm/test@1.0.0",
            );
        }
        conn.execute(
            "UPDATE vulnerabilities SET fix_version='2.0' WHERE fingerprint IN ('new','open')",
            [],
        )
        .unwrap();
        insert_external_intel_item(
            &conn,
            1,
            "canonical-risk",
            "cisa_kev",
            "source-risk",
            "risk",
            "risk",
            "https://example.com",
            "HIGH",
            &[],
            &["returned"],
        );
        conn.execute("UPDATE external_advisories SET is_kev=1 WHERE id=1", [])
            .unwrap();
        drop(conn);
        db.mark_finding_reviewed(1, "open").unwrap();
        db.snooze_finding(
            2,
            "other",
            &timestamps::format(&(OffsetDateTime::now_utc() + Duration::hours(1))),
            None,
        )
        .unwrap();
        db.reconcile_finding_reviews().unwrap();
        let oracle = db.all_review_findings().unwrap();
        let summary = db.review_summary_with_revision().unwrap().review_summary;
        assert_eq!(
            summary,
            ReviewSummary {
                needs_review: 4,
                unreviewed_critical: 1,
                unreviewed_other: 3,
                open_reviewed: 1,
                snoozed: 1,
                recently_resolved: 1,
                due_snoozed: 0,
                failed_scans: 0,
            }
        );
        let mut queries = vec![ReviewQuery::default()];
        for states in [
            vec![RepoFindingState::New],
            vec![RepoFindingState::Open],
            vec![RepoFindingState::Open, RepoFindingState::New],
            vec![RepoFindingState::Resolved],
            vec![RepoFindingState::Open, RepoFindingState::Resolved],
        ] {
            let mut q = ReviewQuery::default();
            q.filters.scanner_states = states;
            queries.push(q);
        }
        for states in [
            vec![ReviewState::Reviewed],
            vec![ReviewState::Unreviewed, ReviewState::Snoozed],
        ] {
            let mut q = ReviewQuery::default();
            q.filters.review_states = states;
            queries.push(q);
        }
        for severity in [Severity::Medium, Severity::High, Severity::Critical] {
            let mut q = ReviewQuery::default();
            q.filters.severities = vec![severity];
            queries.push(q);
        }
        for fix in [true, false] {
            let mut q = ReviewQuery::default();
            q.filters.fix_available = Some(fix);
            queries.push(q);
        }
        for repo in [1, 2, 3, 99] {
            let mut q = ReviewQuery::default();
            q.filters.repo_ids = vec![repo];
            queries.push(q);
        }
        let mut exact = ReviewQuery::default();
        exact.filters.repo_ids = vec![1];
        exact.filters.fingerprint = Some("returned".into());
        queries.push(exact);
        for source in [RepoSourceType::Local, RepoSourceType::Github] {
            let mut q = ReviewQuery::default();
            q.filters.source_types = vec![source];
            queries.push(q);
        }
        for known in [true, false] {
            let mut q = ReviewQuery::default();
            q.filters.known_exploited = Some(known);
            q.filters.ransomware_known = Some(false);
            queries.push(q);
        }
        for search in ["äther", "RETURNED", "not found"] {
            let q = ReviewQuery {
                search: Some(search.into()),
                ..Default::default()
            };
            queries.push(q);
        }
        let mut q = ReviewQuery::default();
        q.filters.packages = vec!["Äther-PACKAGE".into()];
        q.filters.ecosystems = vec!["NPM".into()];
        q.filters.scan_run_ids = vec![3, 5];
        q.filters.changed_since = Some(timestamps::format(
            &(OffsetDateTime::now_utc() - Duration::hours(2)),
        ));
        queries.push(q);
        for mut q in queries {
            for sort in [
                ReviewSort::Priority,
                ReviewSort::Severity,
                ReviewSort::Recent,
                ReviewSort::Repository,
                ReviewSort::Package,
            ] {
                q.sort = sort;
                q.page_size = Some(2);
                q.cursor = None;
                let mut expected = oracle
                    .iter()
                    .filter(|item| review_item_matches(item, &q))
                    .cloned()
                    .collect::<Vec<_>>();
                sort_review_items(&mut expected, sort);
                let counts = review_group_counts(&expected);
                let mut actual = Vec::new();
                loop {
                    let page = db.list_review_findings(&q).unwrap();
                    assert_eq!(page.total, expected.len() as u32);
                    assert_eq!(page.group_counts, counts);
                    actual.extend(page.items);
                    q.cursor = page.next_cursor;
                    if q.cursor.is_none() {
                        break;
                    }
                }
                assert_eq!(actual, expected, "{q:?}");
            }
        }
        let mut q = ReviewQuery {
            page_size: Some(2),
            ..Default::default()
        };
        q.cursor = db.list_review_findings(&q).unwrap().next_cursor;
        db.mark_finding_reviewed(1, "new").unwrap();
        assert!(db
            .list_review_findings(&q)
            .unwrap_err()
            .to_string()
            .contains("REVIEW_CURSOR_STALE"));
    }

    #[test]
    fn review_refresh_preserves_sorted_prefix_and_cursor_under_the_same_filters() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("review-refresh.db")).unwrap();
        seed_companion_repository(&db, 1, "javascript");
        seed_companion_repository(&db, 2, "another-repository");
        let connection = db.connect().unwrap();
        insert_completed_run(&connection, 1, 1, "2026-11-14T11:00:00Z".into());
        insert_completed_run(&connection, 2, 2, "2026-11-14T11:00:00Z".into());
        connection.execute_batch(
            "INSERT INTO vulnerabilities(repo_id,scan_run_id,fingerprint,advisory_id,package_name,severity) VALUES
             (1,1,'first','CVE-2099-1','a-js','HIGH'),
             (1,1,'selected','CVE-2099-2','c-js','HIGH'),
             (1,1,'last','CVE-2099-3','d-js','HIGH'),
             (2,2,'selected','CVE-2099-4','b-js','HIGH');",
        ).unwrap();
        let mut query = ReviewQuery {
            page_size: Some(1),
            sort: ReviewSort::Package,
            search: Some("JS".into()),
            ..Default::default()
        };
        query.filters.scanner_states = vec![RepoFindingState::New, RepoFindingState::Open];
        let first = db.list_review_findings(&query).unwrap();
        assert_eq!(first.total, 4);
        let refreshed = db
            .list_review_findings_preserving(&query, Some("1:selected"), Some(2))
            .unwrap();
        let identities = refreshed
            .items
            .iter()
            .map(|item| (item.repo_id, item.finding.fingerprint.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(identities, [(1, "first"), (2, "selected"), (1, "selected")]);
        assert_eq!(refreshed.total, first.total);
        assert_eq!(refreshed.group_counts, first.group_counts);

        query.cursor = refreshed.next_cursor;
        let last = db
            .list_review_findings_preserving(&query, Some("1:first"), Some(500))
            .unwrap();
        assert_eq!(last.items.len(), 1);
        assert_eq!(last.items[0].finding.fingerprint, "last");
        assert!(last.next_cursor.is_none());

        query.cursor = None;
        query.filters.review_states = vec![ReviewState::Unreviewed];
        db.mark_finding_reviewed(1, "selected").unwrap();
        let filtered = db
            .list_review_findings_preserving(&query, Some("1:selected"), Some(2))
            .unwrap();
        assert!(filtered.revision > refreshed.revision);
        assert_eq!(filtered.total, 3);
        assert_eq!(filtered.items.len(), 2);
        assert!(filtered
            .items
            .iter()
            .all(|item| item.repo_id != 1 || item.finding.fingerprint != "selected"));
        assert!(filtered.next_cursor.is_some());
    }

    #[test]
    fn review_refresh_keeps_a_deep_selection_and_resumes_at_the_correct_row() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("deep-review.db")).unwrap();
        seed_companion_repository(&db, 1, "javascript");
        let connection = db.connect().unwrap();
        insert_completed_run(&connection, 1, 1, "2026-11-14T11:00:00Z".into());
        connection.execute_batch(
            "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM seq WHERE n<2010)
             INSERT INTO vulnerabilities(repo_id,scan_run_id,fingerprint,advisory_id,package_name,severity)
             SELECT 1,1,printf('fp-%04d',n),printf('CVE-2099-%04d',n),printf('pkg-%04d',n),'HIGH'
             FROM seq;",
        ).unwrap();
        let mut query = ReviewQuery {
            page_size: Some(100),
            sort: ReviewSort::Package,
            ..Default::default()
        };
        let first = db.list_review_findings(&query).unwrap();
        assert_eq!(first.items.len(), 100);

        // A finding appeared before the last selection since the user loaded
        // four pages. Keep that selection and advance the cursor past it.
        connection.execute(
            "INSERT INTO vulnerabilities(repo_id,scan_run_id,fingerprint,advisory_id,package_name,severity)
             VALUES (1,1,'earlier','CVE-2099-0000','pkg-0000','HIGH')",
            [],
        ).unwrap();
        let restored = db
            .list_review_findings_preserving(&query, Some("1:fp-0400"), Some(400))
            .unwrap();
        assert_eq!(restored.items.len(), 401);
        assert_eq!(restored.items[0].finding.fingerprint, "earlier");
        assert_eq!(restored.items[400].finding.fingerprint, "fp-0400");
        assert_eq!(restored.total, 2011);
        query.cursor = restored.next_cursor;
        let following = db.list_review_findings(&query).unwrap();
        assert_eq!(following.items[0].finding.fingerprint, "fp-0401");

        query.cursor = None;
        let bounded = db
            .list_review_findings_preserving(&query, None, Some(u32::MAX))
            .unwrap();
        assert_eq!(bounded.items.len(), 2_000);
        assert!(bounded.next_cursor.is_some());
    }

    #[test]
    fn review_page_counts_follow_filters_and_changes_between_pages() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("review-counts.db")).unwrap();
        seed_review_finding(&db, "critical", "CRITICAL", Some("2.0.0"));
        let conn = db.connect().unwrap();
        conn.execute(
            "INSERT INTO vulnerabilities (repo_id,scan_run_id,fingerprint,advisory_id,package_name,severity) VALUES (1,1,'medium','CVE-TEST-MEDIUM','other','MEDIUM')",
            [],
        )
        .unwrap();
        drop(conn);
        let mut query = ReviewQuery {
            page_size: Some(1),
            ..Default::default()
        };
        query.filters.scanner_states = vec![RepoFindingState::Open, RepoFindingState::New];
        query.filters.review_states = vec![ReviewState::Unreviewed];
        let first = db.list_review_findings(&query).unwrap();
        assert_eq!(first.total, 2);
        query.cursor = first.next_cursor.clone();
        let next = db.clone().list_review_findings(&query).unwrap();
        assert_eq!(next.total, 2);
        assert_eq!(next.group_counts, first.group_counts);
        assert_ne!(
            next.items[0].finding.fingerprint,
            first.items[0].finding.fingerprint
        );

        db.mark_finding_reviewed(1, "critical").unwrap();
        assert!(db
            .list_review_findings(&query)
            .unwrap_err()
            .to_string()
            .contains("REVIEW_CURSOR_STALE"));
        query.cursor = None;
        let remaining = db.list_review_findings(&query).unwrap();
        assert_eq!(remaining.total, 1);
        assert_eq!(remaining.group_counts.severity[0].count, 0);
        assert_eq!(remaining.group_counts.severity[2].count, 1);
        assert_eq!(remaining.group_counts.fix_availability[0].count, 0);

        query.filters.review_states = vec![ReviewState::Reviewed];
        let reviewed = db.list_review_findings(&query).unwrap();
        assert_eq!(reviewed.revision, remaining.revision);
        assert_eq!(reviewed.group_counts.severity[0].count, 1);
        assert_eq!(reviewed.group_counts.fix_availability[0].count, 1);
        assert_eq!(reviewed.items[0].finding.fingerprint, "critical");
        query.search = Some("missing".into());
        let filtered = db.list_review_findings(&query).unwrap();
        assert_eq!(filtered.revision, reviewed.revision);
        assert_eq!(filtered.total, 0);
        assert!(filtered.items.is_empty());
        assert!(filtered.next_cursor.is_none());
        assert!(filtered.group_counts.repository.is_empty());
    }

    #[test]
    fn persisted_revisions_are_scoped_and_chat_does_not_invalidate_review() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("revisions.db")).unwrap();
        seed_review_finding(&db, "one", "HIGH", None);
        let before = db.data_revisions().unwrap();
        db.set_setting("theme", "dark".into()).unwrap();
        assert_eq!(db.data_revisions().unwrap().review, before.review);
        let thread = db
            .create_companion_thread(1, "thread-1", "chat", None)
            .unwrap();
        db.rename_companion_thread(thread.id, "Renamed").unwrap();
        assert_eq!(db.data_revisions().unwrap().review, before.review);
        db.mark_finding_reviewed(1, "one").unwrap();
        assert!(db.data_revisions().unwrap().review > before.review);
        assert_eq!(db.data_revisions().unwrap().inventory, before.inventory);
    }

    #[test]
    fn unsuccessful_scan_attempts_preserve_dependency_and_review_cursors() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("failed-scan-revisions.db")).unwrap();
        seed_review_finding(&db, "existing", "HIGH", None);
        insert_vulnerability(&db.connect().unwrap(), 1, 1, "another", "MEDIUM");
        let query = ReviewQuery {
            page_size: Some(1),
            ..Default::default()
        };
        let page = db.list_review_findings(&query).unwrap();
        let continuation = ReviewQuery {
            cursor: Some(page.next_cursor.unwrap()),
            ..query.clone()
        };
        let next_before = db.list_review_findings(&continuation).unwrap();
        let mut dependency_query = super::super::models::DependencyQuery {
            page_size: Some(1),
            expected_scan_run_id: Some(1),
            ..Default::default()
        };
        let dependency_first = db.dependency_page(1, &dependency_query).unwrap();
        dependency_query.cursor = Some(dependency_first.next_cursor.unwrap());
        let dependency_next = db.dependency_page(1, &dependency_query).unwrap();
        let feed = db.external_intel_feed().unwrap();
        let before = db.data_revisions().unwrap();
        let overview = db.repo_overview(1).unwrap();
        let run = db
            .insert_scan_start(
                1,
                &ScannerVersions {
                    syft: "fixture".into(),
                    grype: "fixture".into(),
                },
            )
            .unwrap();
        let started = db.data_revisions().unwrap();
        assert_eq!(started.inventory, before.inventory);
        assert_eq!(started.review, before.review);
        assert!(started.scan_history > before.scan_history);
        let while_running = db.dependency_page(1, &dependency_query).unwrap();
        assert_eq!(while_running.revision, dependency_next.revision);
        assert_eq!(while_running.items, dependency_next.items);

        db.fail_scan(1, run, "Synthetic scanner error", None)
            .unwrap();
        let failed = db.data_revisions().unwrap();
        assert_eq!(failed.inventory, before.inventory);
        assert_eq!(failed.review, before.review);
        assert!(failed.scan_history > started.scan_history);
        assert_eq!(
            db.external_intel_feed().unwrap().inventory_revision,
            feed.inventory_revision
        );
        assert_eq!(db.list_review_findings(&continuation).unwrap(), next_before);
        let after_failure = db.dependency_page(1, &dependency_query).unwrap();
        assert_eq!(after_failure.revision, dependency_next.revision);
        assert_eq!(
            after_failure.inventory_count,
            dependency_next.inventory_count
        );
        assert_eq!(after_failure.items, dependency_next.items);
        let failed_overview = db.repo_overview(1).unwrap();
        assert_eq!(
            failed_overview.summary.last_scan_status,
            Some(ScanRunStatus::Failed)
        );
        assert!(failed_overview.summary.evidence_revision > overview.summary.evidence_revision);
        assert_eq!(
            failed_overview
                .latest_scan
                .unwrap()
                .error_message
                .as_deref(),
            Some("Synthetic scanner error")
        );
        assert_eq!(
            failed_overview.overview.latest_completed_scan,
            overview.overview.latest_completed_scan
        );
        assert_eq!(db.review_summary().unwrap().failed_scans, 1);

        let completed = db
            .insert_scan_start(
                1,
                &ScannerVersions {
                    syft: "fixture".into(),
                    grype: "fixture".into(),
                },
            )
            .unwrap();
        db.complete_scan(1, completed, &[], &[], &DeltaSummary::default())
            .unwrap();
        let after = db.data_revisions().unwrap();
        assert!(after.inventory > before.inventory);
        assert!(after.review > before.review);
        assert!(after.scan_history > failed.scan_history);
        assert!(db
            .list_review_findings(&continuation)
            .unwrap_err()
            .to_string()
            .contains("REVIEW_CURSOR_STALE"));
        assert!(db
            .dependency_page(1, &dependency_query)
            .unwrap_err()
            .to_string()
            .contains("DEPENDENCY_CURSOR_STALE"));
        dependency_query.cursor = None;
        dependency_query.expected_scan_run_id = Some(completed);
        let after_success = db.dependency_page(1, &dependency_query).unwrap();
        assert!(after_success.revision > dependency_next.revision);
        assert_eq!(after_success.inventory_count, 0);
        assert_eq!(db.review_summary().unwrap().failed_scans, 0);
    }

    #[test]
    fn repository_evidence_revisions_follow_only_own_inputs_and_exact_intel() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("repo-revisions.db")).unwrap();
        seed_review_finding(&db, "CVE-EXACT", "HIGH", None);
        let connection = db.connect().unwrap();
        seed_companion_repository(&db, 2, "other");
        let before = db.repo_overview(1).unwrap().summary;
        connection
            .execute("UPDATE repos SET name='other-renamed' WHERE id=2", [])
            .unwrap();
        assert_eq!(
            db.repo_overview(1).unwrap().summary.evidence_revision,
            before.evidence_revision
        );
        insert_external_intel_item(
            &connection,
            90,
            "CVE-UNRELATED",
            "github",
            "GHSA-OTHER",
            "Other",
            "Summary",
            "https://example.test",
            "HIGH",
            &[],
            &[],
        );
        assert_eq!(
            db.repo_overview(1).unwrap().summary.evidence_revision,
            before.evidence_revision
        );
        connection
            .execute(
                "INSERT INTO external_advisory_aliases(advisory_id,alias) VALUES(90,'cve-2026-1000')",
                [],
            )
            .unwrap();
        let exact_revision = db.repo_overview(1).unwrap().summary.evidence_revision;
        assert!(exact_revision > before.evidence_revision);
        connection
            .execute(
                "UPDATE external_advisories SET severity='CRITICAL',is_kev=1 WHERE id=90",
                [],
            )
            .unwrap();
        assert!(db.repo_overview(1).unwrap().summary.evidence_revision > exact_revision);
        let current = db.repo_overview(1).unwrap().summary.evidence_revision;
        connection
            .execute(
                "DELETE FROM external_advisory_aliases WHERE advisory_id=90",
                [],
            )
            .unwrap();
        let after = db.repo_overview(1).unwrap().summary;
        assert!(after.evidence_revision > current);
        assert_eq!(after.dependency_revision, before.dependency_revision);
    }

    #[test]
    fn bulk_scan_revision_and_evidence_commit_or_roll_back_together() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("bulk-rollback.db")).unwrap();
        seed_review_finding(&db, "old", "HIGH", None);
        let versions = ScannerVersions {
            syft: "fixture".into(),
            grype: "fixture".into(),
        };
        let run = db.insert_scan_start(1, &versions).unwrap();
        let finding = NormalizedFinding {
            fingerprint: "new".into(),
            advisory_id: "CVE-NEW".into(),
            package_name: "new-package".into(),
            package_version: Some("1.0".into()),
            package_type: Some("npm".into()),
            package_url: None,
            severity: Severity::High,
            fix_version: None,
            title: None,
            source_url: None,
            advisory_details: None,
        };
        let package = RepoPackage {
            name: "new-package".into(),
            version: Some("1.0".into()),
            package_type: Some("npm".into()),
            package_url: None,
            highest_severity: None,
            finding_count: 0,
            fix_available: false,
        };
        let before = db.data_revisions().unwrap();
        let own_before = db.repo_overview(1).unwrap().summary;
        let connection = db.connect().unwrap();
        connection.execute_batch("CREATE TRIGGER reject_package AFTER INSERT ON scan_package_snapshots BEGIN SELECT RAISE(ABORT,'test write failure'); END;").unwrap();
        assert!(db
            .complete_scan(
                1,
                run,
                std::slice::from_ref(&finding),
                std::slice::from_ref(&package),
                &DeltaSummary::default()
            )
            .is_err());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM performance_write_scope", [], |row| {
                    row.get::<_, u32>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM vulnerabilities WHERE scan_run_id=?1",
                    [run],
                    |row| row.get::<_, u32>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT status FROM scan_runs WHERE id=?1", [run], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "running"
        );
        assert_eq!(db.data_revisions().unwrap(), before);
        assert_eq!(db.repo_overview(1).unwrap().summary, own_before);
        connection
            .execute_batch("DROP TRIGGER reject_package;")
            .unwrap();
        db.complete_scan(1, run, &[finding], &[package], &DeltaSummary::default())
            .unwrap();
        assert_eq!(db.data_revisions().unwrap().inventory, before.inventory + 1);
        let own_after = db.repo_overview(1).unwrap().summary;
        assert_eq!(
            own_after.evidence_revision,
            own_before.evidence_revision + 1
        );
        assert_eq!(
            own_after.dependency_revision,
            own_before.dependency_revision + 1
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM performance_write_scope", [], |row| {
                    row.get::<_, u32>(0)
                })
                .unwrap(),
            0
        );
        connection
            .execute("UPDATE repos SET name='legacy write' WHERE id=1", [])
            .unwrap();
        assert!(db.data_revisions().unwrap().inventory > before.inventory + 1);
    }

    #[test]
    fn overview_remediation_groups_preserve_distinct_fix_targets() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("inventory.db")).unwrap();
        seed_companion_repository(&db, 1, "Grouped fixes");
        let connection = db.connect().unwrap();
        insert_completed_run(&connection, 1, 1, "2026-01-02T12:00:00Z".into());
        connection.execute_batch(
            "INSERT INTO vulnerabilities(repo_id,scan_run_id,fingerprint,advisory_id,package_name,package_version,package_type,package_url,severity,fix_version) VALUES
             (1,1,'one','CVE-1','dep','1.0.0','npm','pkg:npm/dep@1.0.0','HIGH','1.1.0'),
             (1,1,'two','CVE-2','dep','1.0.0','npm','pkg:npm/dep@1.0.0','CRITICAL','1.1.0'),
             (1,1,'three','CVE-3','dep','1.0.0','npm','pkg:npm/dep@1.0.0','HIGH','2.0.0');",
        ).unwrap();
        let metadata = db.repo_overview(1).unwrap().overview;
        assert_eq!(metadata.opportunity_count, 2);
        assert_eq!(
            metadata.opportunities[0].fix_version.as_deref(),
            Some("1.1.0")
        );
        assert_eq!(metadata.opportunities[0].finding_count, 2);
        assert_eq!(
            (
                metadata.opportunities[0].critical,
                metadata.opportunities[0].high
            ),
            (1, 1)
        );
        assert_eq!(
            metadata.opportunities[1].fix_version.as_deref(),
            Some("2.0.0")
        );
        assert_eq!(metadata.opportunities[1].finding_count, 1);
    }

    #[test]
    fn lean_overview_and_dependency_pages_preserve_counts_without_detail_bodies() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("inventory.db")).unwrap();
        seed_review_finding(&db, "inventory-fp", "HIGH", Some("2.0"));
        let conn = db.connect().unwrap();
        for index in 0..243 {
            conn.execute("INSERT INTO scan_packages(repo_id,scan_run_id,name,version,package_type,package_url) VALUES (1,1,?1,'1.0.0','npm',?2)",params![format!("pkg-{index:04}"),format!("pkg:npm/pkg-{index}@1.0.0")]).unwrap();
        }
        drop(conn);
        let full = db.repo_detail(1).unwrap();
        let overview = db.repo_overview(1).unwrap();
        let metadata = &overview.overview;
        let wire = serde_json::to_value(&overview).unwrap();
        assert!(wire.get("packages").is_none() && wire.get("assets").is_none());
        assert!(overview.findings.len() <= 6);
        assert!(overview.findings.iter().all(|item| item.intel.is_none()));
        assert_eq!(metadata.package_count, full.packages.len() as u32);
        assert_eq!(metadata.affected_count, 1);
        assert_eq!(metadata.open_finding_count, 1);
        let expected = build_test_opportunities(&full.findings);
        assert_eq!(metadata.opportunity_count, expected);
        assert_eq!(metadata.opportunities[0].finding_count, 1);
        let mut query = super::super::models::DependencyQuery {
            page_size: Some(17),
            expected_scan_run_id: Some(1),
            ..Default::default()
        };
        let mut items = Vec::new();
        loop {
            let page = db.dependency_page(1, &query).unwrap();
            assert_eq!(page.total, 244);
            assert!(page.items.len() <= 17);
            items.extend(page.items);
            query.cursor = page.next_cursor;
            if query.cursor.is_none() {
                break;
            }
        }
        items.sort_by(|a, b| a.name.cmp(&b.name));
        let mut full_packages = full.packages;
        full_packages.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(items, full_packages);
        query.cursor = None;
        query.affected_only = Some(true);
        let page = db.dependency_page(1, &query).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "review-package");
        assert!(page.items[0].fix_available);
        assert_eq!(page.inventory_count, 244);
        assert_eq!(page.affected_count, 1);
        assert_eq!(page.ecosystems, ["npm"]);
        // A finding can be updated within the same completed run.
        db.connect().unwrap().execute(
            "UPDATE vulnerabilities SET fix_version=NULL, severity='CRITICAL' WHERE fingerprint='inventory-fp'",
            [],
        ).unwrap();
        let updated = db.dependency_page(1, &query).unwrap();
        assert_eq!(updated.scan_run_id, page.scan_run_id);
        assert!(updated.revision > page.revision);
        assert_eq!(updated.items[0].highest_severity, Some(Severity::Critical));
        assert!(!updated.items[0].fix_available);
        query.search = Some("no matching dependency".into());
        let empty = db.dependency_page(1, &query).unwrap();
        assert_eq!(empty.total, 0);
        assert!(empty.items.is_empty());
        assert_eq!(empty.next_cursor, None);
        assert_eq!(empty.inventory_count, 244);
        assert_eq!(empty.affected_count, 1);
        assert_eq!(empty.ecosystems, ["npm"]);
        let conn = db.connect().unwrap();
        insert_completed_run(
            &conn,
            2,
            1,
            timestamps::format(&(OffsetDateTime::now_utc() + Duration::minutes(1))),
        );
        drop(conn);
        assert!(db
            .dependency_page(1, &query)
            .unwrap_err()
            .to_string()
            .contains("DEPENDENCY_CURSOR_STALE"));
        fn build_test_opportunities(findings: &[RepoFinding]) -> u32 {
            findings
                .iter()
                .filter(|item| {
                    item.state != RepoFindingState::Resolved && item.fix_version.is_some()
                })
                .map(|item| {
                    format!(
                        "{}|{:?}|{:?}",
                        item.package_url.as_deref().unwrap_or(&item.package_name),
                        item.package_version,
                        item.fix_version
                    )
                })
                .collect::<std::collections::HashSet<_>>()
                .len() as u32
        }
    }

    #[test]
    fn overview_counts_stored_package_rows_and_adds_each_missing_identity_once() {
        use super::super::models::DependencyQuery;

        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("overview-counts.db")).unwrap();
        seed_companion_repository(&db, 1, "inventory");
        let connection = db.connect().unwrap();
        insert_completed_run(&connection, 1, 1, "2026-08-15T10:00:00Z".into());
        connection.execute_batch(
            "INSERT INTO scan_packages(repo_id,scan_run_id,name,version,package_type,package_url) VALUES
                (1,1,'present','1.0.0','npm','pkg:npm/present@1.0.0'),
                (1,1,'present alias','1.0.0','npm','pkg:npm/present@1.0.0'),
                (1,1,'clean','1.0.0','npm',NULL);
             INSERT INTO vulnerabilities(repo_id,scan_run_id,fingerprint,advisory_id,package_name,package_version,package_type,package_url,severity) VALUES
                (1,1,'present','CVE-1','present','1.0.0','npm','pkg:npm/present@1.0.0','HIGH'),
                (1,1,'missing-1','CVE-2','missing','2.0.0','npm',NULL,'HIGH'),
                (1,1,'missing-2','CVE-3','missing','2.0.0','npm',NULL,'MEDIUM');",
        ).unwrap();
        let overview = db.repo_overview(1).unwrap().overview;
        let page = db.dependency_page(1, &DependencyQuery::default()).unwrap();
        assert_eq!(overview.package_count, 4);
        assert_eq!(overview.affected_count, 3);
        assert_eq!(overview.open_finding_count, 3);
        assert_eq!(page.inventory_count, overview.package_count);
        assert_eq!(page.affected_count, overview.affected_count);
        for (search, expected_total) in [("present", 2), ("missing", 1), ("clean", 1)] {
            let mut query = DependencyQuery {
                search: Some(search.into()),
                page_size: Some(1),
                ..Default::default()
            };
            let first = db.dependency_page(1, &query).unwrap();
            assert_eq!(first.total, expected_total);
            query.sort = Some("ecosystem".into());
            let resorted = db.dependency_page(1, &query).unwrap();
            assert_eq!(resorted.total, expected_total);
            assert_eq!(resorted.affected_count, 3);
            if let Some(cursor) = resorted.next_cursor {
                query.cursor = Some(cursor);
                let next = db.dependency_page(1, &query).unwrap();
                assert_ne!(next.items[0].name, resorted.items[0].name);
                assert_eq!(next.total, expected_total);
                assert!(next.next_cursor.is_none());
            }
        }

        insert_completed_run(&connection, 2, 1, "2026-08-15T11:00:00Z".into());
        connection
            .execute(
                "INSERT INTO scan_packages(repo_id,scan_run_id,name,version,package_type)
             VALUES (1,2,'clean','1.0.0','npm')",
                [],
            )
            .unwrap();
        let detail = db.repo_overview(1).unwrap();
        let overview = detail.overview;
        let page = db.dependency_page(1, &DependencyQuery::default()).unwrap();
        assert_eq!(overview.latest_completed_scan.unwrap().id, 2);
        assert_eq!(overview.package_count, 1);
        assert_eq!(overview.affected_count, 0);
        assert_eq!(overview.open_finding_count, 0);
        assert_eq!(overview.opportunity_count, 0);
        assert!(detail.findings.is_empty());
        assert_eq!(page.inventory_count, 1);
        assert_eq!(page.affected_count, 0);
    }

    #[test]
    fn dependency_cursor_rejects_other_repositories_with_equal_revisions() {
        use super::super::models::DependencyQuery;

        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("cursor.db")).unwrap();
        let connection = db.connect().unwrap();
        for (id, name) in [(1, "javascript"), (2, "python")] {
            seed_companion_repository(&db, id, name);
            insert_completed_run(&connection, id, id, "2026-11-14T12:00:00Z".into());
            connection.execute(
                "INSERT INTO scan_packages(repo_id,scan_run_id,name,package_type) VALUES (?1,?1,'first',?2),(?1,?1,'second',?2)",
                params![id, name],
            ).unwrap();
        }
        let mut query = DependencyQuery {
            page_size: Some(1),
            ..Default::default()
        };
        let first = db.dependency_page(1, &query).unwrap();
        let second = db.dependency_page(2, &query).unwrap();
        assert_eq!(first.revision, second.revision);
        assert_eq!(first.ecosystems, ["javascript"]);
        assert_eq!(second.ecosystems, ["python"]);
        query.cursor = first.next_cursor;
        assert!(db
            .dependency_page(2, &query)
            .unwrap_err()
            .to_string()
            .contains("DEPENDENCY_CURSOR_STALE"));
    }

    #[test]
    fn overview_uses_completed_evidence_after_many_failed_attempts() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("overview.db")).unwrap();
        seed_review_finding(&db, "persistent", "HIGH", Some("2.0"));
        let connection = db.connect().unwrap();
        connection.execute("UPDATE scan_runs SET started_at='2026-01-01T12:00:00Z',finished_at='2026-01-01T12:00:00Z' WHERE id=1", []).unwrap();
        insert_vulnerability(&connection, 1, 1, "resolved", "CRITICAL");
        insert_completed_run(&connection, 2, 1, "2026-01-02T12:00:00Z".into());
        insert_vulnerability(&connection, 1, 2, "persistent", "HIGH");
        for i in 0..8 {
            insert_vulnerability_with_package(
                &connection,
                1,
                2,
                &format!("new-{i}"),
                &format!("CVE-TEST-{i}"),
                &format!("dependency-{i}"),
                "MEDIUM",
                &format!("pkg:npm/dependency-{i}@1.0"),
            );
        }
        connection
            .execute(
                "UPDATE vulnerabilities SET fix_version='2.0' WHERE scan_run_id=2",
                [],
            )
            .unwrap();
        for id in 3..=15 {
            connection.execute(
                "INSERT INTO scan_runs(id,repo_id,status,started_at,finished_at,error_message)
                 VALUES(?1,1,'failed','2026-01-03T12:00:00Z','2026-01-03T12:00:00Z','fixture failure')",
                [id],
            ).unwrap();
        }
        let detail = db.repo_overview(1).unwrap();
        let overview = detail.overview;
        assert_eq!(detail.scan_history.len(), 12);
        assert!(detail
            .scan_history
            .iter()
            .all(|run| run.status == ScanRunStatus::Failed));
        assert_eq!(detail.latest_scan.unwrap().id, 15);
        let completed = overview.latest_completed_scan.unwrap();
        assert_eq!(completed.id, 2);
        assert_eq!(
            (
                completed.open_critical,
                completed.open_high,
                completed.open_medium
            ),
            (0, 1, 8)
        );
        assert_eq!(detail.previous_scan.unwrap().id, 1);
        assert_eq!(overview.open_finding_count, 9);
        assert_eq!(overview.opportunity_count, 9);
        assert_eq!(overview.opportunities.len(), 5);
        assert_eq!(overview.opportunities[0].fingerprints, ["persistent"]);
        assert_eq!(detail.findings.len(), 6);
        assert_eq!(detail.findings[0].fingerprint, "persistent");
        assert_eq!(detail.findings[0].state, RepoFindingState::Open);
        assert!(detail.findings[1..]
            .iter()
            .all(|item| item.state == RepoFindingState::New));
    }

    #[test]
    fn batched_history_rolls_back_event_claims_and_text_together() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("rollback.db")).unwrap();
        seed_companion_repository(&db, 1, "chat");
        let thread = db
            .create_companion_thread(1, "provider-1", "conversation", None)
            .unwrap();
        let make = |sequence, kind, payload| CompanionEventEnvelope {
            cache_epoch: None,
            provider_segment_id: None,
            schema_version: COMPANION_SCHEMA_VERSION,
            sequence,
            repository_id: Some(1),
            thread_id: Some(thread.id),
            provider_thread_id: Some("provider-1".into()),
            connection_id: Some("codex_default".into()),
            driver: Some("codex".into()),
            model_id: None,
            turn_id: Some("turn".into()),
            kind,
            payload,
        };
        let valid = make(
            1,
            CompanionEventKind::ItemDelta,
            serde_json::json!({"itemId":"answer","text":"durable"}),
        );
        let invalid = CompanionTimelineItem {
            id: "".into(),
            kind: CompanionTimelineItemKind::AssistantMessage,
            status: CompanionTimelineItemStatus::Completed,
            text: "invalid".into(),
            activity: None,
            citations: Vec::new(),
            error: None,
            model_id: None,
            created_at: now(),
        };
        let bad = make(
            2,
            CompanionEventKind::ItemCompleted,
            serde_json::to_value(invalid).unwrap(),
        );
        assert!(db.cache_companion_events(&[valid.clone(), bad]).is_err());
        assert!(db
            .cached_companion_thread_snapshot(thread.id)
            .unwrap()
            .items
            .is_empty());
        let conn = db.connect().unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM companion_cached_events", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            0
        );
        drop(conn);
        db.cache_companion_events(&[valid.clone(), valid]).unwrap();
        assert_eq!(
            db.cached_companion_thread_snapshot(thread.id)
                .unwrap()
                .items[0]
                .text,
            "durable"
        );
    }

    #[test]
    fn history_pages_preserve_segment_identity_repeated_questions_and_citations() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("paged-history.db")).unwrap();
        seed_companion_repository(&db, 1, "history");
        let thread = db
            .create_companion_thread(1, "provider-1", "History", None)
            .unwrap();
        let first = db.current_companion_provider_segment(thread.id).unwrap();
        let mut connection = db.connect().unwrap();
        let tx = connection.transaction().unwrap();
        for index in 0..240 {
            let item = CompanionTimelineItem {
                id: format!("same-{index}"),
                kind: CompanionTimelineItemKind::UserMessage,
                status: CompanionTimelineItemStatus::Completed,
                text: "Repeated question".into(),
                activity: None,
                citations: Vec::new(),
                error: None,
                model_id: None,
                created_at: now(),
            };
            db.upsert_companion_timeline_item_on(&tx, thread.id, first.id, &item)
                .unwrap();
        }
        tx.commit().unwrap();
        drop(connection);
        let second = db
            .create_companion_provider_segment(
                thread.id,
                DEFAULT_GROK_CONNECTION_ID,
                "grok",
                "grok-history",
                None,
            )
            .unwrap();
        let mut connection = db.connect().unwrap();
        let tx = connection.transaction().unwrap();
        for index in 0..240 {
            let item = CompanionTimelineItem {
                id: format!("same-{index}"),
                kind: CompanionTimelineItemKind::UserMessage,
                status: CompanionTimelineItemStatus::Completed,
                text: "Repeated question".into(),
                activity: None,
                citations: Vec::new(),
                error: None,
                model_id: None,
                created_at: now(),
            };
            db.upsert_companion_timeline_item_on(&tx, thread.id, second.id, &item)
                .unwrap();
        }
        tx.commit().unwrap();
        drop(connection);
        let mut cursor = None;
        let mut ids = std::collections::HashSet::new();
        let mut count = 0;
        loop {
            let page = db
                .cached_companion_thread_page(thread.id, cursor.as_deref())
                .unwrap();
            assert!(page.items.len() <= 200);
            assert!(serde_json::to_vec(&page).unwrap().len() < 512 * 1024);
            for item in &page.items {
                assert!(ids.insert(item.id.clone()));
                assert_eq!(item.text, "Repeated question");
                assert!(page.provider_ordinals.contains_key(&item.id));
            }
            count += page.items.len();
            cursor = page.older_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(count, 480);
        assert!(db
            .cached_companion_thread_page(thread.id, Some("h1:999:1:1"))
            .is_err());
        let event = CompanionEventEnvelope {
            cache_epoch: Some("first-process".into()),
            provider_segment_id: None,
            schema_version: 1,
            sequence: 100,
            repository_id: Some(1),
            thread_id: Some(thread.id),
            provider_thread_id: Some("grok-history".into()),
            connection_id: Some(DEFAULT_GROK_CONNECTION_ID.into()),
            driver: Some("grok".into()),
            model_id: None,
            turn_id: Some("turn".into()),
            kind: CompanionEventKind::ItemDelta,
            payload: serde_json::json!({"itemId":"live","text":"abc"}),
        };
        db.cache_companion_events(std::slice::from_ref(&event))
            .unwrap();
        let page = db.cached_companion_thread_page(thread.id, None).unwrap();
        let route = format!("{}:{}:first-process", second.id, DEFAULT_GROK_CONNECTION_ID);
        assert_eq!(page.event_watermarks.get(&route), Some(&100));
        assert!(page
            .streaming_item_ids
            .contains(&format!("segment:{}:live", second.id)));
        let next = CompanionEventEnvelope {
            cache_epoch: Some("second-process".into()),
            sequence: 1,
            payload: serde_json::json!({"itemId":"new-live","text":"new"}),
            ..event
        };
        db.cache_companion_events(&[next]).unwrap();
        let page = db.cached_companion_thread_page(thread.id, None).unwrap();
        assert_eq!(
            page.event_watermarks.get(&format!(
                "{}:{}:second-process",
                second.id, DEFAULT_GROK_CONNECTION_ID
            )),
            Some(&1)
        );
        assert!(!page.event_watermarks.contains_key(&route));
        for epoch in ["warning-process-a", "warning-process-b"] {
            db.cache_companion_events(&[CompanionEventEnvelope {
                cache_epoch: Some(epoch.into()), provider_segment_id: Some(second.id),
                schema_version: 1, sequence: 1, repository_id: Some(1), thread_id: Some(thread.id),
                provider_thread_id: Some("grok-history".into()), connection_id: Some(DEFAULT_GROK_CONNECTION_ID.into()),
                driver: Some("grok".into()), model_id: None, turn_id: None,
                kind: CompanionEventKind::Warning,
                payload: serde_json::json!({"code":"action_denied","message":epoch,"retryable":false}),
            }]).unwrap();
        }
        let warnings = db
            .cached_companion_thread_page(thread.id, None)
            .unwrap()
            .items
            .into_iter()
            .filter(|item| item.kind == CompanionTimelineItemKind::Warning)
            .collect::<Vec<_>>();
        assert_eq!(warnings.len(), 2);
        assert_ne!(warnings[0].id, warnings[1].id);
    }

    #[test]
    fn review_query_prioritizes_and_pages_deterministically() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init");
        seed_review_finding(&database, "review-fp", "CRITICAL", Some("2.0.0"));
        let query = ReviewQuery {
            page_size: Some(1),
            ..Default::default()
        };
        let page = database.list_review_findings(&query).expect("review page");
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].finding.fingerprint, "review-fp");
        assert_eq!(page.group_counts.review_state[0].id, "unreviewed");
        assert_eq!(page.group_counts.review_state[0].count, 1);
        assert_eq!(page.group_counts.severity[0].id, "CRITICAL");
        assert_eq!(page.group_counts.severity[0].count, 1);
        assert_eq!(page.group_counts.fix_availability[0].count, 1);
        assert_eq!(page.group_counts.repository[0].id, "repository:1");
        assert!(page.items[0]
            .priority_reasons
            .iter()
            .any(|reason| reason == "CRITICAL severity"));
        assert_eq!(
            database
                .review_summary()
                .expect("summary")
                .unreviewed_critical,
            1
        );
    }

    #[test]
    fn due_snooze_reopens_once_and_summary_keeps_due_attention() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init");
        seed_review_finding(&database, "due-fp", "HIGH", None);
        let future = timestamps::format(&(OffsetDateTime::now_utc() + Duration::hours(1)));
        database
            .snooze_finding(1, "due-fp", &future, None)
            .expect("snooze");
        let connection = database.connect().expect("connect");
        connection.execute("UPDATE finding_reviews SET snooze_until = ?1 WHERE repo_id = 1 AND fingerprint = 'due-fp'", [timestamps::format(&(OffsetDateTime::now_utc() - Duration::minutes(1)))]).expect("make due");
        drop(connection);
        let summary = database.review_summary().expect("summary");
        assert_eq!(summary.due_snoozed, 1);
        assert_eq!(summary.needs_review, 1);
        assert_eq!(database.reconcile_finding_reviews().expect("idempotent"), 0);
    }

    #[test]
    fn review_query_reopens_due_snooze_without_full_reconciliation() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init");
        seed_review_finding(&database, "due-query-fp", "HIGH", None);
        let future = timestamps::format(&(OffsetDateTime::now_utc() + Duration::hours(1)));
        database
            .snooze_finding(1, "due-query-fp", &future, None)
            .expect("snooze");
        let query = ReviewQuery::default();
        let snoozed = database.list_review_findings(&query).unwrap();
        assert_eq!(snoozed.group_counts.review_state[2].count, 1);
        let connection = database.connect().expect("connect");
        connection.execute("UPDATE finding_reviews SET snooze_until = ?1 WHERE repo_id = 1 AND fingerprint = 'due-query-fp'", [timestamps::format(&(OffsetDateTime::now_utc() - Duration::minutes(1)))]).expect("make due");
        drop(connection);

        let page = database.list_review_findings(&query).expect("review page");
        assert_eq!(page.items[0].review_state, ReviewState::Unreviewed);
        assert_eq!(page.group_counts.review_state[2].count, 0);
        assert_eq!(page.group_counts.review_state[0].count, 1);
        assert_eq!(database.reconcile_finding_reviews().expect("idempotent"), 0);
    }

    #[test]
    fn review_summary_counts_resolved_by_evidence_date_and_latest_failed_scan() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("summary.db")).unwrap();
        seed_companion_repository(&db, 1, "Imported runs");
        seed_companion_repository(&db, 2, "Old runs");
        let conn = db.connect().unwrap();
        let date = |days| timestamps::format(&(OffsetDateTime::now_utc() - Duration::days(days)));
        for (run, repo, days) in [(4, 1, 40), (2, 1, 10), (1, 1, 2), (5, 2, 41), (6, 2, 40)] {
            insert_completed_run(&conn, run, repo, date(days));
        }
        for (repo, run, fingerprint) in [
            (1, 4, "older-evidence"),
            (1, 2, "older-evidence"),
            (1, 2, "recent-evidence"),
            (2, 5, "old-resolution"),
        ] {
            insert_vulnerability_with_package(
                &conn,
                repo,
                run,
                fingerprint,
                fingerprint,
                "fixture",
                "HIGH",
                "pkg:npm/fixture@1.0.0",
            );
        }
        let versions = ScannerVersions {
            syft: "fixture".into(),
            grype: "fixture".into(),
        };
        let failed = db.insert_scan_start(1, &versions).unwrap();
        db.fail_scan(1, failed, "fixture failed", None).unwrap();
        assert_eq!(
            db.review_summary().unwrap(),
            ReviewSummary {
                recently_resolved: 1,
                failed_scans: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn interrupted_scan_activity_survives_restart_and_clears_after_retry() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("interrupted-scan.db");
        let db = Database::new(path.clone()).unwrap();
        seed_review_finding(&db, "saved-finding", "HIGH", None);
        let connection = db.connect().unwrap();
        connection
            .execute(
                "INSERT INTO scan_packages(repo_id,scan_run_id,name,version,package_type)
             VALUES (1,1,'review-package','1.0.0','npm')",
                [],
            )
            .unwrap();
        let before = db.repo_overview(1).unwrap();
        let versions = ScannerVersions {
            syft: "fixture".into(),
            grype: "fixture".into(),
        };
        let attempt = db.insert_scan_start(1, &versions).unwrap();
        seed_companion_repository(&db, 2, "inactive");
        db.insert_scan_start(2, &versions).unwrap();
        connection
            .execute("UPDATE repos SET is_active=0 WHERE id=2", [])
            .unwrap();
        drop(connection);
        drop(db);

        let db = Database::new(path.clone()).unwrap();
        let activity = db.interrupted_scan_activity().unwrap().unwrap();
        assert!(activity.interrupted);
        assert_eq!(activity.status, ScanActivityStatus::Failed);
        assert_eq!(activity.scope_repo_id, Some(1));
        assert_eq!(activity.failed_repositories, 1);
        assert_eq!(activity.outcomes.len(), 1);
        assert_eq!(activity.outcomes[0].scan_run_id, Some(attempt));
        assert_eq!(
            activity.outcomes[0].error.as_deref(),
            Some(INTERRUPTED_SCAN_ERROR)
        );
        let attempt_details = db.repo_scan_comparison(1, attempt).unwrap().unwrap().run;
        assert_eq!(attempt_details.status, ScanRunStatus::Failed);
        assert_eq!(attempt_details.finished_at, activity.finished_at);
        assert_eq!(db.review_summary().unwrap().failed_scans, 1);
        let overview = db.repo_overview(1).unwrap();
        assert_eq!(overview.overview, before.overview);
        assert_eq!(overview.findings, before.findings);
        assert_eq!(
            overview.summary.last_scanned_at,
            before.summary.last_scanned_at
        );
        let reopened = Database::new(path).unwrap();
        assert_eq!(
            reopened.interrupted_scan_activity().unwrap(),
            Some(activity)
        );

        let retry = reopened.insert_scan_start(1, &versions).unwrap();
        assert!(reopened.insert_scan_start(2, &versions).is_err());
        reopened
            .complete_scan(1, retry, &[], &[], &DeltaSummary::default())
            .unwrap();
        assert!(reopened.interrupted_scan_activity().unwrap().is_none());
        assert_eq!(reopened.review_summary().unwrap().failed_scans, 0);
        assert_eq!(
            reopened
                .repo_scan_comparison(1, attempt)
                .unwrap()
                .unwrap()
                .run,
            attempt_details
        );
    }

    #[test]
    fn review_query_rejects_bad_cursor_and_applies_and_or_filters() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init");
        seed_review_finding(&database, "filter-fp", "CRITICAL", Some("2.0.0"));
        let invalid = ReviewQuery {
            cursor: Some("not-a-cursor".into()),
            ..Default::default()
        };
        assert!(database.list_review_findings(&invalid).is_err());
        let mut query = ReviewQuery::default();
        query.filters.severities = vec![Severity::Critical, Severity::High];
        query.filters.repo_ids = vec![1];
        query.filters.fix_available = Some(true);
        assert_eq!(
            database
                .list_review_findings(&query)
                .expect("filtered")
                .total,
            1
        );
        query.filters.repo_ids = vec![99];
        assert_eq!(
            database
                .list_review_findings(&query)
                .expect("removed repo")
                .total,
            0
        );
    }

    #[test]
    fn material_risk_basis_reopens_only_for_defined_changes() {
        let original = ReviewRiskBasis {
            severity: Severity::High,
            fix_version: None,
            known_exploited: false,
            ransomware_known: false,
            scanner_state: RepoFindingState::Open,
        };
        let cosmetic = original.clone();
        assert!(!material_risk_increased(&original, &cosmetic));
        assert!(material_risk_increased(
            &original,
            &ReviewRiskBasis {
                severity: Severity::Critical,
                ..cosmetic.clone()
            }
        ));
        assert!(material_risk_increased(
            &original,
            &ReviewRiskBasis {
                fix_version: Some("2.0.0".into()),
                ..cosmetic.clone()
            }
        ));
        assert!(material_risk_increased(
            &original,
            &ReviewRiskBasis {
                known_exploited: true,
                ..cosmetic
            }
        ));
    }

    #[test]
    fn local_search_finds_reviewable_evidence_and_current_packages_without_repository_content() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init");
        seed_review_finding(&database, "resolved-fp", "HIGH", None);
        let connection = database.connect().unwrap();
        insert_completed_run(
            &connection,
            2,
            1,
            timestamps::format(&(OffsetDateTime::now_utc() + Duration::minutes(1))),
        );
        insert_vulnerability_with_package(
            &connection,
            1,
            2,
            "open-fp",
            "CVE-2026-2000",
            "current",
            "HIGH",
            "pkg:npm/current@1.0.0",
        );
        connection
            .execute_batch(
                "INSERT INTO scan_packages(id,repo_id,scan_run_id,name,version,package_type) VALUES
                (1,1,1,'fixture-package','1.0','npm'),
                (2,1,2,'fixture-package','2.0','npm'),
                (10,1,2,'fixture-package','2.0','cargo'),
                (11,1,2,'fixture-package','2.1','npm'),
                (12,1,1,'retired-package','1.0','npm');",
            )
            .unwrap();
        seed_companion_repository(&database, 2, "other-repository");
        insert_completed_run(&connection, 3, 2, now());
        connection.execute("INSERT INTO scan_packages(id,repo_id,scan_run_id,name,version,package_type) VALUES (3,2,3,'fixture-package','4.0','npm')", []).unwrap();
        insert_external_intel_item(
            &connection,
            1,
            "GHSA-EXACT",
            "github_advisory",
            "GHSA-EXACT",
            "Synthetic advisory",
            "Fixture",
            "https://example.invalid",
            "HIGH",
            &[],
            &["search-by-alias"],
        );
        let results = database
            .search_local_entities("CVE-2026", 2)
            .expect("search");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].fingerprint.as_deref(), Some("resolved-fp"));
        assert!(results[0].context.ends_with(" · Resolved"));
        assert_eq!(results[1].fingerprint.as_deref(), Some("open-fp"));
        let mut exact = ReviewQuery::default();
        exact.filters.repo_ids = vec![1];
        exact.filters.fingerprint = results[0].fingerprint.clone();
        let page = database.list_review_findings(&exact).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].finding.state, RepoFindingState::Resolved);
        assert_eq!(page.items[0].finding.fingerprint, "resolved-fp");
        assert!(results
            .iter()
            .all(|result| !result.context.contains("/tmp/review-repo")));
        let packages = database
            .search_local_entities("fixture-package", 2)
            .unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].id, "package:2");
        assert_eq!(packages[1].id, "package:3");
        assert_eq!(
            database
                .search_local_entities("fixture-package", 1)
                .unwrap(),
            packages[..1]
        );
        assert!(database
            .search_local_entities("retired-package", 50)
            .unwrap()
            .is_empty());
        assert!(packages
            .iter()
            .all(|hit| hit.kind == LocalEntityKind::Package));
        assert_eq!(
            packages
                .iter()
                .map(|hit| (hit.repo_id, hit.context.as_str()))
                .collect::<Vec<_>>(),
            [(Some(1), "review-repo"), (Some(2), "other-repository")]
        );
        let alias = database
            .search_local_entities("search-by-alias", 2)
            .unwrap();
        assert_eq!(alias.len(), 1);
        assert_eq!(alias[0].advisory_id, Some(1));
    }

    #[test]
    fn onboarding_snapshot_defaults_and_persists_decision() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");

        let connection = database.connect().unwrap();
        let initial =
            Database::onboarding_snapshot_on(&connection).expect("initial onboarding snapshot");
        assert_eq!(initial.status, OnboardingStatus::NotStarted);
        assert_eq!(initial.decided_at, None);
        assert_eq!(initial.version, None);

        database
            .set_onboarding_status(OnboardingStatus::Skipped, "0.1.0".to_string())
            .expect("set onboarding");
        let skipped =
            Database::onboarding_snapshot_on(&connection).expect("persisted onboarding snapshot");
        assert_eq!(skipped.status, OnboardingStatus::Skipped);
        assert!(skipped.decided_at.is_some());
        assert_eq!(skipped.version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn init_migrates_scan_run_medium_columns() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("cvec.sqlite");
        {
            let connection = Connection::open(&path).expect("open db");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE scan_runs (
                      id INTEGER PRIMARY KEY,
                      repo_id INTEGER NOT NULL,
                      status TEXT NOT NULL,
                      started_at TEXT NOT NULL,
                      finished_at TEXT,
                      scanner_versions TEXT,
                      new_high_count INTEGER NOT NULL DEFAULT 0,
                      new_critical_count INTEGER NOT NULL DEFAULT 0,
                      resolved_high_count INTEGER NOT NULL DEFAULT 0,
                      resolved_critical_count INTEGER NOT NULL DEFAULT 0,
                      error_message TEXT
                    );
                    "#,
                )
                .expect("create old scan_runs");
        }

        let database = Database::new(path).expect("init db");
        let connection = database.connect().expect("connect");
        let columns = table_columns(&connection, "scan_runs");

        assert!(columns.iter().any(|column| column == "new_medium_count"));
        assert!(columns
            .iter()
            .any(|column| column == "resolved_medium_count"));
        for expected in [
            "source_revision",
            "package_count",
            "reopened_count",
            "warnings_json",
            "evidence_complete",
        ] {
            assert!(columns.iter().any(|column| column == expected));
        }
    }

    #[test]
    fn init_migrates_package_inventory_schema() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");

        let vulnerability_columns = table_columns(&connection, "vulnerabilities");
        let package_columns = table_columns(&connection, "scan_packages");

        assert!(vulnerability_columns
            .iter()
            .any(|column| column == "package_url"));
        assert!(package_columns.iter().any(|column| column == "package_url"));
        assert!(table_columns(&connection, "scan_package_snapshots")
            .iter()
            .any(|column| column == "scan_run_id"));
    }

    #[test]
    fn init_adds_remediation_schema_idempotently() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("cvec.sqlite");
        let database = Database::new(path.clone()).expect("init db");
        let connection = database.connect().expect("connect");
        assert!(table_columns(&connection, "scan_runs").contains(&"source_dirty".to_string()));
        assert!(
            table_columns(&connection, "vulnerability_remediation_evidence")
                .contains(&"relative_path".to_string())
        );
        drop(connection);
        Database::new(path).expect("repeated startup");
    }

    #[test]
    fn remediation_upgrade_preserves_populated_old_database_and_rollback_reads() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("cvec.sqlite");
        let database = Database::new(path.clone()).expect("init db");
        database
            .set_setting("review_views", "saved-view-payload".into())
            .expect("setting");
        let connection = database.connect().expect("connect");
        let timestamp = timestamps::now();
        connection.execute("INSERT INTO repos (id, name, path, is_active, created_at, updated_at) VALUES (1, 'preserved', '/tmp/preserved', 1, ?1, ?1)", [&timestamp]).expect("repo");
        insert_completed_run(&connection, 1, 1, timestamp);
        insert_vulnerability(&connection, 1, 1, "preserved-fp", "HIGH");
        connection
            .execute_batch(
                "DROP TABLE vulnerability_remediation_evidence; PRAGMA user_version = 3;",
            )
            .expect("simulate v3");
        drop(connection);

        let upgraded = Database::new(path.clone()).expect("upgrade");
        assert_eq!(
            upgraded
                .get_setting("review_views")
                .expect("setting")
                .as_deref(),
            Some("saved-view-payload")
        );
        let connection = upgraded.connect().expect("connect");
        let repo_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM repos WHERE name = 'preserved'",
                [],
                |row| row.get(0),
            )
            .expect("repo count");
        let finding_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM vulnerabilities WHERE fingerprint = 'preserved-fp'",
                [],
                |row| row.get(0),
            )
            .expect("finding count");
        assert_eq!((repo_count, finding_count), (1, 1));
        assert!(!table_columns(&connection, "vulnerability_remediation_evidence").is_empty());
        drop(connection);
        Database::new(path).expect("rollback-compatible repeated read");
    }

    #[test]
    fn remediation_projection_persists_evidence_and_derives_later_absence() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir(&repo_root).expect("repo root");
        std::fs::write(repo_root.join("package.json"), "{}").expect("manifest");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");
        let timestamp = timestamps::now();
        connection.execute(
            "INSERT INTO repos (id, name, path, is_active, source_type, created_at, updated_at) VALUES (1, 'demo', ?1, 1, 'local', ?2, ?2)",
            params![repo_root.to_string_lossy(), timestamp],
        ).expect("repo");
        connection.execute("INSERT INTO scan_runs (id, repo_id, status, started_at) VALUES (1, 1, 'running', ?1)", [&timestamp]).expect("run");
        drop(connection);
        let finding = NormalizedFinding {
            fingerprint: "fp-demo".into(),
            advisory_id: "CVE-2026-1".into(),
            package_name: "router".into(),
            package_version: Some("1.0.0".into()),
            package_type: Some("npm".into()),
            package_url: Some("pkg:npm/router@1.0.0".into()),
            severity: Severity::Critical,
            fix_version: Some("1.0.1".into()),
            title: None,
            source_url: None,
            advisory_details: None,
        };
        let evidence = NormalizedRemediationEvidence {
            schema_version: REMEDIATION_SCHEMA_VERSION,
            fingerprint: finding.fingerprint.clone(),
            package_name: finding.package_name.clone(),
            package_version: finding.package_version.clone(),
            package_type: finding.package_type.clone(),
            package_url: finding.package_url.clone(),
            relative_path: Some("package.json".into()),
            location_kind: Some(RemediationLocationKind::Manifest),
            relationship: RemediationRelationship::Unknown,
            parent_package: None,
            source: RemediationEvidenceSource::SyftLocation,
            quality: RemediationEvidenceQuality::Partial,
            location_count: 1,
        };
        database
            .complete_scan_with_remediation(ScanCompletion {
                repo_id: 1,
                run_id: 1,
                findings: std::slice::from_ref(&finding),
                packages: &[],
                remediation_evidence: &[evidence],
                source_snapshot: &SourceSnapshot {
                    revision: Some("abc123".into()),
                    dirty: Some(true),
                },
                delta: &DeltaSummary::default(),
            })
            .expect("complete first scan");
        let projection = database
            .finding_remediation(1, "fp-demo", 1)
            .expect("projection");
        assert_eq!(
            projection.verification_state,
            RemediationVerificationState::StillDetected
        );
        assert!(projection.can_open_manifest);
        assert_eq!(projection.relative_path.as_deref(), Some("package.json"));
        assert!(projection
            .brief_markdown
            .contains("Scanner-reported target: 1.0.1"));
        assert!(!projection
            .brief_markdown
            .contains(repo_root.to_string_lossy().as_ref()));
        assert!(!projection.brief_markdown.contains("```"));
        let review_count: i64 = database
            .connect()
            .expect("connect")
            .query_row("SELECT COUNT(*) FROM finding_reviews", [], |row| row.get(0))
            .expect("review count");
        assert_eq!(review_count, 0);

        std::fs::remove_file(repo_root.join("package.json")).expect("move manifest");
        assert!(
            !database
                .finding_remediation(1, "fp-demo", 1)
                .expect("moved projection")
                .can_open_manifest
        );
        std::fs::write(repo_root.join("package.json"), "{}").expect("restore manifest");
        let connection = database.connect().expect("connect");
        connection
            .execute("UPDATE repos SET source_type = 'github' WHERE id = 1", [])
            .expect("remote repo");
        drop(connection);
        assert!(
            !database
                .finding_remediation(1, "fp-demo", 1)
                .expect("remote projection")
                .can_open_manifest
        );
        database
            .connect()
            .expect("connect")
            .execute("UPDATE repos SET source_type = 'local' WHERE id = 1", [])
            .expect("local repo");

        let connection = database.connect().expect("connect");
        connection.execute("INSERT INTO scan_runs (id, repo_id, status, started_at) VALUES (2, 1, 'running', ?1)", [&timestamp]).expect("second run");
        drop(connection);
        database
            .complete_scan_with_remediation(ScanCompletion {
                repo_id: 1,
                run_id: 2,
                findings: &[],
                packages: &[],
                remediation_evidence: &[],
                source_snapshot: &SourceSnapshot {
                    revision: Some("def456".into()),
                    dirty: Some(false),
                },
                delta: &DeltaSummary::default(),
            })
            .expect("complete second scan");
        let verified = database
            .finding_remediation(1, "fp-demo", 1)
            .expect("verified projection");
        assert_eq!(
            verified.verification_state,
            RemediationVerificationState::NotDetected
        );
        assert_eq!(verified.verification_scan_run_id, Some(2));
        assert_eq!(verified.evidence_quality, RemediationEvidenceQuality::Stale);

        let connection = database.connect().expect("connect");
        connection.execute("INSERT INTO scan_runs (id, repo_id, status, started_at, finished_at, error_message) VALUES (3, 1, 'failed', ?1, ?1, 'scanner unavailable')", [&timestamp]).expect("failed run");
        drop(connection);
        let unverifiable = database
            .finding_remediation(1, "fp-demo", 1)
            .expect("unverifiable projection");
        assert_eq!(
            unverifiable.verification_state,
            RemediationVerificationState::Unverifiable
        );

        let connection = database.connect().expect("connect");
        connection.execute("INSERT INTO scan_runs (id, repo_id, status, started_at) VALUES (4, 1, 'running', ?1)", [&timestamp]).expect("reappeared run");
        drop(connection);
        database
            .complete_scan_with_remediation(ScanCompletion {
                repo_id: 1,
                run_id: 4,
                findings: std::slice::from_ref(&finding),
                packages: &[],
                remediation_evidence: &[],
                source_snapshot: &SourceSnapshot {
                    revision: Some("ghi789".into()),
                    dirty: Some(false),
                },
                delta: &DeltaSummary::default(),
            })
            .expect("complete reappeared scan");
        let reappeared = database
            .finding_remediation(1, "fp-demo", 1)
            .expect("reappeared projection");
        assert_eq!(
            reappeared.verification_state,
            RemediationVerificationState::Reappeared
        );
    }

    #[test]
    fn scan_resolutions_are_scoped_to_the_resolving_run() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("cvec.db")).unwrap();
        let connection = db.connect().unwrap();
        for repo_id in 1..=2 {
            seed_companion_repository(&db, repo_id, &format!("Repository {repo_id}"));
            let previous = repo_id * 10 + 1;
            let resolved = previous + 1;
            insert_completed_run(
                &connection,
                previous,
                repo_id,
                "2026-11-12T10:00:00Z".into(),
            );
            insert_vulnerability(&connection, repo_id, previous, "prior", "HIGH");
            insert_completed_run(
                &connection,
                resolved,
                repo_id,
                "2026-11-12T11:00:00Z".into(),
            );
            connection
                .execute(
                    "UPDATE scan_runs SET evidence_complete=1 WHERE id=?1",
                    [resolved],
                )
                .unwrap();
        }
        connection.execute(
            "INSERT INTO scan_runs(id,repo_id,status,started_at) VALUES(13,1,'failed','2026-11-12T12:00:00Z')",
            [],
        ).unwrap();

        let mut query = ReviewQuery::default();
        query.filters.scanner_states = vec![RepoFindingState::Resolved];
        query.filters.scan_resolved_run_ids = vec![12];
        let page = db.list_review_findings(&query).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].repo_id, 1);
        assert_eq!(page.items[0].scan_run_id, Some(11));
        assert_eq!(page.items[0].finding.state, RepoFindingState::Resolved);
        assert_eq!(
            db.review_finding_detail(1, "prior", 11)
                .unwrap()
                .scan_run_id,
            Some(11)
        );

        query.filters.scan_resolved_run_ids = vec![12, 22];
        query.page_size = Some(1);
        let page = db.list_review_findings(&query).unwrap();
        assert_eq!(page.total, 2);
        let first_repo = page.items[0].repo_id;
        query.cursor = page.next_cursor;
        assert!(query.cursor.is_some());
        let next = db.list_review_findings(&query).unwrap();
        assert_ne!(next.items[0].repo_id, first_repo);
        assert_eq!(next.total, 2);

        insert_completed_run(&connection, 14, 1, "2026-11-12T13:00:00Z".into());
        insert_vulnerability(&connection, 1, 14, "prior", "HIGH");
        query.cursor = None;
        query.filters.scan_resolved_run_ids = vec![12, 13, 22];
        let current = db.list_review_findings(&query).unwrap();
        assert_eq!(current.total, 1);
        assert_eq!(current.items[0].repo_id, 2);
    }

    #[test]
    fn historical_resolutions_keep_the_recorded_scan_evidence_after_reappearance() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("cvec.db")).unwrap();
        seed_companion_repository(&db, 1, "History");
        seed_companion_repository(&db, 2, "Unrelated");
        let connection = db.connect().unwrap();
        insert_completed_run(&connection, 31, 1, "2026-11-12T10:00:00Z".into());
        connection.execute(
            "INSERT INTO scan_runs(id,repo_id,status,started_at) VALUES(32,1,'failed','2026-11-12T11:00:00Z')",
            [],
        ).unwrap();
        insert_completed_run(&connection, 33, 1, "2026-11-12T12:00:00Z".into());
        insert_completed_run(&connection, 34, 1, "2026-11-12T13:00:00Z".into());
        connection
            .execute("UPDATE scan_runs SET evidence_complete=1 WHERE id=33", [])
            .unwrap();
        insert_vulnerability(&connection, 1, 31, "historical-finding", "HIGH");
        insert_vulnerability(&connection, 1, 31, "resolved-only", "MEDIUM");
        insert_vulnerability(&connection, 1, 34, "historical-finding", "CRITICAL");
        insert_vulnerability_with_package(
            &connection,
            1,
            31,
            "before-upgrade",
            "CVE-CONTINUING",
            "retained",
            "HIGH",
            "pkg:npm/retained@1.0.0",
        );
        insert_vulnerability_with_package(
            &connection,
            1,
            33,
            "after-upgrade",
            "CVE-CONTINUING",
            "retained",
            "HIGH",
            "pkg:npm/retained@1.1.0",
        );

        let mut query = ReviewQuery {
            page_size: Some(1),
            ..ReviewQuery::default()
        };
        query.filters.scanner_states = vec![RepoFindingState::Resolved];
        query.filters.repo_ids = vec![1];
        query.filters.resolved_in_scan_run_id = Some(33);
        let page = db.list_review_findings(&query).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items[0].finding.fingerprint, "historical-finding");
        assert_eq!(page.items[0].finding.severity, Severity::High);
        assert_eq!(page.items[0].scan_run_id, Some(31));
        assert_eq!(page.items[0].finding.state, RepoFindingState::Resolved);
        query.cursor = page.next_cursor;
        assert!(query.cursor.is_some());
        let next_page = db.list_review_findings(&query).unwrap();
        assert_eq!(next_page.items[0].finding.fingerprint, "resolved-only");
        assert!(next_page.next_cursor.is_none());

        let detail = db
            .review_finding_detail_with_scope(1, "historical-finding", 31, Some(33))
            .unwrap();
        assert_eq!(detail.finding.severity, Severity::High);
        assert_eq!(detail.finding.state, RepoFindingState::Resolved);
        assert!(db
            .review_finding_detail_with_scope(1, "historical-finding", 34, Some(33))
            .is_err());
        assert!(db
            .review_finding_detail_with_scope(2, "historical-finding", 31, Some(33))
            .is_err());
        assert_eq!(
            db.review_finding_detail(1, "historical-finding", 34)
                .unwrap()
                .finding
                .severity,
            Severity::Critical
        );

        query.cursor = None;
        query.filters.repo_ids = vec![2];
        assert_eq!(db.list_review_findings(&query).unwrap().total, 0);
        query.filters.repo_ids = vec![1];
        for run_id in [31, 32, 34] {
            query.filters.resolved_in_scan_run_id = Some(run_id);
            assert_eq!(db.list_review_findings(&query).unwrap().total, 0);
        }
    }

    #[test]
    fn notifications_keep_old_rows_and_fall_back_when_their_scan_is_deleted() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("cvec.db");
        let initialized = Database::new(path.clone()).unwrap();
        let legacy = initialized.connect().unwrap();
        legacy.execute_batch(
            "DROP TABLE notifications;
             CREATE TABLE notifications (
                id INTEGER PRIMARY KEY, kind TEXT NOT NULL, tone TEXT NOT NULL, title TEXT NOT NULL,
                message TEXT NOT NULL, repo_id INTEGER REFERENCES repos(id) ON DELETE SET NULL,
                repo_name TEXT, created_at TEXT NOT NULL
            );
            INSERT INTO notifications(kind,tone,title,message,created_at)
                VALUES ('db_update','info','Existing event','Saved before migration','2026-08-12');",
        ).unwrap();
        drop(legacy);
        let db = Database::new(path).unwrap();
        seed_companion_repository(&db, 1, "History");
        let connection = db.connect().unwrap();
        insert_completed_run(&connection, 5, 1, "2026-11-16T09:00:00Z".into());
        db.insert_notification(
            AppNotificationKind::ScanAlert,
            AppNotificationTone::Warning,
            "Critical finding",
            "From a saved scan",
            Some((1, "History")),
            Some(5),
        )
        .unwrap();
        let saved = Database::list_notifications_on(&connection, 50).unwrap();
        assert_eq!(saved[0].scan_run_id, Some(5));
        assert_eq!(saved[1].title, "Existing event");
        assert_eq!(saved[1].scan_run_id, None);

        connection
            .execute("DELETE FROM scan_runs WHERE id=5", [])
            .unwrap();
        let retained = Database::list_notifications_on(&connection, 50).unwrap();
        assert_eq!(retained[0].scan_run_id, None);
        assert_eq!(retained[0].repo_id, Some(1));
        assert_eq!(retained[0].repo_name.as_deref(), Some("History"));
    }

    #[test]
    fn scan_comparison_reads_the_requested_run_and_its_previous_completion() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("cvec.db")).unwrap();
        seed_companion_repository(&db, 1, "History");
        seed_companion_repository(&db, 2, "Unrelated");
        let connection = db.connect().unwrap();
        insert_completed_run(&connection, 1, 1, "2026-11-10T10:00:00Z".into());
        insert_completed_run(&connection, 2, 2, "2026-11-10T11:00:00Z".into());
        insert_completed_run(&connection, 3, 1, "2026-11-11T10:00:00Z".into());
        connection.execute(
            "UPDATE scan_runs SET package_count=CASE id WHEN 1 THEN 5 WHEN 2 THEN 99 ELSE 8 END",
            [],
        ).unwrap();
        for id in 4..=16 {
            connection
                .execute(
                    "INSERT INTO scan_runs(id,repo_id,status,started_at,error_message)
                 VALUES(?1,1,'failed','2026-11-12T10:00:00Z','fixture error')",
                    [id],
                )
                .unwrap();
        }
        assert!(db
            .repo_overview(1)
            .unwrap()
            .scan_history
            .iter()
            .all(|run| run.id > 3));
        let comparison = db.repo_scan_comparison(1, 3).unwrap().unwrap();
        assert_eq!(comparison.run.id, 3);
        assert_eq!(comparison.run.package_count, Some(8));
        assert_eq!(comparison.previous_run_id, Some(1));
        assert_eq!(comparison.previous_package_count, Some(5));
        let failure = db.repo_scan_comparison(1, 4).unwrap().unwrap();
        assert_eq!(failure.run.error_message.as_deref(), Some("fixture error"));
        assert_eq!(failure.previous_run_id, None);
        assert_eq!(failure.previous_package_count, None);
        assert!(db.repo_scan_comparison(2, 3).unwrap().is_none());
        assert!(db.repo_scan_comparison(1, 99).unwrap().is_none());
        connection
            .execute("UPDATE repos SET is_active=0 WHERE id=1", [])
            .unwrap();
        assert!(db.repo_scan_comparison(1, 3).unwrap().is_none());
    }

    #[test]
    fn advisory_continuing_across_versions_is_not_resolved() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("cvec.db")).unwrap();
        seed_companion_repository(&db, 1, "Fixture");
        let versions = ScannerVersions {
            syft: "fixture".into(),
            grype: "fixture".into(),
        };
        let scan = |findings| {
            let run = db.insert_scan_start(1, &versions).unwrap();
            let changes = db
                .complete_repository_scan(
                    1,
                    run,
                    &ScanOutput {
                        evidence: None,
                        findings,
                        packages: vec![],
                        remediation_evidence: vec![],
                    },
                    &SourceSnapshot::default(),
                    &[],
                    None,
                )
                .unwrap();
            (run, changes)
        };
        let finding = |advisory: &str, scope: &str, version: &str| {
            let purl =
                format!("pkg:npm/%40{scope}/router@{version}?repository_url=example.invalid");
            NormalizedFinding {
                fingerprint: super::super::delta::build_fingerprint(
                    advisory,
                    Some("npm"),
                    Some(&purl),
                    "router",
                    Some(version),
                ),
                advisory_id: advisory.into(),
                package_name: "router".into(),
                package_version: Some(version.into()),
                package_type: Some("npm".into()),
                package_url: Some(purl),
                severity: Severity::High,
                fix_version: None,
                title: None,
                source_url: None,
                advisory_details: None,
            }
        };
        let continuing = finding("CVE-FIXTURE-CONTINUING", "alpha", "1.0");
        let fixed = finding("CVE-FIXTURE-FIXED", "alpha", "1.0");
        let different_namespace = finding("CVE-FIXTURE-CONTINUING", "beta", "1.0");
        let (first, _) = scan(vec![
            continuing.clone(),
            fixed.clone(),
            different_namespace.clone(),
        ]);
        db.mark_finding_reviewed(1, &continuing.fingerprint)
            .unwrap();

        let updated = finding("cve-fixture-continuing", "alpha", "1.1");
        let (second, changes) = scan(vec![updated.clone()]);
        assert_eq!(changes.resolved_findings, 2);
        assert_eq!(changes.new_findings, 1);
        let detail = db.repo_detail(1).unwrap();
        assert_eq!(detail.summary.resolved_high, 2);
        assert_eq!(detail.latest_scan.unwrap().resolved_high, 2);
        assert_eq!(detail.findings.len(), 3);
        assert!(detail
            .findings
            .iter()
            .all(|row| row.fingerprint != continuing.fingerprint));
        assert_eq!(
            db.current_review_finding(1, &updated.fingerprint)
                .unwrap()
                .review_state,
            ReviewState::Unreviewed
        );
        assert!(db
            .finding_review(&db.connect().unwrap(), 1, &continuing.fingerprint)
            .unwrap()
            .is_some());
        let mut resolved = ReviewQuery::default();
        resolved.filters.scanner_states = vec![RepoFindingState::Resolved];
        let page = db.list_review_findings(&resolved).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(db.review_summary().unwrap().recently_resolved, 2);
        assert!(page
            .items
            .iter()
            .any(|row| row.finding.fingerprint == different_namespace.fingerprint));

        let projection = |item: &NormalizedFinding, run| {
            db.finding_remediation(1, &item.fingerprint, run).unwrap()
        };
        assert_eq!(
            projection(&continuing, first).verification_state,
            RemediationVerificationState::StillDetected
        );
        assert_eq!(
            projection(&continuing, first).verification_scan_run_id,
            Some(second)
        );
        assert_eq!(
            projection(&continuing, first).evidence_quality,
            RemediationEvidenceQuality::Stale
        );
        assert_eq!(
            projection(&fixed, first).verification_state,
            RemediationVerificationState::NotDetected
        );
        assert_eq!(
            projection(&different_namespace, first).verification_state,
            RemediationVerificationState::NotDetected
        );
        let search = db
            .search_local_entities("CVE-FIXTURE-CONTINUING", 10)
            .unwrap();
        assert!(search
            .iter()
            .all(|hit| hit.fingerprint.as_deref() != Some(&continuing.fingerprint)));
        assert!(search
            .iter()
            .any(|hit| hit.fingerprint.as_deref() == Some(&updated.fingerprint)));
        db.read_chat_evidence(1, |reader| {
            let counts = reader.latest_finding_counts(&reader.scans()?)?;
            assert_eq!((counts.total_open, counts.resolved), (1, 2));
            assert!(reader.finding(&continuing.fingerprint)?.is_none());
            assert_eq!(
                reader.finding(&fixed.fingerprint)?.unwrap().state,
                RepoFindingState::Resolved
            );
            Ok(())
        })
        .unwrap();

        // Simulate counts written by an older build and reopen the same data.
        let connection = db.connect().unwrap();
        connection
            .execute(
                "UPDATE scan_runs SET resolved_high_count=3 WHERE id=?1",
                [second],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM performance_metadata WHERE key='scan_resolution_counts_by_package_v1'",
                [],
            )
            .unwrap();
        drop(connection);
        let reopened = Database::new(db.path().to_path_buf()).unwrap();
        assert_eq!(reopened.repo_detail(1).unwrap().summary.resolved_high, 2);
        assert_eq!(
            reopened.latest_completed_findings(1).unwrap(),
            vec![updated.clone()]
        );

        let (rolled_back, _) = scan(vec![continuing.clone()]);
        assert_eq!(
            projection(&continuing, rolled_back).verification_state,
            RemediationVerificationState::StillDetected
        );
        let rolled_back_finding = db
            .current_review_finding(1, &continuing.fingerprint)
            .unwrap();
        assert_ne!(
            rolled_back_finding.material_change.as_deref(),
            Some("reappeared")
        );
        assert_eq!(rolled_back_finding.review_state, ReviewState::Unreviewed);

        let incomplete = db.insert_scan_start(1, &versions).unwrap();
        db.connect()
            .unwrap()
            .execute(
                "UPDATE scan_runs SET status='completed', finished_at=started_at WHERE id=?1",
                [incomplete],
            )
            .unwrap();
        assert_eq!(
            projection(&continuing, rolled_back).verification_state,
            RemediationVerificationState::Unverifiable
        );
        let (after_incomplete, _) = scan(vec![updated.clone()]);
        assert_eq!(
            projection(&updated, after_incomplete).verification_state,
            RemediationVerificationState::StillDetected
        );

        let (_, empty) = scan(vec![]);
        assert_eq!(empty.resolved_findings, 1);
        let (returned, _) = scan(vec![updated.clone()]);
        assert_eq!(
            projection(&updated, returned).verification_state,
            RemediationVerificationState::Reappeared
        );
        assert_eq!(
            db.current_review_finding(1, &updated.fingerprint)
                .unwrap()
                .material_change
                .as_deref(),
            Some("reappeared"),
        );
    }

    #[test]
    fn native_remediation_qa_seed_is_idempotent_and_covers_acceptance_states() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let fixture_root = temp.path().join("fixtures");
        database
            .seed_native_remediation_qa(&fixture_root)
            .expect("seed native QA");
        {
            let connection = database.connect().expect("connect seeded database");
            connection
                .execute(
                    "UPDATE repos SET source_key = NULL WHERE source_type = 'local'",
                    [],
                )
                .expect("simulate legacy discovery rewrite");
            connection
                .execute(
                    "UPDATE app_settings SET value = '3' WHERE key = 'native_remediation_qa_seed'",
                    [],
                )
                .expect("simulate prior seed version");
        }
        database
            .seed_native_remediation_qa(&fixture_root)
            .expect("upgrade native QA seed");
        database
            .seed_native_remediation_qa(&fixture_root)
            .expect("repeat native QA seed");

        assert_eq!(
            database
                .get_setting("discovery_root")
                .expect("discovery root"),
            Some(fixture_root.join("discovery-root").display().to_string())
        );
        let discovered = crate::core::repo_discovery::discover_repositories(
            &fixture_root.join("discovery-root"),
        )
        .expect("discover native QA repositories");
        assert!(discovered.is_empty());
        database
            .sync_repositories(&discovered)
            .expect("reconcile native QA repositories");

        let items = database.all_review_findings().expect("review findings");
        assert_eq!(items.len(), 12);
        let projection = |repo_name: &str| {
            let item = items
                .iter()
                .find(|item| item.repo_name == repo_name)
                .unwrap_or_else(|| panic!("missing {repo_name}"));
            database
                .finding_remediation(
                    item.repo_id,
                    &item.finding.fingerprint,
                    item.scan_run_id.expect("evidence scan"),
                )
                .expect("projection")
        };

        let exact = projection("QA Exact target");
        assert_eq!(exact.evidence_quality, RemediationEvidenceQuality::Current);
        assert_eq!(exact.relationship, RemediationRelationship::Direct);
        assert_eq!(
            exact.verification_state,
            RemediationVerificationState::StillDetected
        );
        assert!(exact.can_open_manifest);

        let no_fix = projection("QA No target");
        assert_eq!(no_fix.scanner_reported_target, None);
        assert_eq!(
            no_fix.evidence_quality,
            RemediationEvidenceQuality::Unavailable
        );

        assert_eq!(
            projection("QA Resolved").verification_state,
            RemediationVerificationState::NotDetected
        );
        assert_eq!(
            projection("QA Reappeared").verification_state,
            RemediationVerificationState::Reappeared
        );

        let partial = projection("QA Partial transitive");
        assert_eq!(
            partial.evidence_quality,
            RemediationEvidenceQuality::Partial
        );
        assert_eq!(partial.relationship, RemediationRelationship::Transitive);
        assert_eq!(partial.location_count, 3);

        assert_eq!(
            projection("QA Evidence unavailable").evidence_quality,
            RemediationEvidenceQuality::Unavailable
        );
        let stale = projection("QA Stale resolved");
        assert_eq!(stale.evidence_quality, RemediationEvidenceQuality::Stale);
        assert_eq!(
            stale.verification_state,
            RemediationVerificationState::NotDetected
        );
        assert_eq!(
            projection("QA Failed verification").verification_state,
            RemediationVerificationState::Unverifiable
        );
        assert_eq!(
            projection("QA Incomplete verification").verification_state,
            RemediationVerificationState::Unverifiable
        );
        assert!(!projection("QA Manifest moved").can_open_manifest);
        assert_eq!(
            projection("QA Old data").evidence_quality,
            RemediationEvidenceQuality::Unavailable
        );
        assert!(!projection("QA Linked GitHub").can_open_manifest);

        assert_eq!(
            database
                .list_active_repositories()
                .expect("repositories")
                .len(),
            12
        );
    }

    #[test]
    fn manifest_target_rejects_traversal_without_leaking_absolute_paths() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir(&repo_root).expect("repo root");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");
        let timestamp = timestamps::now();
        connection.execute("INSERT INTO repos (id, name, path, is_active, source_type, created_at, updated_at) VALUES (1, 'demo', ?1, 1, 'local', ?2, ?2)", params![repo_root.to_string_lossy(), timestamp]).expect("repo");
        insert_completed_run(&connection, 1, 1, timestamp);
        insert_vulnerability(&connection, 1, 1, "fp", "HIGH");
        connection.execute(
            "INSERT INTO vulnerability_remediation_evidence (repo_id, scan_run_id, fingerprint, schema_version, relative_path, location_kind, relationship, evidence_source, evidence_quality, location_count) VALUES (1, 1, 'fp', 1, '../secret', 'manifest', 'unknown', 'syft_location', 'partial', 1)",
            [],
        ).expect("malicious evidence");
        drop(connection);
        let error = database
            .finding_manifest_target(1, "fp", 1)
            .expect_err("reject traversal")
            .to_string();
        assert!(!error.contains(temp.path().to_string_lossy().as_ref()));
        assert!(error.contains("not available"));

        #[cfg(unix)]
        {
            let outside = temp.path().join("outside.json");
            std::fs::write(&outside, "{}").expect("outside file");
            std::os::unix::fs::symlink(&outside, repo_root.join("manifest-link")).expect("symlink");
            let connection = database.connect().expect("connect");
            connection
                .execute(
                    "UPDATE vulnerability_remediation_evidence SET relative_path = 'manifest-link' WHERE repo_id = 1 AND scan_run_id = 1 AND fingerprint = 'fp'",
                    [],
                )
                .expect("update evidence");
            drop(connection);
            assert!(database.finding_manifest_target(1, "fp", 1).is_err());
        }
    }

    #[test]
    fn remediation_reads_fail_closed_for_old_missing_duplicate_and_removed_records() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");
        let timestamp = timestamps::now();
        connection.execute("INSERT INTO repos (id, name, path, is_active, source_type, created_at, updated_at) VALUES (1, 'old-repo', '/tmp/old-repo', 1, 'local', ?1, ?1)", [&timestamp]).expect("repo");
        insert_completed_run(&connection, 1, 1, timestamp);
        insert_vulnerability(&connection, 1, 1, "old-fp", "HIGH");
        drop(connection);

        let old = database
            .finding_remediation(1, "old-fp", 1)
            .expect("old projection");
        assert_eq!(
            old.evidence_quality,
            RemediationEvidenceQuality::Unavailable
        );
        assert_eq!(old.relationship, RemediationRelationship::Unknown);
        assert!(database.finding_remediation(1, "old-fp", 999).is_err());

        let connection = database.connect().expect("connect");
        connection.execute("INSERT INTO vulnerability_remediation_evidence (repo_id, scan_run_id, fingerprint, schema_version, relationship, evidence_source, evidence_quality, location_count) VALUES (1, 1, 'old-fp', 99, 'direct', 'supported_manifest', 'current', 0)", []).expect("future evidence");
        assert!(connection.execute("INSERT INTO vulnerability_remediation_evidence (repo_id, scan_run_id, fingerprint, schema_version, relationship, evidence_source, evidence_quality, location_count) VALUES (1, 1, 'old-fp', 1, 'unknown', 'none', 'unavailable', 0)", []).is_err());
        drop(connection);
        let future = database
            .finding_remediation(1, "old-fp", 1)
            .expect("future fallback");
        assert_eq!(
            future.evidence_quality,
            RemediationEvidenceQuality::Unavailable
        );

        let connection = database.connect().expect("connect");
        connection
            .execute("DELETE FROM repos WHERE id = 1", [])
            .expect("delete repo");
        let evidence_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM vulnerability_remediation_evidence",
                [],
                |row| row.get(0),
            )
            .expect("evidence count");
        assert_eq!(evidence_count, 0);
        drop(connection);
        assert!(database.finding_remediation(1, "old-fp", 1).is_err());
    }

    #[test]
    fn successful_scans_retain_two_package_snapshots_and_failures_do_not_prune() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");
        let timestamp = timestamps::now();
        connection.execute(
            "INSERT INTO repos (id, name, path, is_active, last_remote_commit_sha, created_at, updated_at) VALUES (1, 'repo', '/tmp/repo', 1, 'abc123', ?1, ?1)",
            [&timestamp],
        ).expect("repo");
        drop(connection);

        let package_for = |version: &str| RepoPackage {
            name: "example".into(),
            version: Some(version.into()),
            package_type: Some("npm".into()),
            package_url: Some(format!("pkg:npm/example@{version}")),
            highest_severity: None,
            finding_count: 0,
            fix_available: false,
        };
        let versions = ScannerVersions {
            syft: "test-syft".into(),
            grype: "test-grype".into(),
        };

        let run_one = database.insert_scan_start(1, &versions).expect("run one");
        database
            .complete_scan(
                1,
                run_one,
                &[],
                &[package_for("1.0.0")],
                &DeltaSummary::default(),
            )
            .expect("complete one");
        let run_two = database.insert_scan_start(1, &versions).expect("run two");
        database
            .complete_scan(
                1,
                run_two,
                &[],
                &[package_for("2.0.0")],
                &DeltaSummary::default(),
            )
            .expect("complete two");
        let failed = database
            .insert_scan_start(1, &versions)
            .expect("failed run");
        database
            .fail_scan(1, failed, "scanner failed", None)
            .expect("fail scan");

        let connection = database.connect().expect("connect after failure");
        let retained_after_failure: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM scan_package_snapshots WHERE repo_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("count snapshots");
        assert_eq!(retained_after_failure, 2);
        drop(connection);

        let run_three = database.insert_scan_start(1, &versions).expect("run three");
        database
            .complete_scan(
                1,
                run_three,
                &[],
                &[package_for("3.0.0")],
                &DeltaSummary::default(),
            )
            .expect("complete three");
        let detail = database.repo_detail(1).expect("detail");
        assert_eq!(
            detail
                .latest_scan
                .as_ref()
                .and_then(|run| run.package_count),
            Some(1)
        );
        assert_eq!(
            detail
                .latest_scan
                .as_ref()
                .and_then(|run| run.source_revision.as_deref()),
            Some("abc123")
        );
        assert!(detail
            .latest_scan
            .as_ref()
            .is_some_and(|run| run.evidence_complete));

        let connection = database.connect().expect("connect after third scan");
        let retained_runs = connection.prepare("SELECT DISTINCT scan_run_id FROM scan_package_snapshots WHERE repo_id = 1 ORDER BY scan_run_id").expect("prepare").query_map([], |row| row.get::<_, i64>(0)).expect("query").collect::<Result<Vec<_>, _>>().expect("collect");
        assert_eq!(retained_runs, vec![run_two, run_three]);
        connection
            .execute("DELETE FROM repos WHERE id = 1", [])
            .expect("delete repo");
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM scan_package_snapshots", [], |row| {
                row.get(0)
            })
            .expect("remaining snapshots");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn init_migrates_repo_source_schema() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("cvec.sqlite");
        {
            let connection = Connection::open(&path).expect("open db");
            connection
                .execute_batch(
                    r#"
                    CREATE TABLE repos (
                      id INTEGER PRIMARY KEY,
                      name TEXT NOT NULL,
                      path TEXT NOT NULL UNIQUE,
                      is_active INTEGER NOT NULL DEFAULT 1,
                      last_scan_status TEXT NOT NULL DEFAULT 'unknown',
                      created_at TEXT NOT NULL,
                      updated_at TEXT NOT NULL
                    );
                    INSERT INTO repos (id, name, path, is_active, created_at, updated_at)
                    VALUES (1, 'repo-one', '/tmp/repo-one', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
                    "#,
                )
                .expect("create old repos");
        }

        let database = Database::new(path).expect("init db");
        let connection = database.connect().expect("connect");
        let columns = table_columns(&connection, "repos");

        for column in [
            "source_type",
            "source_key",
            "default_branch",
            "last_remote_commit_sha",
            "last_remote_synced_at",
            "remote_file_count",
        ] {
            assert!(
                columns.iter().any(|candidate| candidate == column),
                "{column}"
            );
        }

        let source_type: String = connection
            .query_row("SELECT source_type FROM repos WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("source_type");
        assert_eq!(source_type, "local");
    }

    #[test]
    fn github_upsert_prevents_duplicates_and_stores_remote_files() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let info = github_info();

        let first_id = database
            .upsert_github_repository(&info)
            .expect("first upsert");
        let second_id = database
            .upsert_github_repository(&info)
            .expect("second upsert");
        assert_eq!(first_id, second_id);

        let files = vec![GithubRemoteFile {
            path: "package-lock.json".into(),
            blob_sha: "blob-one".into(),
            size: 123,
        }];
        let run_id = database
            .insert_scan_start(
                first_id,
                &ScannerVersions {
                    syft: "fixture".into(),
                    grype: "fixture".into(),
                },
            )
            .unwrap();
        database
            .complete_repository_scan(
                first_id,
                run_id,
                &ScanOutput {
                    evidence: None,
                    findings: Vec::new(),
                    packages: Vec::new(),
                    remediation_evidence: Vec::new(),
                },
                &SourceSnapshot {
                    revision: Some(info.commit_sha.clone()),
                    dirty: Some(false),
                },
                &[],
                Some((&info, &files)),
            )
            .unwrap();

        let repos = database.list_active_repositories().expect("repos");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].source_type, RepoSourceType::Github);
        assert_eq!(repos[0].source_key.as_deref(), Some("github:example/cvec"));
        assert_eq!(repos[0].remote_file_count, Some(1));

        let connection = database.connect().expect("connect");
        let stored_files: u32 = connection
            .query_row("SELECT COUNT(*) FROM repo_remote_files", [], |row| {
                row.get(0)
            })
            .expect("remote file count");
        assert_eq!(stored_files, 1);
    }

    #[test]
    fn github_scan_keeps_previous_evidence_on_failure_and_can_retry() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("github-scan.db")).unwrap();
        let versions = ScannerVersions {
            syft: "fixture".into(),
            grype: "fixture".into(),
        };
        let old = github_info();
        let repo_id = db.upsert_github_repository(&old).unwrap();
        let connection = db.connect().unwrap();
        let save = |run_id, info: &GithubRepositoryInfo, package_name: &str| {
            let finding = NormalizedFinding {
                fingerprint: package_name.into(),
                advisory_id: "CVE-FIXTURE".into(),
                package_name: package_name.into(),
                package_version: Some("1.0".into()),
                package_type: Some("npm".into()),
                package_url: None,
                severity: Severity::High,
                fix_version: None,
                title: None,
                source_url: None,
                advisory_details: None,
            };
            let package = RepoPackage {
                name: package_name.into(),
                version: Some("1.0".into()),
                package_type: Some("npm".into()),
                package_url: None,
                highest_severity: None,
                finding_count: 0,
                fix_available: false,
            };
            let asset = RepoAsset {
                repo_id,
                vendor: package_name.into(),
                product: package_name.into(),
                version: None,
                asset_kind: "repo_signal".into(),
                source_path: Some("package.json".into()),
                evidence: "fixture".into(),
                confidence: 70,
            };
            let file = GithubRemoteFile {
                path: "package-lock.json".into(),
                blob_sha: package_name.into(),
                size: 10,
            };
            db.complete_repository_scan(
                repo_id,
                run_id,
                &ScanOutput {
                    evidence: None,
                    findings: vec![finding],
                    packages: vec![package],
                    remediation_evidence: Vec::new(),
                },
                &SourceSnapshot {
                    revision: Some(info.commit_sha.clone()),
                    dirty: Some(false),
                },
                &[asset],
                Some((info, &[file])),
            )
        };
        let first_run = db.insert_scan_start(repo_id, &versions).unwrap();
        save(first_run, &old, "old").unwrap();
        let before = db.list_active_repositories().unwrap().pop().unwrap();
        assert_eq!(
            before.last_remote_commit_sha.as_deref(),
            Some(old.commit_sha.as_str())
        );
        assert!(before.last_remote_synced_at.is_some());

        let new = GithubRepositoryInfo {
            commit_sha: "new-commit".into(),
            ..old
        };
        let failed_run = db.insert_scan_start(repo_id, &versions).unwrap();
        let revisions = db.data_revisions().unwrap();
        connection.execute_batch("CREATE TRIGGER reject_github_package AFTER INSERT ON scan_package_snapshots BEGIN SELECT RAISE(ABORT,'fixture persistence failure'); END;").unwrap();
        let error = save(failed_run, &new, "new").unwrap_err().to_string();
        assert!(error.contains("fixture persistence failure"));
        assert_eq!(db.data_revisions().unwrap(), revisions);
        let source = SourceSnapshot {
            revision: Some(new.commit_sha.clone()),
            dirty: Some(false),
        };
        db.fail_scan(repo_id, failed_run, &error, Some(&source))
            .unwrap();

        let old_repo = db.list_active_repositories().unwrap().pop().unwrap();
        assert_eq!(
            old_repo.last_remote_commit_sha,
            before.last_remote_commit_sha
        );
        assert_eq!(old_repo.last_remote_synced_at, before.last_remote_synced_at);
        assert_eq!(old_repo.remote_file_count, before.remote_file_count);
        let old_detail = db.repo_detail(repo_id).unwrap();
        assert_eq!(old_detail.assets[0].product, "old");
        assert_eq!(old_detail.packages[0].name, "old");
        assert_eq!(old_detail.findings[0].package_name, "old");
        assert_eq!(
            old_detail.summary.last_successful_source_revision,
            before.last_remote_commit_sha
        );
        let failed_attempt = old_detail
            .scan_history
            .iter()
            .find(|run| run.id == failed_run)
            .unwrap();
        assert_eq!(failed_attempt.status, ScanRunStatus::Failed);
        assert_eq!(failed_attempt.source_revision, source.revision);
        assert!(!failed_attempt.evidence_complete);
        let stored_blob = || {
            connection
                .query_row(
                    "SELECT blob_sha FROM repo_remote_files WHERE repo_id = ?1",
                    [repo_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        };
        assert_eq!(stored_blob(), "old");

        connection
            .execute_batch("DROP TRIGGER reject_github_package")
            .unwrap();
        assert!(save(failed_run, &new, "new").is_err());
        let retry_run = db.insert_scan_start(repo_id, &versions).unwrap();
        let other = db
            .upsert_github_repository(&GithubRepositoryInfo {
                repo: "another-repo".into(),
                html_url: "https://github.com/example/another-repo".into(),
                ..new.clone()
            })
            .unwrap();
        assert!(db
            .fail_scan(other, retry_run, "wrong repository", None)
            .is_err());
        let delta = save(retry_run, &new, "new").unwrap();
        assert_eq!(delta.new_findings, 1);
        assert_eq!(delta.resolved_findings, 1);
        assert!(db
            .fail_scan(repo_id, retry_run, "late failure", None)
            .is_err());
        let reopened = Database::new(db.path().to_path_buf()).unwrap();
        let detail = reopened.repo_detail(repo_id).unwrap();
        assert_eq!(detail.latest_scan.as_ref().unwrap().id, retry_run);
        assert_eq!(detail.summary.last_remote_commit_sha, source.revision);
        assert_eq!(
            detail.summary.last_successful_source_revision,
            source.revision
        );
        assert_eq!(detail.assets[0].product, "new");
        assert_eq!(detail.packages[0].name, "new");
        assert_eq!(detail.findings[0].package_name, "new");
        assert_eq!(stored_blob(), "new");
    }

    #[test]
    fn sync_repositories_does_not_deactivate_github_repos() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let repo_id = database
            .upsert_github_repository(&github_info())
            .expect("github upsert");

        database
            .sync_repositories(&[])
            .expect("sync empty local set");

        let repos = database.list_active_repositories().expect("repos");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].id, repo_id);
        assert_eq!(repos[0].source_type, RepoSourceType::Github);
    }

    #[test]
    fn unchanged_discovery_keeps_review_pages_valid_and_still_applies_real_changes() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("cvec.sqlite");
        let db = Database::new(path.clone()).unwrap();
        seed_review_finding(&db, "discovery-fp", "HIGH", None);
        // Reopen an existing database whose dirty trigger predates the fix.
        db.connect()
            .unwrap()
            .execute_batch(
                r#"DELETE FROM performance_metadata WHERE key='review_dirty_upsert_v1';
               DROP TRIGGER cvec_review_dirty_repos_UPDATE;
               CREATE TRIGGER cvec_review_dirty_repos_UPDATE AFTER UPDATE ON repos
               BEGIN INSERT OR IGNORE INTO review_dirty_repos VALUES (NEW.id); END;"#,
            )
            .unwrap();
        drop(db);
        let db = Database::new(path).unwrap();
        let repo = db.list_active_repositories().unwrap().remove(0);
        let mut discovered = DiscoveredRepo {
            path: PathBuf::from(repo.path),
            name: repo.name,
            remote_url: None,
            remote_host: None,
            provider: None,
        };
        let before = db.data_revisions().unwrap();
        db.sync_repositories(std::slice::from_ref(&discovered))
            .unwrap();
        db.sync_repositories_with_completeness(&[], false).unwrap();
        assert_eq!(db.data_revisions().unwrap(), before);
        assert_eq!(db.list_active_repositories().unwrap().len(), 1);

        discovered.remote_url = Some("https://example.invalid/fixture/repo".into());
        db.sync_repositories(std::slice::from_ref(&discovered))
            .unwrap();
        assert_eq!(
            db.list_active_repositories().unwrap()[0].remote_url,
            discovered.remote_url
        );
        assert!(db.data_revisions().unwrap().review > before.review);
        discovered.remote_url = None;
        db.sync_repositories(std::slice::from_ref(&discovered))
            .unwrap();
        assert!(db.list_active_repositories().unwrap()[0]
            .remote_url
            .is_none());
        db.sync_repositories(&[]).unwrap();
        assert!(db.list_active_repositories().unwrap().is_empty());
        db.sync_repositories(&[discovered]).unwrap();
        assert_eq!(db.list_active_repositories().unwrap()[0].id, repo.id);
        assert_eq!(
            db.list_review_findings(&ReviewQuery::default())
                .unwrap()
                .total,
            1
        );
    }

    #[test]
    fn unchanged_assets_keep_inventory_revision_and_changed_evidence_is_stored() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("cvec.sqlite")).unwrap();
        seed_companion_repository(&db, 1, "assets");
        let docker = RepoAsset {
            repo_id: 1,
            vendor: "docker".into(),
            product: "docker".into(),
            version: None,
            asset_kind: "service".into(),
            source_path: Some("Dockerfile".into()),
            evidence: "Detected Dockerfile".into(),
            confidence: 90,
        };
        let ci = RepoAsset {
            vendor: "gitlab".into(),
            product: "gitlab".into(),
            source_path: None,
            evidence: "Detected CI".into(),
            ..docker.clone()
        };
        db.replace_repo_assets(1, &[docker.clone(), ci.clone()])
            .unwrap();
        let before = db.data_revisions().unwrap();
        db.replace_repo_assets(1, &[ci.clone(), docker]).unwrap();
        assert_eq!(db.data_revisions().unwrap(), before);
        db.replace_repo_assets(1, std::slice::from_ref(&ci))
            .unwrap();
        assert!(db.data_revisions().unwrap().inventory > before.inventory);
        seed_companion_repository(&db, 2, "other");
        assert_eq!(db.list_repository_assets(1).unwrap(), vec![ci]);
        assert!(db.list_repository_assets(2).unwrap().is_empty());
        db.connect()
            .unwrap()
            .execute("UPDATE repos SET is_active = 0 WHERE id = 1", [])
            .unwrap();
        assert!(db.list_repository_assets(1).is_err());
    }

    #[test]
    fn remove_remote_repository_only_allows_github_repos() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");
        connection
            .execute(
                "INSERT INTO repos (id, name, path, is_active, created_at, updated_at) VALUES (1, 'repo-one', '/tmp/repo-one', 1, ?1, ?1)",
                [now()],
            )
            .expect("insert local repo");
        let github_id = database
            .upsert_github_repository(&github_info())
            .expect("github upsert");

        assert!(database.remove_remote_repository(1).is_err());
        database
            .remove_remote_repository(github_id)
            .expect("remove github");

        let repos = database.list_active_repositories().expect("repos");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].source_type, RepoSourceType::Local);
    }

    #[test]
    fn repo_detail_marks_new_open_and_resolved_findings() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");
        let today = OffsetDateTime::now_utc().date();

        connection
            .execute(
                "INSERT INTO repos (id, name, path, is_active, created_at, updated_at) VALUES (1, 'repo-one', '/tmp/repo-one', 1, ?1, ?1)",
                [timestamp_for(today, 8)],
            )
            .expect("insert repo");
        insert_completed_run(&connection, 1, 1, timestamp_for(today, 9));
        insert_completed_run(&connection, 2, 1, timestamp_for(today, 10));
        insert_vulnerability_with_package(
            &connection,
            1,
            1,
            "resolved-fp",
            "CVE-OLD",
            "old-package",
            "HIGH",
            "pkg:npm/old-package@1.0.0",
        );
        insert_vulnerability_with_package(
            &connection,
            1,
            1,
            "open-fp",
            "CVE-OPEN",
            "open-package",
            "CRITICAL",
            "pkg:npm/open-package@1.0.0",
        );
        insert_vulnerability_with_package(
            &connection,
            1,
            2,
            "open-fp",
            "CVE-OPEN",
            "open-package",
            "CRITICAL",
            "pkg:npm/open-package@1.0.0",
        );
        insert_vulnerability_with_package(
            &connection,
            1,
            2,
            "new-fp",
            "CVE-NEW",
            "new-package",
            "MEDIUM",
            "pkg:npm/new-package@1.0.0",
        );
        connection
            .execute(
                r#"
                INSERT INTO scan_packages (repo_id, scan_run_id, name, version, package_type, package_url)
                VALUES (1, 2, 'new-package', '1.0.0', 'npm', 'pkg:npm/new-package@1.0.0')
                "#,
                [],
            )
            .expect("insert package");

        let detail = database.repo_detail(1).expect("repo detail");
        let states = detail
            .findings
            .iter()
            .map(|finding| (finding.fingerprint.as_str(), finding.state))
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(states.get("new-fp"), Some(&RepoFindingState::New));
        assert_eq!(states.get("open-fp"), Some(&RepoFindingState::Open));
        assert_eq!(states.get("resolved-fp"), Some(&RepoFindingState::Resolved));
        let new_package = detail
            .packages
            .iter()
            .find(|package| package.name == "new-package")
            .expect("new package");
        assert_eq!(new_package.finding_count, 1);
        assert_eq!(new_package.highest_severity, Some(Severity::Medium));
    }

    #[test]
    fn indexed_intel_trigger_upgrade_preserves_saved_data_and_repository_revision_scope() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("intel-trigger-upgrade.sqlite");
        let db = Database::new(path.clone()).unwrap();
        let connection = db.connect().unwrap();
        let date = "2026-11-14T10:00:00Z";
        for (id, name, advisory) in [
            (1, "both", "CVE-2099-1000"),
            (2, "alias", "CVE-2099-1001"),
            (3, "unrelated", "CVE-2099-1002"),
        ] {
            seed_companion_repository(&db, id, name);
            insert_completed_run(&connection, id, id, date.into());
            insert_vulnerability(&connection, id, id, advisory, "HIGH");
        }
        insert_vulnerability(&connection, 1, 1, "CVE-2099-1001", "HIGH");
        insert_external_intel_item(
            &connection,
            1,
            "cve-2099-1000",
            "github",
            "GHSA-fixture",
            "Saved advisory",
            "Fixture summary",
            "https://example.invalid/fixture",
            "high",
            &["github"],
            &["cve-2099-1001"],
        );
        let saved = db.external_intel_feed().unwrap().items;
        connection
            .execute_batch(
                "DELETE FROM performance_metadata WHERE key='indexed_repo_intel_triggers_v1';
             DROP TRIGGER cvec_repo_intel_UPDATE;
             CREATE TRIGGER cvec_repo_intel_UPDATE AFTER UPDATE ON external_advisories
             BEGIN SELECT RAISE(ABORT, 'upgrade must replace the existing trigger'); END;",
            )
            .unwrap();
        drop(connection);
        drop(db);

        let upgraded = Database::new(path).unwrap();
        assert_eq!(upgraded.external_intel_feed().unwrap().items, saved);
        let connection = upgraded.connect().unwrap();
        let revisions = || {
            let mut statement = connection
                .prepare("SELECT revision FROM repository_data_revisions ORDER BY repo_id")
                .unwrap();
            statement
                .query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let before = revisions();
        connection
            .execute(
                "UPDATE external_advisories SET title='Updated advisory' WHERE id=1",
                [],
            )
            .unwrap();
        let after = revisions();
        // OLD and NEW each increment both matching repositories once, including
        // the repository that matches both the canonical ID and the alias.
        assert_eq!(before.len(), 3);
        assert_eq!(after, [before[0] + 2, before[1] + 2, before[2]]);
        assert_eq!(
            upgraded.external_intel_feed().unwrap().items[0].title,
            "Updated advisory"
        );
    }

    #[test]
    fn repo_detail_does_not_duplicate_external_intel_alias_enrichment() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");
        let today = OffsetDateTime::now_utc().date();

        connection
            .execute(
                "INSERT INTO repos (id, name, path, is_active, created_at, updated_at) VALUES (1, 'repo-one', '/tmp/repo-one', 1, ?1, ?1)",
                [timestamp_for(today, 8)],
            )
            .expect("insert repo");
        insert_completed_run(&connection, 1, 1, timestamp_for(today, 9));
        insert_vulnerability_with_package(
            &connection,
            1,
            1,
            "matched-fp",
            "CVE-2026-0001",
            "matched-package",
            "HIGH",
            "pkg:npm/matched-package@1.0.0",
        );
        insert_external_intel_item(
            &connection,
            1,
            "github:GHSA-abcd-efgh-ijkl",
            "github",
            "GHSA-abcd-efgh-ijkl",
            "Matched advisory",
            "Detailed external advisory summary.",
            "https://github.com/advisories/GHSA-abcd-efgh-ijkl",
            "high",
            &["github", "nvd"],
            &["CVE-2026-0001", "GHSA-abcd-efgh-ijkl"],
        );

        let detail = database.repo_detail(1).expect("repo detail");
        let finding = detail
            .findings
            .iter()
            .find(|finding| finding.fingerprint == "matched-fp")
            .expect("matched finding");
        assert!(finding.intel.is_none());
    }

    #[test]
    fn repo_detail_uses_persisted_github_advisory_details() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");
        let today = OffsetDateTime::now_utc().date();

        connection
            .execute(
                "INSERT INTO repos (id, name, path, is_active, created_at, updated_at) VALUES (1, 'repo-one', '/tmp/repo-one', 1, ?1, ?1)",
                [timestamp_for(today, 8)],
            )
            .expect("insert repo");
        let finding = NormalizedFinding {
            fingerprint: "github-detail-fp".into(),
            advisory_id: "GHSA-wc8c-qw6v-h7f6".into(),
            package_name: "@hono/node-server".into(),
            package_version: Some("1.19.9".into()),
            package_type: Some("npm".into()),
            severity: Severity::High,
            fix_version: Some("1.19.10".into()),
            title: Some("short scanner description".into()),
            source_url: Some("https://github.com/advisories/GHSA-wc8c-qw6v-h7f6".into()),
            package_url: Some("pkg:npm/%40hono/node-server@1.19.9".into()),
            advisory_details: Some(crate::core::models::AdvisoryDetails {
                fetched_at: None,
                ghsa_id: Some("GHSA-wc8c-qw6v-h7f6".into()),
                cve_id: Some("CVE-2026-29087".into()),
                summary: "GitHub advisory summary".into(),
                description: "Long GitHub advisory description.".into(),
                severity: ExternalIntelSeverity::High,
                published_at: Some(timestamp_for(today, 10)),
                updated_at: Some(timestamp_for(today, 11)),
                vulnerabilities: vec![crate::core::models::AdvisoryPackageImpact {
                    ecosystem: Some("npm".into()),
                    package_name: Some("@hono/node-server".into()),
                    package_purl: Some("pkg:npm/%40hono/node-server".into()),
                    vulnerable_version_range: Some("< 1.19.10".into()),
                    first_patched_version: Some("1.19.10".into()),
                    affected_versions_count: 0,
                }],
                cvss_score: Some("7.5".into()),
                cvss_vector: Some("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N".into()),
                epss_percentage: Some("0.00018".into()),
                epss_percentile: Some("0.0474".into()),
                cwes: vec![crate::core::models::AdvisoryCwe {
                    cwe_id: "CWE-863".into(),
                    name: "Incorrect Authorization".into(),
                }],
                references: vec![crate::core::models::AdvisoryReference {
                    url: "https://nvd.nist.gov/vuln/detail/CVE-2026-29087".into(),
                    source: Some("NVD".into()),
                    kind: Some("advisory".into()),
                    tags: Vec::new(),
                }],
                source_code_location: Some("https://github.com/honojs/node-server".into()),
                url: "https://github.com/advisories/GHSA-wc8c-qw6v-h7f6".into(),
            }),
        };

        let run = database
            .insert_scan_start(
                1,
                &ScannerVersions {
                    syft: "fixture".into(),
                    grype: "fixture".into(),
                },
            )
            .unwrap();
        database
            .complete_scan(1, run, &[finding], &[], &DeltaSummary::default())
            .expect("complete scan");

        let detail = database.repo_detail(1).expect("repo detail");
        let intel = detail.findings[0].intel.as_ref().expect("advisory intel");

        assert_eq!(intel.title, "GitHub advisory summary");
        assert_eq!(intel.summary, "Long GitHub advisory description.");
        assert_eq!(intel.match_kind, RepoFindingIntelMatchKind::SourceUrl);
        assert_eq!(
            intel.vulnerabilities[0].vulnerable_version_range.as_deref(),
            Some("< 1.19.10")
        );
        assert_eq!(intel.cvss_score.as_deref(), Some("7.5"));
        assert_eq!(intel.cwes[0].cwe_id, "CWE-863");
    }

    #[test]
    fn intelligence_feed_and_details_use_the_same_inventory_and_advisory_revisions() {
        let temp = tempdir().expect("tempdir");
        let db = Database::new(temp.path().join("cvec.sqlite")).expect("db");
        let connection = db.connect().expect("connection");
        let today = OffsetDateTime::now_utc().date();
        for (id, name, active) in [
            (1, "web", true),
            (2, "api", true),
            (3, "inactive", false),
            (4, "python", true),
        ] {
            connection.execute(
                "INSERT INTO repos (id, name, path, is_active, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![id, name, format!("/tmp/{name}"), active, timestamp_for(today, 8)],
            ).unwrap();
            insert_completed_run(&connection, id, id, timestamp_for(today, 9));
        }
        for (repo, name, version, kind, purl) in [
            (
                1,
                "archive",
                "1.0.0",
                "npm",
                "pkg:npm/%40demo/archive@1.0.0",
            ),
            (
                1,
                "archive",
                "2.0.0",
                "npm",
                "pkg:npm/%40demo/archive@2.0.0",
            ),
            (
                1,
                "extra-package",
                "1.0.0",
                "npm",
                "pkg:npm/extra-package@1.0.0",
            ),
            (
                3,
                "archive",
                "1.0.0",
                "npm",
                "pkg:npm/%40demo/archive@1.0.0",
            ),
            (
                4,
                "extra-package",
                "1.0.0",
                "python",
                "pkg:pypi/extra-package@1.0.0",
            ),
        ] {
            connection.execute(
                "INSERT INTO scan_packages(repo_id,scan_run_id,name,version,package_type,package_url)
                 VALUES (?1,?1,?2,?3,?4,?5)",
                params![repo, name, version, kind, purl],
            ).unwrap();
        }
        for repo in [1, 2] {
            insert_vulnerability_with_package(
                &connection,
                repo,
                repo,
                "finding",
                "CVE-2099-1001",
                "@demo/archive",
                "HIGH",
                "pkg:npm/%40demo/archive@1.0.0",
            );
        }
        for (id, key) in [(1, "CVE-2099-1003"), (2, "CVE-2099-1004")] {
            insert_external_intel_item(
                &connection,
                id,
                key,
                "osv",
                key,
                "Fixture advisory",
                "Summary",
                "https://example.com/advisory",
                "high",
                &["github", "osv", "osv"],
                &["GHSA-fixture", key],
            );
        }
        connection
            .execute(
                "UPDATE external_advisories SET package_name='first-package', ecosystem='npm'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE external_advisories SET withdrawn_at=?1 WHERE id=2",
                [timestamp_for(today, 10)],
            )
            .unwrap();

        connection.execute_batch(
            "INSERT INTO external_advisory_references (advisory_id,url) VALUES (1,'https://example.com/patch');
             INSERT INTO external_advisory_affected_packages (advisory_id,ecosystem,package_name,package_purl,vulnerable_range)
                 VALUES (1,'npm','first-package',NULL,'< 1.1.0'),
                        (1,'npm','@demo/archive','pkg:npm/%40demo/archive','< 1.1.0'),
                        (1,'npm','extra-package',NULL,'< 1.1.0'),
                        (2,'npm','@demo/archive',NULL,'< 1.1.0');
             INSERT INTO external_advisory_kev (advisory_id,date_added,due_date,required_action)
                 VALUES (1,'2026-10-12','2026-10-19','Apply patch');
             INSERT INTO external_advisory_cwes (advisory_id,cwe_id,name)
                 VALUES (1,'CWE-20','Fixture weakness');",
        ).unwrap();

        let feed = db.external_intel_feed().unwrap();
        assert_eq!(feed.items.len(), 2);
        let active = feed
            .items
            .iter()
            .find(|item| item.withdrawn_at.is_none())
            .unwrap();
        assert_eq!(active.relevance, ExternalIntelRelevance::Relevant);
        assert_eq!(active.repo_count, 2);
        assert_eq!(active.aliases, ["CVE-2099-1003", "GHSA-fixture"]);
        assert_eq!(
            active.sources,
            [ExternalSource::Osv, ExternalSource::Github]
        );
        assert_eq!(active.cwes[0].cwe_id, "CWE-20");
        assert_eq!(
            active.kev_dates.as_ref().unwrap().due_date.as_deref(),
            Some("2026-10-19")
        );
        let detail = db
            .external_intel_detail(
                &active.canonical_key,
                feed.intel_revision,
                feed.inventory_revision,
            )
            .unwrap()
            .unwrap();
        assert_eq!(detail.source_details.len(), 3);
        assert_eq!(detail.references[0].url, "https://example.com/patch");
        assert_eq!(
            detail.affected_packages[0]
                .vulnerable_version_range
                .as_deref(),
            Some("< 1.1.0")
        );
        assert_eq!(
            detail.kev.unwrap().required_action.as_deref(),
            Some("Apply patch")
        );
        assert_eq!(
            detail
                .local_packages
                .iter()
                .map(|entry| (
                    entry.repo_id,
                    entry.repo_name.as_str(),
                    entry.package.name.as_str(),
                    entry.package.version.as_deref()
                ))
                .collect::<Vec<_>>(),
            [
                (2, "api", "@demo/archive", Some("1.0.0")),
                (1, "web", "archive", Some("1.0.0")),
                (1, "web", "archive", Some("2.0.0")),
                (1, "web", "extra-package", Some("1.0.0")),
            ]
        );
        let fixed_version = &detail.local_packages[2];
        assert_eq!(fixed_version.package_name, "@demo/archive");
        let page = db
            .dependency_page(
                fixed_version.repo_id,
                &crate::core::models::DependencyQuery {
                    exact_package: Some(fixed_version.package.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.inventory_count, 3);
        assert_eq!(page.items[0].version.as_deref(), Some("2.0.0"));
        assert_eq!(page.items[0].finding_count, 0);
        let reopened = Database::new(db.path().to_path_buf()).unwrap();
        let cold_detail = reopened
            .external_intel_detail(
                &active.canonical_key,
                feed.intel_revision,
                feed.inventory_revision,
            )
            .unwrap()
            .unwrap();
        assert_eq!(cold_detail.local_packages, detail.local_packages);
        assert!(db
            .external_intel_detail(
                "CVE-2099-1004",
                feed.intel_revision,
                feed.inventory_revision
            )
            .unwrap()
            .unwrap()
            .local_packages
            .is_empty());

        let failed = db
            .insert_scan_start(
                1,
                &ScannerVersions {
                    syft: "fixture".into(),
                    grype: "fixture".into(),
                },
            )
            .unwrap();
        db.fail_scan(1, failed, "Fixture scan failed", None)
            .unwrap();
        let after_failure = db.external_intel_feed().unwrap();
        assert_eq!(
            db.external_intel_detail(
                &active.canonical_key,
                after_failure.intel_revision,
                after_failure.inventory_revision
            )
            .unwrap()
            .unwrap()
            .local_packages,
            detail.local_packages,
        );

        // A newer completed scan supersedes the old match without refreshing the public feed.
        insert_completed_run(&connection, 6, 1, timestamp_for(today, 11));
        assert!(db
            .external_intel_detail(
                &active.canonical_key,
                feed.intel_revision,
                feed.inventory_revision,
            )
            .unwrap()
            .is_none());
        let next = db.external_intel_feed().unwrap();
        assert!(next.items.iter().all(|item| item.repo_count == 1));
        let refreshed_detail = db
            .external_intel_detail(
                &active.canonical_key,
                next.intel_revision,
                next.inventory_revision,
            )
            .unwrap()
            .unwrap();
        assert_eq!(refreshed_detail.local_packages.len(), 1);
        assert_eq!(refreshed_detail.local_packages[0].repo_id, 2);
        let missing_version = db
            .dependency_page(
                1,
                &crate::core::models::DependencyQuery {
                    exact_package: Some(fixed_version.package.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(missing_version.items.is_empty());
        assert_eq!(missing_version.scan_run_id, Some(6));

        // A refresh can reuse a row ID. It must not substitute another advisory.
        connection
            .execute(
                "UPDATE external_advisories SET canonical_key='CVE-2099-1005' WHERE id=?1",
                [active.id],
            )
            .unwrap();
        assert!(db
            .external_intel_detail(
                &active.canonical_key,
                next.intel_revision,
                next.inventory_revision,
            )
            .unwrap()
            .is_none());
        let refreshed = db.external_intel_feed().unwrap();
        assert!(db
            .external_intel_detail(
                &active.canonical_key,
                refreshed.intel_revision,
                refreshed.inventory_revision,
            )
            .unwrap()
            .is_none());
        assert!(db
            .external_intel_detail(
                "CVE-2099-1005",
                refreshed.intel_revision,
                refreshed.inventory_revision,
            )
            .unwrap()
            .is_some());
        connection.execute(
            "UPDATE external_advisory_affected_packages SET package_name='different-package', package_purl=NULL
             WHERE advisory_id=1",
            [],
        ).unwrap();
        let changed = db.external_intel_feed().unwrap();
        let changed_item = changed
            .items
            .iter()
            .find(|item| item.id == active.id)
            .unwrap();
        assert_eq!(changed_item.relevance, ExternalIntelRelevance::Triage);
        assert!(db
            .external_intel_detail(
                &changed_item.canonical_key,
                changed.intel_revision,
                changed.inventory_revision
            )
            .unwrap()
            .unwrap()
            .local_packages
            .is_empty());
    }

    #[test]
    fn repo_detail_does_not_enrich_findings_from_package_only_external_intel() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");
        let today = OffsetDateTime::now_utc().date();

        connection
            .execute(
                "INSERT INTO repos (id, name, path, is_active, created_at, updated_at) VALUES (1, 'repo-one', '/tmp/repo-one', 1, ?1, ?1)",
                [timestamp_for(today, 8)],
            )
            .expect("insert repo");
        insert_completed_run(&connection, 1, 1, timestamp_for(today, 9));
        insert_vulnerability_with_package(
            &connection,
            1,
            1,
            "unmatched-fp",
            "CVE-2026-0002",
            "lodash",
            "HIGH",
            "pkg:npm/lodash@1.0.0",
        );
        insert_external_intel_item(
            &connection,
            1,
            "github:GHSA-package-only",
            "github",
            "GHSA-package-only",
            "Package-only advisory",
            "This should stay in external intel only.",
            "https://github.com/advisories/GHSA-package-only",
            "high",
            &["github"],
            &["GHSA-package-only"],
        );

        let detail = database.repo_detail(1).expect("repo detail");
        let finding = detail
            .findings
            .iter()
            .find(|finding| finding.fingerprint == "unmatched-fp")
            .expect("unmatched finding");

        assert!(finding.intel.is_none());
    }

    #[test]
    fn risk_trend_uses_latest_completed_run_per_repo_by_day() {
        let temp = tempdir().expect("tempdir");
        let database = Database::new(temp.path().join("cvec.sqlite")).expect("init db");
        let connection = database.connect().expect("connect");
        let today = OffsetDateTime::now_utc().date();
        let yesterday = today - Duration::days(1);
        let tomorrow = today.next_day().unwrap();
        let no_repositories = database.list_risk_trend(14).unwrap();
        assert_eq!(no_repositories.len(), 14);
        assert_eq!(no_repositories[13].date, today.to_string());
        assert!(no_repositories
            .iter()
            .all(|point| point.medium + point.high + point.critical == 0));

        for (id, name) in [
            (1, "repo-one"),
            (2, "repo-two"),
            (3, "inactive"),
            (4, "unscanned"),
        ] {
            seed_companion_repository(&database, id, name);
        }
        connection
            .execute("UPDATE repos SET is_active = 0 WHERE id = 3", [])
            .unwrap();

        insert_completed_run(&connection, 1, 1, timestamp_for(yesterday, 12));
        insert_vulnerability(&connection, 1, 1, "repo-one-medium-old", "MEDIUM");
        let late = format!("{yesterday}T23:59:59.500Z");
        insert_completed_run(&connection, 2, 1, late.clone());
        insert_vulnerability(&connection, 1, 2, "repo-one-medium-today", "MEDIUM");
        insert_completed_run(&connection, 3, 1, late);
        insert_vulnerability(&connection, 1, 3, "repo-one-high-today", "HIGH");
        insert_vulnerability(&connection, 1, 3, "repo-one-low", "LOW");
        insert_completed_run(&connection, 4, 2, format!("{today}T00:00:00+00:00"));
        insert_vulnerability(&connection, 2, 4, "repo-two-critical-today", "CRITICAL");
        insert_completed_run(&connection, 5, 3, timestamp_for(today, 10));
        insert_vulnerability(&connection, 3, 5, "inactive-finding", "CRITICAL");
        connection.execute(
            "INSERT INTO scan_runs (id, repo_id, status, started_at, finished_at) VALUES (6, 1, 'failed', ?1, ?1)",
            [timestamp_for(today, 14)],
        ).unwrap();
        insert_completed_run(&connection, 7, 2, format!("{tomorrow}T00:00:00Z"));
        insert_vulnerability(&connection, 2, 7, "tomorrow-finding", "MEDIUM");

        let trend = database.list_risk_trend(7).expect("trend");
        assert_eq!(trend.len(), 7);
        assert_eq!(&database.list_risk_trend(30).unwrap()[23..], trend);
        assert_eq!(database.list_risk_trend(0).unwrap(), trend);
        let yesterday_point = &trend[trend.len() - 2];
        let today_point = trend.last().expect("today point");

        assert_eq!(yesterday_point.date, yesterday.to_string());
        assert_eq!(yesterday_point.medium, 0);
        assert_eq!(yesterday_point.high, 1);
        assert_eq!(yesterday_point.critical, 0);
        assert_eq!(today_point.date, today.to_string());
        assert_eq!(today_point.medium, 0);
        assert_eq!(today_point.high, 1);
        assert_eq!(today_point.critical, 1);
    }

    #[test]
    fn batched_repository_summaries_match_single_repo_and_keep_latest_completed_evidence() {
        let temp = tempdir().unwrap();
        let db = Database::new(temp.path().join("cvec.sqlite")).unwrap();
        for (id, name) in [
            (1, "failed"),
            (2, "running"),
            (3, "clean"),
            (4, "unscanned"),
            (5, "inactive"),
            (6, "critical"),
            (7, "warning"),
        ] {
            seed_companion_repository(&db, id, name);
        }
        let connection = db.connect().unwrap();
        let day = OffsetDateTime::now_utc().date();
        let completed_at = timestamp_for(day, 12);
        for repo_id in [1, 2, 3, 5, 6, 7] {
            insert_completed_run(&connection, repo_id, repo_id, completed_at.clone());
        }
        for (id, repo_id, status) in [(8, 1, "failed"), (9, 2, "running")] {
            connection
                .execute(
                    "INSERT INTO scan_runs(id,repo_id,status,started_at) VALUES (?1,?2,?3,?4)",
                    params![id, repo_id, status, &completed_at],
                )
                .unwrap();
        }
        connection
            .execute("UPDATE repos SET is_active = 0 WHERE id = 5", [])
            .unwrap();
        connection.execute(
            "UPDATE scan_runs SET source_revision = 'fixture-abc', new_critical_count = 1, resolved_medium_count = 2 WHERE id IN (1,6)",
            [],
        ).unwrap();
        insert_vulnerability(&connection, 1, 1, "completed-critical", "CRITICAL");
        insert_vulnerability(&connection, 6, 6, "new-critical", "CRITICAL");
        insert_vulnerability(&connection, 7, 7, "existing-high", "HIGH");
        let summaries = db.list_repo_summaries().unwrap();
        assert_eq!(
            summaries.iter().map(|repo| repo.id).collect::<Vec<_>>(),
            [6, 1, 7, 3, 2, 4]
        );
        assert_eq!(
            summaries
                .iter()
                .map(|repo| &repo.status)
                .collect::<Vec<_>>(),
            [
                &RepoStatus::Critical,
                &RepoStatus::Error,
                &RepoStatus::Warning,
                &RepoStatus::Clean,
                &RepoStatus::Scanning,
                &RepoStatus::Unknown,
            ]
        );
        let failed = &summaries[1];
        assert_eq!(failed.open_critical, 1);
        assert_eq!(failed.new_critical, 1);
        assert_eq!(failed.resolved_medium, 2);
        assert_eq!(
            failed.last_successful_source_revision.as_deref(),
            Some("fixture-abc")
        );
        assert_eq!(
            failed.last_scanned_at.as_deref(),
            Some(completed_at.as_str())
        );
        assert!(failed.evidence_revision > 0);
        for summary in summaries {
            assert_eq!(db.repo_overview(summary.id).unwrap().summary, summary);
        }
        assert!(db.repo_overview(5).is_err());
    }

    fn github_info() -> GithubRepositoryInfo {
        GithubRepositoryInfo {
            owner: "example".into(),
            repo: "cvec".into(),
            name: "cvec".into(),
            html_url: "https://github.com/example/cvec".into(),
            default_branch: "main".into(),
            commit_sha: "abcdef1234567890".into(),
        }
    }

    fn table_columns(connection: &Connection, table: &str) -> Vec<String> {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("prepare table info");
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query table info")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect columns")
    }

    fn insert_completed_run(connection: &Connection, id: i64, repo_id: i64, finished_at: String) {
        connection
            .execute(
                r#"
                INSERT INTO scan_runs (id, repo_id, status, started_at, finished_at)
                VALUES (?1, ?2, 'completed', ?3, ?3)
                "#,
                params![id, repo_id, finished_at],
            )
            .expect("insert scan run");
    }

    fn insert_vulnerability(
        connection: &Connection,
        repo_id: i64,
        scan_run_id: i64,
        fingerprint: &str,
        severity: &str,
    ) {
        connection
            .execute(
                r#"
                INSERT INTO vulnerabilities
                (repo_id, scan_run_id, fingerprint, advisory_id, package_name, severity)
                VALUES (?1, ?2, ?3, ?3, 'demo-package', ?4)
                "#,
                params![repo_id, scan_run_id, fingerprint, severity],
            )
            .expect("insert vulnerability");
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_vulnerability_with_package(
        connection: &Connection,
        repo_id: i64,
        scan_run_id: i64,
        fingerprint: &str,
        advisory_id: &str,
        package_name: &str,
        severity: &str,
        package_url: &str,
    ) {
        connection
            .execute(
                r#"
                INSERT INTO vulnerabilities
                (repo_id, scan_run_id, fingerprint, advisory_id, package_name, package_version, package_type, package_url, severity)
                VALUES (?1, ?2, ?3, ?4, ?5, '1.0.0', 'npm', ?6, ?7)
                "#,
                params![
                    repo_id,
                    scan_run_id,
                    fingerprint,
                    advisory_id,
                    package_name,
                    package_url,
                    severity
                ],
            )
            .expect("insert vulnerability");
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_external_intel_item(
        connection: &Connection,
        id: i64,
        canonical_key: &str,
        source: &str,
        source_item_id: &str,
        title: &str,
        summary: &str,
        url: &str,
        severity: &str,
        sources: &[&str],
        aliases: &[&str],
    ) {
        connection
            .execute(
                r#"
                INSERT INTO external_advisories (
                  id, primary_source, source_item_id, canonical_key, title, summary, url,
                  severity, fetched_at, raw_primary_json, product_keys_json, topic_keys_json
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}', '[]', '[]')
                "#,
                params![
                    id,
                    source,
                    source_item_id,
                    canonical_key,
                    title,
                    summary,
                    url,
                    severity,
                    timestamps::now(),
                ],
            )
            .expect("insert external intel item");

        for source in sources {
            connection
                .execute(
                    r#"
                    INSERT INTO external_advisory_sources (
                      advisory_id, source, source_item_id, url, severity, raw_json
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, '{}')
                    "#,
                    params![id, source, source_item_id, url, severity],
                )
                .expect("insert external intel source");
        }

        for alias in aliases {
            connection
                .execute(
                    "INSERT INTO external_advisory_aliases (advisory_id, alias) VALUES (?1, ?2)",
                    params![id, alias],
                )
                .expect("insert external intel alias");
        }
    }

    fn timestamp_for(day: time::Date, hour: u8) -> String {
        timestamps::format(
            &day.with_hms(hour, 0, 0)
                .expect("valid timestamp")
                .assume_utc(),
        )
    }

    #[cfg(unix)]
    #[test]
    fn init_sets_owner_only_permissions_on_database_files() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("cvec.db");
        let db = Database::new(db_path.clone()).expect("database");
        db.set_setting("privacy_check", "ok".into())
            .expect("write setting");

        for candidate in [
            db_path.clone(),
            super::sqlite_sidecar_path(&db_path, "-wal"),
            super::sqlite_sidecar_path(&db_path, "-shm"),
        ] {
            if let Ok(metadata) = std::fs::symlink_metadata(&candidate) {
                assert_eq!(
                    metadata.permissions().mode() & 0o777,
                    0o600,
                    "expected 0600 on {}",
                    candidate.display()
                );
            }
        }
    }
}
