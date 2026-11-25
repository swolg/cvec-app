use super::super::models::{
    AdvisoryDetails, DependencyPage, DependencyQuery, RemediationOpportunity, RepoOverview,
    RepoOverviewMetadata,
};
use super::*;
use rusqlite::types::Value;
use std::hash::{Hash, Hasher};

use super::super::companion::{CompanionMention, ContextAttachmentKind};

const FIND_LATEST_INVENTORY_RUN: &str =
    "WITH latest AS (SELECT id FROM scan_runs WHERE repo_id=?1 AND status='completed'
     ORDER BY started_at DESC,id DESC LIMIT 1)";
const SELECTED_INVENTORY_RUN: &str = "WITH latest AS (SELECT ?2 id)";
const INVENTORY_RISKS: &str = r#"
, risks AS MATERIALIZED (
 SELECT COALESCE(package_url,cvec_package_identity(NULL,package_name,package_version,package_type)) identity,
   package_name name,package_version version,package_type,package_url,
   COUNT(*) finding_count,MAX(cvec_severity_rank(severity)) severity_rank,
   MAX(fix_version IS NOT NULL) fix_available
 FROM vulnerabilities WHERE repo_id=?1 AND scan_run_id=(SELECT id FROM latest) GROUP BY identity
)
"#;
// Use the existing repo/PURL index to find represented findings. Only packages
// without a PURL need a temporary identity index for the fallback lookup.
const INVENTORY_ROWS: &str = r#"
, identities_without_purl AS MATERIALIZED (
 SELECT DISTINCT cvec_package_identity(NULL,name,version,package_type) identity
 FROM scan_packages WHERE repo_id=?1 AND scan_run_id=(SELECT id FROM latest)
   AND package_url IS NULL
), inventory AS NOT MATERIALIZED (
 SELECT p.id, p.name, p.version, p.package_type, p.package_url,
        COALESCE(p.package_url,cvec_package_identity(NULL,p.name,p.version,p.package_type)) identity,
        COALESCE(r.finding_count,0) finding_count, COALESCE(r.severity_rank,0) severity_rank,
        COALESCE(r.fix_available,0) fix_available
 FROM scan_packages p LEFT JOIN risks r
   ON r.identity=COALESCE(p.package_url,cvec_package_identity(NULL,p.name,p.version,p.package_type))
 WHERE p.repo_id=?1 AND p.scan_run_id=(SELECT id FROM latest)
 UNION ALL
 SELECT -ROW_NUMBER() OVER (ORDER BY r.identity), r.name, r.version, r.package_type, r.package_url,
        r.identity, r.finding_count, r.severity_rank, r.fix_available
 FROM risks r
 WHERE NOT EXISTS (
   SELECT 1 FROM scan_packages p
   WHERE p.repo_id=?1 AND p.scan_run_id=(SELECT id FROM latest) AND p.package_url=r.identity
 ) AND NOT EXISTS (SELECT 1 FROM identities_without_purl p WHERE p.identity=r.identity)
)
"#;

pub(crate) struct ChatScanEvidence {
    pub latest_attempt: Option<ChatScan>,
    pub latest_completed: Option<ChatScan>,
    pub previous_completed: Option<ChatScan>,
}

pub(crate) struct ChatScan {
    pub id: i64,
    pub status: ScanRunStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub source_revision: Option<String>,
    pub source_dirty: Option<bool>,
    pub package_count: Option<u32>,
    pub evidence_complete: bool,
    pub new_count: u32,
    pub resolved_count: u32,
}

#[derive(Default)]
pub(crate) struct ChatFindingCounts {
    pub total_open: u32,
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub negligible: u32,
    pub unknown: u32,
    pub resolved: u32,
}

pub(crate) struct ChatEvidenceReader<'a> {
    connection: &'a Connection,
    pub repository: RepoRecord,
}

