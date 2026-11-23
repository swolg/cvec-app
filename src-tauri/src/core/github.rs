use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use futures::{stream, StreamExt, TryStreamExt};
use serde::Deserialize;
use tempfile::TempDir;

use crate::Error;

use super::github_api::GithubApiClient;

const MAX_RELEVANT_FILES: usize = 500;
const MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;
const MAX_TOTAL_SIZE: u64 = 50 * 1024 * 1024;
const MAX_CONCURRENT_FILE_FETCHES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRepoRef {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRepositoryInfo {
    pub owner: String,
    pub repo: String,
    pub name: String,
    pub html_url: String,
    pub default_branch: String,
    pub commit_sha: String,
}

impl GithubRepositoryInfo {
    pub fn source_key(&self) -> String {
        format!(
            "github:{}/{}",
            self.owner.to_ascii_lowercase(),
            self.repo.to_ascii_lowercase()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRemoteFile {
    pub path: String,
    pub blob_sha: String,
    pub size: u64,
}

pub struct MaterializedGithubRepository {
    pub temp_dir: TempDir,
    pub info: GithubRepositoryInfo,
    pub files: Vec<GithubRemoteFile>,
}

impl MaterializedGithubRepository {
    pub fn root(&self) -> &Path {
        self.temp_dir.path()
    }
}

#[derive(Debug, Deserialize)]
struct RawRepository {
    name: String,
    html_url: String,
    default_branch: String,
    private: bool,
    owner: RawOwner,
}

#[derive(Debug, Deserialize)]
struct RawOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct RawBranch {
    commit: RawBranchCommit,
}

#[derive(Debug, Deserialize)]
struct RawBranchCommit {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct RawTree {
    tree: Vec<RawTreeEntry>,
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct RawTreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    sha: String,
    size: Option<u64>,
}

pub fn parse_github_url(value: &str) -> Result<GithubRepoRef, Error> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(Error::App("Enter a GitHub repository URL.".into()));
    }
    if trimmed.contains('?') || trimmed.contains('#') {
        return Err(Error::App(
            "GitHub repository URLs cannot include query strings or fragments.".into(),
        ));
    }

    let Some(remainder) = trimmed.strip_prefix("https://github.com/") else {
        return Err(Error::App(
            "Only https://github.com/{owner}/{repo} URLs are supported.".into(),
        ));
    };
    let remainder = remainder.trim_end_matches('/');
    let parts = remainder.split('/').collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err(Error::App(
            "Use the repository root URL, for example https://github.com/owner/repo.".into(),
        ));
    }

    let owner = parts[0].trim();
    let repo = parts[1]
        .trim()
        .strip_suffix(".git")
        .unwrap_or(parts[1].trim());
    if !is_valid_owner(owner) || !is_valid_repo(repo) {
        return Err(Error::App(
            "The GitHub owner or repository name is not valid.".into(),
        ));
    }

    Ok(GithubRepoRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

pub fn is_relevant_remote_path(path: &str) -> bool {
    let normalized = path.trim_matches('/').replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with(".git/")
        || normalized
            .split('/')
            .any(|part| part == ".." || part.is_empty())
    {
        return false;
    }

    let lower = normalized.to_ascii_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or(&normalized);
    let name_lower = name.to_ascii_lowercase();

    if matches!(
        name,
        "package.json"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "pnpm-workspace.yaml"
            | "pyproject.toml"
            | "poetry.lock"
            | "uv.lock"
            | "Pipfile"
            | "Pipfile.lock"
            | "go.mod"
            | "go.sum"
            | "Cargo.toml"
            | "Cargo.lock"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "gradle.lockfile"
            | "composer.json"
            | "composer.lock"
            | "Gemfile"
            | "Gemfile.lock"
            | "gems.locked"
            | "mix.exs"
            | "mix.lock"
            | "Dockerfile"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "compose.yml"
            | "compose.yaml"
            | ".gitlab-ci.yml"
            | "Jenkinsfile"
            | "Chart.yaml"
            | "values.yaml"
    ) {
        return true;
    }

    if name_lower.starts_with("requirements") && name_lower.ends_with(".txt") {
        return true;
    }
    if name.starts_with("Dockerfile.") {
        return true;
    }
    if lower.starts_with(".github/workflows/")
        && (lower.ends_with(".yml") || lower.ends_with(".yaml"))
    {
        return true;
    }
    if name_lower.ends_with(".tf") {
        return true;
    }

    false
}

pub async fn materialize_repository(
    repo_ref: &GithubRepoRef,
    token: Option<String>,
    blob_cache_dir: Option<&Path>,
) -> Result<MaterializedGithubRepository, Error> {
    let client = GithubApiClient::new(token.clone())?;
    let repository = fetch_repository(&client, repo_ref).await?;
    if repository.private && token.is_none() {
        return Err(Error::App(
            "Sign in with GitHub to scan private repositories.".into(),
        ));
    }

    let branch = fetch_branch(
        &client,
        &repo_ref.owner,
        &repo_ref.repo,
        &repository.default_branch,
    )
    .await?;
    let tree = fetch_tree(&client, &repo_ref.owner, &repo_ref.repo, &branch.commit.sha).await?;
    if tree.truncated {
        return Err(Error::App(
            "GitHub returned a truncated repository tree. cvec cannot scan this repository without cloning it."
                .into(),
        ));
    }

    let mut files = tree
        .tree
        .into_iter()
        .filter(|entry| entry.entry_type == "blob" && is_relevant_remote_path(&entry.path))
        .map(|entry| GithubRemoteFile {
            path: entry.path,
            blob_sha: entry.sha,
            size: entry.size.unwrap_or(0),
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));

    validate_file_limits(&files)?;

    let temp_dir = tempfile::Builder::new()
        .prefix("cvec-github-")
        .tempdir()
        .map_err(|cause| {
            Error::App(format!(
                "failed to create temporary scan directory: {cause}"
            ))
        })?;

    let actual_total_size = Arc::new(AtomicU64::new(0));
    let owner = repo_ref.owner.clone();
    let repo = repo_ref.repo.clone();
    let commit_sha = branch.commit.sha.clone();
    let cache_dir = blob_cache_dir.map(Path::to_path_buf);
    let downloaded = stream::iter(files.into_iter().map(|file| {
        let client = client.clone();
        let owner = owner.clone();
        let repo = repo.clone();
        let commit_sha = commit_sha.clone();
        let cache_dir = cache_dir.clone();
        let actual_total_size = Arc::clone(&actual_total_size);
        async move {
            let contents = materialize_file_contents(
                &client,
                &owner,
                &repo,
                &commit_sha,
                &file,
                cache_dir.as_deref(),
                &actual_total_size,
            )
            .await?;
            Ok::<_, Error>((file, contents))
        }
    }))
    .buffer_unordered(MAX_CONCURRENT_FILE_FETCHES)
    .try_collect::<Vec<_>>()
    .await?;

    let mut files = Vec::with_capacity(downloaded.len());
    let mut directories = HashSet::new();
    for (mut file, contents) in downloaded {
        file.size = contents.len() as u64;
        write_materialized_file(temp_dir.path(), &file.path, &contents, &mut directories)?;
        files.push(file);
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let info = GithubRepositoryInfo {
        owner: repository.owner.login,
        repo: repository.name.clone(),
        name: repository.name,
        html_url: repository.html_url,
        default_branch: repository.default_branch,
        commit_sha: branch.commit.sha,
    };

    Ok(MaterializedGithubRepository {
        temp_dir,
        info,
        files,
    })
}

async fn materialize_file_contents(
    client: &GithubApiClient,
    owner: &str,
    repo: &str,
    commit_sha: &str,
    file: &GithubRemoteFile,
    blob_cache_dir: Option<&Path>,
    actual_total_size: &AtomicU64,
) -> Result<Vec<u8>, Error> {
    if let Some(cache_path) = blob_cache_path(blob_cache_dir, &file.blob_sha) {
        if let Ok(contents) = read_cached_blob(&cache_path) {
            if file.size == 0 || contents.len() as u64 == file.size {
                reserve_actual_size(&file.path, contents.len() as u64, actual_total_size)?;
                return Ok(contents);
            }
        }
    }

    let contents =
        fetch_file_contents(client, owner, repo, commit_sha, &file.path, MAX_TOTAL_SIZE).await?;
    reserve_actual_size(&file.path, contents.len() as u64, actual_total_size)?;

    if let Some(cache_path) = blob_cache_path(blob_cache_dir, &file.blob_sha) {
        if let Err(cause) = write_cached_blob(&cache_path, &contents) {
            eprintln!(
                "Failed to cache GitHub blob {} at {}: {cause}",
                file.blob_sha,
                cache_path.display()
            );
        }
    }
    Ok(contents)
}

fn read_cached_blob(path: &Path) -> io::Result<Vec<u8>> {
    #[cfg(unix)]
    let file: fs::File = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?
    .into();
    #[cfg(not(unix))]
    let file = {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cached GitHub blob is not a regular file",
            ));
        }
        fs::File::open(path)?
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cached GitHub blob exceeds remote scan limits",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_FILE_SIZE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cached GitHub blob exceeded remote scan limit while reading",
        ));
    }
    Ok(bytes)
}

