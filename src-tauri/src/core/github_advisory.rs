use crate::core::timestamps;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use time::OffsetDateTime;

use serde::Deserialize;

use crate::Error;

use super::db::Database;
use super::github_api::{ConditionalJson, GithubApiClient};
use super::models::{
    AdvisoryCwe, AdvisoryDetails, AdvisoryPackageImpact, AdvisoryReference, ExternalIntelSeverity,
};
use super::url_policy;

type AdvisoryLocks = Mutex<BTreeMap<(std::path::PathBuf, String), Weak<tokio::sync::Mutex<()>>>>;
static INFLIGHT: OnceLock<AdvisoryLocks> = OnceLock::new();
static REQUESTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
static RETRY_AT: AtomicU64 = AtomicU64::new(0);

/// Run after the scanner transaction commits. Duplicate IDs across repositories
/// share one in-flight request, and the total job includes queueing time.
pub async fn enrich_completed_scan(
    db: Database,
    repo_id: i64,
    run_id: i64,
    token: Result<Option<String>, Error>,
) -> Result<(usize, u32, Vec<String>), Error> {
    let mut warnings = Vec::new();
    let token = token.unwrap_or_else(|_| {
        warnings.push(
            "Advisory authentication was unavailable; local scanner results remain complete."
                .into(),
        );
        None
    });
    let client = GithubApiClient::new(token)?;
    let read = db.clone();
    let identities = crate::runtime::run(move || read.scan_advisory_identities(repo_id, run_id))
        .await
        .map_err(Error::App)??;
    let ids = identities
        .into_iter()
        .map(|(_, id)| id)
        .collect::<std::collections::BTreeSet<_>>();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let results = futures::future::join_all(ids.into_iter().map(|id| {
        let db = db.clone();
        let client = client.clone();
        async move {
            let result =
                tokio::time::timeout_at(deadline, cached_fetch(db, id.clone(), client)).await;
            (id, result)
        }
    }))
    .await;
    let mut details = BTreeMap::new();
    for (id, result) in results {
        match result {
            Ok(Ok(detail))=>{details.insert(id,detail);},
            _=>warnings.push(format!("Optional advisory details for {id} are unavailable; local scanner results remain complete.")),
        }
    }
    if warnings.len() > 12 {
        let extra = warnings.len() - 12;
        warnings.truncate(12);
        warnings.push(format!(
            "{extra} additional optional advisory lookups were unavailable."
        ));
    }
    let write = db.clone();
    let (changed, reopened) =
        crate::runtime::run(move || write.apply_scan_advisories(repo_id, run_id, &details))
            .await
            .map_err(Error::App)??;
    Ok((changed, reopened, warnings))
}

async fn cached_fetch(
    db: Database,
    id: String,
    client: GithubApiClient,
) -> Result<AdvisoryDetails, Error> {
    let requested = id.clone();
    cached_fetch_with(db, id, move |etag| async move {
        let response = client
            .get_advisory_json::<GithubAdvisory>(
                &format!("https://api.github.com/advisories/{requested}"),
                etag.as_deref(),
            )
            .await?;
        Ok(match response {
            ConditionalJson::Modified(value, etag) => {
                ConditionalJson::Modified(value.into_details(), etag)
            }
            ConditionalJson::NotModified => ConditionalJson::NotModified,
            ConditionalJson::RateLimited(at) => ConditionalJson::RateLimited(at),
        })
    })
    .await
}
async fn cached_fetch_with<F, Fut>(
    db: Database,
    id: String,
    fetch: F,
) -> Result<AdvisoryDetails, Error>
where
    F: FnOnce(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<ConditionalJson<AdvisoryDetails>, Error>>,
{
    let lock = {
        let mut locks = INFLIGHT
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map_err(|_| Error::App("Advisory cache lock is unavailable".into()))?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        let key = (db.path().to_path_buf(), id.clone());
        let lock = locks
            .get(&key)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    };
    let _guard = lock.lock().await;
    let read = db.clone();
    let key = id.clone();
    let cached = crate::runtime::run(move || read.cached_advisory(&key))
        .await
        .map_err(Error::App)??;
    if let Some(cached) = cached.as_ref().filter(|value| value.is_fresh()) {
        return Ok(cached.details.clone());
    }
    let _permit = REQUESTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4)))
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| Error::App("Advisory request queue is unavailable".into()))?;
    if RETRY_AT.load(Ordering::Relaxed) > OffsetDateTime::now_utc().unix_timestamp().max(0) as u64 {
        return Err(Error::App(
            "GitHub advisory rate limit is cooling down".into(),
        ));
    }
    let response = fetch(cached.as_ref().and_then(|value| value.etag.clone())).await?;
    let (mut details, etag) = match response {
        ConditionalJson::Modified(advisory, etag) => (advisory, etag),
        ConditionalJson::NotModified => {
            let value = cached
                .ok_or_else(|| Error::App("GitHub returned 304 without cached evidence".into()))?;
            (value.details, value.etag)
        }
        ConditionalJson::RateLimited(at) => {
            RETRY_AT.fetch_max(at, Ordering::Relaxed);
            return Err(Error::App("GitHub advisory rate limit reached".into()));
        }
    };
    if !details
        .ghsa_id
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(&id))
    {
        return Err(Error::App("GitHub advisory identity mismatch".into()));
    }
    details.fetched_at = Some(timestamps::now());
    let write = db.clone();
    let saved = details.clone();
    crate::runtime::run(move || write.cache_advisory(&id, &saved, etag.as_deref()))
        .await
        .map_err(Error::App)??;
    Ok(details)
}

