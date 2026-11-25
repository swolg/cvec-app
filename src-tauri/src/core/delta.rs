use std::collections::{BTreeMap, HashMap, HashSet};

use super::models::{DeltaSummary, NormalizedFinding, Severity};

pub fn build_fingerprint(
    advisory_id: &str,
    package_type: Option<&str>,
    package_url: Option<&str>,
    package_name: &str,
    package_version: Option<&str>,
) -> String {
    let package_identity = package_url.unwrap_or(package_name);
    format!(
        "{}|{}|{}|{}",
        advisory_id,
        package_type.unwrap_or("unknown"),
        package_identity,
        package_version.unwrap_or("unknown")
    )
}

pub fn compute_delta(
    previous: &[NormalizedFinding],
    current: &[NormalizedFinding],
) -> DeltaSummary {
    let previous_map = map_by_fingerprint(previous);
    let current_map = map_by_fingerprint(current);

    let mut summary = DeltaSummary::default();

    for (&key, &finding) in &current_map {
        if !previous_map.contains_key(key) {
            increment_new_counter(&mut summary, &finding.severity);
            summary.new_findings.push(finding.clone());
        }
    }

    for (&key, &finding) in &current_map {
        if previous_map.get(key).is_some_and(|previous| {
            severity_rank(&finding.severity) > severity_rank(&previous.severity)
        }) {
            increment_new_counter(&mut summary, &finding.severity);
            summary.new_findings.push(finding.clone());
        }
    }

    if !previous_map.is_empty() {
        let detected = DetectedFindings::new(current);
        for finding in previous_map.into_values() {
            if !detected.contains(
                &finding.fingerprint,
                &finding.advisory_id,
                &finding.package_name,
                finding.package_type.as_deref(),
                finding.package_url.as_deref(),
            ) {
                increment_resolved_counter(&mut summary, &finding.severity);
                summary.resolved_findings.push(finding.clone());
            }
        }
    }

    summary
}

/// Evidence for absence uses the advisory and package across versions.
/// Review decisions and inventory still use the exact versioned fingerprint.
pub(super) struct DetectedFindings<'a> {
    fingerprints: HashSet<&'a str>,
    packages: HashMap<(&'a str, String), Vec<&'a NormalizedFinding>>,
}

impl<'a> DetectedFindings<'a> {
    pub(super) fn new(findings: &'a [NormalizedFinding]) -> Self {
        let mut detected = Self {
            fingerprints: HashSet::with_capacity(findings.len()),
            packages: HashMap::with_capacity(findings.len()),
        };
        for finding in findings {
            detected.fingerprints.insert(&finding.fingerprint);
            detected
                .packages
                .entry((
                    &finding.package_name,
                    finding.advisory_id.to_ascii_lowercase(),
                ))
                .or_default()
                .push(finding);
        }
        detected
    }

    pub(super) fn contains(
        &self,
        fingerprint: &str,
        advisory_id: &str,
        package_name: &str,
        package_type: Option<&str>,
        package_url: Option<&str>,
    ) -> bool {
        self.fingerprints.contains(fingerprint)
            || self
                .packages
                .get(&(package_name, advisory_id.to_ascii_lowercase()))
                .is_some_and(|findings| {
                    findings.iter().any(|finding| {
                        same_scan_package(
                            package_type,
                            package_url,
                            finding.package_type.as_deref(),
                            finding.package_url.as_deref(),
                        )
                    })
                })
    }
}

// Called only after the advisory and package name match. Keep namespaces,
// qualifiers and subpaths when both scanners provided a package URL.
pub(super) fn same_scan_package(
    left_type: Option<&str>,
    left_url: Option<&str>,
    right_type: Option<&str>,
    right_url: Option<&str>,
) -> bool {
    match (
        left_url.and_then(purl_without_version),
        right_url.and_then(purl_without_version),
    ) {
        (Some(left), Some(right)) => left == right,
        (None, None) if left_url.is_none() && right_url.is_none() => {
            matches!((left_type, right_type), (Some(left), Some(right)) if left == right && !matches!(left, "" | "unknown"))
        }
        _ => false,
    }
}

pub(super) fn purl_without_version(purl: &str) -> Option<(&str, &str)> {
    let purl = purl.strip_prefix("pkg:")?;
    let (path, qualifiers_and_subpath) = purl.split_at(purl.find(['?', '#']).unwrap_or(purl.len()));
    let (prefix, name_and_version) = path.rsplit_once('/')?;
    let name = name_and_version.split('@').next()?;
    if prefix.is_empty() || name.is_empty() {
        return None;
    }
    Some((
        &path[..prefix.len() + 1 + name.len()],
        qualifiers_and_subpath,
    ))
}

pub(crate) struct ScanFindingChanges<'a> {
    pub delta: DeltaSummary,
    pub new: Vec<&'a str>,
    pub worsened: Vec<&'a str>,
}

