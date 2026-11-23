use std::fs;
use std::path::{Path, PathBuf};

use crate::Error;

use super::secure_fs::read_git_metadata;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredRepo {
    pub name: String,
    pub path: PathBuf,
    pub remote_url: Option<String>,
    pub remote_host: Option<String>,
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryReport {
    pub repositories: Vec<DiscoveredRepo>,
    pub warnings: Vec<String>,
}

impl DiscoveryReport {
    pub fn is_complete(&self) -> bool {
        self.warnings.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteMetadata {
    url: String,
    host: Option<String>,
    provider: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct DiscoveryLimits {
    max_depth: usize,
    max_directories: usize,
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            max_depth: 12,
            max_directories: 20_000,
        }
    }
}

pub fn discover_repositories(root: &Path) -> Result<Vec<DiscoveredRepo>, Error> {
    discover_repositories_with_limits(root, DiscoveryLimits::default())
}

pub fn discover_repositories_resilient(root: &Path) -> DiscoveryReport {
    discover_repositories_resilient_with_limits(root, DiscoveryLimits::default())
}

fn discover_repositories_resilient_with_limits(
    root: &Path,
    limits: DiscoveryLimits,
) -> DiscoveryReport {
    if !root.exists() {
        return DiscoveryReport {
            repositories: Vec::new(),
            warnings: vec![format!(
                "Repository discovery root is unavailable: {}",
                root.display()
            )],
        };
    }

    let mut report = DiscoveryReport {
        repositories: Vec::new(),
        warnings: Vec::new(),
    };
    let mut visited_directories = 0usize;
    visit_directory_resilient(root, &mut report, 0, &mut visited_directories, limits);
    report
        .repositories
        .sort_by(|left, right| left.path.cmp(&right.path));
    report
}

fn visit_directory_resilient(
    dir: &Path,
    report: &mut DiscoveryReport,
    depth: usize,
    visited_directories: &mut usize,
    limits: DiscoveryLimits,
) {
    if depth > limits.max_depth {
        report.warnings.push(format!(
            "Skipped {} because repository discovery exceeded depth {}",
            dir.display(),
            limits.max_depth
        ));
        return;
    }

    *visited_directories = visited_directories.saturating_add(1);
    if *visited_directories > limits.max_directories {
        if !report
            .warnings
            .iter()
            .any(|warning| warning.contains("directory limit"))
        {
            report.warnings.push(format!(
                "Stopped repository discovery after the directory limit of {}",
                limits.max_directories
            ));
        }
        return;
    }

    if is_git_repo(dir) {
        report.repositories.push(discover_repo(dir));
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(cause) => {
            report
                .warnings
                .push(format!("Could not read {}: {cause}", dir.display()));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(cause) => {
                report.warnings.push(format!(
                    "Could not read an entry in {}: {cause}",
                    dir.display()
                ));
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(cause) => {
                report.warnings.push(format!(
                    "Could not inspect {}: {cause}",
                    entry.path().display()
                ));
                continue;
            }
        };
        if file_type.is_symlink()
            || !file_type.is_dir()
            || should_skip_directory(&entry.file_name())
        {
            continue;
        }
        visit_directory_resilient(
            &entry.path(),
            report,
            depth + 1,
            visited_directories,
            limits,
        );
    }
}

fn discover_repositories_with_limits(
    root: &Path,
    limits: DiscoveryLimits,
) -> Result<Vec<DiscoveredRepo>, Error> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut repositories = Vec::new();
    let mut visited_directories = 0usize;
    visit_directory(root, &mut repositories, 0, &mut visited_directories, limits)?;
    repositories.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(repositories)
}

fn visit_directory(
    dir: &Path,
    repositories: &mut Vec<DiscoveredRepo>,
    depth: usize,
    visited_directories: &mut usize,
    limits: DiscoveryLimits,
) -> Result<(), Error> {
    if depth > limits.max_depth {
        return Err(discovery_limit_error("depth", limits.max_depth));
    }

    *visited_directories = visited_directories.saturating_add(1);
    if *visited_directories > limits.max_directories {
        return Err(discovery_limit_error(
            "directory count",
            limits.max_directories,
        ));
    }

    if is_git_repo(dir) {
        repositories.push(discover_repo(dir));
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        if should_skip_directory(&entry.file_name()) {
            continue;
        }

        visit_directory(
            &entry.path(),
            repositories,
            depth + 1,
            visited_directories,
            limits,
        )?;
    }

    Ok(())
}

fn discovery_limit_error(limit_name: &str, limit: usize) -> Error {
    Error::App(format!(
        "Repository discovery exceeded the {limit_name} limit of {limit}. Choose a narrower discovery root."
    ))
}

fn should_skip_directory(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_string_lossy().as_ref(),
        ".cache" | ".git" | "dist" | "node_modules" | "target"
    )
}

fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

fn discover_repo(dir: &Path) -> DiscoveredRepo {
    let name = dir
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string());

    let remote = read_remote_metadata(dir).ok().flatten();

    DiscoveredRepo {
        name,
        path: dir.to_path_buf(),
        remote_url: remote.as_ref().map(|value| value.url.clone()),
        remote_host: remote.as_ref().and_then(|value| value.host.clone()),
        provider: remote.and_then(|value| value.provider),
    }
}

