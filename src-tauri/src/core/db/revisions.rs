use super::super::models::DataRevisions;
use super::*;

fn revision_event<'a>(table: &str, operation: &'a str) -> &'a str {
    if table == "repos" && operation == "UPDATE" {
        // Scan status now comes from scan_runs. Updating the legacy columns
        // for compatibility does not change repository evidence.
        "UPDATE OF id, name, path, is_active, source_type, source_key, remote_url, remote_host,
                   provider, default_branch, last_remote_commit_sha, last_remote_synced_at, remote_file_count"
    } else {
        operation
    }
}

fn touches_completed_scan(operation: &str) -> &'static str {
    match operation {
        "INSERT" => "NEW.status='completed'",
        "DELETE" => "OLD.status='completed'",
        _ => "OLD.status='completed' OR NEW.status='completed'",
    }
}

fn dependency_change(table: &str, operation: &str) -> &'static str {
    // The dependency page reads the selected completed run, scan_packages
    // and the package identity, severity and fix availability on findings.
    match (table, operation) {
        ("scan_runs", "UPDATE") => {
            "(OLD.status='completed' OR NEW.status='completed') AND
             (OLD.status IS NOT NEW.status OR OLD.id IS NOT NEW.id OR
              OLD.repo_id IS NOT NEW.repo_id OR OLD.started_at IS NOT NEW.started_at)"
        }
        ("scan_runs", _) => touches_completed_scan(operation),
        ("vulnerabilities", "UPDATE") => {
            "OLD.id IS NOT NEW.id OR OLD.repo_id IS NOT NEW.repo_id OR
             OLD.scan_run_id IS NOT NEW.scan_run_id OR OLD.package_name IS NOT NEW.package_name OR
             OLD.package_version IS NOT NEW.package_version OR OLD.package_type IS NOT NEW.package_type OR
             OLD.package_url IS NOT NEW.package_url OR OLD.severity IS NOT NEW.severity OR
             (OLD.fix_version IS NULL) IS NOT (NEW.fix_version IS NULL)"
        }
        ("repos", "UPDATE") => "OLD.id IS NOT NEW.id OR OLD.is_active IS NOT NEW.is_active",
        ("repos" | "scan_packages" | "vulnerabilities", _) => "1",
        _ => "0",
    }
}

