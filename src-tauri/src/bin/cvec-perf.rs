//! Opt-in local performance harness. All writes are confined to newly created
//! fixtures under the supplied temporary output directory.
use cvec::core::timestamps;
use cvec::core::{
    db::Database, github_auth::GithubAuthSnapshot, models::*, repo_discovery::DiscoveredRepo,
};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};
use std::{path::Path, time::Instant};
use time::OffsetDateTime;

fn measure<T: Serialize>(name: &str, n: usize, mut call: impl FnMut() -> T) -> Value {
    measure_after(name, n, || {}, &mut call)
}

fn measure_after<T: Serialize>(
    name: &str,
    n: usize,
    mut prepare: impl FnMut(),
    mut call: impl FnMut() -> T,
) -> Value {
    prepare();
    let first = Instant::now();
    let first_value = call();
    let first_ms = first.elapsed().as_secs_f64() * 1000.0;
    let mut samples = Vec::with_capacity(n);
    let mut bytes = serde_json::to_vec(&first_value).unwrap().len();
    for _ in 0..n {
        prepare();
        let started = Instant::now();
        let value = call();
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        bytes = serde_json::to_vec(&value).unwrap().len();
        std::hint::black_box(value);
    }
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| sorted[(n as f64 * p).ceil() as usize - 1];
    json!({"operation":name,"warm_runs":n,"first_call_ms":first_ms,"warm_p50_ms":percentile(0.5),"warm_p95_ms":percentile(0.95),"warm_p99_ms":percentile(0.99),"payload_bytes":bytes,"samples_ms":samples})
}
fn fixture(path: &Path, total: usize, repos: usize, intel_benchmark: bool) -> Database {
    assert!(!path.exists(), "Refusing to overwrite a fixture database");
    let db = Database::new(path.to_path_buf()).unwrap();
    let root = path
        .parent()
        .unwrap()
        .join(format!("sources-{total}-{repos}"));
    std::fs::create_dir_all(&root).unwrap();
    let discovered = (0..repos)
        .map(|i| {
            let path = root.join(format!("repo-{i:03}"));
            std::fs::create_dir_all(path.join(".git")).unwrap();
            std::fs::write(path.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            DiscoveredRepo {
                name: format!("repo-{i:03}"),
                path,
                remote_url: None,
                remote_host: None,
                provider: None,
            }
        })
        .collect::<Vec<_>>();
    db.sync_repositories(&discovered).unwrap();
    db.set_setting("discovery_root", root.to_string_lossy().into_owned())
        .unwrap();
    let mut connection = Connection::open(path).unwrap();
    let tx = connection.transaction().unwrap();
    let today = OffsetDateTime::now_utc().date();
    let prior_at = format!("{}T12:00:00Z", today - time::Duration::days(14));
    let latest_at = format!("{today}T12:00:00Z");
    for repo in 1..=repos {
        for previous in [true, false] {
            let id = repo * 2 - usize::from(previous);
            let at = if previous { &prior_at } else { &latest_at };
            tx.execute("INSERT INTO scan_runs(id,repo_id,status,started_at,finished_at,package_count,evidence_complete) VALUES(?1,?2,'completed',?3,?3,?4,1)",params![id as i64,repo as i64,at,(total/repos) as i64]).unwrap();
        }
        tx.execute(
            "UPDATE repos SET last_scan_status='completed',last_scanned_at=?2 WHERE id=?1",
            params![repo as i64, latest_at],
        )
        .unwrap();
    }
    {
        let mut finding=tx.prepare("INSERT INTO vulnerabilities(repo_id,scan_run_id,fingerprint,advisory_id,package_name,package_version,package_type,package_url,severity,fix_version,title) VALUES(?1,?2,?3,?4,?5,'1.0.0','npm',?6,?7,'1.1.0','Synthetic fixture finding')").unwrap();
        let mut package=tx.prepare("INSERT INTO scan_packages(repo_id,scan_run_id,name,version,package_type,package_url) VALUES(?1,?2,?3,'1.0.0','npm',?4)").unwrap();
        for i in 0..total {
            let repo = i % repos + 1;
            let scan = repo * 2;
            let name = format!("package-{i:06}");
            let purl = format!("pkg:npm/{name}@1.0.0");
            finding
                .execute(params![
                    repo as i64,
                    scan as i64,
                    format!("fp-{i:06}"),
                    format!("CVE-PERF-{i:06}"),
                    name,
                    purl,
                    if i % 5 == 0 {
                        "CRITICAL"
                    } else if i % 3 == 0 {
                        "MEDIUM"
                    } else {
                        "HIGH"
                    }
                ])
                .unwrap();
            package
                .execute(params![repo as i64, scan as i64, name, purl])
                .unwrap();
        }
    }
    if total == 50_000 && repos == 100 {
        // Exercise the alias lookup on a populated feed as well as the
        // repository/finding/package ranking. Each advisory has three aliases.
        let mut advisory = tx.prepare(
            r#"INSERT INTO external_advisories
               (id,canonical_key,title,summary,url,package_name,ecosystem,published_at,severity,primary_source,source_item_id,raw_primary_json,fetched_at)
               VALUES (?1,?2,'Synthetic advisory',?3,'https://example.invalid',?4,?5,?6,'MEDIUM','github_advisory',?2,'{}','2026-10-14T00:00:00Z')"#,
        ).unwrap();
        let summary = if intel_benchmark {
            "Fixture description. ".repeat(256)
        } else {
            "Local fixture".to_owned()
        };
        let ecosystem = intel_benchmark.then_some("npm");
        let published_at = intel_benchmark.then_some("2026-10-14T00:00:00Z");
        let mut alias = tx
            .prepare("INSERT INTO external_advisory_aliases(advisory_id,alias) VALUES (?1,?2)")
            .unwrap();
        for id in 1..=3_000_i64 {
            advisory
                .execute(params![
                    id,
                    format!("GHSA-FIXTURE-{id:05}"),
                    summary,
                    intel_benchmark.then(|| format!("package-{id:06}")),
                    ecosystem,
                    published_at,
                ])
                .unwrap();
            for variant in 1..=3 {
                alias
                    .execute(params![id, format!("CVE-FIXTURE-{id:05}-{variant}")])
                    .unwrap();
            }
        }
    }
    tx.commit().unwrap();
    db
}

fn seed_intel_details(db: &Database) {
    let mut connection = Connection::open(db.path()).unwrap();
    let tx = connection.transaction().unwrap();
    tx.execute_batch(
        r#"
        INSERT INTO external_advisory_sources
            (advisory_id,source,source_item_id,url,severity,raw_json)
        SELECT id,'github',canonical_key,'https://example.invalid/advisory','medium','{}'
        FROM external_advisories;
        INSERT INTO external_advisory_references (advisory_id,url)
        SELECT id,'https://example.invalid/patch/' || id FROM external_advisories
        UNION ALL
        SELECT id,'https://example.invalid/discussion/' || id FROM external_advisories;
        INSERT INTO external_advisory_cwes (advisory_id,cwe_id,name)
        SELECT id,'CWE-20','Fixture weakness' FROM external_advisories;
        INSERT INTO external_advisory_affected_packages
            (advisory_id,ecosystem,package_name,vulnerable_range,first_patched_version)
        SELECT id,'npm',package_name,'< 1.1.0','1.1.0' FROM external_advisories;
        "#,
    )
    .unwrap();
    tx.commit().unwrap();
}

fn measure_dependencies(path: &Path, packages: usize, runs: usize) -> Vec<Value> {
    let finding_count = packages / 50;
    let db = fixture(path, finding_count, 1, false);
    let connection = Connection::open(db.path()).unwrap();
    connection
        .execute(
            r#"WITH RECURSIVE sequence(n) AS (
             SELECT ?1 UNION ALL SELECT n+1 FROM sequence WHERE n+1<?2
           )
           INSERT INTO scan_packages(repo_id,scan_run_id,name,version,package_type,package_url)
           SELECT 1,2,printf('package-%06d',n),'1.0.0',
                  CASE WHEN n%10=0 THEN 'pypi' ELSE 'npm' END,
                  printf('pkg:%s/package-%06d@1.0.0',CASE WHEN n%10=0 THEN 'pypi' ELSE 'npm' END,n)
           FROM sequence"#,
            params![finding_count as i64, packages as i64],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE scan_runs SET package_count=?1 WHERE id=2",
            [packages as i64],
        )
        .unwrap();
    drop(connection);

    let first = DependencyQuery {
        expected_scan_run_id: Some(2),
        ..Default::default()
    };
    let first_result = measure("dependencies_80", runs, || {
        db.dependency_page(1, &first).unwrap()
    });
    let first_page = db.dependency_page(1, &first).unwrap();
    assert_eq!(first_page.total as usize, packages);
    let next = DependencyQuery {
        cursor: first_page.next_cursor,
        ..first.clone()
    };
    let filtered = DependencyQuery {
        search: Some("PACKAGE-000".into()),
        ecosystem: Some("npm".into()),
        sort: Some("name".into()),
        ..first.clone()
    };
    let exact = DependencyQuery {
        exact_package: Some(ScannedPackageIdentity {
            name: "package-000010".into(),
            version: Some("1.0.0".into()),
            package_type: Some("npm".into()),
            package_url: Some("pkg:npm/package-000010@1.0.0".into()),
        }),
        ..first.clone()
    };
    assert_eq!(db.dependency_page(1, &exact).unwrap().items.len(), 1);
    vec![
        first_result,
        measure("dependencies_next_80", runs, || {
            db.dependency_page(1, &next).unwrap()
        }),
        measure_after(
            "dependencies_filter_80",
            runs,
            || {
                db.dependency_page(1, &first).unwrap();
            },
            || db.dependency_page(1, &filtered).unwrap(),
        ),
        measure_after(
            "dependencies_linked_package",
            runs,
            || {
                db.dependency_page(1, &first).unwrap();
            },
            || db.dependency_page(1, &exact).unwrap(),
        ),
    ]
}

