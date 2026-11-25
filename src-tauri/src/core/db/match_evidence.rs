use rusqlite::{params, Connection, TransactionBehavior};

use super::{is_safe_stored_relative_path, Database};
use crate::core::models::{FindingMatchEvidence, FindingMatchKind};
use crate::core::scanner::NATIVE_PACKAGE_EVIDENCE_WARNING;
use crate::Error;

impl Database {
    pub(super) fn reconcile_scan_match_evidence(connection: &mut Connection) -> Result<(), Error> {
        const KEY: &str = "scan_match_evidence_v3";
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
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !done(&tx)? {
            let rows = {
                let mut query = tx.prepare(r#"
                    SELECT v.id, v.fix_version, v.match_evidence_json, e.relative_path,
                           COALESCE(v.package_type='go-module', 0) AS go_module,
                           instr(s.coverage_warnings_json, ?1) > 0 AS used_fallback
                    FROM vulnerabilities v
                    JOIN scan_runs s ON s.id=v.scan_run_id AND s.repo_id=v.repo_id
                    LEFT JOIN vulnerability_remediation_evidence e
                      ON e.repo_id=v.repo_id AND e.scan_run_id=v.scan_run_id AND e.fingerprint=v.fingerprint
                    WHERE (go_module AND (instr(v.match_evidence_json,'node_modules/') > 0
                                       OR instr(e.relative_path,'node_modules/') > 0))
                       OR (used_fallback AND v.fix_version IS NOT NULL)
                "#)?;
                let found = query.query_map([NATIVE_PACKAGE_EVIDENCE_WARNING], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, bool>(4)?,
                        row.get::<_, bool>(5)?,
                    ))
                })?;
                found.collect::<Result<Vec<_>, _>>()?
            };
            for (id, mut fix_version, raw, fallback_path, go_module, used_fallback) in rows {
                let mut evidence = raw
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<FindingMatchEvidence>(value).ok())
                    .unwrap_or(FindingMatchEvidence {
                        kind: FindingMatchKind::Unverified,
                        relative_path: None,
                        bundled_tool: None,
                    });
                let unverified_origin = used_fallback
                    && !evidence
                        .relative_path
                        .as_deref()
                        .is_some_and(is_safe_stored_relative_path);
                if go_module && evidence.relative_path.is_none() {
                    evidence.relative_path = fallback_path;
                }
                let bundled_changed =
                    go_module && evidence.normalize_bundled_go_fix(&mut fix_version);
                let fallback_changed = unverified_origin && fix_version.take().is_some();
                if fallback_changed && !evidence.requires_review() {
                    evidence.kind = if go_module {
                        FindingMatchKind::GoModule
                    } else {
                        FindingMatchKind::Unverified
                    };
                }
                if bundled_changed || fallback_changed {
                    tx.execute("UPDATE vulnerabilities SET fix_version=?1, match_evidence_json=?2 WHERE id=?3",
                        params![fix_version, serde_json::to_string(&evidence)?, id])?;
                }
            }
            tx.execute(
                "INSERT INTO performance_metadata(key,value) VALUES(?1,1)",
                [KEY],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
