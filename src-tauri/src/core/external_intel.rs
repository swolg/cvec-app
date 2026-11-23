use crate::core::timestamps;
use std::collections::BTreeSet;

use regex::Regex;
use reqwest::{Client, RequestBuilder, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::Error;

use super::db::{advisory_package_keys, Database, LocalPackageIndex, PersistedExternalIntelItem};
use super::external_topics::{
    normalize_external_ecosystem, product_key, product_key_to_topic, topic_definitions,
};
use super::github_api::GithubApiClient;
use super::models::{
    AdvisoryCwe, AdvisoryPackageImpact, AdvisoryReference, CisaKevDetails, ExternalIntelMatchKind,
    ExternalIntelRelevance, ExternalIntelSeverity, ExternalIntelSourceDetail, ExternalSource,
};
use super::url_policy;

const MAX_SOURCE_ITEMS: usize = 60;
const MAX_MERGED_ITEMS: usize = 100;
const MAX_EXTERNAL_RESPONSE_BYTES: u64 = 10 * 1024 * 1024;
const LOOKBACK_DAYS: i64 = 7;

#[derive(Debug, Clone)]
struct RawIntelItem {
    source: ExternalSource,
    source_item_id: String,
    title: Option<String>,
    summary: Option<String>,
    url: Option<String>,
    ecosystem: Option<String>,
    package_name: Option<String>,
    product_keys: BTreeSet<String>,
    topic_keys: BTreeSet<String>,
    severity: ExternalIntelSeverity,
    published_at: Option<String>,
    updated_at: Option<String>,
    modified_at: Option<String>,
    withdrawn_at: Option<String>,
    cvss_score: Option<String>,
    cvss_vector: Option<String>,
    cvss_version: Option<String>,
    epss_percentage: Option<String>,
    epss_percentile: Option<String>,
    is_kev: bool,
    is_ransomware_known: bool,
    fetched_at: String,
    raw_json: String,
    aliases: BTreeSet<String>,
    affected_packages: Vec<AdvisoryPackageImpact>,
    references: Vec<AdvisoryReference>,
    cwes: Vec<AdvisoryCwe>,
    kev: Option<CisaKevDetails>,
}

#[derive(Debug, Clone)]
struct MergedIntelItem {
    primary_source: ExternalSource,
    source_item_id: String,
    title: String,
    summary: String,
    url: String,
    ecosystem: Option<String>,
    package_name: Option<String>,
    product_keys: BTreeSet<String>,
    topic_keys: BTreeSet<String>,
    severity: ExternalIntelSeverity,
    published_at: Option<String>,
    updated_at: Option<String>,
    modified_at: Option<String>,
    withdrawn_at: Option<String>,
    cvss_score: Option<String>,
    cvss_vector: Option<String>,
    cvss_version: Option<String>,
    epss_percentage: Option<String>,
    epss_percentile: Option<String>,
    is_kev: bool,
    is_ransomware_known: bool,
    fetched_at: String,
    raw_json: String,
    aliases: BTreeSet<String>,
    sources: BTreeSet<ExternalSource>,
    source_details: Vec<ExternalIntelSourceDetail>,
    affected_packages: Vec<AdvisoryPackageImpact>,
    references: Vec<AdvisoryReference>,
    cwes: Vec<AdvisoryCwe>,
    kev: Option<CisaKevDetails>,
}

fn is_allowed_external_redirect(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let normalized = host.trim_matches(['[', ']']).to_ascii_lowercase();
    !(normalized.is_empty()
        || normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
        || normalized.parse::<std::net::IpAddr>().is_ok())
}

pub async fn refresh(db: &Database, github_token: Option<String>) -> Result<(), Error> {
    let client = Client::builder()
        .user_agent(format!("cvec/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many external feed redirects")
            } else if is_allowed_external_redirect(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("blocked non-HTTPS or local redirect")
            }
        }))
        .build()
        .map_err(|cause| Error::App(format!("failed to build external intel client: {cause}")))?;
    let fetched_at = timestamps::now();
    let since = OffsetDateTime::now_utc() - Duration::days(LOOKBACK_DAYS);

    let watched_topics = db.watched_external_intel_topic_keys()?;

    if watched_topics.is_empty() {
        db.replace_external_intel(&[], &fetched_at)?;
        return Ok(());
    }

    let (osv, github, nvd, cisa, vendor) = tokio::join!(
        fetch_osv(&client, since, &fetched_at, &watched_topics),
        fetch_github(github_token, since, &fetched_at),
        fetch_nvd(&client, since, &fetched_at),
        fetch_cisa_kev(&client, since, &fetched_at),
        fetch_vendor_feeds(&client, since, &fetched_at),
    );

    let mut raw_items = Vec::new();
    let mut source_failures = Vec::new();

    collect_source_items("OSV", osv, &mut raw_items, &mut source_failures);
    collect_source_items("GitHub", github, &mut raw_items, &mut source_failures);
    collect_source_items("NVD", nvd, &mut raw_items, &mut source_failures);
    collect_source_items("CISA KEV", cisa, &mut raw_items, &mut source_failures);
    collect_source_items("Vendor", vendor, &mut raw_items, &mut source_failures);

    ensure_complete_refresh(&source_failures)?;

    if raw_items.is_empty() {
        return Err(Error::App(
            "failed to refresh external intel: all public sources returned no data".to_string(),
        ));
    }

    let raw_count = raw_items.len();
    let merged = merge_items(raw_items);
    let merged_count = merged.len();
    let watched_topics = watched_topics
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let watched_items: Vec<_> = merged
        .into_iter()
        .filter(|item| item_matches_watched_topics(item, &watched_topics))
        .collect();
    let db = db.clone();
    let persisted = crate::runtime::run(move || {
        let wanted = watched_items
            .iter()
            .flat_map(|item| {
                advisory_package_keys(
                    item.ecosystem.as_deref(),
                    item.package_name.as_deref(),
                    &item.affected_packages,
                )
            })
            .collect();
        let local_index = db.latest_local_package_index(&wanted)?;
        let mut persisted: Vec<_> = watched_items
            .into_iter()
            .map(|item| to_persisted_item(item, &local_index))
            .collect();
        persisted.sort_by(|left, right| {
            external_published_sort_key(right.published_at.as_deref())
                .cmp(&external_published_sort_key(left.published_at.as_deref()))
                .then_with(|| {
                    repo_count_rank(right.repo_count).cmp(&repo_count_rank(left.repo_count))
                })
                .then_with(|| right.severity.rank().cmp(&left.severity.rank()))
                .then_with(|| left.title.cmp(&right.title))
        });
        persisted.truncate(MAX_MERGED_ITEMS);
        db.replace_external_intel(&persisted, &fetched_at)?;
        Ok::<_, Error>(persisted)
    })
    .await
    .map_err(Error::App)??;

    eprintln!(
        "External intel refresh: fetched={raw_count} merged={merged_count} persisted={} dropped={} matched={} general={} source_failures={}",
        persisted.len(),
        merged_count.saturating_sub(persisted.len()),
        persisted
            .iter()
            .filter(|item| item.relevance == ExternalIntelRelevance::Relevant)
            .count(),
        persisted
            .iter()
            .filter(|item| item.relevance == ExternalIntelRelevance::General)
            .count(),
        source_failures.len()
    );

    Ok(())
}

fn ensure_complete_refresh(source_failures: &[String]) -> Result<(), Error> {
    if source_failures.is_empty() {
        return Ok(());
    }

    Err(Error::App(format!(
        "external intel refresh was not saved, so existing data remains intact; failed sources: {}",
        source_failures.join(" | ")
    )))
}

fn collect_source_items(
    label: &str,
    result: Result<Vec<RawIntelItem>, Error>,
    raw_items: &mut Vec<RawIntelItem>,
    source_failures: &mut Vec<String>,
) {
    match result {
        Ok(items) => raw_items.extend(items),
        Err(cause) => {
            eprintln!("External intel source {label} failed: {cause}");
            source_failures.push(format!("{label}: {cause}"));
        }
    }
}

async fn external_response(request: RequestBuilder, context: &str) -> Result<Response, Error> {
    let response = request
        .send()
        .await
        .map_err(|cause| Error::App(format!("failed to fetch {context}: {cause}")))?;
    let response = response
        .error_for_status()
        .map_err(|cause| Error::App(format!("failed to read {context}: {cause}")))?;

    if response
        .content_length()
        .is_some_and(|length| length > MAX_EXTERNAL_RESPONSE_BYTES)
    {
        return Err(response_size_limit_error(context));
    }

    Ok(response)
}

async fn external_json<T: DeserializeOwned>(response: Response, context: &str) -> Result<T, Error> {
    let bytes = external_bytes(response, context).await?;
    serde_json::from_slice(&bytes)
        .map_err(|cause| Error::App(format!("failed to decode {context}: {cause}")))
}

async fn external_text(response: Response, context: &str) -> Result<String, Error> {
    let bytes = external_bytes(response, context).await?;
    String::from_utf8(bytes)
        .map_err(|cause| Error::App(format!("failed to decode {context}: {cause}")))
}