fn write_cached_blob(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "missing GitHub blob cache dir")
    })?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::symlink_metadata(parent) {
            if metadata.file_type().is_dir() {
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            }
        }
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temp.persist(path).map_err(|cause| cause.error)?;
    Ok(())
}

fn blob_cache_path(cache_dir: Option<&Path>, blob_sha: &str) -> Option<PathBuf> {
    let cache_dir = cache_dir?;
    let valid_sha =
        matches!(blob_sha.len(), 40 | 64) && blob_sha.bytes().all(|byte| byte.is_ascii_hexdigit());
    valid_sha.then(|| cache_dir.join(blob_sha.to_ascii_lowercase()))
}

fn reserve_actual_size(
    path: &str,
    actual_size: u64,
    actual_total_size: &AtomicU64,
) -> Result<(), Error> {
    validate_actual_file_size(path, actual_size, MAX_TOTAL_SIZE)?;
    let previous = actual_total_size.fetch_add(actual_size, Ordering::SeqCst);
    if previous.saturating_add(actual_size) > MAX_TOTAL_SIZE {
        return Err(Error::App(format!(
            "GitHub repository files exceed the {} MiB total download limit",
            MAX_TOTAL_SIZE / 1024 / 1024
        )));
    }
    Ok(())
}

async fn fetch_repository(
    client: &GithubApiClient,
    repo_ref: &GithubRepoRef,
) -> Result<RawRepository, Error> {
    let url = format!(
        "https://api.github.com/repos/{}/{}",
        encode_path_segment(&repo_ref.owner),
        encode_path_segment(&repo_ref.repo)
    );
    client.get_json(&url).await
}