fn read_remote_metadata(repo_dir: &Path) -> Result<Option<RemoteMetadata>, Error> {
    let git_dir = resolve_git_dir(repo_dir)?;
    let config_path = git_dir.join("config");
    if !config_path.exists() {
        return Ok(None);
    }

    let config = read_git_metadata(&config_path)?;
    let remote_url = parse_remote_url(&config);

    Ok(remote_url.map(|url| {
        let host = extract_host(&url);
        let provider = host.as_deref().map(classify_provider);
        RemoteMetadata {
            url,
            host,
            provider,
        }
    }))
}

fn resolve_git_dir(repo_dir: &Path) -> Result<PathBuf, Error> {
    let dot_git = repo_dir.join(".git");
    let metadata = fs::metadata(&dot_git)?;

    if metadata.is_dir() {
        return Ok(dot_git);
    }

    let contents = read_git_metadata(&dot_git)?;
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(relative_git_dir) = trimmed.strip_prefix("gitdir:") {
            let raw_path = relative_git_dir.trim();
            let resolved = Path::new(raw_path);
            return Ok(if resolved.is_absolute() {
                resolved.to_path_buf()
            } else {
                repo_dir.join(resolved)
            });
        }
    }

    Err(Error::App(format!(
        "unsupported .git file format for {}",
        repo_dir.display()
    )))
}

fn parse_remote_url(config: &str) -> Option<String> {
    let mut current_remote_name: Option<&str> = None;
    let mut first_remote_url: Option<String> = None;
    let mut origin_remote_url: Option<String> = None;

    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_remote_name = parse_remote_section_name(trimmed);
            continue;
        }

        let Some(remote_name) = current_remote_name else {
            continue;
        };

        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };

        if key.trim() != "url" {
            continue;
        }

        let url = value.trim().to_string();
        if first_remote_url.is_none() {
            first_remote_url = Some(url.clone());
        }

        if remote_name == "origin" && origin_remote_url.is_none() {
            origin_remote_url = Some(url);
        }
    }

    origin_remote_url.or(first_remote_url)
}

fn parse_remote_section_name(section: &str) -> Option<&str> {
    let section = section.strip_prefix('[')?.strip_suffix(']')?.trim();
    let section = section.strip_prefix("remote")?.trim();
    let section = section.strip_prefix('"')?.strip_suffix('"')?;
    Some(section)
}

