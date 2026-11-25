use super::*;
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

use super::super::delta::ScanFindingChanges;
use super::super::models::{ReviewFilters, ReviewUpdate};

// This relation contains identities and scalar risk facts only. Advisory bodies,
// package inventories and remediation evidence are loaded for a selected item.
fn relation(repo_id: Option<i64>) -> String {
    review_relation(
        &canonical_findings(repo_id, FindingScope::All, "", None),
        false,
    )
}

#[derive(Clone, Copy)]
enum FindingScope {
    All,
    Active,
    Resolved,
}

impl FindingScope {
    fn for_states(states: &[RepoFindingState]) -> Self {
        if states.is_empty() {
            Self::All
        } else if !states.contains(&RepoFindingState::Resolved) {
            Self::Active
        } else if states
            .iter()
            .all(|state| *state == RepoFindingState::Resolved)
        {
            Self::Resolved
        } else {
            Self::All
        }
    }

    fn needs_state_filter(self, states: &[RepoFindingState]) -> bool {
        match self {
            Self::All => !states.is_empty(),
            Self::Active => {
                !states.contains(&RepoFindingState::Open)
                    || !states.contains(&RepoFindingState::New)
            }
            Self::Resolved => false,
        }
    }
}

fn canonical_findings(
    repo_id: Option<i64>,
    scope: FindingScope,
    annotations: &str,
    resolved_in_scan_run_id: Option<i64>,
) -> String {
    let repository = repo_id
        .map(|id| format!(" AND r.id = {id}"))
        .unwrap_or_default();
    let runs = match resolved_in_scan_run_id {
        Some(run_id) => resolution_runs(&repository, run_id),
        None => review_runs(&repository),
    };
    let active = || {
        format!(
            r#"
      SELECT {FINDING_COLUMNS}, rr.has_older_findings,
        {ACTIVE_FINDING_STATE} scanner_state
      FROM runs_with_prior rr JOIN vulnerabilities v ON v.scan_run_id=rr.latest_id
      WHERE 1 {annotations}"#
        )
    };
    let resolved = || {
        format!(
            r#"
      SELECT {FINDING_COLUMNS},0 has_older_findings,'resolved' scanner_state
      FROM repo_runs rr JOIN vulnerabilities v ON v.scan_run_id=rr.previous_id
      WHERE NOT {detected} {annotations}"#,
            detected = scan_has_finding("latest", "rr.latest_id", "v"),
        )
    };
    let (prior, findings) = match scope {
        FindingScope::All => (
            RUNS_WITH_PRIOR,
            format!("{} UNION ALL {}", active(), resolved()),
        ),
        FindingScope::Active => (RUNS_WITH_PRIOR, active()),
        FindingScope::Resolved => ("", resolved()),
    };
    format!("{runs}{prior}, canonical AS ({findings})")
}

const FINDING_COLUMNS: &str = r#"
    v.id vulnerability_id,v.repo_id,rr.repo_name,rr.source_type,rr.previous_id,
    rr.preferred_evidence_id,v.scan_run_id recorded_scan_id,v.fingerprint,v.advisory_id,
    v.package_name,v.package_version,v.package_type,v.package_url,v.severity,v.fix_version,
    v.title,v.source_url,v.advisory_severity,v.match_evidence_json
"#;

const ACTIVE_FINDING_STATE: &str = r#"
    CASE WHEN rr.has_prior_findings THEN
      CASE WHEN EXISTS(SELECT 1 FROM vulnerabilities p WHERE p.scan_run_id=rr.previous_id AND p.fingerprint=v.fingerprint)
        THEN 'open' ELSE 'new' END
      WHEN rr.has_complete_prior THEN 'new' ELSE 'open' END
"#;

// A full clean scan establishes the same baseline as a scan with stored findings.
// Imported runs without complete evidence still provide a baseline when they have findings.
const RUNS_WITH_PRIOR: &str = r#"
    , runs_with_prior AS MATERIALIZED (
      SELECT rr.*, EXISTS(SELECT 1 FROM vulnerabilities p WHERE p.scan_run_id=rr.previous_id) has_prior_findings,
        EXISTS(SELECT 1 FROM scan_runs p WHERE p.id=rr.previous_id AND p.evidence_complete=1) has_complete_prior,
        EXISTS(SELECT 1 FROM scan_runs s JOIN vulnerabilities old ON old.scan_run_id=s.id
               WHERE s.repo_id=rr.repo_id AND s.status='completed'
                 AND (s.started_at,s.id)<(SELECT started_at,id FROM scan_runs WHERE id=rr.previous_id)) has_older_findings
      FROM repo_runs rr
    )
"#;

// Multiple identifiers can resolve to the same advisory. MAX makes the
// duplicate matches harmless, so the identity lookup can use UNION ALL.
const ADVISORY_FACTS: &str = r#"
    , identities AS (
      SELECT lower(canonical_key) identity,id FROM external_advisories
      UNION ALL SELECT lower(source_item_id),id FROM external_advisories
      UNION ALL SELECT lower(alias),advisory_id FROM external_advisory_aliases
    ), exploitation AS MATERIALIZED (
      SELECT i.identity,MAX(ea.is_ransomware_known)*2+MAX(ea.is_kev) score,MAX(cvec_severity_rank(ea.severity)) source_rank
      FROM identities i JOIN external_advisories ea ON ea.id=i.id GROUP BY i.identity
    )
"#;
const EXPLOITATION_FLAGS: &str =
    "exploitation_score%2 known_exploited,exploitation_score>=2 ransomware_known";
const REVIEW_RISK_RANK: &str =
    "MAX(cvec_severity_rank(severity),source_rank,cvec_severity_rank(advisory_severity))";
fn review_finding_reappeared() -> String {
    finding_reappeared(
        "f",
        "f.previous_id",
        "f.has_older_findings AND f.scanner_state='new'",
    )
}

fn review_facts(canonical: &str, include_intel: bool, include_evidence: bool) -> String {
    let (advisory, intel_fields, intel_join) = if include_intel {
        (
            ADVISORY_FACTS,
            ",COALESCE(e.score,0) exploitation_score,COALESCE(e.source_rank,0) source_rank",
            "LEFT JOIN exploitation e ON e.identity=lower(c.advisory_id)",
        )
    } else {
        ("", "", "")
    };
    let evidence = if include_evidence {
        format!(",{REVIEW_EVIDENCE_ID} evidence_id")
    } else {
        String::new()
    };
    format!(
        r#"
    {canonical}{advisory}, facts AS (
      SELECT c.*,COALESCE(fr.state,'unreviewed') review_state,
        fr.reviewed_at,fr.snooze_until,fr.note review_note,fix_version IS NOT NULL fix_available
        {intel_fields}{evidence}
      FROM canonical c LEFT JOIN finding_reviews fr ON fr.repo_id=c.repo_id AND fr.fingerprint=c.fingerprint
      {intel_join}
    )"#
    )
}