async fn fetch_branch(
    client: &GithubApiClient,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<RawBranch, Error> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/branches/{}",
        encode_path_segment(owner),
        encode_path_segment(repo),
        encode_path_segment(branch)
    );
    client.get_json(&url).await
}

async fn fetch_tree(
    client: &GithubApiClient,
    owner: &str,
    repo: &str,
    commit_sha: &str,
) -> Result<RawTree, Error> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1",
        encode_path_segment(owner),
        encode_path_segment(repo),
        encode_path_segment(commit_sha)
    );
    client.get_json(&url).await
}

async fn fetch_file_contents(
    client: &GithubApiClient,
    owner: &str,
    repo: &str,
    commit_sha: &str,
    path: &str,
    remaining_total_size: u64,
) -> Result<Vec<u8>, Error> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
        encode_path_segment(owner),
        encode_path_segment(repo),
        encode_path(path),
        encode_path_segment(commit_sha)
    );
    client
        .get_raw_bytes_checked(&url, path, |actual_size| {
            validate_actual_file_size(path, actual_size, remaining_total_size)
        })
        .await
}

fn validate_file_limits(files: &[GithubRemoteFile]) -> Result<(), Error> {
    if files.is_empty() {
        return Err(Error::App(
            "No supported dependency, lock, CI, Docker, Helm, or Terraform files were found in this repository."
                .into(),
        ));
    }
    if files.len() > MAX_RELEVANT_FILES {
        return Err(Error::App(format!(
            "This repository has {} relevant files. The remote scan limit is {MAX_RELEVANT_FILES}.",
            files.len()
        )));
    }

    let mut total_size = 0u64;
    for file in files {
        if file.size > MAX_FILE_SIZE {
            return Err(Error::App(format!(
                "{} is too large for remote scanning. Maximum file size is 5 MB.",
                file.path
            )));
        }
        total_size = total_size.saturating_add(file.size);
    }
    if total_size > MAX_TOTAL_SIZE {
        return Err(Error::App(
            "The relevant files exceed the 50 MB remote scan limit.".into(),
        ));
    }

    Ok(())
}

