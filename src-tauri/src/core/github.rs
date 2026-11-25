use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use futures::{stream, StreamExt, TryStreamExt};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use tempfile::TempDir;

use crate::Error;

use super::github_api::GithubApiClient;

const MIB: u64 = 1024 * 1024;
const MAX_FILE_SIZE: u64 = 100 * MIB;
const MAX_TOTAL_SIZE: u64 = 512 * MIB;
const MAX_COMPRESSED_SIZE: u64 = 128 * MIB;
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
    pub coverage_gaps: Vec<String>,
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
    mode: String,
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

pub fn is_dependency_input_path(path: &str) -> bool {
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
        name_lower.as_str(),
        "package.json"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "bun.lock"
            | "bun.lockb"
            | "deno.lock"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "pnpm-workspace.yaml"
            | "pyproject.toml"
            | "poetry.lock"
            | "pylock.toml"
            | "pdm.lock"
            | "setup.py"
            | "setup.cfg"
            | "uv.lock"
            | "pipfile"
            | "pipfile.lock"
            | "packages.lock.json"
            | "packages.config"
            | "project.assets.json"
            | "nuget.config"
            | "global.json"
            | "directory.build.props"
            | "directory.build.targets"
            | "directory.packages.props"
            | "go.work"
            | "go.mod"
            | "go.sum"
            | "cargo.toml"
            | "cargo.lock"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "gradle.lockfile"
            | "composer.json"
            | "composer.lock"
            | "gemfile"
            | "gemfile.lock"
            | "gems.locked"
            | "mix.exs"
            | "mix.lock"
            | "dockerfile"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "compose.yml"
            | "compose.yaml"
            | ".gitlab-ci.yml"
            | "jenkinsfile"
            | "chart.yaml"
            | "values.yaml"
    ) {
        return true;
    }

    if [
        ".csproj",
        ".fsproj",
        ".vbproj",
        ".deps.json",
        ".props",
        ".targets",
    ]
    .iter()
    .any(|extension| name_lower.ends_with(extension))
    {
        return true;
    }
    if name_lower.starts_with("packages.") && name_lower.ends_with(".lock.json") {
        return true;
    }
    if name_lower.ends_with(".py.lock")
        || (name_lower.starts_with("pylock.") && name_lower.ends_with(".toml"))
    {
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

    let mut entries = BTreeMap::new();
    let mut coverage_gaps = Vec::new();
    for entry in tree.tree {
        match entry.entry_type.as_str() {
            "blob" => {
                if entries.insert(entry.path.clone(), entry).is_some() {
                    return Err(Error::App("GitHub returned a duplicate path.".into()));
                }
            }
            "commit" => coverage_gaps.push(format!(
                "Submodule contents are unavailable at {}",
                entry.path
            )),
            _ => {}
        }
    }
    validate_file_limits(&entries)?;

    let archive_url = format!(
        "https://api.github.com/repos/{}/{}/tarball/{}",
        encode_path_segment(&repo_ref.owner),
        encode_path_segment(&repo_ref.repo),
        encode_path_segment(&branch.commit.sha),
    );
    let archive = client
        .get_raw_bytes_checked(&archive_url, "commit archive", |size| {
            if size > MAX_COMPRESSED_SIZE {
                return Err(snapshot_size_error());
            }
            Ok(())
        })
        .await?;
    let entries = std::sync::Arc::new(entries);
    let archive_entries = entries.clone();
    let (mut snapshot, missing) = crate::runtime::run_io(move || {
        let mut snapshot = RemoteSnapshot::new()?;
        let found = snapshot.read_archive(&archive[..], &archive_entries)?;
        let missing = archive_entries
            .keys()
            .filter(|path| !found.contains(*path))
            .cloned()
            .collect::<Vec<_>>();
        Ok::<_, Error>((snapshot, missing))
    })
    .await
    .map_err(Error::App)??;

    // Git attributes can omit files from an archive or substitute their contents.
    // Use Git's blob endpoint for those tracked files and verify their identities.
    let downloads = stream::iter(missing.into_iter().map(|path| {
        let entries = entries.clone();
        let client = client.clone();
        async move {
            let bytes =
                load_verified_blob(&client, repo_ref, &entries[&path], blob_cache_dir).await?;
            Ok::<_, Error>((path, bytes))
        }
    }))
    .buffer_unordered(MAX_CONCURRENT_FILE_FETCHES);
    futures::pin_mut!(downloads);
    while let Some((path, bytes)) = downloads.try_next().await? {
        snapshot.write_entry(&entries[&path], &bytes)?;
    }

    coverage_gaps.extend(snapshot.finish_links()?);
    coverage_gaps.append(&mut snapshot.coverage_gaps);
    coverage_gaps.sort();

    let files = entries
        .values()
        .filter(|entry| is_dependency_input_path(&entry.path))
        .map(|entry| GithubRemoteFile {
            path: entry.path.clone(),
            blob_sha: entry.sha.clone(),
            size: entry.size.unwrap_or(0),
        })
        .collect();
    let info = GithubRepositoryInfo {
        owner: repository.owner.login,
        repo: repository.name.clone(),
        name: repository.name,
        html_url: repository.html_url,
        default_branch: repository.default_branch,
        commit_sha: branch.commit.sha,
    };

    Ok(MaterializedGithubRepository {
        temp_dir: snapshot.directory,
        info,
        files,
        coverage_gaps,
    })
}

