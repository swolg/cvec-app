use super::secure_fs::{AnchoredEntry, AnchoredRoot};
use super::*;
use caseless::Caseless;
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Component, time::Duration};
use unicode_normalization::UnicodeNormalization;

pub(super) const MAX_FILE: usize = 1024 * 1024;
const MAX_FILES: usize = 20_000;
const MAX_SOURCE: u64 = 256 * 1024 * 1024;
const MAX_DIFF: usize = 256 * 1024;

pub(super) fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let output = super::super::git::run_sync(root, args, Duration::from_secs(30), 4 * 1024 * 1024)?;
    if !output.status.success() {
        return Err(Error::App("Git could not inspect or create the fix worktree. Check the local checkout and Git installation.".into()));
    }
    Ok(output.stdout)
}

pub(super) fn git_text(root: &Path, args: &[&str]) -> Result<String, Error> {
    Ok(String::from_utf8_lossy(&git(root, args)?)
        .trim()
        .to_string())
}

fn path_key(path: &Path) -> String {
    // Reserve canonical/case-equivalent names even before they exist. Actual
    // directory identity remains enforced by the descriptor-relative entry.
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
        .chars()
        .nfd()
        .default_case_fold()
        .nfd()
        .collect()
}

fn source_path(relative: &str) -> Result<(&Path, String), Error> {
    let path = Path::new(relative);
    if relative.len() > 1024
        || relative.contains(['\\', '\0'])
        || path.as_os_str().is_empty()
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(Error::App(
            "This path is outside the fix workspace source policy.".into(),
        ));
    }
    let key = path_key(path);
    let policy_path = Path::new(&key);
    if super::super::companion_context::is_sensitive_path(policy_path)
        || policy_path.components().any(|part| {
            matches!(
                part.as_os_str()
                    .to_str()
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some(
                    ".agents"
                        | ".npmrc"
                        | ".netrc"
                        | ".cargo"
                        | ".gitmodules"
                        | ".gitattributes"
                        | ".aws"
                        | ".azure"
                        | ".config"
                        | ".gnupg"
                        | ".docker"
                )
            )
        })
    {
        return Err(Error::App(
            "This path is outside the fix workspace source policy.".into(),
        ));
    }
    Ok((path, key))
}

/// An operation-scoped inventory, never a long-lived cache of the Git index.
/// The entry used for I/O also enforces filesystem-aware submodule exclusion.
pub(super) struct SourcePolicy {
    root: PathBuf,
    anchor: AnchoredRoot,
    gitlinks: Vec<PathBuf>,
    gitlink_keys: std::collections::HashSet<String>,
}

impl SourcePolicy {
    pub(super) fn new(root: &Path) -> Result<Self, Error> {
        // Open before canonicalizing: canonicalize would otherwise follow a
        // replaced/symlink root and bypass the anchor's NOFOLLOW boundary.
        let anchor = AnchoredRoot::open(root)?;
        let root = root.canonicalize()?;
        let mut gitlinks = gitlinks(&root)?
            .into_iter()
            .map(|(path, _)| PathBuf::from(path))
            .collect::<Vec<_>>();
        gitlinks.sort();
        let gitlink_keys = gitlinks.iter().map(|path| path_key(path)).collect();
        Ok(Self {
            root,
            anchor,
            gitlinks,
            gitlink_keys,
        })
    }

    fn entry(&self, relative: &str, create: bool) -> Result<AnchoredEntry, Error> {
        let (path, key) = source_path(relative)?;
        let mut ancestor = key.as_str();
        loop {
            if self.gitlink_keys.contains(ancestor) {
                return Err(Error::App(
                    "Submodule paths are outside the isolated workspace tool boundary.".into(),
                ));
            }
            match ancestor.rsplit_once('/') {
                Some((parent, _)) => ancestor = parent,
                None => break,
            }
        }
        Ok(self.anchor.entry_excluding(path, create, &self.gitlinks)?)
    }