fn validate_actual_file_size(
    path: &str,
    actual_size: u64,
    remaining_total_size: u64,
) -> Result<(), Error> {
    if actual_size > MAX_FILE_SIZE {
        return Err(Error::App(format!(
            "{path} is too large for remote scanning. Maximum file size is 5 MB."
        )));
    }
    if actual_size > remaining_total_size {
        return Err(Error::App(format!(
            "The relevant files exceed the 50 MB remote scan limit while downloading {path}."
        )));
    }

    Ok(())
}

fn safe_join(root: &Path, relative_path: &str) -> Result<PathBuf, Error> {
    let relative = Path::new(relative_path);
    if relative_path.is_empty()
        || relative_path.contains('\\')
        || relative.components().any(|component| !matches!(component, Component::Normal(name) if !name.eq_ignore_ascii_case(".git")))
    {
        return Err(Error::App(format!(
            "GitHub returned an unsafe file path: {relative_path}"
        )));
    }
    Ok(root.join(relative))
}

fn write_materialized_file(
    root: &Path,
    relative: &str,
    bytes: &[u8],
    directories: &mut HashSet<PathBuf>,
) -> Result<(), Error> {
    let target = safe_join(root, relative)?;
    if let Some(parent) = Path::new(relative).parent() {
        let mut directory = root.to_path_buf();
        for component in parent.components() {
            directory.push(component.as_os_str());
            if !directories.contains(&directory) {
                fs::create_dir(&directory).map_err(materialized_path_error)?;
                directories.insert(directory.clone());
            }
        }
    }
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .map_err(materialized_path_error)?
        .write_all(bytes)?;
    Ok(())
}

fn materialized_path_error(error: io::Error) -> Error {
    if error.kind() == io::ErrorKind::AlreadyExists {
        Error::App("GitHub returned paths that collide on this filesystem. Scan a local clone on a filesystem that preserves those paths.".into())
    } else {
        error.into()
    }
}