struct RemoteSnapshot {
    directory: TempDir,
    directories: HashSet<PathBuf>,
    links: BTreeMap<String, Vec<u8>>,
    coverage_gaps: Vec<String>,
}

impl RemoteSnapshot {
    fn new() -> Result<Self, Error> {
        Ok(Self {
            directory: tempfile::Builder::new().prefix("cvec-github-").tempdir()?,
            directories: HashSet::new(),
            links: BTreeMap::new(),
            coverage_gaps: Vec::new(),
        })
    }

    fn read_archive(
        &mut self,
        compressed: impl Read,
        entries: &BTreeMap<String, RawTreeEntry>,
    ) -> Result<BTreeSet<String>, Error> {
        // Account for TAR headers and padding separately from the Git blob budget.
        let archive_limit = MAX_TOTAL_SIZE + 64 * MIB;
        let mut archive = tar::Archive::new(GzDecoder::new(compressed).take(archive_limit));
        let mut found = BTreeSet::new();
        let mut prefix = None;
        for item in archive.entries()? {
            let mut item = item?;
            if item.header().entry_type().is_pax_global_extensions() {
                continue;
            }
            let path = item.path()?.into_owned();
            let mut components = path.components();
            let Some(Component::Normal(top)) = components.next() else {
                return Err(Error::App(
                    "GitHub returned an invalid archive path.".into(),
                ));
            };
            if prefix.get_or_insert_with(|| top.to_os_string()) != top {
                return Err(Error::App("GitHub returned multiple archive roots.".into()));
            }
            let relative = components.as_path().to_str().ok_or_else(|| {
                Error::App("GitHub returned a path that cannot be read as UTF-8.".into())
            })?;
            if relative.is_empty() {
                continue;
            }
            safe_join(self.directory.path(), relative)?;
            let Some(entry) = entries.get(relative) else {
                continue;
            };
            let entry_type = item.header().entry_type();
            let contents = if entry.mode == "120000" && entry_type.is_symlink() {
                item.link_name_bytes().map(|bytes| bytes.into_owned())
            } else if entry.mode != "120000" && entry_type.is_file() && item.size() <= MAX_FILE_SIZE
            {
                let mut bytes = Vec::new();
                item.read_to_end(&mut bytes)?;
                Some(bytes)
            } else {
                None
            };
            if let Some(bytes) = contents.filter(|bytes| verified_blob(entry, bytes)) {
                if !found.insert(relative.to_owned()) {
                    return Err(Error::App(
                        "GitHub returned a duplicate archive entry.".into(),
                    ));
                }
                self.write_entry(entry, &bytes)?;
            }
        }
        Ok(found)
    }

