use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    command_failed, grype_output, grype_report, invalid_report, resolve_tool,
    syft_command_for_path, MAX_COMPANION_BYTES, SCAN_COMMAND_TIMEOUT,
};
use crate::core::hashing::lowercase_hex;
use crate::core::models::{ArtifactScanKind, ArtifactScanSummary, NormalizedFinding};
use crate::core::{process, timestamps};
use crate::Error;

const MAX_BUILD_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_BUILD_ENTRIES: usize = 100_000;

#[cfg(unix)]
use crate::core::secure_fs::AnchoredRoot as SourceRoot;
#[cfg(not(unix))]
type SourceRoot = PathBuf;

pub(crate) async fn scan_artifact(
    path: PathBuf,
    kind: ArtifactScanKind,
    cache: &Path,
) -> Result<(ArtifactScanSummary, Vec<NormalizedFinding>), Error> {
    let syft = resolve_tool("syft").ok_or_else(|| {
        Error::Scanner("syft is not installed or not reachable from the app".into())
    })?;
    let capture = crate::runtime::run_io(move || Snapshot::capture(&path, kind))
        .await
        .map_err(Error::Scanner)??;
    let kind = capture.kind;
    let mut command = syft_command_for_path(&syft);
    command.env("SYFT_CHECK_FOR_APP_UPDATE", "false");
    let mut warnings = capture.warnings;
    if kind == ArtifactScanKind::Sbom {
        command.arg("convert").arg(&capture.path);
    } else {
        let config = super::active_syft_configuration(&syft)
            .await
            .ok_or_else(|| {
                Error::Scanner("Could not load Syft's effective configuration.".into())
            })?;
        let (_, exclusions) = super::syft_scan_scope(&config)?;
        command
            .arg("scan")
            .arg(&capture.path)
            .env("SYFT_GOLANG_CAPTURE_SYMBOLS", "all");
        match kind {
            ArtifactScanKind::BuildFile => {
                command.args(["--from", "oci-archive,docker-archive,file"]);
            }
            ArtifactScanKind::BuildFolder if capture.path.join("oci-layout").is_file() => {
                command.args(["--from", "oci-dir"]);
            }
            ArtifactScanKind::BuildFolder => {
                // A build directory can contain installed packages and binaries
                // without the source lockfiles used by directory catalogers.
                command
                    .args([
                        "--from",
                        "dir",
                        "--override-default-catalogers=image",
                        "--base-path",
                    ])
                    .arg(&capture.path);
            }
            ArtifactScanKind::Sbom => unreachable!(),
        }
        command.args(["--scope", "squashed", "--enrich="]);
        if exclusions.is_empty() {
            command.arg("--exclude=");
        } else {
            warnings.push("The active Syft configuration excludes files from this scan.".into());
            for exclusion in exclusions {
                command.arg("--exclude").arg(exclusion);
            }
        }
    }
    command.args(["-o", "syft-json"]);
    let output = process::output(
        command,
        None,
        SCAN_COMMAND_TIMEOUT,
        MAX_COMPANION_BYTES as usize,
    )
    .await?;
    if !output.status.success() {
        return Err(command_failed("syft artifact scan", output.status));
    }

    let grype = grype_output(&output.stdout, Some(cache)).await?;
    crate::runtime::run(move || {
        let inventory = grype_report::SourceInventory::parse(&output.stdout)?;
        let mut packages = Vec::new();
        inventory.include_packages(&mut packages);
        let source: SyftSource = serde_json::from_slice(&output.stdout)
            .map_err(|error| invalid_report("Syft artifact metadata", &error))?;
        let platform = source.platform();
        if packages.is_empty() {
            warnings.push("Syft did not identify any packages in this input. Check the build output or provide its SBOM.".into());
        }
        let incomplete_identities = packages.iter().filter(|package| package.version.is_none() || package.package_type.is_none()).count();
        if incomplete_identities > 0 {
            warnings.push(format!("{incomplete_identities} packages lack a version or ecosystem for matching."));
        }
        if kind == ArtifactScanKind::BuildFile && source.is_generic_archive(&capture.name) {
            warnings.push("Syft treated this as a file archive. Scan the extracted build folder for full directory cataloging.".into());
        }
        let mut findings = grype_report::parse(&grype, Some(grype_report::Verification::Artifact(&inventory)))?;
        findings.sort_by(|left, right| right.severity.cmp(&left.severity).then_with(|| left.advisory_id.cmp(&right.advisory_id)).then_with(|| left.package_name.cmp(&right.package_name)));
        let summary = ArtifactScanSummary {
            kind,
            name: capture.name,
            digest: capture.digest,
            scanned_at: timestamps::now(),
            platform,
            finding_count: findings.len(),
            package_count: packages.len(),
            coverage_warnings: warnings,
        };
        Ok((summary, findings))
    }).await.map_err(Error::Scanner)?
}

#[derive(Deserialize)]
struct SyftSource {
    source: Option<SyftTarget>,
}

