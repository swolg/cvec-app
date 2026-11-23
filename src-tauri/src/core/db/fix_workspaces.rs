use super::*;
use crate::core::fix_workspaces::{FixArchivePage, FixWorkspace, FixWorkspaceSummary};

fn archive_summary(workspace: &FixWorkspace, archived_at: &str) -> FixWorkspaceSummary {
    let finding = workspace
        .baseline_findings
        .iter()
        .find(|finding| Some(&finding.fingerprint) == workspace.finding_fingerprint.as_ref());
    let title: String = if let Some(finding) = finding {
        format!("{} · {}", finding.advisory_id, finding.package_name)
            .chars()
            .take(120)
            .collect()
    } else if let Some(message) = workspace
        .messages
        .iter()
        .find(|message| message.role == "user")
    {
        message.text.chars().take(120).collect()
    } else {
        "Fix workspace".into()
    };
    FixWorkspaceSummary {
        id: workspace.id.clone(),
        repository_id: workspace.repository_id,
        branch: workspace.branch.clone(),
        title,
        finding_fingerprint: workspace.finding_fingerprint.clone(),
        status: Some(workspace.status.clone()),
        archived_at: archived_at.into(),
    }
}

pub(crate) struct FixWorkspaceContext {
    pub summary: RepoSummary,
    pub scan: Option<RepoScanRun>,
    pub findings: Vec<NormalizedFinding>,
}

impl Database {
    pub(crate) fn initialize_fix_workspaces(&self) -> Result<(), Error> {
        self.connect()?.execute_batch(
            "CREATE TABLE IF NOT EXISTS fix_workspaces (
                id TEXT PRIMARY KEY,
                repository_id INTEGER NOT NULL REFERENCES repos(id),
                updated_at TEXT NOT NULL,
                record_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS fix_workspaces_repository ON fix_workspaces(repository_id, updated_at);",
        )?;
        let mut connection = self.connect()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let columns = transaction
            .prepare("PRAGMA table_info(fix_workspaces)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        if !columns.iter().any(|name| name == "archived_at") {
            transaction.execute_batch(
                "ALTER TABLE fix_workspaces ADD COLUMN archived_at TEXT;
                ALTER TABLE fix_workspaces ADD COLUMN summary_json TEXT;",
            )?;
        }
        transaction.execute_batch("CREATE INDEX IF NOT EXISTS fix_workspaces_archive ON fix_workspaces(repository_id, archived_at DESC, id DESC);")?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn save_fix_workspace(&self, workspace: &FixWorkspace) -> Result<(), Error> {
        if !workspace.is_supported_record() {
            return Err(Error::App("The fix workspace record exceeds the supported evidence contract. Existing saved data was preserved.".into()));
        }
        let record = serde_json::to_string(workspace)?;
        if record.len() > 16 * 1024 * 1024 {
            return Err(Error::App("The fix workspace record exceeds the supported storage limit. Existing saved data was preserved.".into()));
        }
        let summary = workspace
            .archived_at
            .as_ref()
            .map(|archived_at| archive_summary(workspace, archived_at))
            .map(|summary| serde_json::to_string(&summary))
            .transpose()?;
        self.connect()?.execute(
            "INSERT INTO fix_workspaces (id, repository_id, updated_at, record_json, archived_at, summary_json) VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(id) DO UPDATE SET updated_at=excluded.updated_at, record_json=excluded.record_json, archived_at=excluded.archived_at, summary_json=excluded.summary_json",
            params![workspace.id, workspace.repository_id, workspace.updated_at, record, workspace.archived_at, summary],
        )?;
        Ok(())
    }

    pub(crate) fn fix_workspace_checkout(&self, repository_id: i64) -> Result<RepoSummary, Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        self.active_repo_summary(&transaction, repository_id)
    }

