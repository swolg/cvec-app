use super::super::models::AdvisoryDetails;
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct CachedAdvisory {
    pub details: AdvisoryDetails,
    pub etag: Option<String>,
    pub fetched_at: String,
}

impl CachedAdvisory {
    pub fn is_fresh(&self) -> bool {
        timestamps::parse(&self.fetched_at).is_ok_and(|at| {
            let age = OffsetDateTime::now_utc() - at;
            age >= Duration::ZERO && age < Duration::hours(6)
        })
    }
}

impl Database {
    pub fn cached_advisory(&self, id: &str) -> Result<Option<CachedAdvisory>, Error> {
        let connection = self.connect()?;
        let row = connection.query_row(
            "SELECT fetched_at,etag,details_json FROM advisory_enrichment_cache WHERE ghsa_id=?1",
            [id.to_ascii_uppercase()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?)),
        ).optional()?;
        row.map(|(fetched_at, etag, json)| {
            Ok(CachedAdvisory {
                details: serde_json::from_str(&json)?,
                etag,
                fetched_at,
            })
        })
        .transpose()
    }

    pub fn cache_advisory(
        &self,
        id: &str,
        details: &AdvisoryDetails,
        etag: Option<&str>,
    ) -> Result<(), Error> {
        if !details
            .ghsa_id
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(id))
        {
            return Err(Error::App(
                "Advisory response identity did not match the requested GHSA".into(),
            ));
        }
        let fetched_at = details
            .fetched_at
            .as_deref()
            .ok_or_else(|| Error::App("Advisory retrieval time is missing".into()))?;
        let json = serde_json::to_string(details)?;
        if json.len() > 2 * 1024 * 1024 {
            return Err(Error::App(
                "Advisory response exceeded the cache size limit".into(),
            ));
        }
        let mut connection = self.connect()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO advisory_enrichment_cache(ghsa_id,fetched_at,etag,details_json) VALUES(?1,?2,?3,?4) ON CONFLICT(ghsa_id) DO UPDATE SET fetched_at=excluded.fetched_at,etag=excluded.etag,details_json=excluded.details_json", params![id.to_ascii_uppercase(),fetched_at,etag,json])?;
        // Bounded persistent cache. Scan evidence retains its own source details.
        tx.execute("DELETE FROM advisory_enrichment_cache WHERE ghsa_id IN (SELECT ghsa_id FROM advisory_enrichment_cache ORDER BY fetched_at DESC,ghsa_id LIMIT -1 OFFSET 1000)", [])?;
        tx.execute("DELETE FROM advisory_enrichment_cache WHERE ghsa_id IN (SELECT ghsa_id FROM (SELECT ghsa_id,SUM(length(details_json)) OVER(ORDER BY fetched_at DESC,ghsa_id) bytes FROM advisory_enrichment_cache) WHERE bytes>67108864)",[])?;
        tx.commit()?;
        Ok(())
    }

    pub fn scan_advisory_identities(
        &self,
        repo_id: i64,
        run_id: i64,
    ) -> Result<Vec<(String, String)>, Error> {
        let connection = self.connect()?;
        let mut statement = connection.prepare("SELECT v.fingerprint,v.advisory_id,v.source_url FROM vulnerabilities v JOIN scan_runs s ON s.id=v.scan_run_id WHERE s.repo_id=?1 AND s.id=?2 AND s.status='completed'")?;
        let mut identities = Vec::new();
        for row in statement.query_map(params![repo_id, run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })? {
            let (fingerprint, advisory, source) = row?;
            if let Some(id) = super::super::github_advisory::exact_github_advisory_id(
                &advisory,
                source.as_deref(),
            ) {
                identities.push((fingerprint, id));
            }
        }
        Ok(identities)
    }

    pub fn apply_scan_advisories(
        &self,
        repo_id: i64,
        run_id: i64,
        details: &BTreeMap<String, AdvisoryDetails>,
    ) -> Result<(usize, u32), Error> {
        let identities = self.scan_advisory_identities(repo_id, run_id)?;
        let mut connection = self.connect()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut changed = Vec::new();
        {
            let mut statement=tx.prepare_cached("UPDATE vulnerabilities SET advisory_details_json=?1,advisory_severity=?2 WHERE repo_id=?3 AND scan_run_id=?4 AND fingerprint=?5 AND EXISTS(SELECT 1 FROM scan_runs WHERE id=?4 AND repo_id=?3 AND status='completed') AND (advisory_details_json IS NOT ?1 OR advisory_severity IS NOT ?2)")?;
            for (fingerprint, id) in identities {
                let Some(detail) = details.get(&id) else {
                    continue;
                };
                if !detail
                    .ghsa_id
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(&id))
                {
                    return Err(Error::App("Advisory evidence identity mismatch".into()));
                }
                let severity = match detail.severity {
                    ExternalIntelSeverity::Critical => Some("CRITICAL"),
                    ExternalIntelSeverity::High => Some("HIGH"),
                    ExternalIntelSeverity::Medium => Some("MEDIUM"),
                    _ => None,
                };
                if statement.execute(params![
                    serde_json::to_string(detail)?,
                    severity,
                    repo_id,
                    run_id,
                    fingerprint
                ])? > 0
                {
                    changed.push(fingerprint);
                }
            }
        }
        let reopened = if !changed.is_empty() {
            let reopened = Self::reopen_scan_reviews(&tx, repo_id, run_id, Some(&changed))?;
            if reopened > 0 {
                tx.execute(
                    "UPDATE scan_runs SET reopened_count=reopened_count+?2 WHERE id=?1",
                    params![run_id, reopened],
                )?;
            }
            reopened
        } else {
            0
        };
        tx.commit()?;
        Ok((changed.len(), reopened))
    }
}
