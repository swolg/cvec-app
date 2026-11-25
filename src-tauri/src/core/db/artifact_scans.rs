use rusqlite::{params, TransactionBehavior};

use super::{ensure_repository_exists, Database};
use crate::core::models::{ArtifactFindings, ArtifactScan, ArtifactScanSummary, NormalizedFinding};
use crate::Error;

impl Database {
    pub(crate) fn record_artifact_scan(
        &self,
        repo_id: i64,
        summary: ArtifactScanSummary,
        findings: &[NormalizedFinding],
    ) -> Result<ArtifactScan, Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_repository_exists(&transaction, repo_id)?;
        transaction.execute(
            "INSERT INTO artifact_scans (repo_id, summary) VALUES (?1, ?2)",
            params![repo_id, serde_json::to_string(&summary)?],
        )?;
        let id = transaction.last_insert_rowid();
        {
            let mut insert = transaction.prepare("INSERT INTO artifact_scan_findings (scan_id, position, finding) VALUES (?1, ?2, ?3)")?;
            for (position, finding) in findings.iter().enumerate() {
                insert.execute(params![
                    id,
                    position as i64,
                    serde_json::to_string(finding)?
                ])?;
            }
        }
        transaction.commit()?;
        Ok(ArtifactScan {
            id,
            repo_id,
            summary,
        })
    }

    pub(crate) fn list_artifact_scans(&self, repo_id: i64) -> Result<Vec<ArtifactScan>, Error> {
        let connection = self.connect()?;
        ensure_repository_exists(&connection, repo_id)?;
        let mut statement = connection.prepare(
            "SELECT id, summary FROM artifact_scans WHERE repo_id = ?1 ORDER BY id DESC LIMIT 50",
        )?;
        let mut rows = statement.query([repo_id])?;
        let mut scans = Vec::new();
        while let Some(row) = rows.next()? {
            scans.push(ArtifactScan {
                id: row.get(0)?,
                repo_id,
                summary: serde_json::from_str(&row.get::<_, String>(1)?)?,
            });
        }
        Ok(scans)
    }

    pub(crate) fn artifact_findings(
        &self,
        repo_id: i64,
        scan_id: i64,
        offset: u32,
    ) -> Result<ArtifactFindings, Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        ensure_repository_exists(&transaction, repo_id)?;
        if !transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM artifact_scans WHERE id = ?1 AND repo_id = ?2)",
            params![scan_id, repo_id],
            |row| row.get::<_, bool>(0),
        )? {
            return Err(Error::App(
                "The artifact scan was not found for this repository.".into(),
            ));
        }
        let findings = {
            let mut statement = transaction.prepare("SELECT finding FROM artifact_scan_findings WHERE scan_id = ?1 AND position >= ?2 ORDER BY position LIMIT 100")?;
            let mut rows = statement.query(params![scan_id, offset])?;
            let mut findings = Vec::new();
            while let Some(row) = rows.next()? {
                let mut finding: NormalizedFinding =
                    serde_json::from_str(&row.get::<_, String>(0)?)?;
                if finding.package_type.as_deref() == Some("go-module") {
                    if let Some(evidence) = finding.match_evidence.as_mut() {
                        evidence.normalize_bundled_go_fix(&mut finding.fix_version);
                    }
                }
                findings.push(finding);
            }
            findings
        };
        transaction.commit()?;
        Ok(ArtifactFindings {
            scan_id,
            repo_id,
            offset,
            findings,
        })
    }
}