#[derive(Deserialize)]
struct SyftTarget {
    #[serde(rename = "type")]
    kind: String,
    metadata: serde_json::Value,
}

impl SyftSource {
    fn platform(&self) -> Option<String> {
        let metadata = &self.source.as_ref()?.metadata;
        let os = metadata.get("os")?.as_str()?;
        let architecture = metadata.get("architecture")?.as_str()?;
        Some(format!("{os}/{architecture}"))
    }

    fn is_generic_archive(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.source
            .as_ref()
            .is_some_and(|source| source.kind == "file")
            && [".zip", ".tar", ".tgz", ".tar.gz", ".tar.xz"]
                .iter()
                .any(|extension| name.ends_with(extension))
    }
}

struct Snapshot {
    _temp: tempfile::TempDir,
    path: PathBuf,
    name: String,
    kind: ArtifactScanKind,
    digest: String,
    warnings: Vec<String>,
}

impl Snapshot {
    fn capture(path: &Path, kind: ArtifactScanKind) -> Result<Self, Error> {
        let source = path.canonicalize()?;
        // macOS file pickers can return application bundles as single items.
        let kind = if kind == ArtifactScanKind::BuildFile && source.is_dir() {
            ArtifactScanKind::BuildFolder
        } else {
            kind
        };
        let name = path
            .file_name()
            .ok_or_else(|| Error::Scanner("Choose a file or a build folder.".into()))?;
        let temp = tempfile::Builder::new()
            .prefix("cvec-artifact-")
            .tempdir()?;
        if kind == ArtifactScanKind::BuildFolder && temp.path().canonicalize()?.starts_with(&source)
        {
            return Err(Error::Scanner(
                "Choose a build folder that does not contain the system temporary directory."
                    .into(),
            ));
        }
        let path = temp.path().join(name);
        let mut warnings = Vec::new();
        let digest = match kind {
            ArtifactScanKind::BuildFolder => {
                fs::create_dir(&path)?;
                let mut tree = TreeCopy::new(&source, &path)?;
                tree.copy_directory(Path::new(""), 0)?;
                if tree.skipped > 0 {
                    warnings.push(format!(
                        "{} broken or out-of-scope links or special files were skipped.",
                        tree.skipped
                    ));
                }
                tree.hash.finalize().into()
            }
            ArtifactScanKind::BuildFile | ArtifactScanKind::Sbom => {
                let parent = source
                    .parent()
                    .ok_or_else(|| io::Error::other("Artifact file has no parent"))?;
                let name = source
                    .file_name()
                    .ok_or_else(|| io::Error::other("Artifact file has no name"))?;
                let limit = if kind == ArtifactScanKind::Sbom {
                    MAX_COMPANION_BYTES
                } else {
                    MAX_BUILD_BYTES
                };
                let root = open_source_root(parent)?;
                copy_file(open_file(&root, Path::new(name))?, &path, limit)?.0
            }
        };
        Ok(Self {
            _temp: temp,
            path,
            name: name.to_string_lossy().into_owned(),
            kind,
            digest: lowercase_hex(&digest),
            warnings,
        })
    }
}

struct TreeCopy<'a> {
    source: &'a Path,
    root: SourceRoot,
    destination: &'a Path,
    hash: Sha256,
    remaining_bytes: u64,
    entries: usize,
    skipped: usize,
}

impl<'a> TreeCopy<'a> {
    fn new(source: &'a Path, destination: &'a Path) -> io::Result<Self> {
        let mut hash = Sha256::new();
        hash.update(b"cvec artifact tree v1\0");
        Ok(Self {
            source,
            root: open_source_root(source)?,
            destination,
            hash,
            remaining_bytes: MAX_BUILD_BYTES,
            entries: 0,
            skipped: 0,
        })
    }

    fn copy_directory(&mut self, relative: &Path, depth: usize) -> io::Result<()> {
        if depth > 128 {
            return Err(io::Error::other(
                "Artifact exceeds the supported directory depth",
            ));
        }
        let mut names = directory_entries(&self.root, relative)?;
        names.sort_unstable();
        for name in names.iter().filter(|name| *name != "." && *name != "..") {
            self.entries += 1;
            if self.entries > MAX_BUILD_ENTRIES {
                return Err(io::Error::other("Artifact exceeds the 100,000-entry limit"));
            }
            let entry = relative.join(name);
            let source = self.source.join(&entry);
            let dest = self.destination.join(&entry);
            let metadata = fs::symlink_metadata(&source)?;
            if metadata.is_dir() {
                fs::create_dir(&dest)?;
                self.record_entry(b'd', &entry);
                self.copy_directory(&entry, depth + 1)?;
            } else if metadata.is_file() {
                let (digest, size, mode) =
                    copy_file(open_file(&self.root, &entry)?, &dest, self.remaining_bytes)?;
                self.remaining_bytes -= size;
                self.record_entry(b'f', &entry);
                self.hash.update(mode.to_le_bytes());
                self.hash.update(digest);
            } else if metadata.is_symlink() && copy_link(self.source, &entry, &dest)? {
                self.record_entry(b'l', &entry);
                self.record_entry(b't', &fs::read_link(&dest)?);
            } else {
                self.skipped += 1;
            }
        }
        Ok(())
    }