impl Database {
    pub(crate) fn read_chat_evidence<T>(
        &self,
        repo_id: i64,
        read: impl FnOnce(ChatEvidenceReader<'_>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut connection = self.connect()?;
        register_chat_relevance(&connection)?;
        let transaction = connection.transaction()?;
        let repository = transaction
            .query_row(
                &format!("{ACTIVE_REPOSITORIES} AND id=?1"),
                [repo_id],
                repo_record_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::App(format!("repository {repo_id} was not found")))?;
        let result = read(ChatEvidenceReader {
            connection: &transaction,
            repository,
        })?;
        transaction.commit()?;
        Ok(result)
    }
}

const CHAT_COMPLETED_RUNS: &str = "SELECT id FROM scan_runs
    WHERE repo_id=?1 AND status='completed' ORDER BY started_at DESC,id DESC";
const CHAT_LATEST_RUN: &str = "ORDER BY started_at DESC,id DESC LIMIT 1";
const CHAT_SCAN: &str = "SELECT id,status,started_at,finished_at,source_revision,source_dirty,
    package_count,evidence_complete,new_medium_count+new_high_count+new_critical_count+new_other_count,
    resolved_medium_count+resolved_high_count+resolved_critical_count+resolved_other_count FROM scan_runs";
const CHAT_FINDING_RUNS: &str = r#"
    WITH recent AS MATERIALIZED (
        SELECT id,started_at FROM scan_runs WHERE repo_id=?1 AND status='completed'
        ORDER BY started_at DESC,id DESC LIMIT 2
    ), runs AS (
        SELECT (SELECT id FROM recent ORDER BY started_at DESC,id DESC LIMIT 1) latest_id,
               (SELECT id FROM recent ORDER BY started_at DESC,id DESC LIMIT 1 OFFSET 1) previous_id
    )
"#;
const CHAT_FINDING_COLUMNS: &str =
    "v.fingerprint,v.advisory_id,v.package_name,v.package_version,v.package_type,
     v.package_url,v.severity,v.fix_version,v.title,v.source_url,v.advisory_details_json,
     fr.state,fr.reviewed_at,fr.snooze_until,fr.note,v.match_evidence_json";
const CHAT_FINDING_STATE: &str = r#"
    CASE WHEN v.scan_run_id=r.previous_id THEN 'resolved'
         WHEN NOT (EXISTS(SELECT 1 FROM scan_runs p WHERE p.id=r.previous_id AND p.evidence_complete=1)
           OR EXISTS(SELECT 1 FROM vulnerabilities p WHERE p.scan_run_id=r.previous_id))
           OR EXISTS(SELECT 1 FROM vulnerabilities p
                     WHERE p.scan_run_id=r.previous_id AND p.fingerprint=v.fingerprint)
         THEN 'open' ELSE 'new' END scanner_state
"#;

impl ChatEvidenceReader<'_> {
    pub(crate) fn latest_scan(&self) -> Result<Option<ChatScan>, Error> {
        Ok(self
            .connection
            .query_row(
                &format!("{CHAT_SCAN} WHERE repo_id=?1 {CHAT_LATEST_RUN}"),
                [self.repository.id],
                chat_scan_from_row,
            )
            .optional()?)
    }

    pub(crate) fn scans(&self) -> Result<ChatScanEvidence, Error> {
        let mut statement = self.connection.prepare(&format!(
            "{CHAT_SCAN} WHERE id IN ({CHAT_COMPLETED_RUNS} LIMIT 2) ORDER BY started_at DESC,id DESC"
        ))?;
        let mut completed = statement.query([self.repository.id])?;
        let latest_completed = completed.next()?.map(chat_scan_from_row).transpose()?;
        let previous_completed = completed.next()?.map(chat_scan_from_row).transpose()?;
        let latest_attempt = self
            .latest_scan()?
            .filter(|scan| scan.status != ScanRunStatus::Completed);
        Ok(ChatScanEvidence {
            latest_attempt,
            latest_completed,
            previous_completed,
        })
    }