    fn write_entry(&mut self, entry: &RawTreeEntry, bytes: &[u8]) -> Result<(), Error> {
        if entry.mode == "120000" {
            self.links.insert(entry.path.clone(), bytes.to_vec());
        } else {
            if bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\n") {
                self.coverage_gaps.push(format!(
                    "Git LFS contents are unavailable at {}",
                    entry.path
                ));
            }
            write_materialized_file(
                self.directory.path(),
                &entry.path,
                bytes,
                &mut self.directories,
            )?;
        }
        Ok(())
    }

    fn finish_links(&mut self) -> Result<Vec<String>, Error> {
        let mut gaps = Vec::new();
        let root = self.directory.path().canonicalize()?;
        let mut created = Vec::new();
        for (path, bytes) in std::mem::take(&mut self.links) {
            let target = std::str::from_utf8(&bytes).ok().map(Path::new);
            let relative = Path::new(&path);
            let Some(target) = target.filter(|target| internal_link(relative, target)) else {
                gaps.push(format!("Symbolic link target is unavailable at {path}"));
                continue;
            };
            #[cfg(unix)]
            {
                let link = safe_join(&root, &path)?;
                ensure_parent_directories(self.directory.path(), &path, &mut self.directories)?;
                std::os::unix::fs::symlink(target, &link).map_err(materialized_path_error)?;
                created.push((path, link));
            }
            #[cfg(not(unix))]
            {
                let _ = target;
                gaps.push(format!("Symbolic link target is unavailable at {path}"));
            }
        }
        for (path, link) in created {
            if !link
                .canonicalize()
                .is_ok_and(|target| target.starts_with(&root))
            {
                fs::remove_file(link)?;
                gaps.push(format!("Symbolic link target is unavailable at {path}"));
            }
        }
        Ok(gaps)
    }
}

fn internal_link(link: &Path, target: &Path) -> bool {
    if target.as_os_str().is_empty() {
        return false;
    }
    let mut depth = link
        .parent()
        .map_or(0, |parent| parent.components().count());
    for component in target.components() {
        match component {
            Component::Normal(name) if !name.eq_ignore_ascii_case(".git") => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return false,
        }
    }
    true
}

fn verified_blob(entry: &RawTreeEntry, bytes: &[u8]) -> bool {
    if entry.size.is_some_and(|size| size != bytes.len() as u64) {
        return false;
    }
    let header = format!("blob {}\0", bytes.len());
    match entry.sha.len() {
        40 => {
            let mut hash = Sha1::new();
            hash.update(header);
            hash.update(bytes);
            super::hashing::lowercase_hex(&hash.finalize()) == entry.sha
        }
        64 => {
            let mut hash = Sha256::new();
            hash.update(header);
            hash.update(bytes);
            super::hashing::lowercase_hex(&hash.finalize()) == entry.sha
        }
        _ => false,
    }
}

