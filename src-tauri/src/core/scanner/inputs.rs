use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::core::models::RepoPackage;
use crate::core::secure_fs::read_regular_text;
use crate::Error;

pub(super) const MAX_LOCKFILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScanInputs {
    root: PathBuf,
    digests: BTreeMap<PathBuf, [u8; 32]>,
}

#[derive(Deserialize)]
struct SyftFileList {
    files: Vec<SyftFile>,
}

#[derive(Deserialize)]
struct SyftFile {
    location: SyftFileLocation,
    metadata: SyftFileMetadata,
}

#[derive(Deserialize)]
struct SyftFileLocation {
    path: String,
}

#[derive(Deserialize)]
struct SyftFileMetadata {
    #[serde(rename = "type")]
    file_type: String,
}

impl ScanInputs {
    pub(super) fn capture(root: &Path, report: &[u8]) -> Result<Self, Error> {
        let report: SyftFileList = serde_json::from_slice(report).map_err(|_| {
            incomplete("Syft did not return a valid file list for this repository.")
        })?;
        let root = root.canonicalize()?;
        let mut digests = BTreeMap::new();
        for file in report.files {
            if !matches!(
                file.metadata.file_type.as_str(),
                "RegularFile" | "SymbolicLink"
            ) {
                continue;
            }
            let path = file
                .location
                .path
                .strip_prefix('/')
                .unwrap_or(&file.location.path);
            if !is_scan_input(path) {
                continue;
            }
            let path = Path::new(path);
            let digest = fingerprint(&root, path).map_err(|_| unreadable(path))?;
            digests.insert(path.to_path_buf(), digest);
        }
        Ok(Self { root, digests })
    }

    pub(super) fn paths(&self) -> impl Iterator<Item = &Path> {
        self.digests.keys().map(PathBuf::as_path)
    }

    pub(super) fn includes(&self, path: &Path) -> bool {
        self.digests.contains_key(path)
    }

    pub(super) fn read_text(&self, path: &Path, limit: u64) -> Result<String, Error> {
        let contents = open_input(&self.root, path)
            .and_then(|file| read_regular_text(file, limit))
            .map_err(|_| unreadable(path))?;
        Ok(contents)
    }
}

pub(super) fn is_project_input(path: &Path) -> bool {
    !path.components().any(|part| matches!(part, Component::Normal(value) if value == "node_modules" || value == ".yarn" || value == ".git"))
}

fn is_scan_input(path: &str) -> bool {
    // Source manifests and lockfiles also drive the saved project metadata.
    // Installed metadata can supply the resolved version of a dependency.
    crate::core::github::is_relevant_remote_path(path)
        || matches!(
            path.rsplit('/').next(),
            Some("bun.lock" | "bun.lockb" | "METADATA" | "PKG-INFO" | ".npmrc" | ".yarnrc.yml")
        )
}

fn fingerprint(root: &Path, relative: &Path) -> io::Result<[u8; 32]> {
    let mut file = open_input(root, relative)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(hash.finalize().into())
}

fn open_input(root: &Path, relative: &Path) -> io::Result<File> {
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid scan input path",
        ));
    }
    let target = root.join(relative).canonicalize()?;
    let anchored = target.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Scan input points outside the source directory",
        )
    })?;
    #[cfg(unix)]
    let file = crate::core::secure_fs::AnchoredRoot::open(root)?
        .entry(anchored, false)?
        .open()?;
    #[cfg(not(unix))]
    let file = File::open(root.join(anchored))?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Expected a regular scan input",
        ));
    }
    Ok(file)
}

pub(super) fn incomplete(reason: &str) -> Error {
    Error::Scanner(format!("Incomplete scan evidence: {reason}"))
}

fn unreadable(path: &Path) -> Error {
    incomplete(&format!(
        "Could not read {} inside the scan source. Check the file and retry.",
        path.display()
    ))
}

#[derive(Deserialize)]
struct CargoLock {
    #[serde(default, rename = "package")]
    packages: Vec<CargoPackage>,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
    source: Option<String>,
}

fn cargo_locks<'a>(
    inputs: &'a ScanInputs,
    inventory: &BTreeSet<(&str, &str)>,
) -> Result<BTreeMap<&'a Path, CargoLock>, Error> {
    let mut locks = BTreeMap::new();
    for path in inputs.paths().filter(|path| {
        is_project_input(path) && path.file_name().is_some_and(|name| name == "Cargo.lock")
    }) {
        let text = inputs.read_text(path, MAX_LOCKFILE_BYTES)?;
        let lock: CargoLock = toml::from_str(&text).map_err(|_| unresolved(path))?;
        if lock
            .packages
            .iter()
            .filter(|package| package.source.is_some())
            .any(|package| !inventory.contains(&(package.name.as_str(), package.version.as_str())))
        {
            return Err(unresolved(path));
        }
        locks.insert(path.parent().unwrap_or_else(|| Path::new("")), lock);
    }
    Ok(locks)
}

