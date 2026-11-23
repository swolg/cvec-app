use std::collections::BTreeMap;
#[cfg(any(test, not(unix)))]
use std::fs;
use std::fs::File;
use std::io;
use std::path::Path;

use super::models::{RepoAsset, RepoRecord};
use super::secure_fs::read_regular_text;
#[cfg(unix)]
use super::secure_fs::AnchoredRoot;

const MAX_ASSET_FILE_BYTES: u64 = 1024 * 1024;
const WORKFLOWS: &str = ".github/workflows";

type AssetKey = (String, String, Option<String>, String, Option<String>);
type AssetMap = BTreeMap<AssetKey, RepoAsset>;

struct AssetSeed<'a> {
    repo_id: i64,
    vendor: &'a str,
    product: &'a str,
    version: Option<&'a str>,
    asset_kind: &'a str,
    source_path: Option<&'a str>,
    evidence: &'a str,
    confidence: u8,
}

pub fn extract_repo_assets(repo: &RepoRecord) -> Vec<RepoAsset> {
    let root = Path::new(&repo.path);
    extract_repo_assets_from_root(repo, root)
}

pub fn extract_repo_assets_from_root(repo: &RepoRecord, root: &Path) -> Vec<RepoAsset> {
    let mut assets = AssetMap::new();

    if let Some(provider) = repo.provider.as_deref() {
        match provider {
            "github" => insert_asset(
                &mut assets,
                AssetSeed {
                    repo_id: repo.id,
                    vendor: "github",
                    product: "github",
                    version: None,
                    asset_kind: "git_remote",
                    source_path: None,
                    evidence: repo.remote_host.as_deref().unwrap_or("github remote"),
                    confidence: 80,
                },
            ),
            "gitlab" => insert_asset(
                &mut assets,
                AssetSeed {
                    repo_id: repo.id,
                    vendor: "gitlab",
                    product: "gitlab",
                    version: None,
                    asset_kind: "git_remote",
                    source_path: None,
                    evidence: repo.remote_host.as_deref().unwrap_or("gitlab remote"),
                    confidence: 80,
                },
            ),
            _ => {}
        }
    }

    scan_repo_files(root, repo.id, &mut assets);
    assets.into_values().collect()
}

fn scan_repo_files(root: &Path, repo_id: i64, assets: &mut AssetMap) {
    #[cfg(unix)]
    let Ok(root) = root
        .canonicalize()
        .and_then(|path| AnchoredRoot::open(&path))
    else {
        return;
    };
    #[cfg(not(unix))]
    let Ok(root) = root.canonicalize() else {
        return;
    };
    #[cfg(unix)]
    let open_file = |path: &Path| root.entry(path, false)?.open();
    #[cfg(not(unix))]
    let open_file = |path: &Path| open_repo_file(&root, path);

    scan_workflows(&root, repo_id, assets);
    for path in [
        "Dockerfile",
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
        "Jenkinsfile",
        ".gitlab-ci.yml",
        "Chart.yaml",
        "values.yaml",
        "terraform/main.tf",
        "main.tf",
        "package.json",
    ] {
        scan_file(open_file(Path::new(path)), repo_id, path, assets);
    }
}

#[cfg(unix)]
fn scan_workflows(root: &AnchoredRoot, repo_id: i64, assets: &mut AssetMap) {
    let Ok(directory) = root.directory(Path::new(WORKFLOWS), false) else {
        return;
    };
    if let Ok(names) = directory.entry_names() {
        scan_workflow_files(
            names,
            |name| directory.entry(name, false)?.open(),
            repo_id,
            assets,
        );
    }
}

#[cfg(not(unix))]
fn scan_workflows(root: &Path, repo_id: i64, assets: &mut AssetMap) {
    let Ok(directory) = root.join(WORKFLOWS).canonicalize() else {
        return;
    };
    if directory.starts_with(root) {
        if let Ok(entries) = fs::read_dir(directory) {
            scan_workflow_files(
                entries.filter_map(|entry| entry.ok().map(|entry| entry.file_name())),
                |name| open_repo_file(root, &Path::new(WORKFLOWS).join(name)),
                repo_id,
                assets,
            );
        }
    }
}