fn run(db: &Database, runs: usize) -> Vec<Value> {
    let repo = db.list_active_repositories().unwrap()[0].id;
    let mut query = ReviewQuery {
        page_size: Some(100),
        ..Default::default()
    };
    query.filters.scanner_states = vec![RepoFindingState::Open, RepoFindingState::New];
    // Model opening the view after a different filter. Warm SQLite pages
    // are allowed, but the timed query must calculate its own group counts.
    let mut empty_view = query.clone();
    empty_view.filters.repo_ids = vec![0];
    let mut records = vec![measure_after(
        "review_page_100",
        runs,
        || {
            db.list_review_findings(&empty_view).unwrap();
        },
        || db.list_review_findings(&query).unwrap(),
    )];
    let mut next = query.clone();
    next.cursor = db.list_review_findings(&query).unwrap().next_cursor;
    if next.cursor.is_some() {
        records.push(measure("review_next_page_100", runs, || {
            db.list_review_findings(&next).unwrap()
        }));
    }
    records.push(measure("review_summary", runs, || {
        db.review_summary().unwrap()
    }));
    records.push(measure("risk_trend_30", runs, || {
        db.list_risk_trend(30).unwrap()
    }));
    let mut one = query.clone();
    one.filters.repo_ids = vec![repo];
    records.push(measure("review_one_repo", runs, || {
        db.list_review_findings(&one).unwrap()
    }));
    records.push(measure("repository_overview", runs, || {
        db.repo_overview(repo).unwrap()
    }));
    records.push(measure("dependencies_80", runs, || {
        db.dependency_page(
            repo,
            &DependencyQuery {
                page_size: Some(80),
                ..Default::default()
            },
        )
        .unwrap()
    }));
    records.push(measure("search_local_entities", runs, || {
        db.search_local_entities("package", 30).unwrap()
    }));
    records.push(measure("search_advisory_aliases", runs, || {
        db.search_local_entities("cve-fixture", 30).unwrap()
    }));
    let auth = GithubAuthSnapshot {
        checking: false,
        deferred: false,
        configured: false,
        client_id: None,
        config_revision: 0,
        session_revision: 0,
        can_configure: false,
        signed_in: false,
        login: None,
        avatar_url: None,
        profile_url: None,
        expires_at: None,
        rate_limit_remaining: None,
        rate_limit_limit: None,
        rate_limit_reset_at: None,
        install_url: None,
        message: None,
    };
    records.push(measure("persisted_snapshot", runs, || {
        db.snapshot(
            ScheduleInterval::Off,
            ScannerAvailability {
                checked_at: None,
                checking: false,
                ready: false,
                message: None,
                versions: None,
                grype_db: None,
            },
            "fixture".into(),
            false,
            "0.1.0".into(),
            auth.clone(),
            None,
            None,
        )
        .unwrap()
    }));
    records.push(
        json!({"operation":"review_query_plan","plans":db.performance_query_plans(repo).unwrap()}),
    );
    records
}
fn seed_chat(db: &Database) {
    use cvec::core::companion::*;
    let repo = db.list_active_repositories().unwrap()[0].id;
    db.set_ai_connection_enabled("codex_default", true).unwrap();
    for number in 0..50 {
        let thread = db
            .create_companion_thread(
                repo,
                &format!(
                    "local-history-stress-{number}-{}",
                    timestamps::unix_millis()
                ),
                &format!("History stress {number:02}"),
                None,
            )
            .unwrap();
        let items = (0..2000)
            .map(|index| CompanionTimelineItem {
                id: format!("message-{index}"),
                kind: CompanionTimelineItemKind::AssistantMessage,
                status: CompanionTimelineItemStatus::Completed,
                text: format!(
                    "History stress {number:02}, message {index}. {}",
                    "Local evidence and bounded history. ".repeat(8)
                ),
                activity: None,
                citations: Vec::new(),
                error: None,
                model_id: None,
                created_at: timestamps::now(),
            })
            .collect();
        db.cache_companion_thread_snapshot(
            thread.id,
            &CompanionThreadSnapshot {
                thread,
                items,
                usage: None,
            },
        )
        .unwrap();
    }
    db.set_ai_connection_enabled("codex_default", false)
        .unwrap();
    println!(
        "{}",
        json!({"threads":50,"messages_per_thread":2000,"external_requests":0})
    );
}
fn persistence(root: &Path, runs: usize) {
    use cvec::core::{db::ScanCompletion, delta};
    std::fs::create_dir_all(root).unwrap();
    for count in [1000, 10_000, 50_000] {
        let findings = (0..count)
            .map(|index| NormalizedFinding {
                fingerprint: format!("persist-{index}"),
                advisory_id: format!("CVE-PERF-{index}"),
                package_name: format!("package-{index}"),
                package_version: Some("1.0.0".into()),
                package_type: Some("npm".into()),
                package_url: Some(format!("pkg:npm/package-{index}@1.0.0")),
                severity: Severity::High,
                fix_version: Some("1.1.0".into()),
                title: None,
                source_url: None,
                advisory_details: None,
            })
            .collect::<Vec<_>>();
        let mut evidence = findings
            .iter()
            .map(|finding| NormalizedRemediationEvidence {
                schema_version: 1,
                fingerprint: finding.fingerprint.clone(),
                package_name: finding.package_name.clone(),
                package_version: finding.package_version.clone(),
                package_type: finding.package_type.clone(),
                package_url: finding.package_url.clone(),
                relative_path: None,
                location_kind: None,
                relationship: RemediationRelationship::Unknown,
                parent_package: None,
                source: RemediationEvidenceSource::None,
                quality: RemediationEvidenceQuality::Unavailable,
                location_count: 0,
            })
            .collect::<Vec<_>>();
        let mut unknown = evidence[0].clone();
        unknown.fingerprint = "unknown-fingerprint".into();
        evidence.push(unknown);
        let delta = delta::compute_delta(&[], &findings);
        let mut samples = Vec::new();
        for _ in 0..runs {
            let temp = tempfile::tempdir_in(root).unwrap();
            let db = Database::new(temp.path().join("persistence.db")).unwrap();
            db.sync_repositories(&[DiscoveredRepo {
                name: "persistence".into(),
                path: temp.path().join("repo"),
                remote_url: None,
                remote_host: None,
                provider: None,
            }])
            .unwrap();
            let repo = db.list_active_repositories().unwrap()[0].id;
            let run = db
                .insert_scan_start(
                    repo,
                    &ScannerVersions {
                        syft: "fixture".into(),
                        grype: "fixture".into(),
                    },
                )
                .unwrap();
            let start = Instant::now();
            db.complete_scan_with_remediation(ScanCompletion {
                repo_id: repo,
                run_id: run,
                findings: &findings,
                packages: &[],
                remediation_evidence: &evidence,
                source_snapshot: &SourceSnapshot {
                    revision: None,
                    dirty: None,
                },
                delta: &delta,
            })
            .unwrap();
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            let connection = Connection::open(db.path()).unwrap();
            assert_eq!(
                connection
                    .query_row("SELECT COUNT(*) FROM vulnerabilities", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                count as i64
            );
            assert_eq!(
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM vulnerability_remediation_evidence",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                count as i64
            );
            assert!(!connection
                .prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap());
        }
        let mut sorted = samples.clone();
        sorted.sort_by(f64::total_cmp);
        println!(
            "{}",
            json!({"findings":count,"evidence":count,"unknown_evidence_excluded":true,"phase":"canonical persistence only","profile":env!("CVEC_BUILD_PROFILE"),"source_sha256":env!("CVEC_BUILD_SOURCE_HASH"),"runs":runs,"p50_ms":sorted[(runs as f64*0.5).ceil() as usize-1],"p95_ms":sorted[(runs as f64*0.95).ceil() as usize-1],"p99_ms":sorted[(runs as f64*0.99).ceil() as usize-1],"samples_ms":samples})
        );
    }
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args.get(1).map(String::as_str).unwrap_or("matrix");
    let output = args.get(2).expect(
        "Usage: cvec-perf matrix|intel|dependencies <new temporary directory> [runs] | copy <temporary database copy> [runs]",
    );
    let path = Path::new(output)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(output).to_path_buf());
    assert!(
        path.starts_with("/tmp") || path.starts_with("/private/tmp"),
        "The harness accepts temporary fixture paths only"
    );
    let runs = args
        .get(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    assert!(runs >= 30, "At least 30 warm repeats are required");
    let identity = json!({"profile":env!("CVEC_BUILD_PROFILE"),"commit":env!("CVEC_BUILD_COMMIT"),"dirty":env!("CVEC_BUILD_DIRTY"),"cargo_lock_sha256":env!("CVEC_BUILD_CARGO_LOCK"),"npm_lock_sha256":env!("CVEC_BUILD_NPM_LOCK"),"source_sha256":env!("CVEC_BUILD_SOURCE_HASH"),"ipc_or_render":false,"first_call":"not a cold OS-cache measurement"});
    if mode == "chat" {
        seed_chat(&Database::new(path).unwrap());
        return;
    }
    if mode == "persistence" {
        assert!(!path.exists());
        persistence(&path, runs);
        return;
    }
    if mode == "copy" {
        let db = Database::new(path).unwrap();
        println!(
            "{}",
            json!({"identity":identity,"kind":"isolated real-data copy","results":run(&db,runs)})
        );
        return;
    }
    if mode == "dependencies" {
        std::fs::create_dir(&path).unwrap();
        for packages in [5_000, 50_000] {
            let results = measure_dependencies(
                &path.join(format!("dependencies-{packages}.db")),
                packages,
                runs,
            );
            println!(
                "{}",
                json!({"identity":identity,"kind":"synthetic","packages":packages,"findings":packages/50,"repositories":1,"results":results})
            );
        }
        return;
    }
    if mode == "intel" {
        std::fs::create_dir(&path).unwrap();
        let db = fixture(&path.join("intelligence.db"), 50_000, 100, true);
        seed_intel_details(&db);
        let connection = Connection::open(db.path()).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        for count in [3_000, 100] {
            // Run at the app's current feed limit as well as above it.
            if count == 100 {
                // Remove child rows in bulk before the parent rows, as refresh does.
                connection
                    .execute_batch(
                        "BEGIN;
                     DELETE FROM external_advisory_aliases WHERE advisory_id > 100;
                     DELETE FROM external_advisory_sources WHERE advisory_id > 100;
                     DELETE FROM external_advisory_cwes WHERE advisory_id > 100;
                     DELETE FROM external_advisory_references WHERE advisory_id > 100;
                     DELETE FROM external_advisory_affected_packages WHERE advisory_id > 100;
                     DELETE FROM external_advisories WHERE id > 100;
                     COMMIT;",
                    )
                    .unwrap();
            }
            // Reopen the database before the first feed call so that the
            // reported initial load also builds the in-process package index.
            let db = Database::new(db.path().to_path_buf()).unwrap();
            let feed_measurement = measure("external_intel_feed", runs, || {
                db.external_intel_feed().unwrap()
            });
            let feed = db.external_intel_feed().unwrap();
            assert_eq!(feed.items.len(), count);
            let results = [
                feed_measurement,
                measure("external_intel_detail", runs, || {
                    db.external_intel_detail(
                        "GHSA-FIXTURE-00001",
                        feed.intel_revision,
                        feed.inventory_revision,
                    )
                    .unwrap()
                    .unwrap()
                }),
            ];
            println!(
                "{}",
                json!({"identity":identity,"kind":"synthetic","advisories":count,"findings":50_000,"repositories":100,"results":results})
            );
        }
        return;
    }
    assert_eq!(mode, "matrix");
    assert!(!path.exists(), "Output directory must be new");
    std::fs::create_dir_all(&path).unwrap();
    for total in [1000, 10_000, 50_000] {
        for repos in [10, 100] {
            let db = fixture(
                &path.join(format!("fixture-{total}-{repos}.db")),
                total,
                repos,
                false,
            );
            println!(
                "{}",
                json!({"identity":identity,"kind":"synthetic","findings":total,"repositories":repos,"results":run(&db,runs)})
            );
        }
    }
}