async fn load_verified_blob(
    client: &GithubApiClient,
    repo: &GithubRepoRef,
    entry: &RawTreeEntry,
    cache: Option<&Path>,
) -> Result<Vec<u8>, Error> {
    let cache = blob_cache_path(
        cache.filter(|_| is_dependency_input_path(&entry.path)),
        &entry.sha,
    );
    if let Some(bytes) = cache
        .as_deref()
        .and_then(|path| read_cached_blob(path).ok())
    {
        if verified_blob(entry, &bytes) {
            return Ok(bytes);
        }
    }
    let url = format!(
        "https://api.github.com/repos/{}/{}/git/blobs/{}",
        encode_path_segment(&repo.owner),
        encode_path_segment(&repo.repo),
        encode_path_segment(&entry.sha),
    );
    let bytes = client
        .get_raw_bytes_checked(&url, &entry.path, |size| {
            validate_blob_size(&entry.path, size)
        })
        .await?;
    if !verified_blob(entry, &bytes) {
        return Err(Error::App(format!(
            "GitHub blob does not match the commit at {}.",
            entry.path
        )));
    }
    if let Some(path) = cache {
        if let Err(cause) = write_cached_blob(&path, &bytes) {
            eprintln!("Failed to cache GitHub blob {}: {cause}", entry.sha);
        }
    }
    Ok(bytes)
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

fn validate_file_limits(entries: &BTreeMap<String, RawTreeEntry>) -> Result<(), Error> {
    let mut total = 0_u64;
    for (path, entry) in entries {
        let size = entry.size.ok_or_else(|| {
            Error::App(format!(
                "GitHub did not report the size of {path}. Scan a local checkout."
            ))
        })?;
        validate_blob_size(path, size)?;
        total = total.saturating_add(size);
    }
    if total > MAX_TOTAL_SIZE {
        return Err(snapshot_size_error());
    }
    Ok(())
}

fn validate_blob_size(path: &str, size: u64) -> Result<(), Error> {
    if size > MAX_FILE_SIZE {
        return Err(Error::App(format!(
            "GitHub file {path} exceeds the 100 MiB download limit. Scan a local checkout."
        )));
    }
    Ok(())
}

fn snapshot_size_error() -> Error {
    Error::App("The GitHub snapshot exceeds the download budget. Scan a local checkout to cover this repository.".into())
}

fn safe_join(root: &Path, relative_path: &str) -> Result<PathBuf, Error> {
    let relative = Path::new(relative_path);
    if relative_path.is_empty()
        || relative_path.contains('\\')
        || relative_path.chars().any(char::is_control)
        || relative.components().any(|component| !matches!(component, Component::Normal(name) if !name.eq_ignore_ascii_case(".git")))
    {
        return Err(Error::App("GitHub returned an unsafe file path.".into()));
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
    ensure_parent_directories(root, relative, directories)?;
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .map_err(materialized_path_error)?
        .write_all(bytes)?;
    Ok(())
}

fn ensure_parent_directories(
    root: &Path,
    relative: &str,
    directories: &mut HashSet<PathBuf>,
) -> Result<(), Error> {
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
    fn recognizes_dependency_metadata_for_source_verification() {
        for path in [
            "package-lock.json",
            "apps/web/package.json",
            "apps/web/bun.lock",
            "apps/native/A.csproj",
            "apps/native/packages.lock.json",
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
            assert!(is_dependency_input_path(path), "{path}");
        }

        for path in [
            "src/main.rs",
            "README.md",
            ".github/ISSUE_TEMPLATE/bug.yml",
            "../package.json",
            ".git/config",
        ] {
            assert!(!is_dependency_input_path(path), "{path}");
        }
    }

    #[test]
    fn archive_materializes_files_outside_the_manifest_list_and_checks_blob_hashes() {
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut metadata = tar::Header::new_gnu();
        metadata.set_entry_type(tar::EntryType::XGlobalHeader);
        metadata.set_size(0);
        archive
            .append_data(&mut metadata, "pax_global_header", io::empty())
            .unwrap();
        let mut entries = BTreeMap::new();
        for (path, archived, committed) in [
            (
                "src/generated.deps.json",
                b"resolved".as_slice(),
                b"resolved".as_slice(),
            ),
            (
                "src/module.example",
                b"arbitrary source".as_slice(),
                b"arbitrary source".as_slice(),
            ),
            (
                "src/substituted.txt",
                b"different".as_slice(),
                b"committed".as_slice(),
            ),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            header.set_size(archived.len() as u64);
            archive
                .append_data(&mut header, format!("fixture/{path}"), archived)
                .unwrap();
            let mut hash = Sha1::new();
            hash.update(format!("blob {}\0", committed.len()));
            hash.update(committed);
            entries.insert(
                path.to_owned(),
                RawTreeEntry {
                    path: path.to_owned(),
                    entry_type: "blob".into(),
                    mode: "100644".into(),
                    sha: super::super::hashing::lowercase_hex(&hash.finalize()),
                    size: Some(committed.len() as u64),
                },
            );
        }
        let bytes = archive.into_inner().unwrap().finish().unwrap();
        let mut snapshot = RemoteSnapshot::new().unwrap();
        let found = snapshot.read_archive(bytes.as_slice(), &entries).unwrap();
        assert_eq!(
            found,
            BTreeSet::from([
                "src/generated.deps.json".to_owned(),
                "src/module.example".to_owned()
            ])
        );
        assert!(!snapshot
            .directory
            .path()
            .join("src/substituted.txt")
            .exists());
        assert_eq!(
            fs::read(snapshot.directory.path().join("src/module.example")).unwrap(),
            b"arbitrary source"
        );
        assert!(!internal_link(
            Path::new("src/package.json"),
            Path::new("../../package.json")
        ));
        assert!(internal_link(
            Path::new("src/package.json"),
            Path::new("../package.json")
        ));
    }

    #[test]
    fn rejects_files_over_the_remote_download_budget() {
        assert!(validate_blob_size("package-lock.json", MAX_FILE_SIZE + 1).is_err());
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