pub(crate) fn exact_github_advisory_id(
    advisory_id: &str,
    source_url: Option<&str>,
) -> Option<String> {
    if is_ghsa_id(advisory_id) {
        return Some(advisory_id.to_ascii_uppercase());
    }

    source_url.and_then(github_advisory_id_from_url)
}

fn github_advisory_id_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if !(trimmed.starts_with("https://github.com/advisories/")
        || trimmed.starts_with("http://github.com/advisories/"))
    {
        return None;
    }

    trimmed
        .rsplit('/')
        .next()
        .filter(|value| is_ghsa_id(value))
        .map(|value| value.to_ascii_uppercase())
}

fn is_ghsa_id(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    parts.len() == 4
        && parts[0].eq_ignore_ascii_case("GHSA")
        && parts[1..]
            .iter()
            .all(|part| part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}

#[derive(Debug, Deserialize)]
struct GithubAdvisory {
    ghsa_id: Option<String>,
    cve_id: Option<String>,
    html_url: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    severity: Option<String>,
    published_at: Option<String>,
    updated_at: Option<String>,
    source_code_location: Option<String>,
    references: Option<Vec<String>>,
    vulnerabilities: Option<Vec<GithubAdvisoryVulnerability>>,
    cvss: Option<GithubCvss>,
    cvss_severities: Option<GithubCvssSeverities>,
    epss: Option<GithubEpss>,
    cwes: Option<Vec<GithubCwe>>,
}

impl GithubAdvisory {
    fn into_details(self) -> AdvisoryDetails {
        let (cvss_score, cvss_vector) = self
            .preferred_cvss()
            .map(|value| {
                (
                    value.score.map(|score| score.to_string()),
                    value.vector_string.clone(),
                )
            })
            .unwrap_or((None, None));

        AdvisoryDetails {
            fetched_at: None,
            ghsa_id: self.ghsa_id.clone(),
            cve_id: self.cve_id,
            summary: self.summary.unwrap_or_else(|| {
                self.ghsa_id
                    .clone()
                    .unwrap_or_else(|| "GitHub advisory".to_string())
            }),
            description: self.description.unwrap_or_default(),
            severity: parse_github_severity(self.severity.as_deref()),
            published_at: self.published_at,
            updated_at: self.updated_at,
            vulnerabilities: self
                .vulnerabilities
                .unwrap_or_default()
                .into_iter()
                .map(GithubAdvisoryVulnerability::into_impact)
                .collect(),
            cvss_score,
            cvss_vector,
            epss_percentage: self
                .epss
                .as_ref()
                .and_then(|epss| epss.percentage.map(|value| value.to_string())),
            epss_percentile: self
                .epss
                .and_then(|epss| epss.percentile.map(|value| value.to_string())),
            cwes: self
                .cwes
                .unwrap_or_default()
                .into_iter()
                .filter_map(GithubCwe::into_cwe)
                .collect(),
            references: self
                .references
                .unwrap_or_default()
                .into_iter()
                .filter_map(|url| {
                    let url = url_policy::external_url(&url)?;
                    Some(AdvisoryReference {
                        url,
                        source: Some("GitHub".to_string()),
                        kind: None,
                        tags: Vec::new(),
                    })
                })
                .collect(),
            source_code_location: self
                .source_code_location
                .and_then(|url| url_policy::external_url(&url)),
            url: self
                .html_url
                .and_then(|url| url_policy::external_url(&url))
                .unwrap_or_else(|| {
                    self.ghsa_id
                        .map(|id| format!("https://github.com/advisories/{id}"))
                        .unwrap_or_else(|| "https://github.com/advisories".to_string())
                }),
        }
    }

    fn preferred_cvss(&self) -> Option<&GithubCvss> {
        self.cvss_severities
            .as_ref()
            .and_then(|severities| {
                severities
                    .cvss_v4
                    .as_ref()
                    .filter(|cvss| cvss.has_signal())
                    .or_else(|| severities.cvss_v3.as_ref().filter(|cvss| cvss.has_signal()))
            })
            .or_else(|| self.cvss.as_ref().filter(|cvss| cvss.has_signal()))
    }
}
#[derive(Debug, Deserialize)]
struct GithubAdvisoryVulnerability {
    package: Option<GithubAdvisoryPackage>,
    vulnerable_version_range: Option<String>,
    first_patched_version: Option<String>,
}

impl GithubAdvisoryVulnerability {
    fn into_impact(self) -> AdvisoryPackageImpact {
        AdvisoryPackageImpact {
            ecosystem: self
                .package
                .as_ref()
                .and_then(|package| package.ecosystem.clone()),
            package_name: self.package.and_then(|package| package.name),
            package_purl: None,
            vulnerable_version_range: self.vulnerable_version_range,
            first_patched_version: self.first_patched_version,
            affected_versions_count: 0,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GithubAdvisoryPackage {
    ecosystem: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubCvss {
    vector_string: Option<String>,
    score: Option<f32>,
}

impl GithubCvss {
    fn has_signal(&self) -> bool {
        self.vector_string
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || self.score.is_some_and(|score| score > 0.0)
    }
}

#[derive(Debug, Deserialize)]
struct GithubCvssSeverities {
    cvss_v3: Option<GithubCvss>,
    cvss_v4: Option<GithubCvss>,
}

#[derive(Debug, Deserialize)]
struct GithubEpss {
    percentage: Option<f32>,
    percentile: Option<GithubNumberOrString>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GithubNumberOrString {
    Number(f32),
    String(String),
}

impl std::fmt::Display for GithubNumberOrString {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(value) => write!(formatter, "{value}"),
            Self::String(value) => formatter.write_str(value),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GithubCwe {
    cwe_id: Option<String>,
    name: Option<String>,
}

impl GithubCwe {
    fn into_cwe(self) -> Option<AdvisoryCwe> {
        Some(AdvisoryCwe {
            cwe_id: self.cwe_id?,
            name: self.name.unwrap_or_default(),
        })
    }
}

fn parse_github_severity(value: Option<&str>) -> ExternalIntelSeverity {
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "critical" => ExternalIntelSeverity::Critical,
        "high" => ExternalIntelSeverity::High,
        "medium" | "moderate" => ExternalIntelSeverity::Medium,
        "low" => ExternalIntelSeverity::Low,
        _ => ExternalIntelSeverity::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::super::models::NormalizedFinding;
    use super::*;

    #[tokio::test]
    async fn github_auth_failure_does_not_discard_scanner_results() {
        let findings = vec![NormalizedFinding {
            fingerprint: "local-finding".into(),
            advisory_id: "CVE-2026-0001".into(),
            package_name: "router".into(),
            package_version: Some("1.0.0".into()),
            package_type: Some("npm".into()),
            severity: super::super::models::Severity::High,
            fix_version: Some("1.1.0".into()),
            title: None,
            source_url: None,
            package_url: None,
            match_evidence: None,
            advisory_details: None,
        }];
        let original = findings.clone();
        let temp = tempfile::tempdir().expect("tempdir");
        let db = super::super::db::Database::new(temp.path().join("cvec.db")).expect("database");
        db.sync_repositories(&[super::super::repo_discovery::DiscoveredRepo {
            name: "local".into(),
            path: temp.path().join("repo"),
            remote_url: None,
            remote_host: None,
            provider: None,
        }])
        .expect("repository");
        let repo = db.list_active_repositories().expect("repositories")[0].id;
        let run = db
            .insert_scan_start(
                repo,
                &super::super::models::ScannerVersions {
                    syft: "test".into(),
                    grype: "test".into(),
                },
            )
            .expect("scan");
        let delta = super::super::delta::compute_delta(&[], &findings);
        db.complete_scan(repo, run, &findings, &[], &delta)
            .expect("save scanner results");
        let (_, _, warnings) = enrich_completed_scan(
            db.clone(),
            repo,
            run,
            Err(Error::App("refresh failed".into())),
        )
        .await
        .expect("optional enrichment");
        assert_eq!(findings, original);
        assert_eq!(warnings.len(), 1);
        db.record_scan_warnings(run, &warnings)
            .expect("persist advisory warning");
        assert_eq!(
            db.repo_detail(repo)
                .expect("repository detail")
                .latest_scan
                .expect("completed scan")
                .warnings,
            warnings
        );
        assert_eq!(
            db.latest_completed_findings(repo).expect("saved findings"),
            original
        );
    }

    #[tokio::test]
    async fn duplicate_repositories_share_fetches_and_slow_requests_are_bounded() {
        use std::sync::atomic::AtomicUsize;
        let temp = tempfile::tempdir().unwrap();
        let db = Database::new(temp.path().join("cache.db")).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let detail = |id: String| {
            serde_json::from_value::<GithubAdvisory>(
                serde_json::json!({"ghsa_id":id,"severity":"high","summary":"local fake"}),
            )
            .unwrap()
            .into_details()
        };
        let tasks = (0..8).map(|_| {
            let calls = calls.clone();
            let db = db.clone();
            async move {
                cached_fetch_with(db, "GHSA-ABCD-1234-ABCD".into(), move |_| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    Ok(ConditionalJson::Modified(
                        detail("GHSA-ABCD-1234-ABCD".into()),
                        Some("etag".into()),
                    ))
                })
                .await
            }
        });
        assert!(futures::future::join_all(tasks)
            .await
            .into_iter()
            .all(|value| value.is_ok()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(140);
        let tasks = (0..10).map(|index| {
            let db = db.clone();
            let active = active.clone();
            let maximum = maximum.clone();
            let id = format!("GHSA-{index:04}-1234-ABCD");
            async move {
                tokio::time::timeout_at(
                    deadline,
                    cached_fetch_with(db, id.clone(), move |_| async move {
                        struct Active(Arc<AtomicUsize>);
                        impl Drop for Active {
                            fn drop(&mut self) {
                                self.0.fetch_sub(1, Ordering::SeqCst);
                            }
                        }
                        let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                        let _active = Active(active);
                        maximum.fetch_max(count, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Ok(ConditionalJson::Modified(detail(id), None))
                    }),
                )
                .await
            }
        });
        let results = futures::future::join_all(tasks).await;
        assert!(results.iter().any(Result::is_err));
        assert!(maximum.load(Ordering::SeqCst) <= 4);
        assert!(db
            .cached_advisory("GHSA-ABCD-1234-ABCD")
            .unwrap()
            .unwrap()
            .is_fresh());
    }

    #[test]
    fn extracts_github_advisory_id_from_source_url() {
        let finding = NormalizedFinding {
            fingerprint: "fp".into(),
            advisory_id: "CVE-2026-29087".into(),
            package_name: "pkg".into(),
            package_version: None,
            package_type: None,
            severity: super::super::models::Severity::High,
            fix_version: None,
            title: None,
            source_url: Some("https://github.com/advisories/GHSA-wc8c-qw6v-h7f6".into()),
            package_url: None,
            match_evidence: None,
            advisory_details: None,
        };

        assert_eq!(
            exact_github_advisory_id(&finding.advisory_id, finding.source_url.as_deref())
                .as_deref(),
            Some("GHSA-WC8C-QW6V-H7F6")
        );
    }
}