pub(crate) fn scan_finding_changes<'a>(
    previous: Vec<(String, Severity)>,
    current: &'a [NormalizedFinding],
) -> ScanFindingChanges<'a> {
    let previous: HashMap<_, _> = previous.into_iter().collect();
    let mut changes = ScanFindingChanges {
        delta: DeltaSummary::default(),
        new: Vec::new(),
        worsened: Vec::new(),
    };
    for finding in current {
        let ids = match previous.get(&finding.fingerprint) {
            None => &mut changes.new,
            Some(severity) if severity_rank(&finding.severity) > severity_rank(severity) => {
                &mut changes.worsened
            }
            Some(_) => continue,
        };
        increment_new_counter(&mut changes.delta, &finding.severity);
        ids.push(finding.fingerprint.as_str());
    }
    changes
}

fn map_by_fingerprint(findings: &[NormalizedFinding]) -> BTreeMap<&str, &NormalizedFinding> {
    findings
        .iter()
        .map(|finding| (finding.fingerprint.as_str(), finding))
        .collect()
}

fn increment_new_counter(summary: &mut DeltaSummary, severity: &Severity) {
    match severity {
        Severity::Medium => summary.new_medium_count += 1,
        Severity::High => summary.new_high_count += 1,
        Severity::Critical => summary.new_critical_count += 1,
        Severity::Low | Severity::Negligible | Severity::Unknown => summary.new_other_count += 1,
    }
}

fn increment_resolved_counter(summary: &mut DeltaSummary, severity: &Severity) {
    match severity {
        Severity::Medium => summary.resolved_medium_count += 1,
        Severity::High => summary.resolved_high_count += 1,
        Severity::Critical => summary.resolved_critical_count += 1,
        Severity::Low | Severity::Negligible | Severity::Unknown => {
            summary.resolved_other_count += 1
        }
    }
}

fn severity_rank(severity: &Severity) -> u8 {
    severity.rank()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(id: &str, severity: Severity) -> NormalizedFinding {
        NormalizedFinding {
            fingerprint: id.to_string(),
            advisory_id: id.to_string(),
            package_name: "demo".into(),
            package_version: Some("1.0.0".into()),
            package_type: Some("npm".into()),
            severity,
            fix_version: None,
            title: None,
            source_url: None,
            package_url: None,
            match_evidence: None,
            advisory_details: None,
        }
    }

    #[test]
    fn computes_new_and_resolved_findings() {
        let previous = vec![
            finding("GHSA-0", Severity::Medium),
            finding("GHSA-1", Severity::High),
            finding("GHSA-2", Severity::Critical),
        ];
        let current = vec![
            finding("GHSA-2", Severity::Critical),
            finding("GHSA-3", Severity::High),
            finding("GHSA-4", Severity::Medium),
        ];

        let delta = compute_delta(&previous, &current);

        assert_eq!(delta.new_medium_count, 1);
        assert_eq!(delta.new_high_count, 1);
        assert_eq!(delta.new_critical_count, 0);
        assert_eq!(delta.resolved_medium_count, 1);
        assert_eq!(delta.resolved_high_count, 1);
        assert_eq!(delta.resolved_critical_count, 0);
        assert_eq!(delta.new_findings[0].advisory_id, "GHSA-3");
        assert_eq!(delta.new_findings[1].advisory_id, "GHSA-4");
        assert_eq!(delta.resolved_findings[0].advisory_id, "GHSA-0");
        assert_eq!(delta.resolved_findings[1].advisory_id, "GHSA-1");
    }

    #[test]
    fn treats_severity_escalations_as_new_findings() {
        let previous = vec![finding("GHSA-1", Severity::Medium)];
        let current = vec![finding("GHSA-1", Severity::Critical)];

        let delta = compute_delta(&previous, &current);

        assert_eq!(delta.new_critical_count, 1);
        assert_eq!(delta.resolved_medium_count, 0);
        assert_eq!(delta.new_findings, current);
        assert!(delta.resolved_findings.is_empty());
    }

    #[test]
    fn ignores_severity_deescalations_for_alerting() {
        let previous = vec![finding("GHSA-1", Severity::Critical)];
        let current = vec![finding("GHSA-1", Severity::High)];

        assert_eq!(compute_delta(&previous, &current), DeltaSummary::default());
    }

    #[test]
    fn scan_counts_match_full_delta_for_new_and_changed_severity() {
        let previous = [
            finding("resolved", Severity::Medium),
            finding("unchanged", Severity::High),
            finding("increased", Severity::Medium),
            finding("decreased", Severity::Critical),
        ];
        let current = [
            finding("unchanged", Severity::High),
            finding("increased", Severity::Critical),
            finding("decreased", Severity::High),
            finding("new", Severity::High),
        ];
        let mut full = compute_delta(&previous, &current);
        full.new_findings.clear();
        full.resolved_findings.clear();
        full.resolved_medium_count = 0;
        let severities = previous
            .into_iter()
            .map(|finding| (finding.fingerprint, finding.severity))
            .collect();
        assert_eq!(scan_finding_changes(severities, &current).delta, full);
    }
}
