use super::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::models::{RepoDetail, RepoFindingState, ScanRunStatus};

const LODASH_MANIFEST: &str =
    r#"{"name":"fixture","version":"1.0.0","dependencies":{"lodash":"VERSION"}}"#;
const NPM_LOCK: &str = r#"{
    "name":"fixture","version":"1.0.0","lockfileVersion":3,"requires":true,
    "packages":{
        "":{"name":"fixture","version":"1.0.0","dependencies":{"lodash":"VERSION"}},
        "node_modules/lodash":{"version":"VERSION"}
    }
}"#;

struct TestRepo {
    path: PathBuf,
    database: PathBuf,
    repo: RepoRecord,
}

impl TestRepo {
    fn new(root: &Path, name: &str) -> Self {
        let path = root.join(name);
        fs::create_dir(&path).unwrap();
        git(&path, &["init", "-q"]);
        let database = root.join(format!("{name}.sqlite"));
        let db = Database::new(database.clone()).unwrap();
        db.sync_repositories(&repo_discovery::discover_repositories(&path).unwrap())
            .unwrap();
        let repo = db.list_active_repositories().unwrap().pop().unwrap();
        Self {
            path,
            database,
            repo,
        }
    }

    fn write(&self, name: &str, contents: &str) {
        let path = self.path.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn detail(&self) -> RepoDetail {
        Database::new(self.database.clone())
            .unwrap()
            .repo_detail(self.repo.id)
            .unwrap()
    }

    async fn scan(
        &self,
        cache: &Path,
        versions: &ScannerVersions,
        after_scan: impl AsyncFnOnce(&PreparedScanRoot),
    ) -> Result<ScanReviewChanges, Error> {
        let db = Database::new(self.database.clone()).unwrap();
        let run = db.insert_scan_start(self.repo.id, versions).unwrap();
        let prepared = prepare_local_scan_root(self.path.clone()).await.unwrap();
        let source = prepared.source_snapshot();
        let result =
            match scanner::scan_repository_with_progress(&self.path, Some(cache), |_| {}).await {
                Ok(output) => {
                    after_scan(&prepared).await;
                    persist_prepared_scan(db.clone(), self.repo.clone(), run, prepared, output)
                        .await
                        .map(|(changes, _)| changes)
                }
                Err(cause) => Err(cause),
            };
        if let Err(cause) = &result {
            db.fail_scan(self.repo.id, run, &cause.to_string(), Some(&source))
                .unwrap();
        }
        let latest = self.detail().latest_scan.unwrap();
        assert_eq!(latest.id, run);
        assert_eq!(latest.evidence_complete, result.is_ok());
        let expected = if result.is_ok() {
            ScanRunStatus::Completed
        } else {
            ScanRunStatus::Failed
        };
        assert_eq!(latest.status, expected);
        println!(
            "{} scan {run}: {:?}",
            self.path.file_name().unwrap().to_string_lossy(),
            result
        );
        result
    }
}

fn git(path: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "{args:?}");
}

fn open_fingerprints(detail: &RepoDetail) -> Vec<&str> {
    let mut fingerprints: Vec<_> = detail
        .findings
        .iter()
        .filter(|finding| finding.state != RepoFindingState::Resolved)
        .map(|finding| finding.fingerprint.as_str())
        .collect();
    fingerprints.sort_unstable();
    fingerprints
}

fn assert_preserved(repo: &TestRepo, baseline: &RepoDetail, error: Error, reason: &str) {
    assert!(error.to_string().contains(reason), "{error}");
    let detail = repo.detail();
    assert_eq!(open_fingerprints(&detail), open_fingerprints(baseline));
    assert_eq!(detail.packages, baseline.packages);
    assert_eq!(
        detail
            .scan_history
            .iter()
            .find(|scan| scan.status == ScanRunStatus::Completed)
            .unwrap()
            .id,
        baseline.latest_scan.as_ref().unwrap().id,
    );
    assert!(detail
        .findings
        .iter()
        .all(|finding| finding.state != RepoFindingState::Resolved));
    let latest = detail.latest_scan.unwrap();
    assert_eq!(
        latest.resolved_medium + latest.resolved_high + latest.resolved_critical,
        0
    );
}