#[cfg(not(unix))]
fn open_repo_file(root: &Path, relative: &Path) -> io::Result<File> {
    let path = root.join(relative);
    if !fs::symlink_metadata(&path)?.is_file() || !path.canonicalize()?.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Expected a regular file within the repository",
        ));
    }
    File::open(path)
}

fn scan_workflow_files(
    names: impl Iterator<Item = std::ffi::OsString>,
    open: impl Fn(&Path) -> io::Result<File>,
    repo_id: i64,
    assets: &mut AssetMap,
) {
    for name in names {
        let path = Path::new(&name);
        if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yml" | "yaml")
        ) {
            let relative = Path::new(WORKFLOWS).join(path);
            scan_file(open(path), repo_id, &relative.to_string_lossy(), assets);
        }
    }
}

fn scan_file(file: io::Result<File>, repo_id: i64, relative: &str, assets: &mut AssetMap) {
    if let Ok(contents) = file.and_then(|file| read_regular_text(file, MAX_ASSET_FILE_BYTES)) {
        scan_contents(repo_id, relative, &contents, assets);
    }
}

fn scan_contents(repo_id: i64, source_path: &str, contents: &str, assets: &mut AssetMap) {
    let lower = contents.to_lowercase();
    let path_lower = source_path.to_lowercase();

    if path_lower.contains("docker") || lower.contains("docker") || lower.contains("container:") {
        insert_asset(
            assets,
            AssetSeed {
                repo_id,
                vendor: "docker",
                product: "docker",
                version: None,
                asset_kind: "repo_signal",
                source_path: Some(source_path),
                evidence: "docker signal",
                confidence: 70,
            },
        );
    }
    if lower.contains("kubernetes")
        || lower.contains("apiversion: apps/v1")
        || lower.contains("kind: deployment")
        || lower.contains("helm.sh")
    {
        insert_asset(
            assets,
            AssetSeed {
                repo_id,
                vendor: "kubernetes",
                product: "kubernetes",
                version: None,
                asset_kind: "repo_signal",
                source_path: Some(source_path),
                evidence: "kubernetes/helm signal",
                confidence: 70,
            },
        );
    }
    if path_lower.contains("jenkinsfile") || lower.contains("jenkins") {
        insert_asset(
            assets,
            AssetSeed {
                repo_id,
                vendor: "jenkins",
                product: "jenkins",
                version: None,
                asset_kind: "repo_signal",
                source_path: Some(source_path),
                evidence: "jenkins signal",
                confidence: 70,
            },
        );
    }
    if path_lower.contains("gitlab") || lower.contains("gitlab") {
        insert_asset(
            assets,
            AssetSeed {
                repo_id,
                vendor: "gitlab",
                product: "gitlab",
                version: None,
                asset_kind: "repo_signal",
                source_path: Some(source_path),
                evidence: "gitlab signal",
                confidence: 70,
            },
        );
    }
    if lower.contains("self-hosted")
        || lower.contains("actions/runner")
        || lower.contains("github actions runner")
    {
        insert_asset(
            assets,
            AssetSeed {
                repo_id,
                vendor: "github",
                product: "actions_runner",
                version: None,
                asset_kind: "repo_signal",
                source_path: Some(source_path),
                evidence: "github actions runner signal",
                confidence: 75,
            },
        );
    }
    if lower.contains("github enterprise server")
        || lower.contains("ghes")
        || lower.contains("github_enterprise_server")
    {
        insert_asset(
            assets,
            AssetSeed {
                repo_id,
                vendor: "github",
                product: "enterprise_server",
                version: None,
                asset_kind: "repo_signal",
                source_path: Some(source_path),
                evidence: "github enterprise server signal",
                confidence: 85,
            },
        );
    }
    if lower.contains("nginx") {
        insert_asset(
            assets,
            AssetSeed {
                repo_id,
                vendor: "nginx",
                product: "nginx",
                version: None,
                asset_kind: "repo_signal",
                source_path: Some(source_path),
                evidence: "nginx signal",
                confidence: 65,
            },
        );
    }
    if lower.contains("postgres") || lower.contains("postgresql") {
        insert_asset(
            assets,
            AssetSeed {
                repo_id,
                vendor: "postgresql",
                product: "postgresql",
                version: None,
                asset_kind: "repo_signal",
                source_path: Some(source_path),
                evidence: "postgresql signal",
                confidence: 65,
            },
        );
    }
    if lower.contains("openssl") {
        insert_asset(
            assets,
            AssetSeed {
                repo_id,
                vendor: "openssl",
                product: "openssl",
                version: None,
                asset_kind: "repo_signal",
                source_path: Some(source_path),
                evidence: "openssl signal",
                confidence: 65,
            },
        );
    }
}