    pub(super) fn safe_path(&self, relative: &str, create: bool) -> Result<PathBuf, Error> {
        match self.entry(relative, false) {
            Ok(entry) => match entry.symlink_target() {
                Ok(Some(_)) => {
                    return Err(Error::App(
                        "Symlinks are not available to fix workspace tools.".into(),
                    ))
                }
                Ok(None) => {}
                Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            },
            Err(Error::Io(error)) if create && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(self.root.join(relative))
    }

    pub(super) fn read(&self, relative: &str) -> Result<String, Error> {
        read_open_file(self.entry(relative, false)?.open()?)
    }
}

fn read_open_file(file: fs::File) -> Result<String, Error> {
    let mut bytes = Vec::new();
    file.take(MAX_FILE as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE || bytes.contains(&0) {
        return Err(Error::App(
            "The file is binary or exceeds the 1 MiB source limit.".into(),
        ));
    }
    let content =
        String::from_utf8(bytes).map_err(|_| Error::App("The file is not UTF-8 source.".into()))?;
    if super::super::companion_context::contains_secret(&content) {
        return Err(Error::App(
            "The file matched the secret-exclusion policy.".into(),
        ));
    }
    Ok(content)
}

pub(super) fn read(root: &Path, relative: &str) -> Result<String, Error> {
    SourcePolicy::new(root)?.read(relative)
}

pub(super) fn write(root: &Path, relative: &str, content: &str) -> Result<(), Error> {
    if content.len() > MAX_FILE
        || content.contains('\0')
        || super::super::companion_context::contains_secret(content)
    {
        return Err(Error::App(
            "The proposed content exceeds the source limit or matches the secret-exclusion policy."
                .into(),
        ));
    }
    let policy = SourcePolicy::new(root)?;
    let entry = policy.entry(relative, true)?;
    let permissions = match entry.open() {
        Ok(file) => {
            let permissions = file.metadata()?.permissions();
            read_open_file(file)?;
            Some(permissions)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    entry.replace(content.as_bytes(), permissions)?;
    Ok(())
}

pub(super) fn delete(root: &Path, relative: &str) -> Result<(), Error> {
    let policy = SourcePolicy::new(root)?;
    let entry = policy.entry(relative, false)?;
    read_open_file(entry.open()?)?;
    entry.remove()?;
    Ok(())
}

pub(super) fn source_files(root: &Path) -> Result<Vec<String>, Error> {
    let bytes = git(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let mut paths: Vec<String> = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| {
            String::from_utf8(part.to_vec())
                .map_err(|_| Error::App("A source path is not UTF-8.".into()))
        })
        .collect::<Result<_, _>>()?;
    paths.sort();
    paths.dedup();
    if paths.len() > MAX_FILES {
        return Err(Error::App(
            "The workspace exceeds the 20,000-file inspection limit.".into(),
        ));
    }
    Ok(paths)
}

fn gitlinks(root: &Path) -> Result<Vec<(String, String)>, Error> {
    let bytes = git(root, &["ls-files", "--stage", "-z"])?;
    let mut links = Vec::new();
    for (index, record) in bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .enumerate()
    {
        if index >= MAX_FILES {
            return Err(Error::App(
                "The workspace exceeds the 20,000-file inspection limit.".into(),
            ));
        }
        let record = std::str::from_utf8(record)
            .map_err(|_| Error::App("A source path is not UTF-8.".into()))?;
        let (header, path) = record
            .split_once('\t')
            .ok_or_else(|| Error::App("Invalid Git source inventory.".into()))?;
        let fields = header.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 || fields[2] != "0" {
            return Err(Error::App(
                "Resolve the worktree's Git index conflicts before continuing.".into(),
            ));
        }
        if fields[0] == "160000" {
            links.push((path.to_owned(), fields[1].to_owned()));
        }
    }
    Ok(links)
}

pub(super) fn verification_warning(root: &Path) -> Result<Option<String>, Error> {
    Ok((!gitlinks(root)?.is_empty()).then(|| "Submodules are excluded from workspace tools. Checks and rescans are unavailable because their contents cannot yet be verified within the isolated worktree. Saved evidence remains readable and is marked stale.".into()))
}

pub(super) fn require_verifiable_source(root: &Path) -> Result<(), Error> {
    if let Some(warning) = verification_warning(root)? {
        return Err(Error::App(warning));
    }
    Ok(())
}

pub(super) fn digest(root: &Path) -> Result<String, Error> {
    let anchor = AnchoredRoot::open(root)?;
    let mut hash = Sha256::new();
    // Mode changes can change whether repository scripts run. Version the digest
    // so evidence recorded before modes were covered becomes stale on refresh.
    hash.update(b"cvec-worktree-source-v3\0");
    let links = gitlinks(root)?;
    let mut total = 0u64;
    for name in source_files(root)? {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        if let Some((_, object)) = links.iter().find(|(path, _)| path == &name) {
            hash.update(b"gitlink\0");
            hash.update(object.as_bytes());
            continue;
        }
        let entry = match anchor.entry(Path::new(&name), false) {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hash.update(b"deleted");
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        match entry.symlink_target() {
            Ok(Some(target)) => {
                hash.update(b"symlink");
                hash.update(target);
                continue;
            }
            Ok(None) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hash.update(b"deleted");
                continue;
            }
            Err(error) => return Err(error.into()),
        }
        if entry.is_directory()? {
            // Git still lists the deleted tracked file beside its new children.
            // A directory is a deletion marker, never source opened as a file.
            hash.update(b"directory-replaces-file\0");
            continue;
        }
        let file = entry.open()?;
        let before = file.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            hash.update((before.permissions().mode() & 0o777).to_le_bytes());
        }
        total = total.saturating_add(before.len());
        if total > MAX_SOURCE {
            return Err(Error::App(
                "The workspace exceeds the 256 MiB source inspection limit.".into(),
            ));
        }
        hash.update(before.len().to_le_bytes());
        let mut reader = file.take(before.len() + 1);
        let mut read = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read += count as u64;
            if read > before.len() {
                return Err(Error::App(
                    "Workspace source changed during inspection. Refresh and retry.".into(),
                ));
            }
            hash.update(&buffer[..count]);
        }
        let after = reader.into_inner().metadata()?;
        #[cfg(unix)]
        let mode_changed = {
            use std::os::unix::fs::PermissionsExt;
            before.permissions().mode() != after.permissions().mode()
        };
        #[cfg(not(unix))]
        let mode_changed = before.permissions() != after.permissions();
        if read != before.len()
            || before.len() != after.len()
            || before.modified()? != after.modified()?
            || mode_changed
        {
            return Err(Error::App(
                "Workspace source changed during inspection. Refresh and retry.".into(),
            ));
        }
    }
    Ok(super::super::hashing::lowercase_hex(&hash.finalize()))
}

pub(super) fn diff(root: &Path, base: &str) -> Result<FixDiff, Error> {
    let policy = SourcePolicy::new(root)?;
    let tracked = git(root, &["diff", "--name-only", "-z", base, "--"])?;
    let untracked = git(root, &["ls-files", "-z", "--others", "--exclude-standard"])?;
    let mut names = tracked
        .split(|byte| *byte == 0)
        .chain(untracked.split(|byte| *byte == 0))
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    let mut result = FixDiff {
        files: Vec::new(),
        omitted_files: 0,
        truncated: false,
        digest: digest(root)?,
    };
    let mut size = 0usize;
    for name in names {
        if result.files.len() >= 100 || policy.safe_path(&name, true).is_err() {
            result.omitted_files += 1;
            continue;
        }
        let current = if root.join(&name).exists() && !policy.entry(&name, false)?.is_directory()? {
            match policy.read(&name) {
                Ok(value) => Some(value),
                Err(_) => {
                    result.omitted_files += 1;
                    continue;
                }
            }
        } else {
            None
        };
        let old_output = git(root, &["cat-file", "-e", &format!("{base}:{name}")]);
        let added = old_output.is_err();
        let patch = if added {
            format!(
                "--- /dev/null\n+++ b/{name}\n{}",
                current
                    .unwrap_or_default()
                    .lines()
                    .map(|line| format!("+{line}\n"))
                    .collect::<String>()
            )
        } else {
            let bytes = git(
                root,
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--no-renames",
                    "--unified=3",
                    base,
                    "--",
                    &name,
                ],
            )?;
            String::from_utf8_lossy(&bytes).into_owned()
        };
        if super::super::companion_context::contains_secret(&patch) {
            result.omitted_files += 1;
            continue;
        }
        let available = MAX_DIFF.saturating_sub(size);
        if available == 0 {
            result.truncated = true;
            result.omitted_files += 1;
            continue;
        }
        let truncated = patch.len() > available;
        let text = bounded(&patch, available);
        size += text.len();
        result.truncated |= truncated;
        result.files.push(FixDiffFile {
            path: name,
            patch: text,
            truncated,
        });
    }
    Ok(result)
}