async fn external_bytes(mut response: Response, context: &str) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    let max_len = MAX_EXTERNAL_RESPONSE_BYTES as usize;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|cause| Error::App(format!("failed to read {context}: {cause}")))?
    {
        if bytes.len().saturating_add(chunk.len()) > max_len {
            return Err(response_size_limit_error(context));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn response_size_limit_error(context: &str) -> Error {
    Error::App(format!(
        "{context} response exceeded the {} limit",
        external_response_size_limit_label()
    ))
}

fn external_response_size_limit_label() -> String {
    const MEBIBYTE: u64 = 1024 * 1024;
    if MAX_EXTERNAL_RESPONSE_BYTES.is_multiple_of(MEBIBYTE) {
        format!("{} MB", MAX_EXTERNAL_RESPONSE_BYTES / MEBIBYTE)
    } else {
        format!("{} bytes", MAX_EXTERNAL_RESPONSE_BYTES)
    }
}

async fn fetch_osv(
    client: &Client,
    since: OffsetDateTime,
    fetched_at: &str,
    watched_topics: &[String],
) -> Result<Vec<RawIntelItem>, Error> {
    let topic_lookup = watched_topics
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let selected_topics = topic_definitions()
        .iter()
        .filter(|topic| topic_lookup.contains(topic.key))
        .collect::<Vec<_>>();
    let per_topic_limit = if selected_topics.is_empty() {
        MAX_SOURCE_ITEMS
    } else {
        (MAX_SOURCE_ITEMS / selected_topics.len()).clamp(4, 12)
    };
    let mut entries = Vec::new();
    for topic in selected_topics {
        let Some(osv_ecosystem) = topic.osv_ecosystem else {
            continue;
        };
        entries
            .extend(fetch_osv_list_entries(client, since, osv_ecosystem, per_topic_limit).await?);
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.published_at));
    let mut seen_ids = BTreeSet::new();
    entries.retain(|entry| seen_ids.insert(entry.id.clone()));
    entries.truncate(MAX_SOURCE_ITEMS);

    let mut items = Vec::new();
    for entry in entries {
        let url = format!("https://api.osv.dev/v1/vulns/{}", entry.id);
        let context = format!("OSV advisory {}", entry.id);
        let response = external_response(client.get(&url), &context).await?;
        let detail = external_json::<OsvAdvisory>(response, &context).await?;
        let Some(item) = normalize_osv(detail, fetched_at)? else {
            continue;
        };
        items.push(item);
    }

    Ok(items)
}

async fn fetch_cisa_kev(
    client: &Client,
    _since: OffsetDateTime,
    fetched_at: &str,
) -> Result<Vec<RawIntelItem>, Error> {
    let context = "CISA KEV catalog";
    let response = external_response(
        client.get(
            "https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json",
        ),
        context,
    )
    .await?;
    let payload = external_json::<CisaKevCatalog>(response, context).await?;

    normalize_cisa_kev_catalog(payload, fetched_at)
}

fn normalize_cisa_kev_catalog(
    payload: CisaKevCatalog,
    fetched_at: &str,
) -> Result<Vec<RawIntelItem>, Error> {
    let catalog_version = payload.catalog_version.clone();
    let catalog_released_at = payload.date_released.clone();
    let mut items = Vec::new();
    for vulnerability in payload
        .vulnerabilities
        .into_iter()
        .rev()
        .take(MAX_SOURCE_ITEMS)
    {
        let raw_json = serde_json::to_string(&vulnerability)?;
        let kev = CisaKevDetails {
            date_added: vulnerability.date_added.clone(),
            due_date: vulnerability.due_date.clone(),
            required_action: vulnerability.required_action.clone(),
            known_ransomware_campaign_use: vulnerability.known_ransomware_campaign_use.clone(),
            notes: vulnerability.notes.clone(),
            catalog_version: catalog_version.clone(),
            catalog_released_at: catalog_released_at.clone(),
        };
        let mut aliases = BTreeSet::new();
        aliases.insert(normalize_alias(&vulnerability.cve_id));
        let mut product_keys = BTreeSet::new();
        if let (Some(vendor), Some(product)) = (
            vulnerability.vendor_project.as_deref(),
            vulnerability.product.as_deref(),
        ) {
            product_keys.insert(product_key(vendor, product));
        }
        let topic_keys = product_keys
            .iter()
            .filter_map(|key| product_key_to_topic(key))
            .collect();

        let published_at = parse_date_as_timestamp(vulnerability.date_added.as_deref())
            .map(|value| timestamps::format(&value))
            .or(vulnerability.date_added);

        items.push(RawIntelItem {
            source: ExternalSource::CisaKev,
            source_item_id: vulnerability.cve_id.clone(),
            title: vulnerability
                .vulnerability_name
                .clone()
                .or_else(|| Some(vulnerability.cve_id.clone())),
            summary: vulnerability.short_description.clone(),
            url: Some("https://www.cisa.gov/known-exploited-vulnerabilities-catalog".to_string()),
            ecosystem: None,
            package_name: None,
            product_keys,
            topic_keys,
            severity: ExternalIntelSeverity::Critical,
            published_at,
            updated_at: None,
            modified_at: None,
            withdrawn_at: None,
            cvss_score: None,
            cvss_vector: None,
            cvss_version: None,
            epss_percentage: None,
            epss_percentile: None,
            is_kev: true,
            is_ransomware_known: vulnerability
                .known_ransomware_campaign_use
                .as_deref()
                .is_some_and(is_positive_ransomware_signal),
            fetched_at: fetched_at.to_string(),
            raw_json,
            aliases,
            affected_packages: Vec::new(),
            references: vec![AdvisoryReference {
                url: "https://www.cisa.gov/known-exploited-vulnerabilities-catalog".to_string(),
                source: Some("CISA KEV".to_string()),
                kind: Some("advisory".to_string()),
                tags: Vec::new(),
            }],
            cwes: vulnerability
                .cwes
                .unwrap_or_default()
                .into_iter()
                .map(|cwe_id| AdvisoryCwe {
                    cwe_id,
                    name: String::new(),
                })
                .collect(),
            kev: Some(kev),
        });
    }

    Ok(items)
}

async fn fetch_vendor_feeds(
    client: &Client,
    since: OffsetDateTime,
    fetched_at: &str,
) -> Result<Vec<RawIntelItem>, Error> {
    let feeds = [
        ("github", "https://github.blog/security/feed/"),
        ("gitlab", "https://about.gitlab.com/releases.xml"),
        ("kubernetes", "https://kubernetes.io/feed.xml"),
        ("docker", "https://www.docker.com/blog/feed/"),
    ];
    let mut items = Vec::new();
    let mut failures = Vec::new();

    for (vendor, url) in feeds {
        let context = format!("{vendor} vendor feed");
        let response = match external_response(client.get(url), &context).await {
            Ok(response) => response,
            Err(cause) => {
                failures.push(format!("{vendor}: {cause}"));
                continue;
            }
        };
        let xml = match external_text(response, &context).await {
            Ok(xml) => xml,
            Err(cause) => {
                failures.push(format!("{vendor}: {cause}"));
                continue;
            }
        };
        let feed_items = parse_vendor_feed(&xml, url)?;
        if feed_items.is_empty() {
            failures.push(format!("{vendor}: no parseable feed entries"));
        }

        for feed_item in feed_items.into_iter().take(20) {
            let title = feed_item.title;
            let link = feed_item
                .link
                .as_deref()
                .and_then(url_policy::external_url)
                .unwrap_or_else(|| url.to_string());
            let published = feed_item.published_at;

            if published.is_some_and(|value| value < since) || !looks_security_relevant(&title) {
                continue;
            }

            let mut product_keys = BTreeSet::new();
            let haystack = format!("{vendor} {title}").to_lowercase();
            collect_product_keys_from_text(&haystack, &mut product_keys);
            if product_keys.is_empty() {
                product_keys.insert(product_key(vendor, vendor));
            }
            let topic_keys = product_keys
                .iter()
                .filter_map(|key| product_key_to_topic(key))
                .collect();
            let source_item_id = format!("vendor:{}:{}", vendor, normalize_slug(&title));

            items.push(RawIntelItem {
                source: ExternalSource::Vendor,
                source_item_id: source_item_id.clone(),
                title: Some(title.clone()),
                summary: feed_item.summary.clone().or(Some(title)),
                url: Some(link.clone()),
                ecosystem: None,
                package_name: None,
                product_keys,
                topic_keys,
                severity: ExternalIntelSeverity::High,
                published_at: published.map(|value| timestamps::format(&value)),
                updated_at: feed_item.updated_at.map(|value| timestamps::format(&value)),
                modified_at: None,
                withdrawn_at: None,
                cvss_score: None,
                cvss_vector: None,
                cvss_version: None,
                epss_percentage: None,
                epss_percentile: None,
                is_kev: false,
                is_ransomware_known: false,
                fetched_at: fetched_at.to_string(),
                raw_json: serde_json::json!({
                    "feed": url,
                    "feedType": feed_item.feed_type,
                    "id": source_item_id,
                    "guid": feed_item.entry_id,
                    "categories": feed_item.categories,
                })
                .to_string(),
                aliases: BTreeSet::new(),
                affected_packages: Vec::new(),
                references: vec![AdvisoryReference {
                    url: link,
                    source: Some(vendor.to_string()),
                    kind: Some(feed_item.feed_type),
                    tags: feed_item.categories,
                }],
                cwes: Vec::new(),
                kev: None,
            });
        }
    }

    if items.is_empty() && !failures.is_empty() {
        return Err(Error::App(format!(
            "failed to fetch vendor feeds: {}",
            failures.join(" | ")
        )));
    }
    if !failures.is_empty() {
        eprintln!(
            "External intel vendor feed partial failures: {}",
            failures.join(" | ")
        );
    }

    Ok(items)
}

async fn fetch_osv_list_entries(
    client: &Client,
    since: OffsetDateTime,
    ecosystem: &str,
    limit: usize,
) -> Result<Vec<OsvListEntry>, Error> {
    let mut entries = Vec::new();
    let mut page = 1usize;

    while entries.len() < limit {
        let page_param = page.to_string();
        let mut query = vec![("ecosystem", ecosystem)];
        if page > 1 {
            query.push(("page", page_param.as_str()));
        }

        let context = "OSV advisory list";
        let response =
            external_response(client.get("https://osv.dev/list").query(&query), context).await?;
        let html = external_text(response, context).await?;

        let page_entries = parse_osv_list_entries(&html)?;
        if page_entries.is_empty() {
            break;
        }

        let mut saw_old_item = false;
        for entry in page_entries {
            if entry.published_at < since {
                saw_old_item = true;
                continue;
            }
            entries.push(entry);
            if entries.len() >= limit {
                break;
            }
        }

        if saw_old_item {
            break;
        }

        page += 1;
    }

    Ok(entries)
}

fn parse_osv_list_entries(html: &str) -> Result<Vec<OsvListEntry>, Error> {
    let row_pattern = Regex::new(
        r#"(?s)<div role="row" class="vuln-table-row mdc-data-table__row">.*?<a href="/vulnerability/([^"]+)">.*?</a>.*?<relative-time datetime="([^"]+)">"#,
    )
    .map_err(|cause| Error::App(format!("failed to prepare OSV list parser: {cause}")))?;

    let mut entries = Vec::new();
    for capture in row_pattern.captures_iter(html) {
        let Some(id_match) = capture.get(1) else {
            continue;
        };
        let Some(datetime_match) = capture.get(2) else {
            continue;
        };
        let published_at = parse_osv_list_timestamp(datetime_match.as_str())?;
        entries.push(OsvListEntry {
            id: id_match.as_str().to_string(),
            published_at,
        });
    }

    Ok(entries)
}

fn parse_osv_list_timestamp(value: &str) -> Result<OffsetDateTime, Error> {
    let mut iso = value.replacen(' ', "T", 1);
    if let Some(time_start) = iso.find('T') {
        if let Some(offset_start) = iso[time_start + 1..].rfind(['+', '-']) {
            let offset_start = time_start + 1 + offset_start;
            let digits = &iso.as_bytes()[offset_start + 1..];
            if digits.iter().all(u8::is_ascii_digit) {
                match digits.len() {
                    2 => iso.push_str(":00"),
                    4 => iso.insert(offset_start + 3, ':'),
                    _ => {}
                }
            }
        }
    }
    timestamps::parse(&iso).map_err(|cause| {
        Error::App(format!(
            "failed to parse OSV list timestamp '{value}': {cause}"
        ))
    })
}

async fn fetch_github(
    token: Option<String>,
    since: OffsetDateTime,
    fetched_at: &str,
) -> Result<Vec<RawIntelItem>, Error> {
    let client = GithubApiClient::new(token)?;
    let mut advisories = client
        .get_json::<Vec<GithubAdvisory>>(
            "https://api.github.com/advisories?per_page=100&sort=published&direction=desc&type=reviewed",
        )
        .await?;

    advisories.retain(|advisory| {
        parse_timestamp(advisory.published_at.as_deref()).is_some_and(|value| value >= since)
    });
    advisories.truncate(MAX_SOURCE_ITEMS);

    advisories
        .into_iter()
        .map(|advisory| normalize_github(advisory, fetched_at))
        .collect()
}

async fn fetch_nvd(
    client: &Client,
    since: OffsetDateTime,
    fetched_at: &str,
) -> Result<Vec<RawIntelItem>, Error> {
    let now = OffsetDateTime::now_utc();
    let context = "NVD vulnerabilities";
    let response = external_response(
        client
            .get("https://services.nvd.nist.gov/rest/json/cves/2.0")
            .query(&[
                ("pubStartDate", format_nvd_timestamp(since)),
                ("pubEndDate", format_nvd_timestamp(now)),
                ("resultsPerPage", "100".to_string()),
            ]),
        context,
    )
    .await?;
    let payload = external_json::<NvdResponse>(response, context).await?;

    payload
        .vulnerabilities
        .into_iter()
        .take(MAX_SOURCE_ITEMS)
        .map(|item| normalize_nvd(item, fetched_at))
        .collect()
}

fn normalize_osv(detail: OsvAdvisory, fetched_at: &str) -> Result<Option<RawIntelItem>, Error> {
    let raw_json = serde_json::to_string(&detail)?;
    let package = detail
        .affected
        .as_ref()
        .and_then(|affected| affected.first())
        .and_then(|affected| affected.package.as_ref());
    let ecosystem = package
        .and_then(|package| package.ecosystem.as_deref())
        .and_then(normalize_external_ecosystem);
    let package_name = package.and_then(|package| package.name.clone());

    let mut aliases = BTreeSet::new();
    aliases.insert(normalize_alias(&detail.id));
    if let Some(extra_aliases) = detail.aliases.as_ref() {
        for alias in extra_aliases {
            aliases.insert(normalize_alias(alias));
        }
    }
    if let Some(related) = detail.related.as_ref() {
        for alias in related {
            aliases.insert(normalize_alias(alias));
        }
    }
    if let Some(upstream) = detail.upstream.as_ref() {
        for alias in upstream {
            aliases.insert(normalize_alias(alias));
        }
    }

    let top_level_severity = detail
        .severity
        .as_ref()
        .and_then(|items| items.first())
        .map(|entry| parse_external_severity_text(entry.score.as_str()));
    let source_severity = detail
        .affected
        .as_ref()
        .and_then(|affected| {
            affected.iter().find_map(|entry| {
                entry
                    .ecosystem_specific
                    .as_ref()
                    .and_then(|specific| specific.severity.as_deref())
            })
        })
        .map(parse_external_severity_text)
        .or(top_level_severity)
        .unwrap_or(ExternalIntelSeverity::Unknown);
    let severity = if source_severity == ExternalIntelSeverity::Unknown
        && is_malware_advisory(
            &detail.id,
            detail.summary.as_deref(),
            detail.details.as_deref(),
        ) {
        ExternalIntelSeverity::Malware
    } else {
        source_severity
    };

    let references = detail
        .references
        .as_ref()
        .map(|references| {
            references
                .iter()
                .filter_map(|reference| {
                    Some(AdvisoryReference {
                        url: reference.url.clone()?,
                        source: Some("OSV".to_string()),
                        kind: reference.kind.clone(),
                        tags: Vec::new(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let url = references
        .first()
        .map(|reference| reference.url.clone())
        .or_else(|| {
            detail
                .references
                .as_ref()
                .and_then(|references| references.first())
                .and_then(|reference| reference.url.clone())
        })
        .unwrap_or_else(|| format!("https://osv.dev/vulnerability/{}", detail.id));
    let affected_packages = detail
        .affected
        .as_ref()
        .map(|affected| {
            affected
                .iter()
                .map(|affected| AdvisoryPackageImpact {
                    ecosystem: affected
                        .package
                        .as_ref()
                        .and_then(|package| package.ecosystem.as_deref())
                        .and_then(normalize_external_ecosystem),
                    package_name: affected
                        .package
                        .as_ref()
                        .and_then(|package| package.name.clone()),
                    package_purl: affected
                        .package
                        .as_ref()
                        .and_then(|package| package.purl.clone()),
                    vulnerable_version_range: affected
                        .ranges
                        .as_ref()
                        .and_then(|ranges| summarize_osv_ranges(ranges)),
                    first_patched_version: affected
                        .ranges
                        .as_ref()
                        .and_then(|ranges| first_osv_fixed_version(ranges)),
                    affected_versions_count: affected
                        .versions
                        .as_ref()
                        .map(|versions| versions.len() as u32)
                        .unwrap_or(0),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(Some(RawIntelItem {
        source: ExternalSource::Osv,
        source_item_id: detail.id.clone(),
        title: detail.summary.clone(),
        summary: detail.details.clone().or(detail.summary.clone()),
        url: Some(url),
        ecosystem,
        package_name,
        product_keys: BTreeSet::new(),
        topic_keys: BTreeSet::new(),
        severity,
        published_at: detail.published,
        updated_at: detail.modified.clone(),
        modified_at: detail.modified,
        withdrawn_at: detail.withdrawn,
        cvss_score: None,
        cvss_vector: None,
        cvss_version: None,
        epss_percentage: None,
        epss_percentile: None,
        is_kev: false,
        is_ransomware_known: false,
        fetched_at: fetched_at.to_string(),
        raw_json,
        aliases,
        affected_packages,
        references,
        cwes: Vec::new(),
        kev: None,
    }))
}

fn normalize_github(advisory: GithubAdvisory, fetched_at: &str) -> Result<RawIntelItem, Error> {
    let raw_json = serde_json::to_string(&advisory)?;
    let vulnerability = advisory
        .vulnerabilities
        .as_ref()
        .and_then(|items| items.first());
    let ecosystem = vulnerability
        .and_then(|item| item.package.as_ref())
        .and_then(|package| package.ecosystem.as_deref())
        .and_then(normalize_external_ecosystem);
    let package_name = vulnerability
        .and_then(|item| item.package.as_ref())
        .and_then(|package| package.name.clone());

    let mut aliases = BTreeSet::new();
    aliases.insert(normalize_alias(&advisory.ghsa_id));
    if let Some(cve_id) = advisory.cve_id.as_ref() {
        aliases.insert(normalize_alias(cve_id));
    }
    if let Some(identifiers) = advisory.identifiers.as_ref() {
        for identifier in identifiers {
            aliases.insert(normalize_alias(&identifier.value));
        }
    }
    let (cvss_score, cvss_vector, cvss_version) = advisory
        .preferred_cvss()
        .map(|(version, cvss)| {
            (
                cvss.score.map(|score| score.to_string()),
                cvss.vector_string.clone(),
                Some(version.to_string()),
            )
        })
        .unwrap_or((None, None, None));
    let references = advisory
        .references
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|url| AdvisoryReference {
            url,
            source: Some("GitHub".to_string()),
            kind: None,
            tags: Vec::new(),
        })
        .collect::<Vec<_>>();
    let affected_packages = advisory
        .vulnerabilities
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|vulnerability| AdvisoryPackageImpact {
            ecosystem: vulnerability
                .package
                .as_ref()
                .and_then(|package| package.ecosystem.as_deref())
                .and_then(normalize_external_ecosystem),
            package_name: vulnerability
                .package
                .as_ref()
                .and_then(|package| package.name.clone()),
            package_purl: None,
            vulnerable_version_range: vulnerability.vulnerable_version_range,
            first_patched_version: vulnerability.first_patched_version,
            affected_versions_count: 0,
        })
        .collect::<Vec<_>>();

    Ok(RawIntelItem {
        source: ExternalSource::Github,
        source_item_id: advisory.ghsa_id.clone(),
        title: advisory.summary.clone(),
        summary: advisory.description.clone().or(advisory.summary.clone()),
        url: advisory.html_url.clone().or(advisory.url.clone()),
        ecosystem,
        package_name,
        product_keys: BTreeSet::new(),
        topic_keys: BTreeSet::new(),
        severity: advisory
            .severity
            .as_deref()
            .map(parse_external_severity_text)
            .unwrap_or(ExternalIntelSeverity::Unknown),
        published_at: advisory.published_at,
        updated_at: advisory.updated_at,
        modified_at: None,
        withdrawn_at: advisory.withdrawn_at,
        cvss_score,
        cvss_vector,
        cvss_version,
        epss_percentage: advisory
            .epss
            .as_ref()
            .and_then(|epss| epss.percentage.map(|value| value.to_string())),
        epss_percentile: advisory
            .epss
            .as_ref()
            .and_then(|epss| epss.percentile.as_ref().map(ToString::to_string)),
        is_kev: false,
        is_ransomware_known: false,
        fetched_at: fetched_at.to_string(),
        raw_json,
        aliases,
        affected_packages,
        references,
        cwes: advisory
            .cwes
            .unwrap_or_default()
            .into_iter()
            .filter_map(|cwe| {
                Some(AdvisoryCwe {
                    cwe_id: cwe.cwe_id?,
                    name: cwe.name.unwrap_or_default(),
                })
            })
            .collect(),
        kev: None,
    })
}

fn normalize_nvd(item: NvdVulnerability, fetched_at: &str) -> Result<RawIntelItem, Error> {
    let raw_json = serde_json::to_string(&item)?;
    let cve = item.cve;
    let cve_id = cve.id.clone();
    let description = cve
        .descriptions
        .as_ref()
        .and_then(|descriptions| descriptions.iter().find(|entry| entry.lang == "en"))
        .map(|entry| entry.value.clone())
        .or_else(|| {
            cve.descriptions
                .as_ref()
                .and_then(|entries| entries.first())
                .map(|entry| entry.value.clone())
        });

    let cpe_criteria = cve
        .configurations
        .as_ref()
        .map(|configurations| extract_cpe_criteria(configurations))
        .unwrap_or_default();
    let package_name = cpe_criteria
        .first()
        .and_then(|criteria| parse_cpe_product_name(criteria));
    let product_keys = cpe_criteria
        .iter()
        .filter_map(|criteria| parse_cpe_product_key(criteria))
        .collect::<BTreeSet<_>>();
    let topic_keys = product_keys
        .iter()
        .filter_map(|key| product_key_to_topic(key))
        .collect::<BTreeSet<_>>();

    let severity = cve
        .metrics
        .as_ref()
        .and_then(nvd_metrics_severity)
        .unwrap_or(ExternalIntelSeverity::Unknown);
    let (cvss_score, cvss_vector, cvss_version) = cve
        .metrics
        .as_ref()
        .and_then(nvd_preferred_cvss)
        .unwrap_or((None, None, None));

    let url = format!("https://nvd.nist.gov/vuln/detail/{cve_id}");
    let mut aliases = BTreeSet::new();
    aliases.insert(normalize_alias(&cve_id));
    let references = cve
        .references
        .as_ref()
        .map(|references| {
            references
                .reference_data
                .iter()
                .filter_map(|reference| {
                    Some(AdvisoryReference {
                        url: reference.url.clone()?,
                        source: reference.source.clone(),
                        kind: None,
                        tags: reference.tags.clone().unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let cwes = cve
        .weaknesses
        .as_ref()
        .map(|weaknesses| {
            weaknesses
                .iter()
                .flat_map(|weakness| weakness.description.iter())
                .filter(|description| description.lang == "en")
                .map(|description| AdvisoryCwe {
                    cwe_id: description.value.clone(),
                    name: String::new(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let affected_packages = cpe_criteria
        .iter()
        .map(|criteria| AdvisoryPackageImpact {
            ecosystem: None,
            package_name: parse_cpe_product_name(criteria),
            package_purl: None,
            vulnerable_version_range: Some(criteria.clone()),
            first_patched_version: None,
            affected_versions_count: 0,
        })
        .collect::<Vec<_>>();

    Ok(RawIntelItem {
        source: ExternalSource::Nvd,
        source_item_id: cve_id.clone(),
        title: Some(cve_id.clone()),
        summary: description,
        url: Some(url),
        ecosystem: None,
        package_name,
        product_keys,
        topic_keys,
        severity,
        published_at: cve.published,
        updated_at: cve.last_modified.clone(),
        modified_at: cve.last_modified,
        withdrawn_at: None,
        cvss_score,
        cvss_vector,
        cvss_version,
        epss_percentage: None,
        epss_percentile: None,
        is_kev: false,
        is_ransomware_known: false,
        fetched_at: fetched_at.to_string(),
        raw_json,
        aliases,
        affected_packages,
        references,
        cwes,
        kev: None,
    })
}

fn merge_items(items: Vec<RawIntelItem>) -> Vec<MergedIntelItem> {
    let mut groups: Vec<MergedIntelItem> = Vec::new();

    for item in items {
        let mut matching_indices = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            if should_merge(group, &item) {
                matching_indices.push(index);
            }
        }

        if matching_indices.is_empty() {
            groups.push(from_raw(item));
            continue;
        }

        let first_index = matching_indices[0];
        merge_raw_into_group(&mut groups[first_index], item);

        for extra_index in matching_indices.into_iter().skip(1).rev() {
            let extra = groups.remove(extra_index);
            merge_group_into_group(&mut groups[first_index], extra);
        }
    }

    groups
}

fn to_persisted_item(
    item: MergedIntelItem,
    local_index: &LocalPackageIndex,
) -> PersistedExternalIntelItem {
    let canonical_key = if item.aliases.is_empty() {
        fallback_canonical_key(&item)
    } else {
        item.aliases.iter().cloned().collect::<Vec<_>>().join("|")
    };
    let product_keys = item.product_keys.into_iter().collect::<Vec<_>>();
    let affected_packages = dedupe_affected_packages(item.affected_packages);
    let (match_kind, repo_count) = local_index.match_advisory(
        &advisory_package_keys(
            item.ecosystem.as_deref(),
            item.package_name.as_deref(),
            &affected_packages,
        ),
        &product_keys,
        item.ecosystem.as_deref(),
    );
    let relevance = ExternalIntelMatchKind::relevance(match_kind.as_ref());
    let aliases = item.aliases.into_iter().collect::<Vec<_>>();
    let sources = item.sources.into_iter().collect::<Vec<_>>();
    let topic_keys = item.topic_keys.into_iter().collect::<Vec<_>>();
    let references = dedupe_references(item.references);
    let cwes = dedupe_cwes(item.cwes);
    let source_details = dedupe_source_details(item.source_details);
    let url = url_policy::external_url(&item.url)
        .or_else(|| references.first().map(|reference| reference.url.clone()))
        .unwrap_or_else(|| fallback_url_for_source(&item.primary_source, &item.source_item_id));

    PersistedExternalIntelItem {
        canonical_key,
        primary_source: item.primary_source,
        source_item_id: item.source_item_id,
        title: item.title,
        summary: item.summary,
        url,
        ecosystem: item.ecosystem,
        package_name: item.package_name,
        product_keys,
        topic_keys,
        severity: item.severity,
        published_at: item.published_at,
        updated_at: item.updated_at,
        modified_at: item.modified_at,
        withdrawn_at: item.withdrawn_at,
        cvss_score: item.cvss_score,
        cvss_vector: item.cvss_vector,
        cvss_version: item.cvss_version,
        epss_percentage: item.epss_percentage,
        epss_percentile: item.epss_percentile,
        is_kev: item.is_kev,
        is_ransomware_known: item.is_ransomware_known,
        fetched_at: item.fetched_at,
        raw_json: item.raw_json,
        sources,
        source_details,
        aliases,
        affected_packages,
        references,
        cwes,
        kev: item.kev,
        match_kind,
        repo_count: repo_count as u32,
        relevance,
    }
}

fn item_matches_watched_topics(
    item: &MergedIntelItem,
    watched_topics: &std::collections::BTreeSet<String>,
) -> bool {
    item.ecosystem
        .as_ref()
        .is_some_and(|ecosystem| watched_topics.contains(ecosystem))
        || item.affected_packages.iter().any(|package| {
            package
                .ecosystem
                .as_ref()
                .is_some_and(|ecosystem| watched_topics.contains(ecosystem))
        })
        || item
            .topic_keys
            .iter()
            .any(|topic_key| watched_topics.contains(topic_key))
        || (is_high_interest_external_severity(item.severity)
            && (!item.product_keys.is_empty()
                || item.sources.contains(&ExternalSource::CisaKev)
                || item.sources.contains(&ExternalSource::Vendor)))
}

fn should_merge(group: &MergedIntelItem, item: &RawIntelItem) -> bool {
    if !group.aliases.is_disjoint(&item.aliases) {
        return true;
    }

    (has_package_or_ecosystem(group, item)
        && group.package_name == item.package_name
        && group.ecosystem == item.ecosystem
        && published_date_key(group.published_at.as_deref())
            == published_date_key(item.published_at.as_deref()))
        || (!group.product_keys.is_disjoint(&item.product_keys)
            && published_date_key(group.published_at.as_deref())
                == published_date_key(item.published_at.as_deref()))
}

fn has_package_or_ecosystem(group: &MergedIntelItem, item: &RawIntelItem) -> bool {
    group.package_name.is_some()
        || item.package_name.is_some()
        || group.ecosystem.is_some()
        || item.ecosystem.is_some()
}

fn from_raw(item: RawIntelItem) -> MergedIntelItem {
    let title = item
        .title
        .clone()
        .or_else(|| item.package_name.clone())
        .unwrap_or_else(|| item.source_item_id.clone());
    let summary = item
        .summary
        .clone()
        .or_else(|| item.title.clone())
        .unwrap_or_else(|| title.clone());
    let references = dedupe_references(item.references.clone());
    let url = item
        .url
        .as_deref()
        .and_then(url_policy::external_url)
        .or_else(|| references.first().map(|reference| reference.url.clone()))
        .unwrap_or_else(|| fallback_url_for_raw(&item));
    let primary_source = item.source.clone();
    let source_item_id = item.source_item_id.clone();
    let ecosystem = item.ecosystem.clone();
    let package_name = item.package_name.clone();
    let product_keys = item.product_keys.clone();
    let topic_keys = item.topic_keys.clone();
    let severity = item.severity;
    let published_at = item.published_at.clone();
    let updated_at = item.updated_at.clone();
    let modified_at = item.modified_at.clone();
    let withdrawn_at = item.withdrawn_at.clone();
    let cvss_score = item.cvss_score.clone();
    let cvss_vector = item.cvss_vector.clone();
    let cvss_version = item.cvss_version.clone();
    let epss_percentage = item.epss_percentage.clone();
    let epss_percentile = item.epss_percentile.clone();
    let is_kev = item.is_kev;
    let is_ransomware_known = item.is_ransomware_known;
    let fetched_at = item.fetched_at.clone();
    let raw_json = item.raw_json.clone();
    let aliases = item.aliases.clone();
    let source_detail = source_detail_from_raw(&item);
    let affected_packages = item.affected_packages.clone();
    let cwes = item.cwes.clone();
    let kev = item.kev.clone();
    let mut sources = BTreeSet::new();
    sources.insert(item.source);

    MergedIntelItem {
        primary_source,
        source_item_id,
        title,
        summary,
        url,
        ecosystem,
        package_name,
        product_keys,
        topic_keys,
        severity,
        published_at,
        updated_at,
        modified_at,
        withdrawn_at,
        cvss_score,
        cvss_vector,
        cvss_version,
        epss_percentage,
        epss_percentile,
        is_kev,
        is_ransomware_known,
        fetched_at,
        raw_json,
        aliases,
        sources,
        source_details: vec![source_detail],
        affected_packages,
        references,
        cwes,
        kev,
    }
}

fn merge_raw_into_group(group: &mut MergedIntelItem, item: RawIntelItem) {
    group.severity = max_severity(group.severity, item.severity);
    group.published_at = earliest_timestamp(group.published_at.take(), item.published_at.clone());
    group.updated_at = latest_timestamp(group.updated_at.take(), item.updated_at.clone());
    group.modified_at = latest_timestamp(group.modified_at.take(), item.modified_at.clone());
    group.withdrawn_at = group.withdrawn_at.take().or(item.withdrawn_at.clone());
    group.cvss_score = prefer_score(group.cvss_score.take(), item.cvss_score.clone());
    if group.cvss_vector.is_none() {
        group.cvss_vector = item.cvss_vector.clone();
    }
    if group.cvss_version.is_none() {
        group.cvss_version = item.cvss_version.clone();
    }
    group.epss_percentage =
        prefer_score(group.epss_percentage.take(), item.epss_percentage.clone());
    if group.epss_percentile.is_none() {
        group.epss_percentile = item.epss_percentile.clone();
    }
    group.is_kev = group.is_kev || item.is_kev;
    group.is_ransomware_known = group.is_ransomware_known || item.is_ransomware_known;
    group.fetched_at = item.fetched_at.clone();
    group.aliases.extend(item.aliases.iter().cloned());
    group.product_keys.extend(item.product_keys.iter().cloned());
    group.topic_keys.extend(item.topic_keys.iter().cloned());
    group.sources.insert(item.source.clone());
    group.source_details.push(source_detail_from_raw(&item));
    group
        .affected_packages
        .extend(item.affected_packages.iter().cloned());
    group.references.extend(item.references.iter().cloned());
    group.cwes.extend(item.cwes.iter().cloned());
    if group.kev.is_none() {
        group.kev = item.kev.clone();
    }

    if group.package_name.is_none()
        || source_rank(&item.source) > source_rank(&group.primary_source)
    {
        if item.package_name.is_some() {
            group.package_name = item.package_name.clone();
        }
        if item.ecosystem.is_some() {
            group.ecosystem = item.ecosystem.clone();
        }
    }

    if is_better_for_text(
        &item.source,
        &group.primary_source,
        item.title.as_deref(),
        Some(&group.title),
    ) {
        if let Some(title) = item.title.clone() {
            group.title = title;
            group.primary_source = item.source.clone();
            group.source_item_id = item.source_item_id.clone();
            group.raw_json = item.raw_json.clone();
        }
    }

    if is_better_for_text(
        &item.source,
        &group.primary_source,
        item.summary.as_deref(),
        Some(&group.summary),
    ) {
        if let Some(summary) = item.summary.clone() {
            group.summary = summary;
        }
    }

    let sanitized_url = item.url.as_deref().and_then(url_policy::external_url);
    if is_better_for_text(
        &item.source,
        &group.primary_source,
        sanitized_url.as_deref(),
        Some(&group.url),
    ) {
        if let Some(url) = sanitized_url {
            group.url = url;
        }
    }
}

fn merge_group_into_group(target: &mut MergedIntelItem, incoming: MergedIntelItem) {
    target.severity = max_severity(target.severity, incoming.severity);
    target.published_at = earliest_timestamp(target.published_at.take(), incoming.published_at);
    target.updated_at = latest_timestamp(target.updated_at.take(), incoming.updated_at);
    target.modified_at = latest_timestamp(target.modified_at.take(), incoming.modified_at);
    target.withdrawn_at = target.withdrawn_at.take().or(incoming.withdrawn_at);
    target.cvss_score = prefer_score(target.cvss_score.take(), incoming.cvss_score);
    target.cvss_vector = target.cvss_vector.take().or(incoming.cvss_vector);
    target.cvss_version = target.cvss_version.take().or(incoming.cvss_version);
    target.epss_percentage = prefer_score(target.epss_percentage.take(), incoming.epss_percentage);
    target.epss_percentile = target.epss_percentile.take().or(incoming.epss_percentile);
    target.is_kev = target.is_kev || incoming.is_kev;
    target.is_ransomware_known = target.is_ransomware_known || incoming.is_ransomware_known;
    target.aliases.extend(incoming.aliases);
    target.product_keys.extend(incoming.product_keys);
    target.topic_keys.extend(incoming.topic_keys);
    target.sources.extend(incoming.sources);
    target.source_details.extend(incoming.source_details);
    target.affected_packages.extend(incoming.affected_packages);
    target.references.extend(incoming.references);
    target.cwes.extend(incoming.cwes);
    if target.kev.is_none() {
        target.kev = incoming.kev;
    }

    if source_rank(&incoming.primary_source) > source_rank(&target.primary_source) {
        target.primary_source = incoming.primary_source;
        target.source_item_id = incoming.source_item_id;
        target.raw_json = incoming.raw_json;
        target.title = incoming.title;
        target.summary = incoming.summary;
        target.url = incoming.url;
        if incoming.package_name.is_some() {
            target.package_name = incoming.package_name;
        }
        if incoming.ecosystem.is_some() {
            target.ecosystem = incoming.ecosystem;
        }
    }
}

fn source_rank(source: &ExternalSource) -> u8 {
    match source {
        ExternalSource::Github => 3,
        ExternalSource::Osv => 2,
        ExternalSource::Nvd => 1,
        ExternalSource::CisaKev => 4,
        ExternalSource::Vendor => 2,
    }
}

fn repo_count_rank(repo_count: u32) -> u32 {
    if repo_count > 0 {
        1
    } else {
        0
    }
}

fn external_published_sort_key(value: Option<&str>) -> Option<OffsetDateTime> {
    parse_timestamp(value)
}

fn max_severity(
    left: ExternalIntelSeverity,
    right: ExternalIntelSeverity,
) -> ExternalIntelSeverity {
    if right.rank() > left.rank() {
        right
    } else {
        left
    }
}

fn earliest_timestamp(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left_dt = parse_timestamp(Some(left.as_str()));
            let right_dt = parse_timestamp(Some(right.as_str()));
            match (left_dt, right_dt) {
                (Some(left_dt), Some(right_dt)) => {
                    if right_dt < left_dt {
                        Some(right)
                    } else {
                        Some(left)
                    }
                }
                _ => Some(left),
            }
        }
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn latest_timestamp(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left_dt = parse_timestamp(Some(left.as_str()));
            let right_dt = parse_timestamp(Some(right.as_str()));
            match (left_dt, right_dt) {
                (Some(left_dt), Some(right_dt)) => {
                    if right_dt > left_dt {
                        Some(right)
                    } else {
                        Some(left)
                    }
                }
                _ => Some(left),
            }
        }
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn prefer_score(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left_score = left.parse::<f32>().ok();
            let right_score = right.parse::<f32>().ok();
            match (left_score, right_score) {
                (Some(left_score), Some(right_score)) if right_score > left_score => Some(right),
                _ => Some(left),
            }
        }
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn source_detail_from_raw(item: &RawIntelItem) -> ExternalIntelSourceDetail {
    let references = dedupe_references(item.references.clone());
    let url = item
        .url
        .as_deref()
        .and_then(url_policy::external_url)
        .or_else(|| references.first().map(|reference| reference.url.clone()))
        .unwrap_or_else(|| fallback_url_for_raw(item));

    ExternalIntelSourceDetail {
        source: item.source.clone(),
        source_item_id: item.source_item_id.clone(),
        url,
        published_at: item.published_at.clone(),
        updated_at: item.updated_at.clone().or(item.modified_at.clone()),
        severity: item.severity,
        cvss_score: item.cvss_score.clone(),
        epss_percentage: item.epss_percentage.clone(),
        reference_count: references.len() as u32,
        raw_json: item.raw_json.clone(),
    }
}

fn dedupe_source_details(items: Vec<ExternalIntelSourceDetail>) -> Vec<ExternalIntelSourceDetail> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert((item.source.clone(), item.source_item_id.clone())))
        .collect()
}

fn dedupe_affected_packages(items: Vec<AdvisoryPackageImpact>) -> Vec<AdvisoryPackageImpact> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter(|item| {
            seen.insert((
                item.ecosystem.clone(),
                item.package_name.clone(),
                item.package_purl.clone(),
                item.vulnerable_version_range.clone(),
            ))
        })
        .collect()
}

fn dedupe_references(items: Vec<AdvisoryReference>) -> Vec<AdvisoryReference> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter_map(|mut item| {
            item.url = url_policy::external_url(&item.url)?;
            Some(item)
        })
        .filter(|item| seen.insert(item.url.clone()))
        .collect()
}

fn dedupe_cwes(items: Vec<AdvisoryCwe>) -> Vec<AdvisoryCwe> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.cwe_id.clone()))
        .collect()
}

fn fallback_url_for_raw(item: &RawIntelItem) -> String {
    fallback_url_for_source(&item.source, &item.source_item_id)
}

fn fallback_url_for_source(source: &ExternalSource, source_item_id: &str) -> String {
    match source {
        ExternalSource::Osv if is_safe_url_token(source_item_id) => {
            format!("https://osv.dev/vulnerability/{source_item_id}")
        }
        ExternalSource::Osv => "https://osv.dev/list".to_string(),
        ExternalSource::Github if is_safe_ghsa_id(source_item_id) => {
            format!("https://github.com/advisories/{source_item_id}")
        }
        ExternalSource::Github => "https://github.com/advisories".to_string(),
        ExternalSource::Nvd if is_safe_cve_id(source_item_id) => {
            format!("https://nvd.nist.gov/vuln/detail/{source_item_id}")
        }
        ExternalSource::Nvd => "https://nvd.nist.gov/vuln".to_string(),
        ExternalSource::CisaKev => {
            "https://www.cisa.gov/known-exploited-vulnerabilities-catalog".to_string()
        }
        ExternalSource::Vendor => fallback_vendor_url(source_item_id),
    }
}

fn fallback_vendor_url(source_item_id: &str) -> String {
    match source_item_id
        .strip_prefix("vendor:")
        .and_then(|value| value.split(':').next())
    {
        Some("github") => "https://github.blog/security/".to_string(),
        Some("gitlab") => "https://about.gitlab.com/releases/".to_string(),
        Some("kubernetes") => "https://kubernetes.io/blog/".to_string(),
        Some("docker") => "https://www.docker.com/blog/".to_string(),
        _ => "https://www.cisa.gov/known-exploited-vulnerabilities-catalog".to_string(),
    }
}

fn is_safe_ghsa_id(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    parts.len() == 4
        && parts[0].eq_ignore_ascii_case("GHSA")
        && parts[1..].iter().all(|part| is_safe_url_token(part))
}

fn is_safe_cve_id(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0].eq_ignore_ascii_case("CVE")
        && parts[1..]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_safe_url_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_better_for_text(
    candidate_source: &ExternalSource,
    current_source: &ExternalSource,
    candidate: Option<&str>,
    current: Option<&str>,
) -> bool {
    match candidate {
        Some(candidate) if !candidate.trim().is_empty() => {
            current.map(|value| value.trim().is_empty()).unwrap_or(true)
                || source_rank(candidate_source) > source_rank(current_source)
        }
        _ => false,
    }
}

fn fallback_canonical_key(item: &MergedIntelItem) -> String {
    if !item.source_item_id.trim().is_empty() {
        return format!(
            "{}:{}",
            external_source_key(&item.primary_source),
            item.source_item_id
        );
    }

    if !item.url.trim().is_empty() {
        return format!("url:{}", item.url);
    }

    format!(
        "{}:{}:{}:{}",
        external_source_key(&item.primary_source),
        item.product_keys
            .iter()
            .next()
            .map(String::as_str)
            .unwrap_or("unknown"),
        item.package_name.as_deref().unwrap_or("unknown"),
        published_date_key(item.published_at.as_deref()).unwrap_or_else(|| "unknown".to_string())
    )
}

fn external_source_key(source: &ExternalSource) -> &'static str {
    match source {
        ExternalSource::Osv => "osv",
        ExternalSource::Github => "github",
        ExternalSource::Nvd => "nvd",
        ExternalSource::CisaKev => "cisa_kev",
        ExternalSource::Vendor => "vendor",
    }
}

fn published_date_key(value: Option<&str>) -> Option<String> {
    parse_timestamp(value).map(|date| date.date().to_string())
}

fn parse_timestamp(value: Option<&str>) -> Option<OffsetDateTime> {
    value.and_then(|entry| timestamps::parse(entry).ok())
}

fn format_nvd_timestamp(value: OffsetDateTime) -> String {
    value
        .to_offset(time::UtcOffset::UTC)
        .format(&time::macros::format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        ))
        .expect("UTC NVD timestamp format")
}

fn normalize_alias(value: &str) -> String {
    value.trim().to_uppercase()
}

fn parse_external_severity_text(value: &str) -> ExternalIntelSeverity {
    match value.to_lowercase().as_str() {
        "malware" | "malicious" => ExternalIntelSeverity::Malware,
        "critical" => ExternalIntelSeverity::Critical,
        "high" => ExternalIntelSeverity::High,
        "medium" | "moderate" => ExternalIntelSeverity::Medium,
        "low" => ExternalIntelSeverity::Low,
        _ => ExternalIntelSeverity::Unknown,
    }
}

fn is_high_interest_external_severity(severity: ExternalIntelSeverity) -> bool {
    matches!(
        severity,
        ExternalIntelSeverity::Malware
            | ExternalIntelSeverity::Critical
            | ExternalIntelSeverity::High
    )
}

fn is_malware_advisory(id: &str, title: Option<&str>, summary: Option<&str>) -> bool {
    id.to_ascii_uppercase().starts_with("MAL-")
        || title.is_some_and(|value| value.to_lowercase().contains("malicious code"))
        || summary.is_some_and(|value| value.to_lowercase().contains("malicious code"))
}

fn is_positive_ransomware_signal(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "known" | "yes" | "true"
    )
}

fn summarize_osv_ranges(ranges: &[OsvRange]) -> Option<String> {
    let parts = ranges
        .iter()
        .flat_map(|range| range.events.iter().flatten())
        .filter_map(|event| {
            if let Some(introduced) = event.introduced.as_ref() {
                Some(format!("introduced {introduced}"))
            } else if let Some(fixed) = event.fixed.as_ref() {
                Some(format!("fixed {fixed}"))
            } else if let Some(last_affected) = event.last_affected.as_ref() {
                Some(format!("last affected {last_affected}"))
            } else {
                event.limit.as_ref().map(|limit| format!("limit {limit}"))
            }
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn first_osv_fixed_version(ranges: &[OsvRange]) -> Option<String> {
    ranges
        .iter()
        .flat_map(|range| range.events.iter().flatten())
        .find_map(|event| event.fixed.clone())
}

fn extract_cpe_criteria(configurations: &[NvdConfiguration]) -> Vec<String> {
    let mut criteria = BTreeSet::new();
    for configuration in configurations {
        collect_node_cpes(configuration.nodes.as_deref(), &mut criteria);
    }
    criteria.into_iter().collect()
}

fn collect_node_cpes(nodes: Option<&[NvdNode]>, criteria: &mut BTreeSet<String>) {
    let Some(nodes) = nodes else {
        return;
    };
    for node in nodes {
        if let Some(matches) = node.cpe_match.as_ref() {
            for cpe_match in matches {
                if !cpe_match.vulnerable.unwrap_or(false) {
                    continue;
                }
                if let Some(value) = cpe_match.criteria.as_ref() {
                    criteria.insert(value.clone());
                }
            }
        }
        collect_node_cpes(node.children.as_deref(), criteria);
    }
}

fn parse_cpe_product_name(value: &str) -> Option<String> {
    let mut parts = value.split(':');
    let _prefix = parts.next();
    let _version = parts.next();
    let _part = parts.next();
    let _vendor = parts.next();
    parts.next().map(|product| product.replace('_', "-"))
}

fn parse_cpe_product_key(value: &str) -> Option<String> {
    let mut parts = value.split(':');
    let _prefix = parts.next()?;
    let _version = parts.next()?;
    let _part = parts.next()?;
    let vendor = parts.next()?;
    let product = parts.next()?;
    Some(product_key(vendor, product))
}

fn collect_product_keys_from_text(value: &str, product_keys: &mut BTreeSet<String>) {
    if value.contains("github enterprise server") || value.contains("ghes") {
        product_keys.insert(product_key("github", "enterprise_server"));
    }
    if value.contains("github actions runner") || value.contains("actions runner") {
        product_keys.insert(product_key("github", "actions_runner"));
    }
    for (needle, vendor, product) in [
        ("gitlab", "gitlab", "gitlab"),
        ("jenkins", "jenkins", "jenkins"),
        ("kubernetes", "kubernetes", "kubernetes"),
        ("docker", "docker", "docker"),
        ("nginx", "nginx", "nginx"),
        ("postgres", "postgresql", "postgresql"),
        ("openssl", "openssl", "openssl"),
    ] {
        if value.contains(needle) {
            product_keys.insert(product_key(vendor, product));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VendorFeedItem {
    title: String,
    link: Option<String>,
    published_at: Option<OffsetDateTime>,
    updated_at: Option<OffsetDateTime>,
    entry_id: Option<String>,
    summary: Option<String>,
    categories: Vec<String>,
    feed_type: String,
}

fn parse_vendor_feed(xml: &str, source_url: &str) -> Result<Vec<VendorFeedItem>, Error> {
    let mut items = parse_rss_feed(xml)?;
    items.extend(parse_atom_feed(xml)?);
    if items.is_empty() && !xml.trim().is_empty() {
        eprintln!("External intel vendor feed had no parseable items: {source_url}");
    }
    Ok(items)
}

fn parse_rss_feed(xml: &str) -> Result<Vec<VendorFeedItem>, Error> {
    let item_pattern = Regex::new(r"(?s)<item\b[^>]*>(.*?)</item>")
        .map_err(|cause| Error::App(format!("failed to prepare RSS item parser: {cause}")))?;
    Ok(item_pattern
        .captures_iter(xml)
        .filter_map(|capture| {
            let body = capture.get(1)?.as_str();
            let title = extract_xml_text(body, "title")?;
            Some(VendorFeedItem {
                title,
                link: extract_xml_text(body, "link"),
                published_at: extract_xml_text(body, "pubDate")
                    .as_deref()
                    .and_then(parse_rfc2822_timestamp)
                    .or_else(|| {
                        extract_xml_text(body, "dc:date")
                            .as_deref()
                            .and_then(parse_flexible_timestamp)
                    }),
                updated_at: extract_xml_text(body, "updated")
                    .as_deref()
                    .and_then(parse_flexible_timestamp),
                entry_id: extract_xml_text(body, "guid"),
                summary: extract_xml_text(body, "description")
                    .or_else(|| extract_xml_text(body, "content:encoded")),
                categories: extract_xml_texts(body, "category"),
                feed_type: "rss".to_string(),
            })
        })
        .collect())
}

fn parse_atom_feed(xml: &str) -> Result<Vec<VendorFeedItem>, Error> {
    let entry_pattern = Regex::new(r"(?s)<entry\b[^>]*>(.*?)</entry>")
        .map_err(|cause| Error::App(format!("failed to prepare Atom entry parser: {cause}")))?;
    let link_pattern = Regex::new(r#"<link\b[^>]*href=["']([^"']+)["'][^>]*/?>"#)
        .map_err(|cause| Error::App(format!("failed to prepare Atom link parser: {cause}")))?;
    Ok(entry_pattern
        .captures_iter(xml)
        .filter_map(|capture| {
            let body = capture.get(1)?.as_str();
            let title = extract_xml_text(body, "title")?;
            let link = link_pattern
                .captures(body)
                .and_then(|link_capture| link_capture.get(1))
                .map(|value| html_unescape(value.as_str()))
                .or_else(|| extract_xml_text(body, "link"));
            let published_at = extract_xml_text(body, "updated")
                .or_else(|| extract_xml_text(body, "published"))
                .as_deref()
                .and_then(parse_flexible_timestamp);
            Some(VendorFeedItem {
                title,
                link,
                published_at,
                updated_at: extract_xml_text(body, "updated")
                    .as_deref()
                    .and_then(parse_flexible_timestamp),
                entry_id: extract_xml_text(body, "id"),
                summary: extract_xml_text(body, "summary")
                    .or_else(|| extract_xml_text(body, "content")),
                categories: extract_atom_categories(body)?,
                feed_type: "atom".to_string(),
            })
        })
        .collect())
}

fn extract_xml_text(xml: &str, tag: &str) -> Option<String> {
    let pattern = Regex::new(&format!(r"(?s)<{tag}\b[^>]*>(.*?)</{tag}>")).ok()?;
    let raw = pattern.captures(xml)?.get(1)?.as_str().trim();
    let raw = raw
        .strip_prefix("<![CDATA[")
        .and_then(|value| value.strip_suffix("]]>"))
        .unwrap_or(raw);
    let value = html_unescape(raw).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn extract_xml_texts(xml: &str, tag: &str) -> Vec<String> {
    let Ok(pattern) = Regex::new(&format!(r"(?s)<{tag}\b[^>]*>(.*?)</{tag}>")) else {
        return Vec::new();
    };
    pattern
        .captures_iter(xml)
        .filter_map(|capture| {
            let raw = capture.get(1)?.as_str().trim();
            let value = html_unescape(raw).trim().to_string();
            (!value.is_empty()).then_some(value)
        })
        .collect()
}

fn extract_atom_categories(xml: &str) -> Option<Vec<String>> {
    let pattern = Regex::new(r#"<category\b[^>]*term=["']([^"']+)["'][^>]*/?>"#).ok()?;
    Some(
        pattern
            .captures_iter(xml)
            .filter_map(|capture| capture.get(1).map(|value| html_unescape(value.as_str())))
            .collect(),
    )
}

fn looks_security_relevant(title: &str) -> bool {
    let lower = title.to_lowercase();
    lower.contains("security")
        || lower.contains("vulnerab")
        || lower.contains("cve-")
        || lower.contains("critical")
        || lower.contains("patch")
}

fn parse_date_as_timestamp(value: Option<&str>) -> Option<OffsetDateTime> {
    value
        .and_then(|entry| {
            time::Date::parse(
                entry,
                &time::macros::format_description!("[year]-[month]-[day]"),
            )
            .ok()
        })
        .map(|date| date.midnight().assume_utc())
}

fn parse_rfc2822_timestamp(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc2822)
        .ok()
        .map(|timestamp| timestamp.to_offset(time::UtcOffset::UTC))
}

fn parse_flexible_timestamp(value: &str) -> Option<OffsetDateTime> {
    parse_rfc2822_timestamp(value).or_else(|| timestamps::parse(value).ok())
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn normalize_slug(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-")
}

fn nvd_metrics_severity(metrics: &NvdMetrics) -> Option<ExternalIntelSeverity> {
    let v40 = metrics
        .cvss_metric_v40
        .as_ref()
        .and_then(|items| items.first())
        .and_then(|entry| entry.cvss_data.as_ref())
        .and_then(|entry| entry.base_severity.as_deref());
    let v31 = metrics
        .cvss_metric_v31
        .as_ref()
        .and_then(|items| items.first())
        .and_then(|entry| entry.cvss_data.as_ref())
        .and_then(|entry| entry.base_severity.as_deref());
    let v30 = metrics
        .cvss_metric_v30
        .as_ref()
        .and_then(|items| items.first())
        .and_then(|entry| entry.cvss_data.as_ref())
        .and_then(|entry| entry.base_severity.as_deref());

    v40.or(v31).or(v30).map(parse_external_severity_text)
}

fn nvd_preferred_cvss(
    metrics: &NvdMetrics,
) -> Option<(Option<String>, Option<String>, Option<String>)> {
    nvd_cvss_from_entries(metrics.cvss_metric_v40.as_ref(), "4.0")
        .or_else(|| nvd_cvss_from_entries(metrics.cvss_metric_v31.as_ref(), "3.1"))
        .or_else(|| nvd_cvss_from_entries(metrics.cvss_metric_v30.as_ref(), "3.0"))
        .or_else(|| nvd_cvss_from_entries(metrics.cvss_metric_v2.as_ref(), "2.0"))
}

fn nvd_cvss_from_entries(
    entries: Option<&Vec<NvdMetricEntry>>,
    version: &str,
) -> Option<(Option<String>, Option<String>, Option<String>)> {
    let cvss = entries?.first()?.cvss_data.as_ref()?;
    Some((
        cvss.base_score.map(|score| score.to_string()),
        cvss.vector_string.clone(),
        Some(version.to_string()),
    ))
}

#[derive(Debug, Clone)]
struct OsvListEntry {
    id: String,
    published_at: OffsetDateTime,
}

#[derive(Debug, Deserialize, Serialize)]
struct OsvAdvisory {
    id: String,
    summary: Option<String>,
    details: Option<String>,
    published: Option<String>,
    modified: Option<String>,
    withdrawn: Option<String>,
    aliases: Option<Vec<String>>,
    related: Option<Vec<String>>,
    upstream: Option<Vec<String>>,
    severity: Option<Vec<OsvSeverity>>,
    references: Option<Vec<OsvReference>>,
    affected: Option<Vec<OsvAffected>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OsvReference {
    #[serde(rename = "type")]
    kind: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OsvAffected {
    package: Option<OsvPackage>,
    ranges: Option<Vec<OsvRange>>,
    versions: Option<Vec<String>>,
    ecosystem_specific: Option<OsvEcosystemSpecific>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OsvPackage {
    name: Option<String>,
    ecosystem: Option<String>,
    purl: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OsvEcosystemSpecific {
    severity: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OsvSeverity {
    #[serde(rename = "type")]
    kind: Option<String>,
    score: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct OsvRange {
    #[serde(rename = "type")]
    kind: Option<String>,
    events: Option<Vec<OsvRangeEvent>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OsvRangeEvent {
    introduced: Option<String>,
    fixed: Option<String>,
    #[serde(rename = "last_affected")]
    last_affected: Option<String>,
    limit: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct GithubAdvisory {
    ghsa_id: String,
    cve_id: Option<String>,
    url: Option<String>,
    html_url: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    severity: Option<String>,
    identifiers: Option<Vec<GithubIdentifier>>,
    vulnerabilities: Option<Vec<GithubVulnerability>>,
    published_at: Option<String>,
    updated_at: Option<String>,
    github_reviewed_at: Option<String>,
    nvd_published_at: Option<String>,
    withdrawn_at: Option<String>,
    source_code_location: Option<String>,
    references: Option<Vec<String>>,
    cvss: Option<GithubCvss>,
    cvss_severities: Option<GithubCvssSeverities>,
    epss: Option<GithubEpss>,
    cwes: Option<Vec<GithubCwe>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct GithubIdentifier {
    value: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct GithubVulnerability {
    package: Option<GithubPackage>,
    vulnerable_version_range: Option<String>,
    first_patched_version: Option<String>,
    vulnerable_functions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct GithubPackage {
    ecosystem: Option<String>,
    name: Option<String>,
}

impl GithubAdvisory {
    fn preferred_cvss(&self) -> Option<(&'static str, &GithubCvss)> {
        self.cvss_severities
            .as_ref()
            .and_then(|severities| {
                severities
                    .cvss_v4
                    .as_ref()
                    .filter(|cvss| cvss.has_signal())
                    .map(|cvss| ("4.0", cvss))
                    .or_else(|| {
                        severities
                            .cvss_v3
                            .as_ref()
                            .filter(|cvss| cvss.has_signal())
                            .map(|cvss| ("3.1", cvss))
                    })
            })
            .or_else(|| {
                self.cvss
                    .as_ref()
                    .filter(|cvss| cvss.has_signal())
                    .map(|cvss| ("3.1", cvss))
            })
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
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

#[derive(Debug, Deserialize, Serialize, Clone)]
struct GithubCvssSeverities {
    cvss_v3: Option<GithubCvss>,
    cvss_v4: Option<GithubCvss>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct GithubEpss {
    percentage: Option<f32>,
    percentile: Option<GithubNumberOrString>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
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

#[derive(Debug, Deserialize, Serialize, Clone)]
struct GithubCwe {
    cwe_id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdResponse {
    vulnerabilities: Vec<NvdVulnerability>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdVulnerability {
    cve: NvdCve,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdCve {
    #[serde(rename = "id")]
    id: String,
    published: Option<String>,
    #[serde(rename = "lastModified")]
    last_modified: Option<String>,
    #[serde(rename = "vulnStatus")]
    vuln_status: Option<String>,
    descriptions: Option<Vec<NvdDescription>>,
    metrics: Option<NvdMetrics>,
    weaknesses: Option<Vec<NvdWeakness>>,
    references: Option<NvdReferences>,
    configurations: Option<Vec<NvdConfiguration>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdDescription {
    lang: String,
    value: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdMetrics {
    #[serde(rename = "cvssMetricV40")]
    cvss_metric_v40: Option<Vec<NvdMetricEntry>>,
    #[serde(rename = "cvssMetricV31")]
    cvss_metric_v31: Option<Vec<NvdMetricEntry>>,
    #[serde(rename = "cvssMetricV30")]
    cvss_metric_v30: Option<Vec<NvdMetricEntry>>,
    #[serde(rename = "cvssMetricV2")]
    cvss_metric_v2: Option<Vec<NvdMetricEntry>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdMetricEntry {
    #[serde(rename = "cvssData")]
    cvss_data: Option<NvdCvssData>,
    #[serde(rename = "exploitabilityScore")]
    exploitability_score: Option<f32>,
    #[serde(rename = "impactScore")]
    impact_score: Option<f32>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdCvssData {
    #[serde(rename = "baseSeverity")]
    base_severity: Option<String>,
    #[serde(rename = "baseScore")]
    base_score: Option<f32>,
    #[serde(rename = "vectorString")]
    vector_string: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdWeakness {
    description: Vec<NvdDescription>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdReferences {
    #[serde(rename = "referenceData")]
    reference_data: Vec<NvdReference>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdReference {
    url: Option<String>,
    source: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdConfiguration {
    nodes: Option<Vec<NvdNode>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdNode {
    #[serde(rename = "cpeMatch")]
    cpe_match: Option<Vec<NvdCpeMatch>>,
    children: Option<Vec<NvdNode>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvdCpeMatch {
    criteria: Option<String>,
    vulnerable: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CisaKevCatalog {
    #[serde(rename = "catalogVersion")]
    catalog_version: Option<String>,
    #[serde(rename = "dateReleased")]
    date_released: Option<String>,
    count: Option<u32>,
    vulnerabilities: Vec<CisaKevVulnerability>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CisaKevVulnerability {
    #[serde(rename = "cveID")]
    cve_id: String,
    #[serde(rename = "vendorProject")]
    vendor_project: Option<String>,
    product: Option<String>,
    #[serde(rename = "vulnerabilityName")]
    vulnerability_name: Option<String>,
    #[serde(rename = "shortDescription")]
    short_description: Option<String>,
    #[serde(rename = "dateAdded")]
    date_added: Option<String>,
    #[serde(rename = "dueDate")]
    due_date: Option<String>,
    #[serde(rename = "requiredAction")]
    required_action: Option<String>,
    #[serde(rename = "knownRansomwareCampaignUse")]
    known_ransomware_campaign_use: Option<String>,
    notes: Option<String>,
    cwes: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_item(
        source: ExternalSource,
        source_item_id: &str,
        aliases: &[&str],
        package_name: Option<&str>,
        ecosystem: Option<&str>,
        severity: ExternalIntelSeverity,
        published_at: &str,
    ) -> RawIntelItem {
        RawIntelItem {
            source,
            source_item_id: source_item_id.to_string(),
            title: Some(source_item_id.to_string()),
            summary: Some(format!("summary {source_item_id}")),
            url: Some(format!("https://example.com/{source_item_id}")),
            ecosystem: ecosystem.map(str::to_string),
            package_name: package_name.map(str::to_string),
            severity,
            published_at: Some(published_at.to_string()),
            fetched_at: "2026-01-01T00:00:00Z".to_string(),
            raw_json: "{}".to_string(),
            aliases: aliases.iter().map(|alias| normalize_alias(alias)).collect(),
            product_keys: BTreeSet::new(),
            topic_keys: BTreeSet::new(),
            updated_at: None,
            modified_at: None,
            withdrawn_at: None,
            cvss_score: None,
            cvss_vector: None,
            cvss_version: None,
            epss_percentage: None,
            epss_percentile: None,
            is_kev: false,
            is_ransomware_known: false,
            affected_packages: Vec::new(),
            references: Vec::new(),
            cwes: Vec::new(),
            kev: None,
        }
    }

    fn product_raw_item(
        source_item_id: &str,
        product_key_value: &str,
        published_at: &str,
    ) -> RawIntelItem {
        RawIntelItem {
            source: ExternalSource::Vendor,
            source_item_id: source_item_id.to_string(),
            title: Some(source_item_id.to_string()),
            summary: Some(format!("summary {source_item_id}")),
            url: Some(format!("https://example.com/{source_item_id}")),
            ecosystem: None,
            package_name: None,
            severity: ExternalIntelSeverity::High,
            published_at: Some(published_at.to_string()),
            fetched_at: "2026-01-01T00:00:00Z".to_string(),
            raw_json: "{}".to_string(),
            aliases: BTreeSet::new(),
            product_keys: BTreeSet::from([product_key_value.to_string()]),
            topic_keys: product_key_to_topic(product_key_value)
                .into_iter()
                .collect(),
            updated_at: None,
            modified_at: None,
            withdrawn_at: None,
            cvss_score: None,
            cvss_vector: None,
            cvss_version: None,
            epss_percentage: None,
            epss_percentile: None,
            is_kev: false,
            is_ransomware_known: false,
            affected_packages: Vec::new(),
            references: Vec::new(),
            cwes: Vec::new(),
            kev: None,
        }
    }

    fn merged_item_defaults() -> MergedIntelItem {
        MergedIntelItem {
            primary_source: ExternalSource::Github,
            source_item_id: "default".to_string(),
            title: "default".to_string(),
            summary: "summary".to_string(),
            url: "https://example.com".to_string(),
            ecosystem: None,
            package_name: None,
            product_keys: BTreeSet::new(),
            topic_keys: BTreeSet::new(),
            severity: ExternalIntelSeverity::Unknown,
            published_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
            modified_at: None,
            withdrawn_at: None,
            cvss_score: None,
            cvss_vector: None,
            cvss_version: None,
            epss_percentage: None,
            epss_percentile: None,
            is_kev: false,
            is_ransomware_known: false,
            fetched_at: "2026-01-01T00:00:00Z".to_string(),
            raw_json: "{}".to_string(),
            aliases: BTreeSet::new(),
            sources: BTreeSet::from([ExternalSource::Github]),
            source_details: Vec::new(),
            affected_packages: Vec::new(),
            references: Vec::new(),
            cwes: Vec::new(),
            kev: None,
        }
    }

    fn persisted_item_defaults() -> PersistedExternalIntelItem {
        PersistedExternalIntelItem {
            canonical_key: "default".to_string(),
            primary_source: ExternalSource::Github,
            source_item_id: "default".to_string(),
            title: "default".to_string(),
            summary: "summary".to_string(),
            url: "https://example.com".to_string(),
            ecosystem: None,
            package_name: None,
            product_keys: Vec::new(),
            topic_keys: Vec::new(),
            severity: ExternalIntelSeverity::Unknown,
            published_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
            modified_at: None,
            withdrawn_at: None,
            cvss_score: None,
            cvss_vector: None,
            cvss_version: None,
            epss_percentage: None,
            epss_percentile: None,
            is_kev: false,
            is_ransomware_known: false,
            fetched_at: "2026-01-01T00:00:00Z".to_string(),
            raw_json: "{}".to_string(),
            sources: vec![ExternalSource::Github],
            source_details: Vec::new(),
            aliases: Vec::new(),
            affected_packages: Vec::new(),
            references: Vec::new(),
            cwes: Vec::new(),
            kev: None,
            match_kind: None,
            repo_count: 0,
            relevance: ExternalIntelRelevance::General,
        }
    }

    #[test]
    fn normalizes_osv_malware_advisory_as_malware_without_source_severity() {
        let advisory = OsvAdvisory {
            id: "MAL-2026-3353".to_string(),
            summary: Some("Malicious code in money-badger-open-rpc (npm)".to_string()),
            details: None,
            published: Some("2026-05-06T10:00:00Z".to_string()),
            modified: None,
            withdrawn: None,
            aliases: None,
            related: None,
            upstream: None,
            severity: None,
            references: None,
            affected: Some(vec![OsvAffected {
                package: Some(OsvPackage {
                    name: Some("money-badger-open-rpc".to_string()),
                    ecosystem: Some("npm".to_string()),
                    purl: None,
                }),
                ranges: None,
                versions: None,
                ecosystem_specific: None,
            }]),
        };

        let item = normalize_osv(advisory, "2026-05-06T12:00:00Z")
            .expect("normalization should succeed")
            .expect("malware advisory should produce an item");

        assert_eq!(item.severity, ExternalIntelSeverity::Malware);
        assert_eq!(item.package_name.as_deref(), Some("money-badger-open-rpc"));
    }

    #[test]
    fn merges_items_by_shared_alias() {
        let items = vec![
            raw_item(
                ExternalSource::Osv,
                "OSV-1",
                &["CVE-2026-0001"],
                Some("lodash"),
                Some("npm"),
                ExternalIntelSeverity::High,
                "2026-01-01T00:00:00Z",
            ),
            raw_item(
                ExternalSource::Github,
                "GHSA-1",
                &["CVE-2026-0001", "GHSA-aaaa-bbbb-cccc"],
                Some("lodash"),
                Some("npm"),
                ExternalIntelSeverity::Critical,
                "2026-01-02T00:00:00Z",
            ),
        ];

        let merged = merge_items(items);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].primary_source, ExternalSource::Github);
        assert_eq!(merged[0].severity, ExternalIntelSeverity::Critical);
        assert!(merged[0].aliases.contains("CVE-2026-0001"));
    }

    #[test]
    fn merges_items_by_package_ecosystem_and_date_without_alias() {
        let items = vec![
            raw_item(
                ExternalSource::Osv,
                "OSV-2",
                &[],
                Some("requests"),
                Some("pip"),
                ExternalIntelSeverity::High,
                "2026-01-03T10:00:00Z",
            ),
            raw_item(
                ExternalSource::Nvd,
                "CVE-2026-0002",
                &[],
                Some("requests"),
                Some("pip"),
                ExternalIntelSeverity::Medium,
                "2026-01-03T18:00:00Z",
            ),
        ];

        let merged = merge_items(items);
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn does_not_merge_unrelated_product_items_on_same_day() {
        let items = vec![
            product_raw_item(
                "vendor-1",
                "github:enterprise_server",
                "2026-01-03T10:00:00Z",
            ),
            product_raw_item("vendor-2", "docker:docker", "2026-01-03T18:00:00Z"),
        ];

        let merged = merge_items(items);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn aliasless_vendor_items_get_distinct_canonical_keys() {
        let index = LocalPackageIndex::default();
        let left = to_persisted_item(
            from_raw(product_raw_item(
                "vendor-ghes",
                "github:enterprise_server",
                "2026-01-03T10:00:00Z",
            )),
            &index,
        );
        let right = to_persisted_item(
            from_raw(product_raw_item(
                "vendor-docker",
                "docker:docker",
                "2026-01-03T18:00:00Z",
            )),
            &index,
        );

        assert_ne!(left.canonical_key, right.canonical_key);
        assert_eq!(left.canonical_key, "vendor:vendor-ghes");
        assert_eq!(right.canonical_key, "vendor:vendor-docker");
    }

    #[test]
    fn computes_package_match_before_ecosystem_match() {
        let mut index = LocalPackageIndex::default();
        index.add_test_package("npm", "lodash", 1);
        index.add_test_package("npm", "lodash", 2);
        index.ecosystem_repo_counts.insert("npm".to_string(), 5);

        let item = MergedIntelItem {
            primary_source: ExternalSource::Github,
            source_item_id: "GHSA-2".to_string(),
            title: "lodash".to_string(),
            summary: "summary".to_string(),
            url: "https://example.com".to_string(),
            ecosystem: Some("npm".to_string()),
            package_name: Some("lodash".to_string()),
            product_keys: BTreeSet::new(),
            topic_keys: BTreeSet::new(),
            severity: ExternalIntelSeverity::High,
            published_at: Some("2026-01-01T00:00:00Z".to_string()),
            fetched_at: "2026-01-01T00:00:00Z".to_string(),
            raw_json: "{}".to_string(),
            aliases: BTreeSet::new(),
            sources: BTreeSet::from([ExternalSource::Github]),
            ..merged_item_defaults()
        };

        let persisted = to_persisted_item(item, &index);
        assert_eq!(
            persisted.match_kind,
            Some(ExternalIntelMatchKind::PackageName)
        );
        assert_eq!(persisted.repo_count, 2);
    }

    #[test]
    fn parses_github_enterprise_server_cpe_to_product_topic() {
        let key = parse_cpe_product_key("cpe:2.3:a:github:enterprise_server:*:*:*:*:*:*:*:*")
            .expect("product key");
        assert_eq!(key, "github:enterprise_server");
        assert_eq!(
            product_key_to_topic(&key).as_deref(),
            Some("github_enterprise_server")
        );
    }

    #[test]
    fn keeps_high_product_advisory_visible_as_general_without_local_package_match() {
        let item = MergedIntelItem {
            primary_source: ExternalSource::Nvd,
            source_item_id: "CVE-2026-3854".to_string(),
            title: "CVE-2026-3854".to_string(),
            summary: "GitHub Enterprise Server vulnerability".to_string(),
            url: "https://nvd.nist.gov/vuln/detail/CVE-2026-3854".to_string(),
            ecosystem: None,
            package_name: None,
            product_keys: BTreeSet::from(["github:enterprise_server".to_string()]),
            topic_keys: BTreeSet::from(["github_enterprise_server".to_string()]),
            severity: ExternalIntelSeverity::Critical,
            published_at: Some("2026-04-28T00:00:00Z".to_string()),
            fetched_at: "2026-04-28T00:00:00Z".to_string(),
            raw_json: "{}".to_string(),
            aliases: BTreeSet::from(["CVE-2026-3854".to_string()]),
            sources: BTreeSet::from([ExternalSource::Nvd]),
            ..merged_item_defaults()
        };
        let watched = BTreeSet::from(["github_enterprise_server".to_string()]);
        assert!(item_matches_watched_topics(&item, &watched));

        let persisted = to_persisted_item(item, &LocalPackageIndex::default());
        assert_eq!(persisted.relevance, ExternalIntelRelevance::General);
        assert_eq!(persisted.match_kind, None);
        assert_eq!(persisted.repo_count, 0);
    }

    #[test]
    fn keeps_high_vendor_advisory_visible_even_without_watched_topic_match() {
        let item = MergedIntelItem {
            primary_source: ExternalSource::Vendor,
            source_item_id: "vendor-generic".to_string(),
            title: "Critical vendor advisory".to_string(),
            summary: "summary".to_string(),
            url: "https://example.com".to_string(),
            ecosystem: None,
            package_name: None,
            product_keys: BTreeSet::new(),
            topic_keys: BTreeSet::new(),
            severity: ExternalIntelSeverity::High,
            published_at: Some("2026-01-01T00:00:00Z".to_string()),
            fetched_at: "2026-01-01T00:00:00Z".to_string(),
            raw_json: "{}".to_string(),
            aliases: BTreeSet::new(),
            sources: BTreeSet::from([ExternalSource::Vendor]),
            ..merged_item_defaults()
        };

        assert!(item_matches_watched_topics(&item, &BTreeSet::new()));
    }

    #[test]
    fn normalizes_cisa_kev_entries_outside_recent_lookback() {
        let catalog = CisaKevCatalog {
            catalog_version: Some("2026.06.09".to_string()),
            date_released: Some("2026-06-09T00:00:00Z".to_string()),
            count: Some(1),
            vulnerabilities: vec![CisaKevVulnerability {
                cve_id: "CVE-2024-3094".to_string(),
                vendor_project: Some("Example Vendor".to_string()),
                product: Some("Example Product".to_string()),
                vulnerability_name: Some("Example Product Vulnerability".to_string()),
                short_description: Some("A known exploited vulnerability.".to_string()),
                date_added: Some("2024-03-29".to_string()),
                due_date: Some("2024-04-19".to_string()),
                required_action: Some("Apply mitigations per vendor instructions.".to_string()),
                known_ransomware_campaign_use: Some("Known".to_string()),
                notes: None,
                cwes: Some(vec!["CWE-502".to_string()]),
            }],
        };

        let items =
            normalize_cisa_kev_catalog(catalog, "2026-06-09T12:00:00Z").expect("KEV normalization");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, ExternalSource::CisaKev);
        assert_eq!(items[0].source_item_id, "CVE-2024-3094");
        assert!(items[0].is_kev);
        assert!(items[0].is_ransomware_known);
        assert_eq!(
            items[0].published_at.as_deref(),
            Some("2024-03-29T00:00:00+00:00")
        );
        assert_eq!(
            items[0]
                .kev
                .as_ref()
                .and_then(|kev| kev.due_date.as_deref()),
            Some("2024-04-19"),
        );
    }

    #[test]
    fn parses_rss_and_atom_vendor_feeds() {
        let rss = r#"
          <rss><channel><item>
            <title><![CDATA[Critical GitHub Enterprise Server security update]]></title>
            <link>https://example.com/rss</link>
            <pubDate>Tue, 28 Apr 2026 12:00:00 GMT</pubDate>
          </item></channel></rss>
        "#;
        let atom = r#"
          <feed><entry>
            <title>Docker security patch</title>
            <link href="https://example.com/atom" />
            <updated>2026-04-28T12:00:00Z</updated>
          </entry></feed>
        "#;

        let rss_items = parse_vendor_feed(rss, "rss").expect("rss feed");
        let atom_items = parse_vendor_feed(atom, "atom").expect("atom feed");

        assert_eq!(rss_items.len(), 1);
        assert_eq!(
            rss_items[0].title,
            "Critical GitHub Enterprise Server security update"
        );
        assert_eq!(
            rss_items[0].link.as_deref(),
            Some("https://example.com/rss")
        );
        assert_eq!(atom_items.len(), 1);
        assert_eq!(atom_items[0].title, "Docker security patch");
        assert_eq!(
            atom_items[0].link.as_deref(),
            Some("https://example.com/atom")
        );
    }

    #[test]
    fn persisted_sort_prefers_newest_item_over_relevance() {
        let mut items = [
            PersistedExternalIntelItem {
                canonical_key: "old-relevant".to_string(),
                primary_source: ExternalSource::Github,
                source_item_id: "old-relevant".to_string(),
                title: "old relevant".to_string(),
                summary: "summary".to_string(),
                url: "https://example.com/old".to_string(),
                ecosystem: Some("npm".to_string()),
                package_name: Some("lodash".to_string()),
                product_keys: Vec::new(),
                topic_keys: Vec::new(),
                severity: ExternalIntelSeverity::Critical,
                published_at: Some("2026-01-01T00:00:00Z".to_string()),
                fetched_at: "2026-01-03T00:00:00Z".to_string(),
                raw_json: "{}".to_string(),
                sources: vec![ExternalSource::Github],
                aliases: Vec::new(),
                match_kind: Some(ExternalIntelMatchKind::PackageName),
                repo_count: 1,
                relevance: ExternalIntelRelevance::Relevant,
                ..persisted_item_defaults()
            },
            PersistedExternalIntelItem {
                canonical_key: "new-general".to_string(),
                primary_source: ExternalSource::Vendor,
                source_item_id: "new-general".to_string(),
                title: "new general".to_string(),
                summary: "summary".to_string(),
                url: "https://example.com/new".to_string(),
                ecosystem: None,
                package_name: None,
                product_keys: vec!["docker:docker".to_string()],
                topic_keys: vec!["docker".to_string()],
                severity: ExternalIntelSeverity::High,
                published_at: Some("2026-01-03T00:00:00Z".to_string()),
                fetched_at: "2026-01-03T00:00:00Z".to_string(),
                raw_json: "{}".to_string(),
                sources: vec![ExternalSource::Vendor],
                aliases: Vec::new(),
                match_kind: None,
                repo_count: 0,
                relevance: ExternalIntelRelevance::General,
                ..persisted_item_defaults()
            },
        ];

        items.sort_by(|left, right| {
            external_published_sort_key(right.published_at.as_deref())
                .cmp(&external_published_sort_key(left.published_at.as_deref()))
                .then_with(|| {
                    repo_count_rank(right.repo_count).cmp(&repo_count_rank(left.repo_count))
                })
                .then_with(|| right.severity.rank().cmp(&left.severity.rank()))
                .then_with(|| left.title.cmp(&right.title))
        });

        assert_eq!(items[0].canonical_key, "new-general");
    }

    #[test]
    fn matches_product_before_ecosystem_without_exact_package_match() {
        let mut index = LocalPackageIndex::default();
        index
            .product_repo_counts
            .insert("github:enterprise_server".to_string(), 1);
        index.ecosystem_repo_counts.insert("npm".to_string(), 5);

        let item = MergedIntelItem {
            primary_source: ExternalSource::Nvd,
            source_item_id: "CVE-2026-3854".to_string(),
            title: "GHES".to_string(),
            summary: "summary".to_string(),
            url: "https://example.com".to_string(),
            ecosystem: Some("npm".to_string()),
            package_name: None,
            product_keys: BTreeSet::from(["github:enterprise_server".to_string()]),
            topic_keys: BTreeSet::from(["github_enterprise_server".to_string()]),
            severity: ExternalIntelSeverity::High,
            published_at: Some("2026-01-01T00:00:00Z".to_string()),
            fetched_at: "2026-01-01T00:00:00Z".to_string(),
            raw_json: "{}".to_string(),
            aliases: BTreeSet::new(),
            sources: BTreeSet::from([ExternalSource::Nvd]),
            ..merged_item_defaults()
        };

        let persisted = to_persisted_item(item, &index);
        assert_eq!(persisted.match_kind, Some(ExternalIntelMatchKind::Product));
        assert_eq!(persisted.repo_count, 1);
    }

    #[test]
    fn treats_malware_ecosystem_signal_as_triage_without_exact_package_match() {
        let mut index = LocalPackageIndex::default();
        index.ecosystem_repo_counts.insert("npm".to_string(), 4);

        let item = MergedIntelItem {
            primary_source: ExternalSource::Osv,
            source_item_id: "MAL-2026-3353".to_string(),
            title: "Malicious code in money-badger-open-rpc (npm)".to_string(),
            summary: "summary".to_string(),
            url: "https://osv.dev/vulnerability/MAL-2026-3353".to_string(),
            ecosystem: Some("npm".to_string()),
            package_name: Some("money-badger-open-rpc".to_string()),
            product_keys: BTreeSet::new(),
            topic_keys: BTreeSet::new(),
            severity: ExternalIntelSeverity::Malware,
            published_at: Some("2026-05-06T10:00:00Z".to_string()),
            fetched_at: "2026-05-06T12:00:00Z".to_string(),
            raw_json: "{}".to_string(),
            aliases: BTreeSet::from(["MAL-2026-3353".to_string()]),
            sources: BTreeSet::from([ExternalSource::Osv]),
            ..merged_item_defaults()
        };

        let persisted = to_persisted_item(item, &index);

        assert_eq!(
            persisted.match_kind,
            Some(ExternalIntelMatchKind::Ecosystem)
        );
        assert_eq!(persisted.repo_count, 4);
        assert_eq!(persisted.relevance, ExternalIntelRelevance::Triage);
    }

    #[test]
    fn parses_osv_list_entries_from_public_html() {
        let html = r#"
          <div role="row" class="vuln-table-row mdc-data-table__row">
            <span role="cell" class="vuln-table-cell mdc-data-table__cell">
              <a href="/vulnerability/GHSA-test-1234">GHSA-test-1234</a>
            </span>
            <span role="cell" class="vuln-table-cell mdc-data-table__cell">
              <relative-time datetime="2026-04-23 20:01:02.633948+00:00">
                23 minutes ago
              </relative-time>
            </span>
          </div>
        "#;

        let entries = parse_osv_list_entries(html).expect("should parse OSV list HTML");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "GHSA-test-1234");
        assert_eq!(
            timestamps::format(&entries[0].published_at),
            "2026-04-23T20:01:02.633948+00:00"
        );
        assert_eq!(
            parse_osv_list_timestamp("2026-04-23 21:31:02.633948+0130").unwrap(),
            entries[0].published_at
        );
        assert_eq!(
            parse_osv_list_timestamp("2026-04-23 21:01:02.633948+01").unwrap(),
            entries[0].published_at
        );
    }

    #[test]
    fn rejects_partial_refreshes_before_persistence() {
        let error = ensure_complete_refresh(&[
            "NVD: request timed out".to_string(),
            "Vendor: invalid feed".to_string(),
        ])
        .expect_err("a partial result must not replace existing intel");

        let message = error.to_string();
        assert!(message.contains("existing data remains intact"));
        assert!(message.contains("NVD: request timed out"));
        assert!(message.contains("Vendor: invalid feed"));
    }

    #[test]
    fn accepts_refreshes_when_every_source_succeeded() {
        ensure_complete_refresh(&[]).expect("complete results may be persisted");
    }

    #[test]
    fn allows_only_public_https_external_redirects() {
        assert!(is_allowed_external_redirect(
            &reqwest::Url::parse("https://osv.dev/vulnerability/GHSA-1234").unwrap()
        ));
        assert!(!is_allowed_external_redirect(
            &reqwest::Url::parse("http://osv.dev/vulnerability/GHSA-1234").unwrap()
        ));
        assert!(!is_allowed_external_redirect(
            &reqwest::Url::parse("https://localhost/internal").unwrap()
        ));
        assert!(!is_allowed_external_redirect(
            &reqwest::Url::parse("https://127.0.0.1/internal").unwrap()
        ));
        assert!(!is_allowed_external_redirect(
            &reqwest::Url::parse("https://[::1]/internal").unwrap()
        ));
    }
}