fn verify_cargo_manifest(
    inputs: &ScanInputs,
    path: &Path,
    locks: &BTreeMap<&Path, CargoLock>,
) -> Result<(), Error> {
    let text = inputs.read_text(path, super::MAX_SUPPORTED_MANIFEST_BYTES)?;
    let manifest: toml::Value = toml::from_str(&text).map_err(|_| unresolved(path))?;
    let directory = path.parent().unwrap_or_else(|| Path::new(""));
    let lock = directory.ancestors().find_map(|dir| locks.get(dir));
    let dependencies = manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten();
    for (name, specification) in dependencies {
        if specification.get("path").is_some() {
            continue;
        }
        let package = specification
            .get("package")
            .and_then(toml::Value::as_str)
            .unwrap_or(name);
        let inherited = specification
            .get("workspace")
            .and_then(toml::Value::as_bool)
            == Some(true);
        if !lock.is_some_and(|lock| {
            inherited
                || lock
                    .packages
                    .iter()
                    .any(|resolved| resolved.name == package)
        }) {
            return Err(unresolved(path));
        }
    }
    Ok(())
}

pub(super) fn verify_cargo_and_python(
    inputs: &ScanInputs,
    packages: &[RepoPackage],
    python: &PythonCoverage,
) -> Result<(), Error> {
    let cargo = packages
        .iter()
        .filter(|package| package.package_type.as_deref() == Some("cargo"))
        .filter_map(|package| Some((package.name.as_str(), package.version.as_deref()?)))
        .collect();

    let locks = cargo_locks(inputs, &cargo)?;
    for path in inputs.paths().filter(|path| is_project_input(path)) {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        match name {
            "Cargo.toml" => {
                verify_cargo_manifest(inputs, path, &locks)?;
            }
            "uv.lock" | "poetry.lock" | "Pipfile.lock" => {
                let text = inputs.read_text(path, MAX_LOCKFILE_BYTES)?;
                let valid = if name == "Pipfile.lock" {
                    serde_json::from_str::<serde_json::Value>(&text)
                        .is_ok_and(|value| value.is_object())
                } else {
                    toml::from_str::<toml::Table>(&text).is_ok()
                };
                if !valid {
                    return Err(unresolved(path));
                }
            }
            "pyproject.toml" => {
                let text = inputs.read_text(path, super::MAX_SUPPORTED_MANIFEST_BYTES)?;
                let manifest: toml::Value = toml::from_str(&text).map_err(|_| unresolved(path))?;
                let requirements = manifest
                    .get("project")
                    .and_then(|project| project.get("dependencies"))
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(toml::Value::as_str);
                verify_python_requirements(path, requirements, python)?;
            }
            name if is_requirements_file(name) => {
                let text = inputs.read_text(path, MAX_LOCKFILE_BYTES)?;
                verify_python_requirements(path, text.lines(), python)?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Default)]
pub(super) struct PythonCoverage {
    packages_by_root: BTreeMap<PathBuf, BTreeSet<String>>,
}

impl PythonCoverage {
    pub(super) fn record(&mut self, name: &str, component: &Value) {
        let Some(name) = super::normalize_python_package_name(name) else {
            return;
        };
        for root in super::syft_locations(component)
            .filter_map(|location| location.get("value")?.as_str())
            .filter_map(python_resolution_root)
        {
            self.packages_by_root
                .entry(root.to_path_buf())
                .or_default()
                .insert(name.clone());
        }
    }

    fn covers(&self, path: &Path, name: &str) -> bool {
        path.parent().is_some_and(|directory| {
            directory.ancestors().any(|root| {
                self.packages_by_root
                    .get(root)
                    .is_some_and(|packages| packages.contains(name))
            })
        })
    }
}

fn is_requirements_file(name: &str) -> bool {
    name.starts_with("requirements") && name.ends_with(".txt")
}

fn python_resolution_root(location: &str) -> Option<&Path> {
    let path = Path::new(location.trim_start_matches('/'));
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    if is_requirements_file(name)
        || matches!(
            name,
            "uv.lock" | "poetry.lock" | "Pipfile.lock" | "pyproject.toml"
        )
    {
        return path.parent();
    }
    if !matches!(name, "METADATA" | "PKG-INFO") {
        return None;
    }
    // Use the project containing the virtual environment, or the directory
    // where packages were installed directly (for example pip install -t).
    let installation = path.ancestors().nth(2)?;
    let environment = installation.ancestors().find(|directory| {
        directory
            .file_name()
            .is_some_and(|name| name == ".venv" || name == "venv")
    });
    environment.and_then(Path::parent).or(Some(installation))
}

fn verify_python_requirements<'a>(
    path: &Path,
    requirements: impl Iterator<Item = &'a str>,
    packages: &PythonCoverage,
) -> Result<(), Error> {
    let missing = requirements
        // Platform-qualified dependencies may legitimately be absent.
        .filter(|requirement| !requirement.contains(';'))
        .filter_map(super::normalize_python_requirement)
        .any(|name| !packages.covers(path, &name));
    if missing {
        return Err(unresolved(path));
    }
    Ok(())
}

fn unresolved(path: &Path) -> Error {
    incomplete(&format!("Could not verify the dependency inventory from {}. Check the manifest and restore its resolution before retrying.", path.display()))
}