impl Database {
    // Additive and backward readable: previous clients ignore the table, while
    // triggers also observe their writes. Canonical data is never replaced.
    pub(super) fn init_data_revisions(&self, connection: &Connection) -> Result<(), Error> {
        let transaction = connection.unchecked_transaction()?;
        let connection = &transaction;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS performance_metadata(key TEXT PRIMARY KEY,value INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS performance_write_scope(repo_id INTEGER PRIMARY KEY);")?;
        let has_bulk_revision: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM performance_metadata WHERE key='bulk_scan_revision_v1')",
            [],
            |row| row.get(0),
        )?;
        if !has_bulk_revision {
            for table in [
                "repos",
                "scan_runs",
                "vulnerabilities",
                "scan_packages",
                "scan_package_snapshots",
                "vulnerability_remediation_evidence",
                "repo_assets",
            ] {
                for operation in ["INSERT", "UPDATE", "DELETE"] {
                    for prefix in ["cvec_review_dirty", "cvec_revision", "cvec_repo_revision"] {
                        connection.execute_batch(&format!(
                            "DROP TRIGGER IF EXISTS {prefix}_{table}_{operation};"
                        ))?;
                    }
                }
            }
        }
        connection.execute_batch("CREATE TABLE IF NOT EXISTS data_revisions (domain TEXT PRIMARY KEY, revision INTEGER NOT NULL DEFAULT 0);")?;
        for domain in [
            "inventory",
            "scan_history",
            "advisory",
            "review",
            "intel",
            "scanner",
            "notifications",
            "settings",
            "risk",
        ] {
            connection.execute(
                "INSERT OR IGNORE INTO data_revisions(domain) VALUES (?1)",
                [domain],
            )?;
        }
        let has_scan_history_revision: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM performance_metadata WHERE key='scoped_scan_history_v1')",
            [],
            |row| row.get(0),
        )?;
        if !has_scan_history_revision {
            connection.execute_batch(
                "DROP TRIGGER IF EXISTS cvec_revision_repos_UPDATE;
                 DROP TRIGGER IF EXISTS cvec_review_dirty_repos_UPDATE;",
            )?;
            for operation in ["INSERT", "UPDATE", "DELETE"] {
                connection.execute_batch(&format!(
                    "DROP TRIGGER IF EXISTS cvec_revision_scan_runs_{operation};
                     DROP TRIGGER IF EXISTS cvec_review_dirty_scan_runs_{operation};"
                ))?;
            }
        }
        connection.execute_batch("CREATE TABLE IF NOT EXISTS review_dirty_repos(repo_id INTEGER PRIMARY KEY); CREATE TABLE IF NOT EXISTS review_dirty_advisories(identity TEXT PRIMARY KEY);")?;
        let has_dirty_upsert_fix: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM performance_metadata WHERE key='review_dirty_upsert_v1')",
            [],
            |row| row.get(0),
        )?;
        // An outer UPSERT can override INSERT OR IGNORE inside a trigger.
        // Use an explicit conflict target so repeated dirty markers are harmless.
        for (table, column) in [
            ("repos", "id"),
            ("scan_runs", "repo_id"),
            ("vulnerabilities", "repo_id"),
        ] {
            for operation in ["INSERT", "UPDATE", "DELETE"] {
                let row = if operation == "DELETE" { "OLD" } else { "NEW" };
                if !has_dirty_upsert_fix {
                    connection.execute_batch(&format!(
                        "DROP TRIGGER IF EXISTS cvec_review_dirty_{table}_{operation};"
                    ))?;
                }
                let event = revision_event(table, operation);
                let scan_scope = if table == "scan_runs" {
                    format!(" AND ({})", touches_completed_scan(operation))
                } else {
                    String::new()
                };
                connection.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS cvec_review_dirty_{table}_{operation} AFTER {event} ON {table} WHEN NOT EXISTS(SELECT 1 FROM performance_write_scope WHERE repo_id={row}.{column}){scan_scope} BEGIN INSERT INTO review_dirty_repos(repo_id) VALUES ({row}.{column}) ON CONFLICT(repo_id) DO NOTHING; END;"))?;
            }
        }
        for (table, columns) in [
            (
                "external_advisories",
                vec!["canonical_key", "source_item_id"],
            ),
            ("external_advisory_aliases", vec!["alias"]),
        ] {
            for operation in ["INSERT", "UPDATE", "DELETE"] {
                let rows = match operation {
                    "INSERT" => vec!["NEW"],
                    "DELETE" => vec!["OLD"],
                    _ => vec!["OLD", "NEW"],
                };
                if !has_dirty_upsert_fix {
                    connection.execute_batch(&format!(
                        "DROP TRIGGER IF EXISTS cvec_review_dirty_{table}_{operation};"
                    ))?;
                }
                let inserts=rows.iter().flat_map(|row|columns.iter().map(move |column|format!("INSERT INTO review_dirty_advisories(identity) VALUES (lower({row}.{column})) ON CONFLICT(identity) DO NOTHING;"))).collect::<String>();
                connection.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS cvec_review_dirty_{table}_{operation} AFTER {operation} ON {table} BEGIN {inserts} END;"))?;
            }
        }
        if !has_dirty_upsert_fix {
            connection.execute(
                "INSERT INTO performance_metadata(key,value) VALUES('review_dirty_upsert_v1',1)",
                [],
            )?;
        }
        connection.execute_batch("CREATE TABLE IF NOT EXISTS companion_route_sequences(thread_id INTEGER NOT NULL REFERENCES companion_threads(id) ON DELETE CASCADE,provider_segment_id INTEGER NOT NULL REFERENCES companion_provider_segments(id) ON DELETE CASCADE,connection_id TEXT NOT NULL,epoch TEXT NOT NULL DEFAULT '',sequence INTEGER NOT NULL,PRIMARY KEY(thread_id,provider_segment_id,connection_id));")?;
        let columns = {
            let mut statement =
                connection.prepare("PRAGMA table_info(companion_route_sequences)")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        if !columns.iter().any(|column| column == "epoch") {
            connection.execute(
                "ALTER TABLE companion_route_sequences ADD COLUMN epoch TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        let columns = {
            let mut statement = connection.prepare("PRAGMA table_info(vulnerabilities)")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        if !columns.iter().any(|column| column == "advisory_severity") {
            connection.execute(
                "ALTER TABLE vulnerabilities ADD COLUMN advisory_severity TEXT",
                [],
            )?;
        }
        connection.execute_batch("CREATE TABLE IF NOT EXISTS advisory_enrichment_cache(ghsa_id TEXT PRIMARY KEY,fetched_at TEXT NOT NULL,etag TEXT,details_json TEXT NOT NULL);")?;
        let tables: &[(&str, &[&str])] = &[
            ("repos", &["inventory", "review", "risk"]),
            ("vulnerabilities", &["inventory", "review", "risk"]),
            ("scan_packages", &["inventory"]),
            ("scan_package_snapshots", &["inventory"]),
            ("vulnerability_remediation_evidence", &["inventory"]),
            ("repo_assets", &["inventory"]),
            ("finding_reviews", &["review"]),
            ("notifications", &["notifications"]),
            ("external_advisories", &["intel", "review", "risk"]),
            ("external_advisory_aliases", &["intel", "review", "risk"]),
            ("external_advisory_sources", &["intel"]),
            ("external_advisory_affected_packages", &["intel"]),
            ("external_advisory_references", &["intel"]),
            ("external_advisory_cwes", &["intel"]),
            ("external_advisory_kev", &["intel"]),
            ("external_advisory_products", &["intel"]),
            ("external_advisory_matches", &["intel"]),
        ];
        for (table, domains) in tables {
            let domains = domains
                .iter()
                .map(|d| format!("'{d}'"))
                .collect::<Vec<_>>()
                .join(",");
            for operation in ["INSERT", "UPDATE", "DELETE"] {
                let row = if operation == "DELETE" { "OLD" } else { "NEW" };
                let event = revision_event(table, operation);
                let guard = match *table {
                    "repos" => format!("WHEN NOT EXISTS(SELECT 1 FROM performance_write_scope WHERE repo_id={row}.id)"),
                    "vulnerabilities"|"scan_packages"|"scan_package_snapshots"|"vulnerability_remediation_evidence"|"repo_assets" => format!("WHEN NOT EXISTS(SELECT 1 FROM performance_write_scope WHERE repo_id={row}.repo_id)"),
                    _ => String::new(),
                };
                connection.execute_batch(&format!(
                    "CREATE TRIGGER IF NOT EXISTS cvec_revision_{table}_{operation} AFTER {event} ON {table} {guard} BEGIN UPDATE data_revisions SET revision = revision + 1 WHERE domain IN ({domains}); END;"
                ))?;
            }
        }
        for (operation, row) in [("INSERT", "NEW"), ("DELETE", "OLD")] {
            let evidence = touches_completed_scan(operation);
            connection.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS cvec_revision_scan_runs_{operation}
                 AFTER {operation} ON scan_runs WHEN NOT EXISTS(SELECT 1 FROM performance_write_scope WHERE repo_id={row}.repo_id)
                 BEGIN
                   UPDATE data_revisions SET revision=revision+1
                   WHERE domain='scan_history' OR (domain IN ('inventory','review','risk') AND {evidence});
                 END;"
            ))?;
        }
        connection.execute_batch("DROP TRIGGER IF EXISTS cvec_revision_scan_runs_UPDATE;
            CREATE TRIGGER cvec_revision_scan_runs_UPDATE AFTER UPDATE ON scan_runs WHEN NOT EXISTS(SELECT 1 FROM performance_write_scope WHERE repo_id=NEW.repo_id) BEGIN
              UPDATE data_revisions SET revision=revision+1 WHERE domain='scan_history';
              UPDATE data_revisions SET revision=revision+1 WHERE domain='inventory'
                AND (OLD.status='completed' OR NEW.status='completed') AND (
                OLD.repo_id IS NOT NEW.repo_id OR OLD.status IS NOT NEW.status OR OLD.started_at IS NOT NEW.started_at OR
                OLD.finished_at IS NOT NEW.finished_at OR OLD.source_revision IS NOT NEW.source_revision OR
                OLD.source_dirty IS NOT NEW.source_dirty OR OLD.evidence_complete IS NOT NEW.evidence_complete OR
                OLD.new_medium_count IS NOT NEW.new_medium_count OR OLD.new_high_count IS NOT NEW.new_high_count OR
                OLD.new_critical_count IS NOT NEW.new_critical_count OR OLD.resolved_medium_count IS NOT NEW.resolved_medium_count OR
                OLD.resolved_high_count IS NOT NEW.resolved_high_count OR OLD.resolved_critical_count IS NOT NEW.resolved_critical_count);
              UPDATE data_revisions SET revision=revision+1 WHERE domain IN ('review','risk')
                AND (OLD.status='completed' OR NEW.status='completed') AND (
                OLD.repo_id IS NOT NEW.repo_id OR OLD.status IS NOT NEW.status OR OLD.started_at IS NOT NEW.started_at OR
                OLD.finished_at IS NOT NEW.finished_at OR OLD.evidence_complete IS NOT NEW.evidence_complete);
            END;
            DROP TRIGGER IF EXISTS cvec_revision_vulnerabilities_UPDATE;
            CREATE TRIGGER cvec_revision_vulnerabilities_UPDATE AFTER UPDATE ON vulnerabilities WHEN NOT EXISTS(SELECT 1 FROM performance_write_scope WHERE repo_id=NEW.repo_id) BEGIN
              UPDATE data_revisions SET revision=revision+1 WHERE domain IN ('advisory','review','risk');
              UPDATE data_revisions SET revision=revision+1 WHERE domain='inventory' AND (
                OLD.repo_id IS NOT NEW.repo_id OR OLD.scan_run_id IS NOT NEW.scan_run_id OR
                OLD.fingerprint IS NOT NEW.fingerprint OR OLD.advisory_id IS NOT NEW.advisory_id OR
                OLD.package_name IS NOT NEW.package_name OR OLD.package_version IS NOT NEW.package_version OR
                OLD.package_type IS NOT NEW.package_type OR OLD.package_url IS NOT NEW.package_url OR
                OLD.severity IS NOT NEW.severity OR OLD.fix_version IS NOT NEW.fix_version OR
                OLD.title IS NOT NEW.title OR OLD.source_url IS NOT NEW.source_url);
            END;")?;
        if !has_scan_history_revision {
            connection.execute(
                "INSERT INTO performance_metadata(key,value) VALUES('scoped_scan_history_v1',1)",
                [],
            )?;
        }
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS repository_data_revisions(
               repo_id INTEGER PRIMARY KEY,
               revision INTEGER NOT NULL,
               dependency_revision INTEGER NOT NULL DEFAULT 0
             );",
        )?;
        let has_dependency_revision: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('repository_data_revisions')
                           WHERE name='dependency_revision')",
            [],
            |row| row.get(0),
        )?;
        if !has_dependency_revision {
            connection.execute_batch(
                "ALTER TABLE repository_data_revisions ADD COLUMN dependency_revision INTEGER NOT NULL DEFAULT 0;
                 UPDATE repository_data_revisions SET dependency_revision=revision;",
            )?;
        }
        for (table, column) in [
            ("repos", "id"),
            ("scan_runs", "repo_id"),
            ("vulnerabilities", "repo_id"),
            ("scan_packages", "repo_id"),
            ("scan_package_snapshots", "repo_id"),
            ("vulnerability_remediation_evidence", "repo_id"),
            ("repo_assets", "repo_id"),
        ] {
            for operation in ["INSERT", "UPDATE", "DELETE"] {
                let row = if operation == "DELETE" { "OLD" } else { "NEW" };
                let trigger = format!("cvec_repo_revision_{table}_{operation}");
                if !has_dependency_revision {
                    connection.execute_batch(&format!("DROP TRIGGER IF EXISTS {trigger};"))?;
                }
                let dependency_change = dependency_change(table, operation);
                connection.execute_batch(&format!(
                    "CREATE TRIGGER IF NOT EXISTS {trigger} AFTER {operation} ON {table}
                     WHEN NOT EXISTS(SELECT 1 FROM performance_write_scope WHERE repo_id={row}.{column})
                     BEGIN
                       INSERT INTO repository_data_revisions(repo_id,revision,dependency_revision)
                       VALUES({row}.{column},1,({dependency_change}))
                       ON CONFLICT(repo_id) DO UPDATE SET revision=revision+1,
                         dependency_revision=dependency_revision+excluded.dependency_revision;
                     END;"
                ))?;
            }
        }
        connection.execute_batch("CREATE INDEX IF NOT EXISTS idx_vulns_advisory_nocase_repo ON vulnerabilities(advisory_id COLLATE NOCASE,repo_id);")?;
        let indexed_intel_triggers: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM performance_metadata WHERE key='indexed_repo_intel_triggers_v1')",
            [],
            |row| row.get(0),
        )?;
        for operation in ["INSERT", "UPDATE", "DELETE"] {
            let rows = match operation {
                "INSERT" => vec!["NEW"],
                "DELETE" => vec!["OLD"],
                _ => vec!["OLD", "NEW"],
            };
            let trigger = format!("cvec_repo_intel_{operation}");
            if !indexed_intel_triggers {
                connection.execute_batch(&format!("DROP TRIGGER IF EXISTS {trigger};"))?;
            }
            // SQLite can scan every finding for the combined OR predicate.
            // UNION keeps one revision increment per repository and uses the
            // advisory index for each lookup.
            let direct = rows
                .iter()
                .map(|row| {
                    format!(
                        "INSERT INTO repository_data_revisions(repo_id,revision)
                 SELECT repo_id,1 FROM (
                    SELECT repo_id FROM vulnerabilities
                    WHERE advisory_id COLLATE NOCASE IN ({row}.canonical_key,{row}.source_item_id)
                    UNION
                    SELECT repo_id FROM vulnerabilities
                    WHERE advisory_id COLLATE NOCASE IN (
                        SELECT alias FROM external_advisory_aliases WHERE advisory_id={row}.id
                    )
                 ) WHERE 1 ON CONFLICT(repo_id) DO UPDATE SET revision=revision+1;"
                    )
                })
                .collect::<String>();
            connection.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS {trigger} AFTER {operation} ON external_advisories BEGIN {direct} END;"
            ))?;
            let aliases=rows.iter().map(|row|format!("INSERT INTO repository_data_revisions(repo_id,revision) SELECT DISTINCT repo_id,1 FROM vulnerabilities WHERE advisory_id={row}.alias COLLATE NOCASE ON CONFLICT(repo_id) DO UPDATE SET revision=revision+1;")).collect::<String>();
            connection.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS cvec_repo_alias_{operation} AFTER {operation} ON external_advisory_aliases BEGIN {aliases} END;"))?;
        }
        if !indexed_intel_triggers {
            connection.execute(
                "INSERT INTO performance_metadata(key,value) VALUES('indexed_repo_intel_triggers_v1',1)",
                [],
            )?;
        }
        // Settings have deliberately narrow domain effects. Chat/provider writes
        // cannot invalidate security pages merely by persisting a text delta.
        for operation in ["INSERT", "UPDATE", "DELETE"] {
            let row = if operation == "DELETE" { "OLD" } else { "NEW" };
            connection.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS cvec_revision_settings_{operation} AFTER {operation} ON app_settings WHEN {row}.key <> 'review_reconciled_risk_revision' BEGIN
                 UPDATE data_revisions SET revision = revision + 1 WHERE domain = 'settings';
                 UPDATE data_revisions SET revision = revision + 1 WHERE domain = 'intel' AND {row}.key IN ('external_intel_topics', 'external_intel_last_fetched_at');
                 UPDATE data_revisions SET revision = revision + 1 WHERE domain = 'scanner' AND {row}.key IN ('scanner_status', 'last_grype_db_update', 'grype_db_updated_at');
                 END;"
            ))?;
        }
        connection.execute("INSERT OR IGNORE INTO app_settings(key, value) VALUES ('review_reconciled_risk_revision', '-1')", [])?;
        connection.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_vulns_run_severity ON vulnerabilities(scan_run_id, severity);
             CREATE INDEX IF NOT EXISTS idx_vulns_repo_fp_run ON vulnerabilities(repo_id, fingerprint, scan_run_id DESC);
             CREATE INDEX IF NOT EXISTS idx_scan_completed_id ON scan_runs(repo_id,status,id DESC);
             CREATE INDEX IF NOT EXISTS idx_scan_completed_started ON scan_runs(repo_id, status, started_at DESC, id DESC);
             CREATE INDEX IF NOT EXISTS idx_scan_completed_finished ON scan_runs(repo_id, status, finished_at DESC, id DESC);"
        )?;
        connection.execute("INSERT OR REPLACE INTO performance_metadata(key,value) VALUES('bulk_scan_revision_v1',1)",[])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn data_revisions(&self) -> Result<DataRevisions, Error> {
        Self::read_data_revisions(&self.connect()?)
    }

    pub(super) fn read_data_revisions(connection: &Connection) -> Result<DataRevisions, Error> {
        let mut result = DataRevisions::default();
        let mut statement = connection.prepare("SELECT domain, revision FROM data_revisions")?;
        for row in statement.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
        })? {
            let (domain, revision) = row?;
            match domain.as_str() {
                "advisory" => result.advisory = revision,
                "inventory" => result.inventory = revision,
                "scan_history" => result.scan_history = revision,
                "review" => result.review = revision,
                "intel" => result.intel = revision,
                "scanner" => result.scanner = revision,
                "notifications" => result.notifications = revision,
                "settings" => result.settings = revision,
                _ => {}
            }
        }
        Ok(result)
    }
}