fn is_valid_owner(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 39
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn is_valid_repo(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(encode_path_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_github_urls() {
        assert_eq!(
            parse_github_url("https://github.com/example/cvec").unwrap(),
            GithubRepoRef {
                owner: "example".into(),
                repo: "cvec".into()
            }
        );
        assert_eq!(
            parse_github_url("https://github.com/example/cvec.git/").unwrap(),
            GithubRepoRef {
                owner: "example".into(),
                repo: "cvec".into()
            }
        );
    }

    #[test]
    fn rejects_unsupported_github_urls() {
        for url in [
            "git@github.com:example/cvec.git",
            "http://github.com/example/cvec",
            "https://raw.githubusercontent.com/example/cvec/main/package.json",
            "https://api.github.com/repos/example/cvec",
            "https://github.com/example-owner/example-repo/tree/main",
            "https://github.com/example/cvec?tab=readme",
            "https://localhost/example/cvec",
        ] {
            assert!(parse_github_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn builds_cache_paths_only_for_valid_blob_shas() {
        let cache = Path::new("/tmp/cvec-blobs");
        let sha = "0123456789abcdef0123456789abcdef01234567";

        assert_eq!(blob_cache_path(Some(cache), sha), Some(cache.join(sha)));
        assert_eq!(blob_cache_path(Some(cache), "../token"), None);
        assert_eq!(blob_cache_path(Some(cache), "not-a-git-sha"), None);
        assert_eq!(blob_cache_path(None, sha), None);
    }

    #[test]
    fn materialization_preserves_distinct_paths_without_overwriting_or_merging() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut directories = HashSet::new();
        write_materialized_file(root, "pkg/package.json", b"original", &mut directories).unwrap();
        write_materialized_file(root, "pkg/package-lock.json", b"lock", &mut directories).unwrap();
        assert!(write_materialized_file(
            root,
            "pkg/package.json",
            b"replacement",
            &mut directories
        )
        .is_err());
        assert_eq!(
            fs::read(root.join("pkg/package.json")).unwrap(),
            b"original"
        );

        let aliased_parent = root.join("PKG").exists();
        let different_case =
            write_materialized_file(root, "PKG/Cargo.toml", b"manifest", &mut directories);
        assert_eq!(different_case.is_err(), aliased_parent);
        assert!(!root.join("pkg/Cargo.toml").exists());
        assert!(safe_join(root, ".GIT/package.json").is_err());
    }

    #[test]
    fn matches_relevant_remote_paths() {
        for path in [
            "package-lock.json",
            "apps/web/package.json",
            "requirements-dev.txt",
            "pyproject.toml",
            "uv.lock",
            "go.mod",
            "Cargo.lock",
            "pom.xml",
            "build.gradle.kts",
            ".github/workflows/deploy.yml",
            ".gitlab-ci.yml",
            "Dockerfile.prod",
            "compose.yaml",
            "terraform/main.tf",
            "Chart.yaml",
        ] {
            assert!(is_relevant_remote_path(path), "{path}");
        }

        for path in [
            "src/main.rs",
            "README.md",
            ".github/ISSUE_TEMPLATE/bug.yml",
            "../package.json",
            ".git/config",
        ] {
            assert!(!is_relevant_remote_path(path), "{path}");
        }
    }

    #[test]
    fn rejects_actual_file_over_remote_scan_limit() {
        let result =
            validate_actual_file_size("package-lock.json", MAX_FILE_SIZE + 1, MAX_TOTAL_SIZE);

        assert!(result.is_err());
    }

    #[test]
    fn rejects_actual_size_over_remaining_remote_scan_limit() {
        // Used both during streaming (in get_raw_bytes_checked) and as final guard
        // after fetch_file_contents has returned the real bytes.
        let result = validate_actual_file_size("package-lock.json", 2, 1);

        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn blob_cache_enforces_owner_only_permissions_and_rejects_symlinks_and_oversized_files() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("blobs");
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let cached = blob_cache_path(Some(&cache_dir), sha).unwrap();

        write_cached_blob(&cached, b"{\"name\":\"fixture\"}").unwrap();
        assert_eq!(
            fs::symlink_metadata(&cache_dir)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::symlink_metadata(&cached).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            read_cached_blob(&cached).unwrap(),
            b"{\"name\":\"fixture\"}"
        );

        let external = temp.path().join("outside.txt");
        fs::write(&external, b"outside").unwrap();
        let symlink_blob = cache_dir.join("fedcba9876543210fedcba9876543210fedcba98");
        symlink(&external, &symlink_blob).unwrap();
        assert!(read_cached_blob(&symlink_blob).is_err());

        // Atomic write must replace the symlink rather than writing through it.
        write_cached_blob(&symlink_blob, b"safe").unwrap();
        assert_eq!(fs::read(&external).unwrap(), b"outside");
        assert_eq!(read_cached_blob(&symlink_blob).unwrap(), b"safe");

        let oversized = cache_dir.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let oversized_file = fs::File::create(&oversized).unwrap();
        oversized_file.set_len(MAX_FILE_SIZE + 1).unwrap();
        assert!(read_cached_blob(&oversized).is_err());
    }
}