fn extract_host(remote_url: &str) -> Option<String> {
    let trimmed = remote_url.trim();

    if !trimmed.contains("://") && trimmed.contains(':') {
        let without_user = trimmed
            .rsplit_once('@')
            .map(|(_, value)| value)
            .unwrap_or(trimmed);
        let host = without_user.split(':').next()?.trim();
        if !host.is_empty() && !host.contains('/') {
            return Some(host.to_ascii_lowercase());
        }
    }

    let (_, remainder) = trimmed.split_once("://")?;
    let authority = remainder.split('/').next().unwrap_or(remainder);
    let authority = authority
        .rsplit_once('@')
        .map(|(_, value)| value)
        .unwrap_or(authority);
    let host = authority
        .split(':')
        .next()
        .unwrap_or(authority)
        .trim_matches(&['[', ']'][..]);

    if host.is_empty() {
        return None;
    }

    Some(host.to_ascii_lowercase())
}

fn classify_provider(host: &str) -> String {
    if host == "github.com" || host.contains("github.") {
        "github".into()
    } else if host == "gitlab.com" || host.contains("gitlab.") {
        "gitlab".into()
    } else if host == "bitbucket.org" || host.contains("bitbucket.") {
        "bitbucket".into()
    } else if host == "dev.azure.com" || host.ends_with(".visualstudio.com") {
        "azure_devops".into()
    } else if host.contains("sourcehut.") {
        "sourcehut".into()
    } else {
        "custom".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn unavailable_root_preserves_monitored_repositories() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("mounted");
        fs::create_dir_all(root.join("app/.git")).expect("repository");
        let db = crate::core::db::Database::new(temp.path().join("cvec.db")).expect("database");
        let initial = discover_repositories_resilient(&root);
        db.sync_repositories_with_completeness(&initial.repositories, initial.is_complete())
            .expect("initial sync");
        fs::rename(&root, temp.path().join("unmounted")).expect("temporarily missing volume");
        let unavailable = discover_repositories_resilient(&root);
        assert!(!unavailable.is_complete());
        assert_eq!(unavailable.warnings.len(), 1);
        db.sync_repositories_with_completeness(
            &unavailable.repositories,
            unavailable.is_complete(),
        )
        .expect("partial sync");
        assert_eq!(
            db.list_active_repositories()
                .expect("monitored repositories")
                .len(),
            1
        );
        fs::rename(temp.path().join("unmounted"), &root).expect("remounted");
        let restored = discover_repositories_resilient(&root);
        assert!(restored.is_complete());
        assert_eq!(restored.repositories.len(), 1);
    }

    #[test]
    fn finds_git_directories_and_stops_descending() {
        let root = TempDir::new().unwrap();
        let top_repo = root.path().join("top");
        let nested_repo = top_repo.join("nested");
        let org_repo = root.path().join("org").join("service");

        fs::create_dir_all(top_repo.join(".git")).unwrap();
        fs::create_dir_all(&nested_repo).unwrap();
        fs::create_dir_all(org_repo.join(".git")).unwrap();

        let repos = discover_repositories(root.path()).unwrap();
        let paths: Vec<PathBuf> = repos.into_iter().map(|repo| repo.path).collect();

        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&top_repo));
        assert!(paths.contains(&org_repo));
        assert!(!paths.contains(&nested_repo));
    }

    #[test]
    fn reads_origin_remote_and_classifies_provider() {
        let root = TempDir::new().unwrap();
        let repo = root.path().join("service");
        let git_dir = repo.join(".git");

        fs::create_dir_all(&git_dir).unwrap();
        fs::write(
            git_dir.join("config"),
            r#"
[core]
    repositoryformatversion = 0
[remote "origin"]
    url = git@github.com:example/cvec.git
[remote "backup"]
    url = https://gitlab.com/example/cvec.git
"#,
        )
        .unwrap();

        let discovered = discover_repositories(root.path()).unwrap();
        let repo = discovered.first().unwrap();

        assert_eq!(
            repo.remote_url.as_deref(),
            Some("git@github.com:example/cvec.git")
        );
        assert_eq!(repo.remote_host.as_deref(), Some("github.com"));
        assert_eq!(repo.provider.as_deref(), Some("github"));
    }

    #[test]
    fn supports_gitdir_pointer_files() {
        let root = TempDir::new().unwrap();
        let repo = root.path().join("linked");
        let actual_git_dir = root.path().join(".git-linked");

        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&actual_git_dir).unwrap();
        fs::write(repo.join(".git"), "gitdir: ../.git-linked\n").unwrap();
        fs::write(
            actual_git_dir.join("config"),
            r#"
[remote "origin"]
    url = https://gitlab.com/example/cvec.git
"#,
        )
        .unwrap();

        let discovered = discover_repositories(root.path()).unwrap();
        let repo = discovered
            .iter()
            .find(|candidate| candidate.name == "linked")
            .unwrap();

        assert_eq!(repo.remote_host.as_deref(), Some("gitlab.com"));
        assert_eq!(repo.provider.as_deref(), Some("gitlab"));
    }

    #[cfg(unix)]
    #[test]
    fn discovery_continues_past_pipes_used_as_git_metadata() {
        use std::process::Command;
        use std::sync::mpsc;
        use std::time::Duration;

        let root = TempDir::new().unwrap();
        fs::create_dir(root.path().join("a-pointer")).unwrap();
        fs::create_dir_all(root.path().join("b-config/.git")).unwrap();
        fs::create_dir_all(root.path().join("c-valid/.git")).unwrap();
        fs::write(
            root.path().join("c-valid/.git/config"),
            "[remote \"origin\"]\n  url = git@github.com:example/service.git\n",
        )
        .unwrap();
        assert!(Command::new("mkfifo")
            .arg(root.path().join("a-pointer/.git"))
            .arg(root.path().join("b-config/.git/config"))
            .status()
            .unwrap()
            .success());

        let path = root.path().to_path_buf();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(discover_repositories_resilient(&path)).unwrap());
        let report = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("discovery blocked");
        assert_eq!(report.repositories.len(), 3);
        assert!(report.repositories[0].provider.is_none());
        assert!(report.repositories[1].provider.is_none());
        assert_eq!(report.repositories[2].provider.as_deref(), Some("github"));
    }

    #[test]
    fn skips_heavy_generated_directories() {
        let root = TempDir::new().unwrap();
        let ignored_repo = root.path().join("node_modules").join("package");
        let visible_repo = root.path().join("service");

        fs::create_dir_all(ignored_repo.join(".git")).unwrap();
        fs::create_dir_all(visible_repo.join(".git")).unwrap();

        let repos = discover_repositories(root.path()).unwrap();
        let paths = repos.into_iter().map(|repo| repo.path).collect::<Vec<_>>();

        assert_eq!(paths, vec![visible_repo]);
    }

    #[test]
    fn resilient_discovery_keeps_partial_results_and_reports_warnings() {
        let root = TempDir::new().unwrap();
        let visible_repo = root.path().join("repo");
        fs::create_dir_all(visible_repo.join(".git")).unwrap();
        fs::create_dir_all(root.path().join("too").join("deep")).unwrap();

        let report = discover_repositories_resilient_with_limits(
            root.path(),
            DiscoveryLimits {
                max_depth: 1,
                max_directories: 20_000,
            },
        );

        assert_eq!(report.repositories.len(), 1);
        assert_eq!(report.repositories[0].path, visible_repo);
        assert!(!report.is_complete());
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("depth")));
    }

    #[test]
    fn fails_when_discovery_depth_limit_is_exceeded() {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("org").join("service")).unwrap();

        let result = discover_repositories_with_limits(
            root.path(),
            DiscoveryLimits {
                max_depth: 0,
                max_directories: 20_000,
            },
        );

        assert!(result.is_err());
    }

    #[test]
    fn fails_when_discovery_directory_limit_is_exceeded() {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("org")).unwrap();

        let result = discover_repositories_with_limits(
            root.path(),
            DiscoveryLimits {
                max_depth: 12,
                max_directories: 1,
            },
        );

        assert!(result.is_err());
    }
}