fn insert_asset(assets: &mut AssetMap, seed: AssetSeed<'_>) {
    let vendor = normalize_asset_part(seed.vendor);
    let product = normalize_asset_part(seed.product);
    let version = seed.version.map(str::to_string);
    let asset_kind = seed.asset_kind.to_string();
    let source_path = seed.source_path.map(str::to_string);
    let key = (
        vendor.clone(),
        product.clone(),
        version.clone(),
        asset_kind.clone(),
        source_path.clone(),
    );
    assets.entry(key).or_insert(RepoAsset {
        repo_id: seed.repo_id,
        vendor,
        product,
        version,
        asset_kind,
        source_path,
        evidence: seed.evidence.to_string(),
        confidence: seed.confidence,
    });
}

fn normalize_asset_part(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn detects_infra_assets_from_known_repo_signals() {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join(".github/workflows")).unwrap();
        fs::write(
            root.path().join(".github/workflows/deploy.yml"),
            "runs-on: [self-hosted]\n# GHES deployment\n",
        )
        .unwrap();
        fs::write(root.path().join("Dockerfile"), "FROM nginx:1.25\n").unwrap();
        fs::write(root.path().join("Chart.yaml"), "apiVersion: apps/v1\n").unwrap();

        let mut repo = fixture_repo(root.path());
        repo.remote_host = Some("github.example.com".into());
        repo.provider = Some("github".into());
        let keys = extract_repo_assets(&repo)
            .into_iter()
            .map(|asset| format!("{}:{}", asset.vendor, asset.product))
            .collect::<std::collections::BTreeSet<_>>();

        assert!(keys.contains("github:actions_runner"));
        assert!(keys.contains("github:enterprise_server"));
        assert!(keys.contains("docker:docker"));
        assert!(keys.contains("nginx:nginx"));
        assert!(keys.contains("kubernetes:kubernetes"));
    }

    #[cfg(unix)]
    #[test]
    fn asset_discovery_skips_pipes_links_and_oversized_files() {
        use std::os::unix::fs::symlink;
        use std::process::Command;
        use std::sync::mpsc;
        use std::time::Duration;

        let root = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let workflows = root.path().join(WORKFLOWS);
        fs::create_dir_all(&workflows).unwrap();
        fs::write(workflows.join("valid.yml"), "runs-on: [self-hosted]").unwrap();
        fs::write(outside.path().join("linked.yml"), "# jenkins").unwrap();
        symlink(
            outside.path().join("linked.yml"),
            workflows.join("linked.yml"),
        )
        .unwrap();
        fs::write(outside.path().join("main.tf"), "# openssl").unwrap();
        symlink(outside.path(), root.path().join("terraform")).unwrap();
        let oversized = root.path().join("package.json");
        fs::write(&oversized, "nginx").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(oversized)
            .unwrap()
            .set_len(MAX_ASSET_FILE_BYTES + 1)
            .unwrap();
        let pipe = root.path().join("Dockerfile");
        assert!(Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("mkfifo")
            .arg(workflows.join("pipe.yaml"))
            .status()
            .unwrap()
            .success());

        let repo = fixture_repo(root.path());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(extract_repo_assets(&repo)).unwrap());
        let assets = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("asset discovery blocked");
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].product, "actions_runner");
        assert_eq!(
            assets[0].source_path.as_deref(),
            Some(".github/workflows/valid.yml")
        );
    }

    fn fixture_repo(root: &Path) -> RepoRecord {
        RepoRecord {
            id: 42,
            name: "service".to_string(),
            path: root.display().to_string(),
            is_active: true,
            source_type: crate::core::models::RepoSourceType::Local,
            source_key: None,
            remote_url: None,
            remote_host: None,
            provider: None,
            default_branch: None,
            last_remote_commit_sha: None,
            last_remote_synced_at: None,
            remote_file_count: None,
        }
    }
}