    fn record_entry(&mut self, kind: u8, path: &Path) {
        let bytes = path.as_os_str().as_encoded_bytes();
        self.hash.update([kind]);
        self.hash.update((bytes.len() as u64).to_le_bytes());
        self.hash.update(bytes);
    }
}

fn copy_file(mut file: File, dest: &Path, limit: u64) -> io::Result<([u8; 32], u64, u32)> {
    let before = file.metadata()?;
    if !before.is_file() || before.len() > limit {
        return Err(io::Error::other(
            "Artifact exceeds the supported input size or is not a regular file",
        ));
    }
    let mut copy = File::create_new(dest)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut written = 0;
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        written += size as u64;
        if written > limit {
            return Err(io::Error::other(
                "Artifact exceeds the supported input size",
            ));
        }
        hash.update(&buffer[..size]);
        copy.write_all(&buffer[..size])?;
    }
    let after = file.metadata()?;
    if after.len() != written
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(io::Error::other("Artifact changed while it was being read"));
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        let mode = before.permissions().mode() & 0o111;
        copy.set_permissions(fs::Permissions::from_mode(0o600 | mode))?;
        mode
    };
    #[cfg(not(unix))]
    let mode = 0;
    Ok((hash.finalize().into(), written, mode))
}

#[cfg(unix)]
fn open_source_root(path: &Path) -> io::Result<SourceRoot> {
    SourceRoot::open(path)
}

#[cfg(not(unix))]
fn open_source_root(path: &Path) -> io::Result<SourceRoot> {
    Ok(path.to_path_buf())
}

#[cfg(unix)]
fn open_file(root: &SourceRoot, relative: &Path) -> io::Result<File> {
    root.entry(relative, false)?.open_regular()
}

#[cfg(not(unix))]
fn open_file(root: &SourceRoot, relative: &Path) -> io::Result<File> {
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(root) {
        return Err(io::Error::other("Artifact entry points outside the folder"));
    }
    File::open(path)
}

#[cfg(unix)]
fn directory_entries(root: &SourceRoot, relative: &Path) -> io::Result<Vec<OsString>> {
    if relative.as_os_str().is_empty() {
        root.entry_names()?.collect()
    } else {
        root.directory(relative, false)?.entry_names()?.collect()
    }
}

#[cfg(not(unix))]
fn directory_entries(root: &SourceRoot, relative: &Path) -> io::Result<Vec<OsString>> {
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(root) {
        return Err(io::Error::other("Artifact entry points outside the folder"));
    }
    fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect()
}

#[cfg(unix)]
fn copy_link(root: &Path, entry: &Path, dest: &Path) -> io::Result<bool> {
    let Ok(target) = root.join(entry).canonicalize() else {
        return Ok(false);
    };
    let Ok(target) = target.strip_prefix(root) else {
        return Ok(false);
    };
    let parents = entry.components().count().saturating_sub(1);
    let mut rebased = PathBuf::from(".");
    rebased.extend(std::iter::repeat_n("..", parents));
    rebased.push(target);
    std::os::unix::fs::symlink(rebased, dest)?;
    Ok(true)
}

#[cfg(not(unix))]
fn copy_link(_: &Path, _: &Path, _: &Path) -> io::Result<bool> {
    Ok(false)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn snapshot_reads_from_the_opened_directory_after_a_rename() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("selected-build");
        let destination = temp.path().join("snapshot");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("build.txt"), "chosen build").unwrap();
        let mut tree = TreeCopy::new(&source, &destination).unwrap();

        fs::rename(&source, temp.path().join("previous-build")).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("build.txt"), "new build").unwrap();

        tree.copy_directory(Path::new(""), 0).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("build.txt")).unwrap(),
            "chosen build"
        );
    }

    #[test]
    fn captures_app_bundle_files_and_keeps_links_within_the_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("Test.app");
        let contents = bundle.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("binary"), b"first build").unwrap();
        fs::write(temp.path().join("outside"), b"outside the bundle").unwrap();
        symlink("Contents/binary", bundle.join("internal")).unwrap();
        symlink("../outside", bundle.join("external")).unwrap();

        let capture = Snapshot::capture(&bundle, ArtifactScanKind::BuildFile).unwrap();
        assert_eq!(capture.kind, ArtifactScanKind::BuildFolder);
        assert_eq!(
            fs::read(capture.path.join("internal")).unwrap(),
            b"first build"
        );
        assert!(!capture.path.join("external").exists());
        assert_eq!(capture.warnings.len(), 1);

        fs::write(contents.join("binary"), b"second build").unwrap();
        let next = Snapshot::capture(&bundle, ArtifactScanKind::BuildFolder).unwrap();
        assert_ne!(capture.digest, next.digest);
        assert_eq!(
            fs::read(capture.path.join("internal")).unwrap(),
            b"first build"
        );
    }
}