fn review_relation(canonical: &str, include_scan_changes: bool) -> String {
    let facts = review_facts(canonical, true, true);
    let (recorded_change, change_join) = if include_scan_changes {
        (
            "WHEN change.kind IS NOT NULL THEN change.kind",
            "LEFT JOIN scan_review_changes change ON change.scan_run_id=f.recorded_scan_id
              AND change.fingerprint=f.fingerprint",
        )
    } else {
        ("", "")
    };
    format!(
        r#"
    {facts}, ranked AS (
      SELECT f.*,
        CASE WHEN {reappeared}
          THEN 'reappeared'
          {recorded_change}
          WHEN f.scanner_state='new' THEN 'new' END material_change
      FROM facts f
      {change_join}
    ), review AS (
      SELECT f.*,s.finished_at scanned_at,COALESCE(s.finished_at,'') scanned_key,
        {EXPLOITATION_FLAGS},
        CASE WHEN exploitation_score>=2 THEN 2 ELSE exploitation_score END exploitation_rank,
        cvec_severity_rank(severity) severity_rank,
        {REVIEW_RISK_RANK} risk_rank,
        CASE scanner_state WHEN 'new' THEN 2 WHEN 'open' THEN 1 ELSE 0 END scanner_rank,
        CASE material_change WHEN 'reappeared' THEN 4 WHEN 'severity_increased' THEN 3
          WHEN 'reopened' THEN 2 WHEN 'new' THEN 1 ELSE 0 END change_rank
      FROM ranked f LEFT JOIN scan_runs s ON s.id=f.evidence_id
    )
    "#,
        reappeared = review_finding_reappeared(),
    )
}

fn count_relation(canonical: &str, filters: &ReviewFilters) -> String {
    let intel = filters.known_exploited.is_some() || filters.ransomware_known.is_some();
    let dates = filters.changed_since.is_some();
    let facts = review_facts(canonical, intel, dates || !filters.scan_run_ids.is_empty());
    let flags = if intel {
        format!(",{EXPLOITATION_FLAGS}")
    } else {
        String::new()
    };
    let (date, join) = if dates {
        (
            ",s.finished_at scanned_at",
            "LEFT JOIN scan_runs s ON s.id=f.evidence_id",
        )
    } else {
        ("", "")
    };
    format!("{facts}, review AS (SELECT f.*{flags}{date} FROM facts f {join})")
}

fn review_runs(scope: &str) -> String {
    format!(
        r#"
    WITH repo_runs AS MATERIALIZED (
      SELECT r.id repo_id, r.name repo_name, r.source_type,
        (SELECT MAX(id) FROM scan_runs WHERE repo_id=r.id AND status='completed') preferred_evidence_id,
        (SELECT id FROM scan_runs WHERE repo_id=r.id AND status='completed' ORDER BY started_at DESC,id DESC LIMIT 1) latest_id,
        (SELECT id FROM scan_runs WHERE repo_id=r.id AND status='completed' ORDER BY started_at DESC,id DESC LIMIT 1 OFFSET 1) previous_id
      FROM repos r WHERE r.is_active=1 {scope}
    )
    "#
    )
}

fn resolution_runs(scope: &str, run_id: i64) -> String {
    format!(
        r#"
    WITH repo_runs AS MATERIALIZED (
      SELECT r.id repo_id,r.name repo_name,r.source_type,
        previous.id preferred_evidence_id,selected.id latest_id,previous.id previous_id
      FROM scan_runs selected
      JOIN repos r ON r.id=selected.repo_id
      JOIN scan_runs previous ON previous.id=(
        SELECT p.id FROM scan_runs p WHERE p.repo_id=selected.repo_id AND p.status='completed'
          AND (p.started_at,p.id)<(selected.started_at,selected.id)
        ORDER BY p.started_at DESC,p.id DESC LIMIT 1
      )
      WHERE selected.id={run_id} AND selected.status='completed' AND selected.evidence_complete=1
        AND r.is_active=1 {scope}
    )
    "#
    )
}

// A finding's evidence can come from an imported run whose id is newer than
// the run picked by chronological order. Historical comparisons prefer the
// original observation so a later reappearance cannot replace that evidence.
const REVIEW_EVIDENCE_ID: &str = r#"
    CASE WHEN c.recorded_scan_id=c.preferred_evidence_id THEN c.recorded_scan_id
    ELSE (SELECT s.id FROM vulnerabilities ev JOIN scan_runs s ON s.id=ev.scan_run_id
          WHERE ev.repo_id=c.repo_id AND ev.fingerprint=c.fingerprint AND s.status='completed'
          ORDER BY s.id DESC LIMIT 1) END