    pub(crate) fn fix_workspace_context(
        &self,
        repository_id: i64,
    ) -> Result<FixWorkspaceContext, Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let summary = self.active_repo_summary(&transaction, repository_id)?;
        let scan = self
            .repo_scan_history_filtered(&transaction, repository_id, 1, true)?
            .into_iter()
            .next();
        let findings = match &scan {
            Some(scan) => self.findings_for_run(&transaction, repository_id, scan.id)?,
            None => Vec::new(),
        };
        transaction.commit()?;
        Ok(FixWorkspaceContext {
            summary,
            scan,
            findings,
        })
    }

    pub(crate) fn load_fix_workspaces(
        &self,
        repository_id: i64,
    ) -> Result<Vec<FixWorkspace>, Error> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, CASE WHEN length(record_json)<=16777216 THEN record_json END
             FROM fix_workspaces WHERE repository_id=?1 AND archived_at IS NULL ORDER BY updated_at DESC LIMIT 20",
        )?;
        let rows = statement.query_map([repository_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut records = Vec::new();
        for row in rows {
            let (id, json) = row?;
            let parsed = json.and_then(|json| serde_json::from_str::<FixWorkspace>(&json).ok());
            if let Some(data) = parsed.filter(|data| {
                data.is_supported_record()
                    && data.archived_at.is_none()
                    && data.id == id
                    && data.repository_id == repository_id
            }) {
                records.push(data);
            } else {
                // Preserve unreadable/future-version records. They must not prevent
                // CVEC startup or overwrite otherwise recoverable workspace files.
                eprintln!("cvec: skipped an unsupported or unreadable saved fix workspace");
            }
        }
        Ok(records)
    }
    pub(crate) fn active_fix_workspace_count(&self, repository_id: i64) -> Result<i64, Error> {
        Ok(self.connect()?.query_row(
            "SELECT count(*) FROM fix_workspaces WHERE repository_id=?1 AND archived_at IS NULL",
            [repository_id],
            |row| row.get(0),
        )?)
    }

    pub(crate) fn load_fix_workspace(
        &self,
        repository_id: i64,
        id: &str,
    ) -> Result<FixWorkspace, Error> {
        let json: Option<String> = self.connect()?.query_row(
            "SELECT CASE WHEN length(record_json)<=16777216 THEN record_json END FROM fix_workspaces WHERE repository_id=?1 AND id=?2",
            params![repository_id, id], |row| row.get(0),
        ).optional()?.flatten();
        let data = json.and_then(|json| serde_json::from_str::<FixWorkspace>(&json).ok())
            .filter(|data| data.is_supported_record() && data.repository_id == repository_id && data.id == id)
            .ok_or_else(|| Error::App("Fix workspace not found or its saved format is unsupported. Existing data was preserved.".into()))?;
        Ok(data)
    }

    pub(crate) fn archived_fix_workspaces(
        &self,
        repository_id: i64,
        cursor: Option<&str>,
        finding_fingerprint: Option<&str>,
    ) -> Result<FixArchivePage, Error> {
        let (date, id) = match cursor {
            None => ("9999", ""),
            Some(cursor) => cursor
                .split_once('|')
                .filter(|(date, id)| {
                    date.len() <= 64
                        && !date.is_empty()
                        && id.len() == 24
                        && id.bytes().all(|c| c.is_ascii_hexdigit())
                })
                .ok_or_else(|| Error::App("Invalid archive page cursor.".into()))?,
        };
        let connection = self.connect()?;
        let mut statement = connection.prepare("SELECT id, archived_at, CASE WHEN length(summary_json)<=16384 THEN summary_json END,
            CASE WHEN length(record_json)<=16777216 AND
                CASE WHEN json_valid(summary_json) THEN json_type(summary_json,'$.status') IS NULL ELSE 1 END
                THEN record_json END FROM fix_workspaces
            WHERE repository_id=?1 AND archived_at IS NOT NULL AND (archived_at,id)<(?2,?3)
              AND (?4 IS NULL OR COALESCE(
                CASE WHEN length(summary_json)<=16384 AND json_valid(summary_json)
                  THEN json_extract(summary_json,'$.findingFingerprint') END,
                CASE WHEN length(record_json)<=16777216 AND json_valid(record_json)
                  THEN json_extract(record_json,'$.findingFingerprint') END
              )=?4)
            ORDER BY archived_at DESC,id DESC LIMIT 21")?;
        let rows = statement
            .query_map(
                params![repository_id, date, id, finding_fingerprint],
                |row| {
                    let id: String = row.get(0)?;
                    let archived_at: String = row.get(1)?;
                    let json: Option<String> = row.get(2)?;
                    let legacy: Option<String> = row.get(3)?;
                    let summary = legacy
                        .and_then(|record| serde_json::from_str::<FixWorkspace>(&record).ok())
                        .filter(|record| {
                            record.is_supported_record()
                                && record.archived_at.as_deref() == Some(&archived_at)
                        })
                        .map(|record| archive_summary(&record, &archived_at))
                        .or_else(|| {
                            json.and_then(|json| {
                                serde_json::from_str::<FixWorkspaceSummary>(&json).ok()
                            })
                        });
                    Ok((id, archived_at, summary))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = (rows.len() > 20).then(|| format!("{}|{}", rows[19].1, rows[19].0));
        let mut entries = Vec::new();
        for (id, archived_at, summary) in rows.into_iter().take(20) {
            if let Some(data) = summary.filter(|data| {
                data.id == id
                    && data.repository_id == repository_id
                    && data.archived_at == archived_at
                    && data.id.len() == 24
                    && data.id.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && data.branch.len() <= 4096
                    && data.title.len() <= 480
                    && data.status.as_ref().is_none_or(|status| status.len() <= 64)
                    && data
                        .finding_fingerprint
                        .as_ref()
                        .is_none_or(|fingerprint| fingerprint.len() <= 4096)
                    && data.archived_at.len() <= 64
            }) {
                entries.push(data);
            }
        }
        Ok(FixArchivePage {
            entries,
            next_cursor,
        })
    }
}