    pub(crate) fn scan_history(&self) -> Result<Vec<ChatScan>, Error> {
        let mut stmt = self.connection.prepare(&format!(
            "{CHAT_SCAN} WHERE id IN (
                SELECT id FROM (SELECT id FROM scan_runs WHERE repo_id=?1
                                ORDER BY started_at DESC,id DESC LIMIT 12)
                UNION SELECT id FROM ({CHAT_COMPLETED_RUNS} LIMIT 1)
            )"
        ))?;
        let rows = stmt.query_map([self.repository.id], chat_scan_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn scan(&self, run_id: i64) -> Result<Option<ChatScan>, Error> {
        Ok(self
            .connection
            .query_row(
                &format!("{CHAT_SCAN} WHERE repo_id=?1 AND id=?2"),
                params![self.repository.id, run_id],
                chat_scan_from_row,
            )
            .optional()?)
    }

    pub(crate) fn completed_scan(&self, offset: u32) -> Result<Option<ChatScan>, Error> {
        Ok(self
            .connection
            .query_row(
                &format!("{CHAT_SCAN} WHERE id=({CHAT_COMPLETED_RUNS} LIMIT 1 OFFSET ?2)"),
                params![self.repository.id, offset],
                chat_scan_from_row,
            )
            .optional()?)
    }

    pub(crate) fn finding_counts_for_scan(
        &self,
        scan: &ChatScan,
    ) -> Result<Option<ChatFindingCounts>, Error> {
        if scan.status != ScanRunStatus::Completed {
            return Ok(None);
        }
        let previous_id = self
            .connection
            .query_row(
                "SELECT id FROM scan_runs
             WHERE repo_id=?1 AND status='completed' AND (started_at,id)<(?2,?3)
             ORDER BY started_at DESC,id DESC LIMIT 1",
                params![self.repository.id, scan.started_at, scan.id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(Some(self.finding_counts(scan.id, previous_id)?))
    }

    pub(crate) fn finding(&self, fingerprint: &str) -> Result<Option<RepoFinding>, Error> {
        let mut rows = self.findings(&[fingerprint], &[], 0)?;
        Ok(rows.pop())
    }

    pub(crate) fn finding_in_scan(
        &self,
        run_id: i64,
        fingerprint: &str,
    ) -> Result<Option<(RepoFinding, ChatScan)>, Error> {
        let Some(scan) = self
            .scan(run_id)?
            .filter(|scan| scan.status == ScanRunStatus::Completed)
        else {
            return Ok(None);
        };
        let finding = self.connection.query_row(
            &format!(
                "SELECT {CHAT_FINDING_COLUMNS}, 'open' FROM vulnerabilities v
                 LEFT JOIN finding_reviews fr ON fr.repo_id=v.repo_id AND fr.fingerprint=v.fingerprint
                 WHERE v.repo_id=?1 AND v.scan_run_id=?2 AND v.fingerprint=?3"
            ),
            params![self.repository.id, run_id, fingerprint],
            chat_finding_from_row,
        ).optional()?;
        Ok(finding.map(|finding| (finding, scan)))
    }

    pub(crate) fn advisory_finding(&self, key: &str) -> Result<Option<RepoFinding>, Error> {
        let mut statement = self.connection.prepare(&format!(
            r#"{CHAT_FINDING_RUNS}
            SELECT {CHAT_FINDING_COLUMNS}, {CHAT_FINDING_STATE}
            FROM runs r CROSS JOIN vulnerabilities v
            LEFT JOIN finding_reviews fr ON fr.repo_id=v.repo_id AND fr.fingerprint=v.fingerprint
            WHERE v.repo_id=?1
              AND (v.scan_run_id=r.latest_id OR (
                  v.scan_run_id=r.previous_id AND NOT {detected}
              ))
              AND CASE WHEN json_valid(v.advisory_details_json) THEN
                COALESCE(json_extract(v.advisory_details_json,'$.ghsaId'),
                         json_extract(v.advisory_details_json,'$.cveId'),
                         json_extract(v.advisory_details_json,'$.url'))=?2
              ELSE 0 END
            ORDER BY (v.scan_run_id=r.latest_id) DESC,
                     cvec_severity_rank(v.severity) DESC,
                     v.package_name,v.advisory_id,v.fingerprint
            "#,
            detected = scan_has_finding("current", "r.latest_id", "v"),
        ))?;
        let mut rows = statement.query(params![self.repository.id, key])?;
        while let Some(row) = rows.next()? {
            let finding = chat_finding_from_row(row)?;
            if finding.intel.is_some() {
                return Ok(Some(finding));
            }
        }
        Ok(None)
    }

    pub(crate) fn findings(
        &self,
        explicit: &[&str],
        terms: &[String],
        limit: usize,
    ) -> Result<Vec<RepoFinding>, Error> {
        if limit == 0 && explicit.is_empty() {
            return Ok(Vec::new());
        }
        let mut statement = self.connection.prepare(&format!(
            r#"{CHAT_FINDING_RUNS}, selected AS MATERIALIZED (
                SELECT v.id FROM runs r CROSS JOIN vulnerabilities v
                WHERE v.repo_id=?1 AND v.scan_run_id=r.latest_id AND ?4>0
                ORDER BY cvec_chat_relevance(v.advisory_id||' '||v.package_name||' '||
                           v.fingerprint||' '||COALESCE(v.title,'')||' '||
                           v.severity,?3) DESC,
                         cvec_severity_rank(v.severity) DESC,
                         v.fingerprint
                LIMIT ?4
            ), exact AS (
                SELECT v.id FROM runs r CROSS JOIN json_each(?2) requested
                CROSS JOIN vulnerabilities v ON v.scan_run_id=r.latest_id AND v.fingerprint=requested.value
                WHERE v.repo_id=?1
                UNION ALL
                SELECT v.id FROM runs r CROSS JOIN json_each(?2) requested
                CROSS JOIN vulnerabilities v ON v.scan_run_id=r.previous_id AND v.fingerprint=requested.value
                WHERE v.repo_id=?1 AND NOT {detected}
            )
            SELECT {CHAT_FINDING_COLUMNS}, {CHAT_FINDING_STATE}
            FROM (SELECT id FROM selected UNION SELECT id FROM exact) chosen
            JOIN vulnerabilities v ON v.id=chosen.id
            CROSS JOIN runs r
            LEFT JOIN finding_reviews fr ON fr.repo_id=v.repo_id AND fr.fingerprint=v.fingerprint
            ORDER BY v.id NOT IN (SELECT id FROM selected),v.fingerprint
            "#,
            detected = scan_has_finding("current", "r.latest_id", "v"),
        ))?;
        let rows = statement.query_map(
            params![
                self.repository.id,
                serde_json::to_string(explicit)?,
                serde_json::to_string(terms)?,
                limit as i64
            ],
            chat_finding_from_row,
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn finding_mentions(
        &self,
        needle: &str,
        limit: usize,
    ) -> Result<Vec<CompanionMention>, Error> {
        let mut statement = self.connection.prepare(&format!(
            r#"{CHAT_FINDING_RUNS}, candidates AS (
                SELECT v.fingerprint,v.advisory_id,v.package_name,v.package_version,v.severity
                FROM runs r CROSS JOIN vulnerabilities v
                WHERE v.repo_id=?1 AND v.scan_run_id=r.latest_id
                UNION ALL
                SELECT v.fingerprint,v.advisory_id,v.package_name,v.package_version,v.severity
                FROM runs r CROSS JOIN vulnerabilities v
                WHERE v.repo_id=?1 AND v.scan_run_id=r.previous_id
                  AND NOT {detected}
            )
            SELECT * FROM candidates
            WHERE ?2='' OR cvec_contains(advisory_id||' '||package_name||' '||fingerprint,?2)
            ORDER BY advisory_id,fingerprint LIMIT ?3"#,
            detected = scan_has_finding("current", "r.latest_id", "v"),
        ))?;
        let rows =
            statement.query_map(params![self.repository.id, needle, limit as i64], |row| {
                let version: Option<String> = row.get(3)?;
                let severity =
                    Severity::parse(row.get_ref(4)?.as_str()?).unwrap_or(Severity::Unknown);
                Ok(CompanionMention {
                    id: row.get(0)?,
                    kind: ContextAttachmentKind::Finding,
                    label: row.get(1)?,
                    detail: format!(
                        "{} · {} · {}",
                        row.get_ref(2)?.as_str()?,
                        version.as_deref().unwrap_or("unknown version"),
                        severity.as_str()
                    ),
                    available: true,
                    unavailable_reason: None,
                })
            })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn packages(
        &self,
        explicit: &[&str],
        terms: &[String],
        limit: usize,
    ) -> Result<(Vec<RepoPackage>, u32), Error> {
        if limit == 0 && explicit.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let mut statement = self.connection.prepare(&format!(
            r#"{FIND_LATEST_INVENTORY_RUN}{INVENTORY_RISKS}{INVENTORY_ROWS},
            chat_packages AS MATERIALIZED (
                SELECT *,COALESCE(package_url, COALESCE(package_type,'unknown')||':'||name||':'||COALESCE(version,'unknown')) chat_id
                FROM inventory
            ), ranked AS (
                SELECT id FROM chat_packages WHERE ?4>0
                ORDER BY cvec_chat_relevance(name||' '||COALESCE(version,'')||' '||
                         COALESCE(package_type,'')||' '||COALESCE(package_url,''),?3) DESC,
                         severity_rank DESC,name,finding_count DESC,version,identity,id
                LIMIT ?4
            ), selected AS (
                SELECT id FROM ranked
                UNION
                SELECT (SELECT id FROM chat_packages p WHERE p.chat_id=requested.value
                        ORDER BY severity_rank DESC,finding_count DESC,name,version,identity,id LIMIT 1)
                FROM json_each(?2) requested
            )
            SELECT name,version,package_type,package_url,severity_rank,finding_count,fix_available,
                   CASE WHEN ?4>0 THEN (SELECT COUNT(*) FROM chat_packages) ELSE 0 END total
            FROM chat_packages
            WHERE id IN (SELECT id FROM selected)
            ORDER BY id NOT IN (SELECT id FROM ranked),severity_rank DESC,finding_count DESC,name,version,identity,id
            "#,
        ))?;
        let mut rows = statement.query(params![
            self.repository.id,
            serde_json::to_string(explicit)?,
            serde_json::to_string(terms)?,
            limit as i64
        ])?;
        let mut packages = Vec::new();
        let mut total = 0;
        while let Some(row) = rows.next()? {
            packages.push(package_from_row(row, 0)?);
            total = row.get("total")?;
        }
        Ok((packages, total))
    }

    pub(crate) fn package_mentions(
        &self,
        needle: &str,
        limit: usize,
    ) -> Result<Vec<CompanionMention>, Error> {
        let mut statement = self.connection.prepare(&format!(
            r#"{FIND_LATEST_INVENTORY_RUN}{INVENTORY_RISKS}{INVENTORY_ROWS}
            SELECT name,version,package_type,package_url,severity_rank,finding_count,fix_available
            FROM inventory
            WHERE ?2='' OR cvec_contains(name||' '||COALESCE(version,'')||' '||COALESCE(package_type,''),?2)
            ORDER BY name,COALESCE(package_url,COALESCE(package_type,'unknown')||':'||name||':'||COALESCE(version,'unknown')),
                     severity_rank DESC,finding_count DESC,version,id LIMIT ?3"#,
        ))?;
        let rows =
            statement.query_map(params![self.repository.id, needle, limit as i64], |row| {
                let package = package_from_row(row, 0)?;
                Ok(CompanionMention {
                    id: super::super::companion_context::package_identity(&package),
                    kind: ContextAttachmentKind::Package,
                    label: package.name,
                    detail: format!(
                        "{} · {} findings",
                        package.version.as_deref().unwrap_or("unknown version"),
                        package.finding_count
                    ),
                    available: true,
                    unavailable_reason: None,
                })
            })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn latest_finding_counts(
        &self,
        scans: &ChatScanEvidence,
    ) -> Result<ChatFindingCounts, Error> {
        match scans.latest_completed.as_ref() {
            Some(scan) => self.finding_counts(
                scan.id,
                scans
                    .previous_completed
                    .as_ref()
                    .map(|previous| previous.id),
            ),
            None => Ok(ChatFindingCounts::default()),
        }
    }

    fn finding_counts(
        &self,
        scan_id: i64,
        previous_id: Option<i64>,
    ) -> Result<ChatFindingCounts, Error> {
        let repo_id = self.repository.id;
        let mut counts = self.connection.query_row(
            "SELECT COUNT(*), COUNT(*) FILTER (WHERE severity='CRITICAL' COLLATE NOCASE),
                    COUNT(*) FILTER (WHERE severity='HIGH' COLLATE NOCASE),
                    COUNT(*) FILTER (WHERE severity='MEDIUM' COLLATE NOCASE),
                    COUNT(*) FILTER (WHERE severity='LOW' COLLATE NOCASE),
                    COUNT(*) FILTER (WHERE severity='NEGLIGIBLE' COLLATE NOCASE)
             FROM vulnerabilities WHERE repo_id=?1 AND scan_run_id=?2",
            params![repo_id, scan_id],
            |row| {
                let total_open = row.get::<_, u32>(0)?;
                let critical = row.get::<_, u32>(1)?;
                let high = row.get::<_, u32>(2)?;
                let medium = row.get::<_, u32>(3)?;
                let low = row.get::<_, u32>(4)?;
                let negligible = row.get::<_, u32>(5)?;
                Ok(ChatFindingCounts {
                    total_open,
                    critical,
                    medium,
                    high,
                    low,
                    negligible,
                    unknown: total_open - critical - high - medium - low - negligible,
                    resolved: 0,
                })
            },
        )?;
        let (medium, high, critical, other) =
            Database::scan_resolution_counts(self.connection, previous_id, scan_id)?;
        counts.resolved = medium + high + critical + other;
        Ok(counts)
    }
}

fn chat_scan_from_row(row: &Row<'_>) -> rusqlite::Result<ChatScan> {
    Ok(ChatScan {
        id: row.get(0)?,
        status: ScanRunStatus::parse(row.get_ref(1)?.as_str()?),
        started_at: row.get(2)?,
        finished_at: row.get(3)?,
        source_revision: row.get(4)?,
        source_dirty: row.get(5)?,
        package_count: row.get(6)?,
        evidence_complete: row.get(7)?,
        new_count: row.get(8)?,
        resolved_count: row.get(9)?,
    })
}

fn chat_finding_from_row(row: &Row<'_>) -> rusqlite::Result<RepoFinding> {
    let state = row.get_ref(16)?.as_str()?;
    let advisory = match row.get_ref(10)? {
        rusqlite::types::ValueRef::Text(bytes) => {
            serde_json::from_slice::<AdvisoryDetails>(bytes).ok()
        }
        _ => None,
    };
    Ok(RepoFinding {
        fingerprint: row.get(0)?,
        advisory_id: row.get(1)?,
        package_name: row.get(2)?,
        package_version: row.get(3)?,
        package_type: row.get(4)?,
        package_url: row.get(5)?,
        severity: Severity::parse(row.get_ref(6)?.as_str()?).unwrap_or(Severity::Unknown),
        fix_version: row.get(7)?,
        title: row.get(8)?,
        source_url: row.get(9)?,
        match_evidence: row
            .get::<_, Option<String>>(15)?
            .and_then(|value| serde_json::from_str(&value).ok()),
        state: match state {
            "resolved" => RepoFindingState::Resolved,
            "new" => RepoFindingState::New,
            _ => RepoFindingState::Open,
        },
        intel: advisory
            .as_ref()
            .map(repo_finding_intel_from_advisory_details),
        review_state: row
            .get::<_, Option<String>>(11)?
            .as_deref()
            .map(ReviewState::parse)
            .unwrap_or(ReviewState::Unreviewed),
        reviewed_at: row.get(12)?,
        snooze_until: row.get(13)?,
        review_note: row.get(14)?,
    })
}

fn package_from_row(row: &Row<'_>, start: usize) -> rusqlite::Result<RepoPackage> {
    Ok(RepoPackage {
        name: row.get(start)?,
        version: row.get(start + 1)?,
        package_type: row.get(start + 2)?,
        package_url: row.get(start + 3)?,
        highest_severity: Severity::from_rank(row.get(start + 4)?),
        finding_count: row.get(start + 5)?,
        fix_available: row.get(start + 6)?,
    })
}

fn register_chat_relevance(connection: &Connection) -> Result<(), Error> {
    use rusqlite::functions::FunctionFlags;
    connection.create_scalar_function(
        "cvec_chat_relevance",
        2,
        FunctionFlags::SQLITE_DETERMINISTIC,
        |context| {
            let terms = context.get_or_create_aux(1, |value| {
                serde_json::from_slice::<Vec<String>>(value.as_bytes()?)
                    .map_err(|cause| rusqlite::Error::UserFunctionError(Box::new(cause)))
            })?;
            let candidate = context.get_raw(0).as_str().map_err(|_| {
                rusqlite::Error::InvalidFunctionParameterType(0, context.get_raw(0).data_type())
            })?;
            Ok(super::super::companion_context::query_relevance(
                candidate, &terms,
            ))
        },
    )?;
    Ok(())
}

fn remediation_opportunities(
    connection: &Connection,
    repo_id: i64,
    run_id: i64,
) -> Result<(u32, Vec<RemediationOpportunity>), Error> {
    let mut statement = connection.prepare(
        r#"
        WITH opportunities AS (
            SELECT COALESCE(package_url, COALESCE(package_type,'unknown')||':'||package_name) identity,
                   package_name, package_type, package_url, package_version, fix_version,
                   COUNT(*) finding_count, SUM(severity='CRITICAL') critical,
                   SUM(severity='HIGH') high, SUM(severity='MEDIUM') medium,
                   MIN(fingerprint) fingerprint
            FROM vulnerabilities
            WHERE repo_id=?1 AND scan_run_id=?2 AND fix_version IS NOT NULL
            GROUP BY identity, package_version, fix_version
        )
        SELECT *, COUNT(*) OVER () AS total
        FROM opportunities
        ORDER BY critical DESC, high DESC, finding_count DESC, package_name, fix_version,
                 identity, package_version
        LIMIT 5
        "#,
    )?;
    let mut rows = statement.query(params![repo_id, run_id])?;
    let mut total = 0;
    let mut opportunities = Vec::new();
    while let Some(row) = rows.next()? {
        let identity: String = row.get("identity")?;
        let installed_version: Option<String> = row.get("package_version")?;
        let fix_version: String = row.get("fix_version")?;
        total = row.get("total")?;
        opportunities.push(RemediationOpportunity {
            key: format!(
                "{identity}|{}|{fix_version}",
                installed_version.as_deref().unwrap_or("unknown"),
            ),
            package_name: row.get("package_name")?,
            package_type: row.get("package_type")?,
            package_url: row.get("package_url")?,
            installed_version,
            fix_version: Some(fix_version),
            finding_count: row.get("finding_count")?,
            critical: row.get("critical")?,
            high: row.get("high")?,
            medium: row.get("medium")?,
            fingerprints: vec![row.get("fingerprint")?],
        });
    }
    Ok((total, opportunities))
}

fn inventory_counts(
    connection: &Connection,
    repo_id: i64,
    run_id: i64,
) -> Result<(u32, u32, u32), Error> {
    let has_findings = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM vulnerabilities WHERE repo_id=?1 AND scan_run_id=?2)",
        params![repo_id, run_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !has_findings {
        let packages = connection.query_row(
            "SELECT COUNT(*) FROM scan_packages WHERE repo_id=?1 AND scan_run_id=?2",
            params![repo_id, run_id],
            |row| row.get(0),
        )?;
        return Ok((packages, 0, 0));
    }

    // Each stored package row counts. Add one row for each finding identity
    // absent from that inventory, matching the Dependencies page.
    Ok(connection.query_row(
        &format!(
            "{SELECTED_INVENTORY_RUN}{INVENTORY_RISKS}, scanned AS (
               SELECT COUNT(*) packages, COUNT(r.identity) affected,
                      COUNT(DISTINCT r.identity) matched_identities
               FROM scan_packages p LEFT JOIN risks r
                 ON r.identity=cvec_package_identity(p.package_url,p.name,p.version,p.package_type)
               WHERE p.repo_id=?1 AND p.scan_run_id=(SELECT id FROM latest)
             ), findings AS (
               SELECT COUNT(*) identities, COALESCE(SUM(finding_count),0) total FROM risks
             )
             SELECT packages + identities - matched_identities,
                    affected + identities - matched_identities,
                    total
             FROM scanned CROSS JOIN findings"
        ),
        params![repo_id, run_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?)
}

#[derive(Clone, Debug)]
struct DependencyCounts {
    total: u32,
    inventory_count: u32,
    affected_count: u32,
    ecosystems: Vec<String>,
}

#[derive(Debug)]
pub(super) struct CachedDependencyCounts {
    revision: u64,
    predicate: String,
    parameters: Vec<Value>,
    counts: DependencyCounts,
}

impl Database {
    pub fn repo_overview(&self, repo_id: i64) -> Result<RepoOverview, Error> {
        const HISTORY_LIMIT: usize = 12;
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let summary = self.active_repo_summary(&tx, repo_id)?;
        let scan_history = self.repo_scan_history(&tx, repo_id, HISTORY_LIMIT)?;
        let mut completed = scan_history
            .iter()
            .filter(|run| run.status == ScanRunStatus::Completed);
        let mut latest_completed_scan = completed.next().cloned();
        let mut previous_scan = completed.next().cloned();
        if previous_scan.is_none() && scan_history.len() == HISTORY_LIMIT {
            let mut completed = self
                .repo_scan_history_filtered(&tx, repo_id, 2, true)?
                .into_iter();
            latest_completed_scan = completed.next();
            previous_scan = completed.next();
        }
        let mut metadata = RepoOverviewMetadata {
            latest_completed_scan,
            package_count: 0,
            affected_count: 0,
            open_finding_count: 0,
            opportunity_count: 0,
            opportunities: Vec::new(),
        };
        let mut findings = Vec::new();
        let mut nuget_projects = Vec::new();
        if let Some(scan) = metadata.latest_completed_scan.as_ref() {
            let projects: String = tx.query_row(
                "SELECT nuget_projects_json FROM scan_runs WHERE repo_id = ?1 AND id = ?2",
                params![repo_id, scan.id],
                |row| row.get(0),
            )?;
            nuget_projects = serde_json::from_str(&projects)?;
            (
                metadata.package_count,
                metadata.affected_count,
                metadata.open_finding_count,
            ) = inventory_counts(&tx, repo_id, scan.id)?;
            if metadata.open_finding_count > 0 {
                (metadata.opportunity_count, metadata.opportunities) =
                    remediation_opportunities(&tx, repo_id, scan.id)?;
                findings = super::review::repo_finding_preview(
                    &tx,
                    repo_id,
                    scan.id,
                    previous_scan.as_ref().map(|run| run.id),
                )?;
            }
        }
        tx.commit()?;
        Ok(RepoOverview {
            summary,
            latest_scan: scan_history.first().cloned(),
            previous_scan,
            findings,
            scan_history,
            overview: metadata,
            nuget_projects,
        })
    }
    pub fn dependency_page(
        &self,
        repo_id: i64,
        query: &DependencyQuery,
    ) -> Result<DependencyPage, Error> {
        if query.page_size.is_some_and(|n| n == 0 || n > 200)
            || query
                .search
                .as_ref()
                .is_some_and(|s| s.chars().count() > 200)
            || query.cursor.as_ref().is_some_and(|s| s.len() > 256)
        {
            return Err(Error::App("invalid dependency query".into()));
        }
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let (revision, scan_run_id) = tx
            .query_row(
                r#"SELECT COALESCE(rev.dependency_revision,0),
                   (SELECT id FROM scan_runs WHERE repo_id=r.id AND status='completed' ORDER BY started_at DESC,id DESC LIMIT 1)
                   FROM repos r LEFT JOIN repository_data_revisions rev ON rev.repo_id=r.id
                   WHERE r.id=?1 AND r.is_active=1"#,
                [repo_id],
                |row| Ok((row.get::<_, i64>(0)?.max(0) as u64, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::App(format!("repository {repo_id} was not found")))?;
        if query.expected_scan_run_id.is_some() && query.expected_scan_run_id != scan_run_id {
            return Err(Error::App(
                "DEPENDENCY_CURSOR_STALE: Scan evidence changed. Refresh this repository.".into(),
            ));
        }
        let mut canonical = query.clone();
        canonical.cursor = None;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        repo_id.hash(&mut hash);
        scan_run_id.hash(&mut hash);
        serde_json::to_string(&canonical)?.hash(&mut hash);
        let signature = hash.finish();
        let offset = if let Some(cursor) = &query.cursor {
            let fields = cursor.split(':').collect::<Vec<_>>();
            if fields.len() != 4 || fields[0] != "d1" {
                return Err(Error::App("invalid dependency cursor".into()));
            }
            if fields[1].parse::<u64>().ok() != Some(revision)
                || fields[2].parse::<u64>().ok() != Some(signature)
            {
                return Err(Error::App(
                    "DEPENDENCY_CURSOR_STALE: Scan evidence changed. Refresh this repository."
                        .into(),
                ));
            }
            fields[3]
                .parse::<u32>()
                .map_err(|_| Error::App("invalid dependency cursor".into()))?
        } else {
            0
        };
        let mut values = vec![
            Value::Integer(repo_id),
            scan_run_id.map_or(Value::Null, Value::Integer),
        ];
        let mut predicates = Vec::new();
        if let Some(package) = &query.exact_package {
            values.push(Value::Text(package_identity(
                package.package_url.as_deref(),
                &package.name,
                package.version.as_deref(),
                package.package_type.as_deref(),
            )));
            values.push(package.version.clone().map_or(Value::Null, Value::Text));
            predicates.push(format!(
                "identity=?{} AND version IS ?{}",
                values.len() - 1,
                values.len()
            ));
        }
        if let Some(search) = query
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            values.push(Value::Text(search.into()));
            predicates.push(format!("cvec_contains(name||' '||COALESCE(version,'')||' '||COALESCE(package_type,'')||' '||COALESCE(package_url,''),?{})",values.len()));
        }
        if let Some(ecosystem) = &query.ecosystem {
            values.push(Value::Text(ecosystem.clone()));
            predicates.push(format!("package_type=?{}", values.len()));
        }
        if let Some(severity) = query.severity {
            values.push(Value::Integer(severity_rank(&severity) as i64));
            predicates.push(format!("severity_rank=?{}", values.len()));
        }
        if query.affected_only == Some(true) {
            predicates.push("finding_count>0".into());
        }
        if let Some(fix) = query.fix_available {
            values.push(Value::Integer(i64::from(fix)));
            predicates.push(format!("fix_available=?{}", values.len()));
        }
        let clause = if predicates.is_empty() {
            "1".into()
        } else {
            predicates.join(" AND ")
        };
        let order = match query.sort.as_deref().unwrap_or("risk") {
            "risk" => "finding_count DESC,severity_rank DESC,name,identity,id",
            "name" => "name,identity,id",
            "ecosystem" => "COALESCE(package_type,''),name,identity,id",
            _ => return Err(Error::App("invalid dependency sort".into())),
        };
        let filter_parameter_count = values.len();
        let cached_counts = self.dependency_counts.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|entry| {
                    entry.revision == revision
                        && entry.predicate == clause
                        && entry.parameters == values
                })
                .map(|entry| entry.counts.clone())
        });
        let cache_miss = cached_counts.is_none();
        values.push(Value::Integer(i64::from(query.page_size.unwrap_or(80))));
        values.push(Value::Integer(offset as i64));
        let limit = format!("LIMIT ?{} OFFSET ?{}", values.len() - 1, values.len());
        let (counts, items) = if let Some(counts) = cached_counts {
            let mut stmt = tx.prepare(&format!(
                r#"{SELECTED_INVENTORY_RUN}{INVENTORY_RISKS}{INVENTORY_ROWS}
                SELECT name,version,package_type,package_url,severity_rank,finding_count,fix_available
                FROM inventory WHERE {clause} ORDER BY {order} {limit}"#
            ))?;
            let items = stmt
                .query_map(rusqlite::params_from_iter(&values), |row| {
                    package_from_row(row, 0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            (counts, items)
        } else {
            // Share one inventory and one filter evaluation across the counts
            // and page. The totals row also survives an empty page.
            let mut stmt = tx.prepare(&format!(
                r#"{SELECTED_INVENTORY_RUN}{INVENTORY_RISKS}{INVENTORY_ROWS},
                scored AS MATERIALIZED (SELECT *, ({clause}) matches FROM inventory),
                totals AS (
                  SELECT COUNT(*) FILTER (WHERE matches) total,
                    COUNT(*) inventory_count, COUNT(*) FILTER (WHERE finding_count>0) affected_count,
                    json_group_array(DISTINCT package_type ORDER BY package_type)
                      FILTER (WHERE package_type IS NOT NULL) ecosystems
                  FROM scored
                ), page AS (
                  SELECT * FROM scored WHERE matches ORDER BY {order} {limit}
                )
                SELECT total,inventory_count,affected_count,ecosystems,
                  page.name,page.version,page.package_type,page.package_url,page.severity_rank,page.finding_count,page.fix_available
                FROM totals LEFT JOIN page ON 1 ORDER BY {order}"#
            ))?;
            let mut rows = stmt.query(rusqlite::params_from_iter(&values))?;
            let mut row = rows.next()?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            let counts = DependencyCounts {
                total: row.get(0)?,
                inventory_count: row.get(1)?,
                affected_count: row.get(2)?,
                ecosystems: serde_json::from_str(&row.get::<_, String>(3)?)?,
            };
            let mut items = Vec::new();
            loop {
                if !matches!(row.get_ref(4)?, rusqlite::types::ValueRef::Null) {
                    items.push(package_from_row(row, 4)?);
                }
                let Some(next) = rows.next()? else { break };
                row = next;
            }
            (counts, items)
        };
        tx.commit()?;
        if cache_miss {
            if let Ok(mut cache) = self.dependency_counts.lock() {
                // App clones can finish reads out of order. Keep newer
                // counts for the same repository.
                if cache.as_ref().is_none_or(|entry| {
                    entry.parameters.first() != values.first() || entry.revision <= revision
                }) {
                    values.truncate(filter_parameter_count);
                    *cache = Some(CachedDependencyCounts {
                        revision,
                        predicate: clause,
                        parameters: values,
                        counts: counts.clone(),
                    });
                }
            }
        }
        let end = offset + items.len() as u32;
        let next_cursor = (end < counts.total).then(|| format!("d1:{revision}:{signature}:{end}"));
        Ok(DependencyPage {
            items,
            total: counts.total,
            inventory_count: counts.inventory_count,
            affected_count: counts.affected_count,
            ecosystems: counts.ecosystems,
            next_cursor,
            scan_run_id,
            revision,
        })
    }
}