"#;

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum CursorValue {
    Number(i64),
    Text(String),
}
impl CursorValue {
    fn value(&self) -> Value {
        match self {
            Self::Number(n) => Value::Integer(*n),
            Self::Text(s) => Value::Text(s.clone()),
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
struct Cursor {
    revision: u64,
    query: u64,
    last: Vec<CursorValue>,
}

fn signature(query: &ReviewQuery) -> Result<u64, Error> {
    let mut canonical = query.clone();
    canonical.cursor = None;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&canonical)?.hash(&mut hash);
    Ok(hash.finish())
}
fn query_ordering(query: &ReviewQuery) -> Result<Vec<(&'static str, bool)>, Error> {
    use super::super::models::ReviewDirection;
    if query.sorts.is_empty() {
        return Ok(ordering(query.sort));
    }
    if query.sorts.len() > 3 {
        return Err(Error::App(
            "Review supports at most three sort fields".into(),
        ));
    }
    let mut terms = Vec::new();
    let mut fields = Vec::new();
    for sort in &query.sorts {
        if fields.contains(&sort.field) {
            return Err(Error::App("Review sort fields must be unique".into()));
        }
        fields.push(sort.field);
        let base = ordering(sort.field);
        let count = if sort.field == ReviewSort::Priority {
            5
        } else {
            1
        };
        for (field, _) in base.into_iter().take(count) {
            if !terms.iter().any(|(existing, _)| *existing == field) {
                terms.push((field, sort.direction == ReviewDirection::Desc));
            }
        }
    }
    for term in ordering(query.sorts[0].field) {
        if !terms.iter().any(|(field, _)| *field == term.0) {
            terms.push(term);
        }
    }
    Ok(terms)
}
fn ordering(sort: ReviewSort) -> Vec<(&'static str, bool)> {
    let mut terms = match sort {
        ReviewSort::Priority => vec![
            ("exploitation_rank", true),
            ("risk_rank", true),
            ("change_rank", true),
            ("fix_available", true),
            ("scanned_key", true),
            ("repo_name", false),
            ("advisory_id", false),
            ("fingerprint", false),
        ],
        ReviewSort::Severity => vec![
            ("severity_rank", true),
            ("repo_name", false),
            ("scanner_rank", true),
            ("package_name", false),
            ("advisory_id", false),
        ],
        ReviewSort::Recent => vec![
            ("scanned_key", true),
            ("repo_name", false),
            ("scanner_rank", true),
            ("severity_rank", true),
            ("package_name", false),
            ("advisory_id", false),
        ],
        ReviewSort::Repository => vec![
            ("repo_name", false),
            ("severity_rank", true),
            ("scanner_rank", true),
            ("package_name", false),
            ("advisory_id", false),
        ],
        ReviewSort::Package => vec![
            ("package_name", false),
            ("advisory_id", false),
            ("repo_name", false),
            ("scanner_rank", true),
            ("severity_rank", true),
        ],
    };
    if !terms.iter().any(|(key, _)| *key == "fingerprint") {
        terms.push(("fingerprint", false));
    }
    terms.push(("repo_id", false));
    terms
}
fn filters(
    query: &ReviewQuery,
    scope: FindingScope,
    values: &mut Vec<Value>,
) -> Result<String, Error> {
    let f = &query.filters;
    let mut parts = Vec::new();
    let mut add_array =
        |column: &str, value: serde_json::Value, lowercase: bool| -> Result<(), Error> {
            if value.as_array().is_none_or(|items| items.is_empty()) {
                return Ok(());
            }
            values.push(Value::Text(serde_json::to_string(&value)?));
            let number = values.len();
            parts.push(if lowercase {
                format!("lower({column}) IN (SELECT lower(value) FROM json_each(?{number}))")
            } else {
                format!("{column} IN (SELECT value FROM json_each(?{number}))")
            });
            Ok(())
        };
    add_array(
        "review_state",
        serde_json::to_value(&f.review_states)?,
        false,
    )?;
    if scope.needs_state_filter(&f.scanner_states) {
        add_array(
            "scanner_state",
            serde_json::to_value(&f.scanner_states)?,
            false,
        )?;
    }
    add_array("severity", serde_json::to_value(&f.severities)?, false)?;
    add_array("repo_id", serde_json::to_value(&f.repo_ids)?, false)?;
    add_array("source_type", serde_json::to_value(&f.source_types)?, false)?;
    add_array("package_name", serde_json::to_value(&f.packages)?, true)?;
    add_array("package_type", serde_json::to_value(&f.ecosystems)?, true)?;
    add_array("evidence_id", serde_json::to_value(&f.scan_run_ids)?, false)?;
    if let Some(fingerprint) = &f.fingerprint {
        values.push(Value::Text(fingerprint.clone()));
        parts.push(format!("fingerprint=?{}", values.len()));
    }
    for (column, expected) in [
        ("fix_available", f.fix_available),
        ("known_exploited", f.known_exploited),
        ("ransomware_known", f.ransomware_known),
    ] {
        if let Some(expected) = expected {
            values.push(Value::Integer(i64::from(expected)));
            parts.push(format!("{column}=?{}", values.len()));
        }
    }
    if let Some(identity) = &f.package_identity {
        if identity.any_fix {
            values.push(Value::Text(package_identity(
                identity.package_url.as_deref(),
                &identity.package_name,
                identity.installed_version.as_deref(),
                identity.package_type.as_deref(),
            )));
            parts.push(format!(
                "cvec_package_identity(package_url,package_name,package_version,package_type)=?{}",
                values.len()
            ));
        } else {
            for (column, value) in [
                ("package_url", &identity.package_url),
                ("package_type", &identity.package_type),
                ("package_version", &identity.installed_version),
                ("fix_version", &identity.fix_version),
            ] {
                values.push(
                    value
                        .as_ref()
                        .map(|s| Value::Text(s.clone()))
                        .unwrap_or(Value::Null),
                );
                if (column != "package_url" || identity.package_url.is_some())
                    && (column != "package_type" || identity.package_url.is_none())
                {
                    parts.push(format!("{column} IS ?{}", values.len()));
                } else {
                    values.pop();
                }
            }
            if identity.package_url.is_none() {
                values.push(Value::Text(identity.package_name.clone()));
                parts.push(format!("lower(package_name)=lower(?{})", values.len()));
            }
        }
    }
    if let Some(since) = &f.changed_since {
        values.push(Value::Text(since.clone()));
        parts.push(format!(
            "julianday(scanned_at)>=julianday(?{})",
            values.len()
        ));
    }
    if let Some(search) = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        values.push(Value::Text(search.to_string()));
        parts.push(format!("cvec_contains(repo_name||' '||advisory_id||' '||package_name||' '||COALESCE(package_version,'')||' '||COALESCE(title,''),?{})",values.len()));
    }
    Ok(if parts.is_empty() {
        "1".into()
    } else {
        parts.join(" AND ")
    })
}
fn keyset(terms: &[(&str, bool)], last: &[CursorValue], values: &mut Vec<Value>) -> String {
    let first = values.len() + 1;
    values.extend(last.iter().map(CursorValue::value));
    // Sort keys are non-null. Stop at the first difference so equal prefixes
    // don't repeat expensive rank expressions for every remaining sort key.
    let comparisons = terms
        .iter()
        .enumerate()
        .map(|(index, (key, desc))| {
            let parameter = first + index;
            let comparison = if *desc { "<" } else { ">" };
            format!("WHEN {key}<>?{parameter} THEN {key}{comparison}?{parameter}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("CASE {comparisons} ELSE 0 END")
}

fn local_search_query() -> String {
    // Versions of the same package name open the same dependency search.
    // Rank those destinations before joining their labels to the other hits.
    format!(
        r#"{runs}, package_matches AS (
              SELECT MIN(id) id,name,repo_id,cvec_text_score(name,?1) score
              FROM scan_packages
              WHERE (repo_id,scan_run_id) IN (SELECT repo_id,latest_id FROM repo_runs)
                AND cvec_text_score(name,?1) IS NOT NULL
              GROUP BY repo_id,name
              ORDER BY score,name,'package:'||MIN(id) LIMIT ?2
            ), candidates AS (
              SELECT 'repo:'||r.repo_id id,'repository' kind,r.repo_name label,r.source_type context,r.repo_id,
                NULL fingerprint,NULL advisory_id,
                cvec_text_score(r.repo_name,?1) score
              FROM repo_runs r
              UNION ALL
              SELECT 'finding:'||r.repo_id||':'||v.fingerprint,'finding',v.advisory_id,r.repo_name||' · '||v.package_name,
                r.repo_id,v.fingerprint,NULL,
                cvec_text_score(v.advisory_id||' '||v.package_name||' '||COALESCE(v.title,''),?1)
              FROM repo_runs r CROSS JOIN vulnerabilities v ON v.repo_id=r.repo_id AND v.scan_run_id=r.latest_id
              UNION ALL
              SELECT 'finding:'||r.repo_id||':'||v.fingerprint,'finding',v.advisory_id,r.repo_name||' · '||v.package_name||' · Resolved',
                r.repo_id,v.fingerprint,NULL,
                cvec_text_score(v.advisory_id||' '||v.package_name||' '||COALESCE(v.title,''),?1)
              FROM repo_runs r CROSS JOIN vulnerabilities v ON v.repo_id=r.repo_id AND v.scan_run_id=r.previous_id
              WHERE CASE WHEN cvec_text_score(v.advisory_id||' '||v.package_name||' '||COALESCE(v.title,''),?1) IS NOT NULL
                         THEN NOT {detected} ELSE 0 END
              UNION ALL
              SELECT 'package:'||p.id,'package',p.name,r.repo_name,p.repo_id,NULL,NULL,p.score
              FROM package_matches p JOIN repo_runs r ON r.repo_id=p.repo_id
              UNION ALL
              SELECT 'advisory:'||ea.id,'advisory',ea.canonical_key,ea.title,NULL,NULL,ea.id,
                cvec_text_score(ea.canonical_key||' '||ea.title||' '||COALESCE(
                  (SELECT group_concat(alias,' ') FROM external_advisory_aliases WHERE advisory_id=ea.id),''),?1)
              FROM external_advisories ea
            ) SELECT id,kind,label,context,repo_id,fingerprint,advisory_id FROM candidates
              WHERE score IS NOT NULL ORDER BY score,label,id LIMIT ?2"#,
        runs = review_runs(""),
        detected = scan_has_finding("current", "r.latest_id", "v"),
    )
}

fn map_search_hit(row: &Row<'_>) -> rusqlite::Result<LocalEntityResult> {
    Ok(LocalEntityResult {
        id: row.get(0)?,
        kind: match row.get_ref(1)?.as_str()? {
            "repository" => LocalEntityKind::Repository,
            "finding" => LocalEntityKind::Finding,
            "package" => LocalEntityKind::Package,
            _ => LocalEntityKind::Advisory,
        },
        label: row.get(2)?,
        context: row.get(3)?,
        repo_id: row.get(4)?,
        fingerprint: row.get(5)?,
        advisory_id: row.get(6)?,
    })
}

pub(super) fn repo_finding_preview(
    connection: &Connection,
    repo_id: i64,
    latest_id: i64,
    previous_id: Option<i64>,
) -> Result<Vec<RepoFinding>, Error> {
    let mut statement = connection.prepare(&format!(
        r#"
        WITH repo_runs AS (SELECT ?1 repo_id, ?3 previous_id)
        {RUNS_WITH_PRIOR}, preview AS MATERIALIZED (
            SELECT fingerprint, advisory_id, package_name, package_version, package_type,
                   package_url, severity, fix_version, title, source_url, match_evidence_json,
                   cvec_severity_rank(severity) severity_rank
            FROM vulnerabilities
            WHERE repo_id=?1 AND scan_run_id=?2
            ORDER BY severity_rank DESC, advisory_id, fingerprint
            LIMIT 6
        )
        SELECT v.*, {ACTIVE_FINDING_STATE} scanner_state
        FROM preview v CROSS JOIN runs_with_prior rr
        ORDER BY severity_rank DESC, advisory_id, fingerprint
        "#,
    ))?;
    let rows = statement.query_map(params![repo_id, latest_id, previous_id], map_repo_finding)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn map_repo_finding(row: &Row<'_>) -> rusqlite::Result<RepoFinding> {
    Ok(RepoFinding {
        fingerprint: row.get("fingerprint")?,
        advisory_id: row.get("advisory_id")?,
        package_name: row.get("package_name")?,
        package_version: row.get("package_version")?,
        package_type: row.get("package_type")?,
        package_url: row.get("package_url")?,
        severity: Severity::parse(row.get_ref("severity")?.as_str()?).unwrap_or(Severity::Unknown),
        fix_version: row.get("fix_version")?,
        title: row.get("title")?,
        source_url: row.get("source_url")?,
        match_evidence: row
            .get::<_, Option<String>>("match_evidence_json")?
            .and_then(|value| serde_json::from_str(&value).ok()),
        state: parse_finding_state(row.get_ref("scanner_state")?.as_str()?),
        intel: None,
        review_state: ReviewState::Unreviewed,
        reviewed_at: None,
        snooze_until: None,
        review_note: None,
    })
}

pub(super) fn map_item(row: &Row<'_>) -> rusqlite::Result<ReviewFinding> {
    let finding = map_repo_finding(row)?;
    let severity = finding.severity;
    let advisory_rank = row.get::<_, u8>("risk_rank")?;
    let advisory_severity = if advisory_rank > severity_rank(&severity) {
        Severity::from_rank(advisory_rank)
    } else {
        None
    };
    let material_change: Option<String> = row.get("material_change")?;
    let known_exploited: bool = row.get("known_exploited")?;
    let ransomware_known: bool = row.get("ransomware_known")?;
    let mut reasons = Vec::with_capacity(4);
    if ransomware_known {
        reasons.push("Known ransomware use".into());
    } else if known_exploited {
        reasons.push("Known exploited vulnerability".into());
    }
    reasons.push(format!("{} severity", severity.as_str()));
    if let Some(advisory) = advisory_severity {
        reasons.push(format!("{} upstream advisory severity", advisory.as_str()));
    }
    if let Some(change) = &material_change {
        reasons.push(
            match change.as_str() {
                "reappeared" => "Reappeared after resolution",
                "severity_increased" => "Severity increased",
                "reopened" => "Returned to Review after risk changed",
                _ => "New since previous scan",
            }
            .into(),
        );
    }
    if finding.fix_version.is_some() {
        reasons.push("Fix available".into());
    }
    Ok(ReviewFinding {
        advisory_severity,
        repo_id: row.get("repo_id")?,
        repo_name: row.get("repo_name")?,
        repo_source_type: RepoSourceType::parse(&row.get::<_, String>("source_type")?),
        scan_run_id: row.get("evidence_id")?,
        scanned_at: row.get("scanned_at")?,
        finding,
        review_state: ReviewState::parse(&row.get::<_, String>("review_state")?),
        reviewed_at: row.get("reviewed_at")?,
        snooze_until: row.get("snooze_until")?,
        review_note: row.get("review_note")?,
        known_exploited,
        ransomware_known,
        material_change,
        priority_reasons: reasons,
    })
}
fn groups(
    connection: &Connection,
    sql: &str,
    clause: &str,
    values: &[Value],
) -> Result<(u32, ReviewGroupCounts), Error> {
    let mut counts = review_group_counts(&[]);
    let mut total = 0;
    let mut stmt = connection.prepare(&format!(
        r#"{sql}
        SELECT repo_id,repo_name,COUNT(*),
          COUNT(*) FILTER (WHERE review_state='unreviewed'),
          COUNT(*) FILTER (WHERE review_state='reviewed'),
          COUNT(*) FILTER (WHERE review_state='snoozed'),
          COUNT(*) FILTER (WHERE severity='CRITICAL'),
          COUNT(*) FILTER (WHERE severity='HIGH'),
          COUNT(*) FILTER (WHERE severity='MEDIUM'),
          COUNT(*) FILTER (WHERE severity='LOW'),
          COUNT(*) FILTER (WHERE severity='NEGLIGIBLE'),
          COUNT(*) FILTER (WHERE cvec_severity_rank(severity)<=1),
          COUNT(*) FILTER (WHERE fix_available),
          COUNT(*) FILTER (WHERE NOT fix_available)
        FROM review WHERE {clause} GROUP BY repo_id"#
    ))?;
    let mut rows = stmt.query(rusqlite::params_from_iter(values))?;
    while let Some(row) = rows.next()? {
        let repo_id: i64 = row.get(0)?;
        let count: u32 = row.get(2)?;
        total += count;
        counts.repository.push(ReviewLaneCount {
            id: format!("repository:{repo_id}"),
            label: row.get(1)?,
            count,
        });
        for (index, lane) in counts
            .review_state
            .iter_mut()
            .chain(&mut counts.severity)
            .chain(&mut counts.fix_availability)
            .enumerate()
        {
            lane.count += row.get::<_, u32>(index + 3)?;
        }
    }
    counts.repository.sort_by(|a, b| {
        a.label
            .to_lowercase()
            .cmp(&b.label.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok((total, counts))
}

#[derive(Debug)]
pub(super) struct CachedReviewCounts {
    revision: u64,
    filters: ReviewFilters,
    search: Option<String>,
    total: u32,
    group_counts: ReviewGroupCounts,
}

impl Database {
    fn review_page_counts(
        &self,
        connection: &Connection,
        query: &ReviewQuery,
        revision: u64,
        canonical: &str,
        clause: &str,
        values: &[Value],
    ) -> Result<(u32, ReviewGroupCounts), Error> {
        // Counts don't depend on sort or cursor. Keep just the last query and
        // match its exact filters against the revision of this read transaction.
        if let Some((total, counts)) = self.review_counts.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|entry| {
                    entry.revision == revision
                        && entry.filters == query.filters
                        && entry.search == query.search
                })
                .map(|entry| (entry.total, entry.group_counts.clone()))
        }) {
            return Ok((total, counts));
        }
        let (total, group_counts) = groups(
            connection,
            &count_relation(canonical, &query.filters),
            clause,
            values,
        )?;
        if let Ok(mut cache) = self.review_counts.lock() {
            // A slower read of an earlier revision should not evict newer counts.
            if cache
                .as_ref()
                .is_none_or(|entry| entry.revision <= revision)
            {
                *cache = Some(CachedReviewCounts {
                    revision,
                    filters: query.filters.clone(),
                    search: query.search.clone(),
                    total,
                    group_counts: group_counts.clone(),
                });
            }
        }
        Ok((total, group_counts))
    }

    pub fn performance_query_plans(&self, repo_id: i64) -> Result<Vec<String>, Error> {
        let connection = self.connect()?;
        let mut plans = Vec::new();
        for scope in [None, Some(repo_id)] {
            let sql = relation(scope);
            let mut statement=connection.prepare(&format!("EXPLAIN QUERY PLAN {sql} SELECT repo_id,fingerprint FROM review WHERE scanner_state<>'resolved' ORDER BY exploitation_rank DESC,risk_rank DESC,change_rank DESC,fix_available DESC,scanned_key DESC,repo_name,advisory_id,fingerprint,repo_id LIMIT 100"))?;
            for row in statement.query_map([], |row| row.get::<_, String>(3))? {
                plans.push(format!("scope={scope:?} {}", row?));
            }
        }
        Ok(plans)
    }

    pub fn search_local_entities(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<LocalEntityResult>, Error> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        if needle.chars().count() > 200 {
            return Err(Error::App(
                "local search must be 200 characters or fewer".into(),
            ));
        }
        // Search the same active and resolved findings Review can open.
        let connection = self.connect()?;
        let limit = limit.clamp(1, 50);
        let mut stmt = connection.prepare(&local_search_query())?;
        let rows = stmt.query_map(params![needle, limit], map_search_hit)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn has_due_snoozes(&self) -> Result<bool, Error> {
        Ok(self.connect()?.query_row("SELECT EXISTS(SELECT 1 FROM finding_reviews WHERE state='snoozed' AND snooze_until<=?1)",[now()],|r|r.get(0))?)
    }
    fn reconcile_review_transaction(&self) -> Result<(u32, u32), Error> {
        Self::reconcile_review_with_connection(&mut self.connect()?)
    }
    fn reconcile_review_with_connection(connection: &mut Connection) -> Result<(u32, u32), Error> {
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let result = Self::reconcile_review_on(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    pub(super) fn record_scan_review_changes(
        connection: &Connection,
        run_id: i64,
        changes: &ScanFindingChanges<'_>,
    ) -> Result<(), Error> {
        let mut insert = connection.prepare_cached(
            "INSERT INTO scan_review_changes (scan_run_id, fingerprint, kind) VALUES (?1, ?2, ?3)",
        )?;
        for (ids, kind) in [
            (&changes.new, "new"),
            (&changes.worsened, "severity_increased"),
        ] {
            for fingerprint in ids {
                insert.execute(params![run_id, fingerprint, kind])?;
            }
        }
        Ok(())
    }

    pub(super) fn reopen_scan_reviews(
        connection: &Connection,
        repo_id: i64,
        run_id: i64,
        affected: Option<&[String]>,
    ) -> Result<u32, Error> {
        let has_reviews: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM finding_reviews WHERE repo_id=?1)",
            [repo_id],
            |row| row.get(0),
        )?;
        if !has_reviews {
            return Ok(0);
        }
        let mut filter = " AND v.scan_run_id=?1 AND EXISTS(SELECT 1 FROM finding_reviews fr WHERE fr.repo_id=v.repo_id AND fr.fingerprint=v.fingerprint AND (fr.snooze_until IS NULL OR fr.snooze_until>?2))".to_owned();
        let mut params = vec![Value::Integer(run_id), Value::Text(now())];
        if let Some(affected) = affected {
            filter.push_str(" AND v.fingerprint IN (SELECT value FROM json_each(?3))");
            params.push(Value::Text(serde_json::to_string(affected)?));
        }
        let canonical = canonical_findings(Some(repo_id), FindingScope::Active, &filter, None);
        let facts = review_facts(&canonical, true, false);
        let changed = Self::material_review_changes(connection, &facts, &params)?;
        let mut insert = connection.prepare_cached(
            "INSERT INTO scan_review_changes (scan_run_id, fingerprint, kind) VALUES (?1, ?2, 'reopened')
             ON CONFLICT (scan_run_id, fingerprint) DO NOTHING",
        )?;
        let mut recorded = 0;
        for (_, fingerprint) in &changed {
            recorded += insert.execute(params![run_id, fingerprint])? as u32;
        }
        Self::delete_review_changes(connection, changed)?;
        Ok(recorded)
    }

    fn material_review_changes(
        connection: &Connection,
        facts: &str,
        params: &[Value],
    ) -> Result<Vec<(i64, String)>, Error> {
        let mut stmt = connection.prepare(&format!(
            "{facts} SELECT f.repo_id, f.fingerprint
             FROM facts f JOIN finding_reviews fr ON fr.repo_id=f.repo_id AND fr.fingerprint=f.fingerprint
             WHERE f.scanner_state<>'resolved' AND (
                {REVIEW_RISK_RANK} > cvec_severity_rank(fr.basis_severity)
                OR (fr.basis_known_exploited=0 AND f.exploitation_score%2=1)
                OR (fr.basis_ransomware_known=0 AND f.exploitation_score>=2)
                OR (f.fix_version IS NOT NULL AND f.fix_version IS NOT fr.basis_fix_version)
                OR fr.basis_scanner_state='resolved'
                OR (f.scanner_state='new' AND (fr.basis_scanner_state<>'new'
                    OR fr.updated_at<COALESCE(
                      (SELECT s.finished_at FROM scan_runs s WHERE s.id=f.recorded_scan_id),''))))"
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn delete_review_changes(
        connection: &Connection,
        changes: Vec<(i64, String)>,
    ) -> Result<u32, Error> {
        let mut delete = connection
            .prepare_cached("DELETE FROM finding_reviews WHERE repo_id=?1 AND fingerprint=?2")?;
        let mut reopened = 0;
        for (repo, fingerprint) in changes {
            reopened += delete.execute(params![repo, fingerprint])? as u32;
        }
        Ok(reopened)
    }

    fn reconcile_review_on(connection: &Connection) -> Result<(u32, u32), Error> {
        let due = Self::reopen_due_snoozes(connection)?;
        let (risk, checkpoint, _) = Self::review_reconciliation_status(connection)?;
        let mut reopened = due;
        if checkpoint.parse::<i64>().ok() != Some(risk) {
            let mut scope = " AND EXISTS (SELECT 1 FROM finding_reviews fr WHERE fr.repo_id=v.repo_id AND fr.fingerprint=v.fingerprint)".to_owned();
            if checkpoint != "-1" {
                scope.push_str(" AND (v.repo_id IN (SELECT repo_id FROM review_dirty_repos) OR lower(v.advisory_id) IN (SELECT identity FROM review_dirty_advisories))");
            }
            let canonical = canonical_findings(None, FindingScope::Active, &scope, None);
            let facts = review_facts(&canonical, true, false);
            let changed = Self::material_review_changes(connection, &facts, &[])?;
            reopened += Self::delete_review_changes(connection, changed)?;
            connection.execute(
                "UPDATE app_settings SET value=?1 WHERE key='review_reconciled_risk_revision'",
                [risk.to_string()],
            )?;
            connection.execute("DELETE FROM review_dirty_repos", [])?;
            connection.execute("DELETE FROM review_dirty_advisories", [])?;
        }
        Ok((due, reopened))
    }

    fn review_reconciliation_status(connection: &Connection) -> Result<(i64, String, bool), Error> {
        Ok(connection.query_row(
            "SELECT revision,
                    (SELECT value FROM app_settings WHERE key = 'review_reconciled_risk_revision'),
                    EXISTS(SELECT 1 FROM finding_reviews WHERE state = 'snoozed' AND snooze_until <= ?1)
             FROM data_revisions WHERE domain = 'risk'",
            [now()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?)
    }

    pub(super) fn read_reconciled_review<T>(
        &self,
        read: impl FnOnce(&Connection, u32, u32) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut connection = self.connect()?;
        let mut due = 0;
        let mut reopened = 0;
        for _ in 0..2 {
            let tx = connection.transaction()?;
            let (risk, checkpoint, has_due) = Self::review_reconciliation_status(&tx)?;
            if !has_due && checkpoint.parse::<i64>().ok() == Some(risk) {
                return read(&tx, due, reopened);
            }
            tx.rollback()?;
            let (expired, changed) = Self::reconcile_review_with_connection(&mut connection)?;
            due += expired;
            reopened += changed;
        }

        // If writes keep racing reconciliation, finish under one writer lock
        // so the returned revisions and review counts still agree.
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (expired, changed) = Self::reconcile_review_on(&tx)?;
        let result = read(&tx, due + expired, reopened + changed);
        tx.commit()?;
        result
    }

    pub fn reconcile_finding_reviews(&self) -> Result<u32, Error> {
        Ok(self.reconcile_review_transaction()?.1)
    }

    pub fn list_review_findings(&self, query: &ReviewQuery) -> Result<ReviewPage, Error> {
        self.list_review_findings_preserving(query, None, None)
    }

    pub(crate) fn list_review_findings_preserving(
        &self,
        query: &ReviewQuery,
        selected: Option<&str>,
        loaded_rows: Option<u32>,
    ) -> Result<ReviewPage, Error> {
        validate_review_query(query)?;
        self.read_reconciled_review(|connection, due, reopened| {
            self.list_review_findings_on(connection, query, selected, loaded_rows, due, reopened)
        })
    }

    fn list_review_findings_on(
        &self,
        connection: &Connection,
        query: &ReviewQuery,
        selected: Option<&str>,
        loaded_rows: Option<u32>,
        due: u32,
        reopened: u32,
    ) -> Result<ReviewPage, Error> {
        let revision = Self::read_data_revisions(connection)?.review;
        let signature = signature(query)?;
        let terms = query_ordering(query)?;
        let cursor=query.cursor.as_ref().map(|raw| {
            let cursor:Cursor=serde_json::from_str(raw.strip_prefix("r1:").ok_or_else(||Error::App("review cursor is invalid".into()))?).map_err(|_|Error::App("review cursor is invalid".into()))?;
            if cursor.revision!=revision || cursor.query!=signature {return Err(Error::App("REVIEW_CURSOR_STALE: Review results changed. Reload this view to continue.".into()));}
            if cursor.last.len()!=terms.len(){return Err(Error::App("review cursor is invalid".into()));}
            Ok(cursor)
        }).transpose()?;
        let repo_id = match query.filters.repo_ids.as_slice() {
            [id] => Some(*id),
            _ => None,
        };
        let mut values = Vec::new();
        let resolved_in_scan_run_id = query.filters.resolved_in_scan_run_id;
        let scope = if resolved_in_scan_run_id.is_some() {
            FindingScope::All
        } else {
            FindingScope::for_states(&query.filters.scanner_states)
        };
        let (canonical_scope, scan_filter) = if resolved_in_scan_run_id.is_some() {
            (FindingScope::Resolved, "")
        } else if !query.filters.scan_resolved_run_ids.is_empty() {
            // A resolution keeps its evidence scan ID from the previous run.
            // Filter by the run that established absence in the current view.
            values.push(Value::Text(serde_json::to_string(
                &query.filters.scan_resolved_run_ids,
            )?));
            (
                FindingScope::Resolved,
                " AND rr.latest_id IN (SELECT value FROM json_each(?1))",
            )
        } else if query.filters.scan_change_run_ids.is_empty() {
            (scope, "")
        } else if matches!(scope, FindingScope::Resolved) {
            (scope, " AND 0")
        } else {
            values.push(Value::Text(serde_json::to_string(
                &query.filters.scan_change_run_ids,
            )?));
            (
                FindingScope::Active,
                " AND v.id IN (
                    SELECT recorded.id FROM json_each(?1) requested
                    JOIN scan_review_changes change ON change.scan_run_id=requested.value
                    JOIN vulnerabilities recorded ON recorded.scan_run_id=change.scan_run_id
                      AND recorded.fingerprint=change.fingerprint
                )",
            )
        };
        let canonical = canonical_findings(
            repo_id,
            canonical_scope,
            scan_filter,
            resolved_in_scan_run_id,
        );
        let clause = filters(query, scope, &mut values)?;
        let (total, group_counts) =
            self.review_page_counts(connection, query, revision, &canonical, &clause, &values)?;
        // Match the maximum prefix retained by the Review session cache.
        const MAX_RESTORED_ROWS: usize = 2_000;
        const MIN_SELECTION_LOOKAHEAD: usize = 300;
        let page_size = query.page_size.unwrap_or(100) as usize;
        let restore_rows = if cursor.is_none() {
            page_size.max(
                loaded_rows
                    .unwrap_or_default()
                    .min(MAX_RESTORED_ROWS as u32) as usize,
            )
        } else {
            page_size
        };
        let selected = selected
            .filter(|_| cursor.is_none() && total > restore_rows as u32)
            .and_then(|key| key.split_once(':'))
            .and_then(|(repo, fingerprint)| {
                let repo = repo.parse::<i64>().ok()?;
                (repo > 0 && !fingerprint.is_empty() && fingerprint.len() <= 512)
                    .then_some((repo, fingerprint))
            });
        let fetch_limit = if selected.is_some() {
            // Read up to one further page when new findings move the selected
            // row. Small views retain the existing bounded selection search.
            (restore_rows + page_size).clamp(MIN_SELECTION_LOOKAHEAD, MAX_RESTORED_ROWS)
        } else {
            restore_rows
        };
        let (items, next_cursor) = if total == 0 {
            // Counts and rows use the same filters and read transaction.
            (Vec::new(), None)
        } else {
            let continuation = cursor
                .as_ref()
                .map(|c| format!(" AND ({})", keyset(&terms, &c.last, &mut values)))
                .unwrap_or_default();
            let order = terms
                .iter()
                .map(|(key, desc)| format!("{key} {}", if *desc { "DESC" } else { "ASC" }))
                .collect::<Vec<_>>()
                .join(",");
            values.push(Value::Integer((fetch_limit + 1) as i64));
            let limit = values.len();
            let sql = review_relation(&canonical, !query.filters.scan_change_run_ids.is_empty());
            let mut stmt = connection.prepare(&format!(
                "{sql} SELECT * FROM review WHERE {clause}{continuation} ORDER BY {order} LIMIT ?{limit}"
            ))?;
            let mut rows = stmt.query(rusqlite::params_from_iter(&values))?;
            let mut items = Vec::with_capacity(restore_rows + 1);
            let mut keys = Vec::with_capacity(restore_rows + 1);
            let mut selection_found = selected.is_none();
            let mut limit_rows = restore_rows;
            while let Some(row) = rows.next()? {
                let item = map_item(row)?;
                if !selection_found
                    && selected == Some((item.repo_id, item.finding.fingerprint.as_str()))
                {
                    selection_found = true;
                    limit_rows = restore_rows.max(items.len() + 1).min(fetch_limit);
                }
                let key = terms
                    .iter()
                    .map(|(column, _)| match row.get_ref(*column)? {
                        rusqlite::types::ValueRef::Integer(n) => Ok(CursorValue::Number(n)),
                        rusqlite::types::ValueRef::Text(s) => {
                            Ok(CursorValue::Text(String::from_utf8_lossy(s).into_owned()))
                        }
                        _ => Err(rusqlite::Error::InvalidQuery),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                items.push(item);
                keys.push(key);
                if selection_found && items.len() > limit_rows {
                    break;
                }
            }
            let next = if items.len() > limit_rows {
                Some(format!(
                    "r1:{}",
                    serde_json::to_string(&Cursor {
                        revision,
                        query: signature,
                        last: std::mem::take(&mut keys[limit_rows - 1]),
                    })?
                ))
            } else {
                None
            };
            // Preserve the loaded prefix when the selected row has left the
            // filtered results or moved beyond the bounded restoration range.
            items.truncate(limit_rows);
            (items, next)
        };
        let review_summary = if reopened > 0 {
            Some(self.review_summary_on(connection, due)?)
        } else {
            None
        };
        let valid_until = connection.query_row(
            "SELECT MIN(snooze_until) FROM finding_reviews WHERE state='snoozed'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )?;
        Ok(ReviewPage {
            valid_until,
            items,
            next_cursor,
            total,
            group_counts,
            revision,
            review_summary,
        })
    }
    pub(super) fn review_summary_on(
        &self,
        connection: &Connection,
        due: u32,
    ) -> Result<ReviewSummary, Error> {
        // Open and new findings have the same review counts. Only resolved
        // findings need evidence timestamps, and none of the counts use intel.
        let runs = review_runs("");
        let cutoff = timestamps::format(&(OffsetDateTime::now_utc() - Duration::days(30)));
        Ok(connection.query_row(&format!(r#"{runs}, open_counts AS (
          SELECT
            COUNT(*) FILTER (WHERE fr.state IS NULL) needs_review,
            COUNT(*) FILTER (WHERE fr.state IS NULL AND v.severity='CRITICAL') unreviewed_critical,
            COUNT(*) FILTER (WHERE fr.state='reviewed') open_reviewed,
            COUNT(*) FILTER (WHERE fr.state='snoozed') snoozed
          FROM repo_runs rr JOIN vulnerabilities v ON v.scan_run_id=rr.latest_id
          LEFT JOIN finding_reviews fr ON fr.repo_id=v.repo_id AND fr.fingerprint=v.fingerprint
        )
        SELECT open_counts.*,
          (SELECT COUNT(*)
           FROM (
             SELECT v.repo_id, v.fingerprint, v.scan_run_id recorded_scan_id, rr.preferred_evidence_id
             FROM repo_runs rr JOIN vulnerabilities v ON v.scan_run_id=rr.previous_id
             WHERE NOT {detected}
           ) c
           JOIN scan_runs evidence ON evidence.id={REVIEW_EVIDENCE_ID}
           WHERE julianday(evidence.finished_at)>=julianday(?1)
          ),
          (SELECT COUNT(*) FROM repo_runs rr WHERE
            (SELECT status FROM scan_runs WHERE repo_id=rr.repo_id ORDER BY started_at DESC,id DESC LIMIT 1)='failed'
          )
        FROM open_counts"#, detected = scan_has_finding("latest", "rr.latest_id", "v")), [cutoff], |row| {
            let needs_review: u32 = row.get(0)?;
            let unreviewed_critical: u32 = row.get(1)?;
            Ok(ReviewSummary {
                needs_review,
                unreviewed_critical,
                unreviewed_other: needs_review - unreviewed_critical,
                open_reviewed: row.get(2)?,
                snoozed: row.get(3)?,
                recently_resolved: row.get(4)?,
                due_snoozed: due,
                failed_scans: row.get(5)?,
            })
        })?)
    }
    pub fn review_summary(&self) -> Result<ReviewSummary, Error> {
        Ok(self.review_summary_with_revision()?.review_summary)
    }

    pub fn review_summary_with_revision(&self) -> Result<ReviewUpdate, Error> {
        self.read_reconciled_review(|connection, due, _| {
            let revision: i64 = connection.query_row(
                "SELECT revision FROM data_revisions WHERE domain = 'review'",
                [],
                |row| row.get(0),
            )?;
            Ok(ReviewUpdate {
                review_summary: self.review_summary_on(connection, due)?,
                revision: revision.max(0) as u64,
            })
        })
    }
    #[cfg(test)]
    pub(super) fn current_review_finding(
        &self,
        repo_id: i64,
        fingerprint: &str,
    ) -> Result<ReviewFinding, Error> {
        Self::current_review_finding_on(&self.connect()?, repo_id, fingerprint)
    }

    pub(super) fn current_review_finding_on(
        connection: &Connection,
        repo_id: i64,
        fingerprint: &str,
    ) -> Result<ReviewFinding, Error> {
        Self::find_current_review_finding_on(connection, repo_id, fingerprint)?
            .ok_or_else(|| Error::App("finding was not found in current or previous scan".into()))
    }

    pub(super) fn find_current_review_finding_on(
        connection: &Connection,
        repo_id: i64,
        fingerprint: &str,
    ) -> Result<Option<ReviewFinding>, Error> {
        let canonical = canonical_findings(
            Some(repo_id),
            FindingScope::All,
            " AND v.fingerprint=?1",
            None,
        );
        Ok(connection
            .query_row(
                &format!(
                    "{} SELECT * FROM review",
                    review_relation(&canonical, false)
                ),
                [fingerprint],
                map_item,
            )
            .optional()?)
    }
    pub fn review_finding_detail(
        &self,
        repo_id: i64,
        fingerprint: &str,
        evidence_scan_run_id: i64,
    ) -> Result<ReviewFinding, Error> {
        self.review_finding_detail_with_scope(repo_id, fingerprint, evidence_scan_run_id, None)
    }

    pub(crate) fn review_finding_detail_with_scope(
        &self,
        repo_id: i64,
        fingerprint: &str,
        evidence_scan_run_id: i64,
        resolved_in_scan_run_id: Option<i64>,
    ) -> Result<ReviewFinding, Error> {
        if fingerprint.is_empty()
            || fingerprint.len() > 512
            || repo_id <= 0
            || evidence_scan_run_id <= 0
            || resolved_in_scan_run_id.is_some_and(|id| id <= 0)
        {
            return Err(Error::App("invalid finding detail request".into()));
        }
        let scope = if resolved_in_scan_run_id.is_some() {
            FindingScope::Resolved
        } else {
            FindingScope::All
        };
        let canonical = canonical_findings(
            Some(repo_id),
            scope,
            " AND v.fingerprint=?1",
            resolved_in_scan_run_id,
        );
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let mut item = transaction
            .query_row(
                &format!(
                    "{} SELECT * FROM review WHERE evidence_id=?2",
                    review_relation(&canonical, false)
                ),
                params![fingerprint, evidence_scan_run_id],
                map_item,
            )
            .optional()?
            .ok_or_else(|| {
                Error::App(
                    "Finding evidence changed. Refresh this view before opening its details."
                        .into(),
                )
            })?;
        let json: Option<String> = transaction.query_row(
            "SELECT advisory_details_json FROM vulnerabilities WHERE repo_id=?1 AND fingerprint=?2 AND scan_run_id=?3",
            params![repo_id, fingerprint, evidence_scan_run_id],
            |r| r.get(0),
        )?;
        item.finding.intel = json
            .and_then(|j| serde_json::from_str::<super::super::models::AdvisoryDetails>(&j).ok())
            .map(|d| repo_finding_intel_from_advisory_details(&d));
        transaction.commit()?;
        Ok(item)
    }
    pub fn companion_repository_findings(
        &self,
        repository_id: i64,
    ) -> Result<Vec<ReviewFinding>, Error> {
        let connection = self.connect()?;
        let mut stmt=connection.prepare(&format!("{} SELECT * FROM review ORDER BY scanner_rank DESC,severity_rank DESC,package_name,advisory_id,fingerprint",relation(Some(repository_id))))?;
        let items = stmt
            .query_map([], map_item)?
            .collect::<Result<Vec<_>, _>>()?;
        items
            .into_iter()
            .map(|item| match item.scan_run_id {
                Some(id) => {
                    self.review_finding_detail(repository_id, &item.finding.fingerprint, id)
                }
                None => Ok(item),
            })
            .collect()
    }
}