// Keep live scanner dependencies and their vulnerability database out of the
// ordinary test suite. The subprocess confines scanner configuration to a
// disposable directory and the supplied cache should be an isolated copy.
#[tokio::test]
#[ignore = "requires Syft, Grype and CVEC_TEST_GRYPE_CACHE_DIR"]
async fn real_scans_preserve_and_reconcile_sqlite_findings() {
    let cache = PathBuf::from(std::env::var_os("CVEC_TEST_GRYPE_CACHE_DIR").unwrap())
        .canonicalize()
        .unwrap();
    assert!(cache.join("6/vulnerability.db").is_file());
    const TEST: &str =
        "commands::real_scan_tests::real_scans_preserve_and_reconcile_sqlite_findings";
    let temp = tempfile::tempdir().unwrap();
    if std::env::var_os("CVEC_TEST_REAL_SCAN_CHILD").is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([TEST, "--exact", "--ignored", "--nocapture"])
            .current_dir(temp.path())
            .env("CVEC_TEST_REAL_SCAN_CHILD", "1")
            .env("GRYPE_DB_AUTO_UPDATE", "false")
            .env("SYFT_CHECK_FOR_APP_UPDATE", "false")
            .env_remove("SYFT_EXCLUDE")
            .output()
            .unwrap();
        println!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let root = temp.path();
    let versions = scanner::versions().unwrap();
    println!("{} / {}", versions.syft, versions.grype);
    let npm = TestRepo::new(root, "npm");
    let config = std::env::current_dir().unwrap().join(".syft.yaml");
    let manifest = "apps/ä/package.json";
    let lockfile = "apps/ä/package-lock.json";
    let write_npm = |version| {
        npm.write(manifest, &LODASH_MANIFEST.replace("VERSION", version));
        npm.write(lockfile, &NPM_LOCK.replace("VERSION", version));
    };
    write_npm("4.17.20");
    npm.write(
        "README.md",
        "An exclusion here does not affect dependencies.\n",
    );
    git(&npm.path, &["add", "."]);
    npm.scan(&cache, &versions, async |_| {}).await.unwrap();
    let baseline = npm.detail();
    assert!(open_fingerprints(&baseline).len() >= 4);

    // Reopen an older database with existing findings and no stored scan scope.
    rusqlite::Connection::open(&npm.database)
        .unwrap()
        .execute_batch("ALTER TABLE scan_runs DROP COLUMN scan_scope; PRAGMA user_version = 10;")
        .unwrap();

    npm.write(manifest, &LODASH_MANIFEST.replace("VERSION", "4.17.21"));
    fs::write(&config, "exclude: ['./apps/??/**']\n").unwrap();
    let error = npm.scan(&cache, &versions, async |_| {}).await.unwrap_err();
    assert_preserved(&npm, &baseline, error, "Incomplete npm inventory");

    fs::write(&config, "exclude: ['./apps/?/**']\n").unwrap();
    let report = scanner::scan_repository(&npm.path, Some(&cache))
        .await
        .unwrap();
    assert!(report.findings.is_empty());
    let error = npm.scan(&cache, &versions, async |_| {}).await.unwrap_err();
    assert_preserved(&npm, &baseline, error, "scan scope changed");
    fs::remove_file(&config).unwrap();

    write_npm("4.17.21");
    let error = npm
        .scan(&cache, &versions, async |prepared| {
            npm.write(
                manifest,
                &(LODASH_MANIFEST.replace("VERSION", "4.17.20") + "\n"),
            );
            npm.write(lockfile, &(NPM_LOCK.replace("VERSION", "4.17.20") + "\n"));
            let PreparedScanRoot::Local { path, source } = prepared else {
                unreachable!()
            };
            // Git still shows the same dirty paths and status codes.
            source.verify(path).await.unwrap();
        })
        .await
        .unwrap_err();
    assert_preserved(
        &npm,
        &baseline,
        error,
        "Dependency inputs or Syft settings changed",
    );

    let missing_database = root.join("scanner-error/grype/db");
    let error = npm
        .scan(&missing_database, &versions, async |_| {})
        .await
        .unwrap_err();
    assert_preserved(&npm, &baseline, error, "grype failed");

    fs::write(&config, "exclude: ['./README.md']\n").unwrap();
    let narrowed = npm.scan(&cache, &versions, async |_| {}).await.unwrap();
    assert_eq!(narrowed.resolved_findings, 0);
    assert_eq!(
        open_fingerprints(&npm.detail()),
        open_fingerprints(&baseline)
    );
    fs::remove_file(config).unwrap();

    write_npm("4.17.21");
    let changes = npm.scan(&cache, &versions, async |_| {}).await.unwrap();
    let updated = npm.detail();
    assert!(changes.resolved_findings > 0);
    assert!(open_fingerprints(&updated).len() < open_fingerprints(&baseline).len());
    assert!(updated
        .findings
        .iter()
        .any(|finding| finding.state == RepoFindingState::Resolved));
    assert!(updated
        .packages
        .iter()
        .any(|package| package.name == "lodash" && package.version.as_deref() == Some("4.17.21")));

    let cargo = TestRepo::new(root, "cargo");
    cargo.write("Cargo.toml", "[package]\nname = 'fixture'\nversion = '0.1.0'\n[dependencies]\nhyper = '=0.14.1'\nopenssl = '=0.10.30'\ntime = '=0.3.3'\n");
    cargo.write(
        "Cargo.lock",
        r#"version = 4

[[package]]
name = "fixture"
version = "0.1.0"
dependencies = ["hyper", "openssl", "time"]

[[package]]
name = "hyper"
version = "0.14.1"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "openssl"
version = "0.10.30"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "time"
version = "0.3.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
    );
    cargo.scan(&cache, &versions, async |_| {}).await.unwrap();
    let baseline = cargo.detail();
    assert!(!open_fingerprints(&baseline).is_empty());
    cargo.write("Cargo.lock", "[[package]]\nname =\n");
    let error = cargo
        .scan(&cache, &versions, async |_| {})
        .await
        .unwrap_err();
    assert_preserved(&cargo, &baseline, error, "Cargo.lock");
    cargo.write("Cargo.lock", "version = 4\n");
    let error = cargo
        .scan(&cache, &versions, async |_| {})
        .await
        .unwrap_err();
    assert_preserved(&cargo, &baseline, error, "Cargo.toml");

    let python = TestRepo::new(root, "python");
    python.write("legacy/requirements.txt", "requests==2.19.1\n");
    python.write("current/requirements.txt", "requests==2.32.5\n");
    python.scan(&cache, &versions, async |_| {}).await.unwrap();
    let baseline = python.detail();
    assert!(!open_fingerprints(&baseline).is_empty());
    python.write("legacy/requirements.txt", "requests>=2.19.1\n");
    let error = python
        .scan(&cache, &versions, async |_| {})
        .await
        .unwrap_err();
    assert_preserved(&python, &baseline, error, "legacy/requirements.txt");
    python.write(
        "legacy/.venv/lib/python3.13/site-packages/requests-2.32.5.dist-info/METADATA",
        "Metadata-Version: 2.1\nName: requests\nVersion: 2.32.5\n",
    );
    let resolved = python.scan(&cache, &versions, async |_| {}).await.unwrap();
    assert!(resolved.resolved_findings > 0);

    let linked = TestRepo::new(root, "linked");
    linked.write(
        "declared.json",
        &LODASH_MANIFEST.replace("VERSION", "4.17.20"),
    );
    linked.write(
        "app/package-lock.json",
        &NPM_LOCK.replace("VERSION", "4.17.20"),
    );
    symlink("../declared.json", linked.path.join("app/package.json")).unwrap();
    linked.scan(&cache, &versions, async |_| {}).await.unwrap();
    let baseline = linked.detail();
    assert!(!open_fingerprints(&baseline).is_empty());
    linked.write(
        "declared.json",
        &LODASH_MANIFEST.replace("VERSION", "4.17.21"),
    );
    let error = linked
        .scan(&cache, &versions, async |_| {})
        .await
        .unwrap_err();
    assert_preserved(&linked, &baseline, error, "Incomplete npm inventory");
    fs::remove_file(linked.path.join("app/package.json")).unwrap();
    symlink(
        npm.path.join(manifest),
        linked.path.join("app/package.json"),
    )
    .unwrap();
    let error = linked
        .scan(&cache, &versions, async |_| {})
        .await
        .unwrap_err();
    assert_preserved(&linked, &baseline, error, "inside the scan source");

    let pnpm = TestRepo::new(root, "pnpm");
    pnpm.write(
        "package.json",
        &LODASH_MANIFEST.replace("VERSION", "4.17.20"),
    );
    pnpm.write("pnpm-workspace.yaml", "overrides:\n  lodash: 4.17.21\n");
    pnpm.write("pnpm-lock.yaml", r#"lockfileVersion: '9.0'
settings:
  autoInstallPeers: true
  excludeLinksFromLockfile: false
overrides:
  lodash: 4.17.21
importers:
  .:
    dependencies:
      lodash:
        specifier: 4.17.21
        version: 4.17.21
packages:
  lodash@4.17.21:
    resolution: {integrity: sha512-v2kDEe57lecTulaDIuNTPy3Ry4gLGJ6Z1O3vE1krgXZNrsQ+LFTGHVxVjcXPs17LhbZVGedAJv8XZ1tvj5FvSg==}
snapshots:
  lodash@4.17.21: {}
"#);
    pnpm.scan(&cache, &versions, async |_| {}).await.unwrap();
    let baseline = pnpm.detail();
    assert!(baseline
        .packages
        .iter()
        .any(|package| package.name == "lodash" && package.version.as_deref() == Some("4.17.21")));
    pnpm.write("pnpm-workspace.yaml", "overrides:\n  lodash: 4.17.20\n");
    let error = pnpm
        .scan(&cache, &versions, async |_| {})
        .await
        .unwrap_err();
    assert_preserved(&pnpm, &baseline, error, "pnpm-lock.yaml");
    pnpm.write("package.json", r#"{"name":"fixture","version":"1.0.0","dependencies":{"lodash":"4.17.20"},"pnpm":{"overrides":{"lodash":"4.17.21"}}}"#);
    let legacy = pnpm.scan(&cache, &versions, async |_| {}).await.unwrap();
    assert_eq!(legacy.resolved_findings, 0);
    assert_eq!(pnpm.detail().packages, baseline.packages);
    pnpm.write(
        "package.json",
        &LODASH_MANIFEST.replace("VERSION", "4.17.20"),
    );
    pnpm.write(
        "pnpm-workspace.yaml",
        "catalog:\n  lodash: 4.17.21\noverrides:\n  lodash: 'catalog:'\n",
    );
    pnpm.scan(&cache, &versions, async |_| {}).await.unwrap();
    assert_eq!(pnpm.detail().packages, baseline.packages);

    let empty = TestRepo::new(root, "empty");
    empty.write("README.md", "No dependencies\n");
    empty.scan(&cache, &versions, async |_| {}).await.unwrap();
    assert!(empty.detail().packages.is_empty());
    assert!(empty.detail().findings.is_empty());
    for repo in [&npm, &cargo, &python, &linked, &pnpm, &empty] {
        let db = rusqlite::Connection::open(&repo.database).unwrap();
        let integrity: String = db
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        assert!(!db
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
    }
}
