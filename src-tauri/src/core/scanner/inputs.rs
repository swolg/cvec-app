use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::core::secure_fs::read_regular_text;
use crate::Error;

pub(super) const MAX_LOCKFILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScanInputs {
    root: PathBuf,
    files: BTreeMap<PathBuf, InputFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputFingerprint {
    digest: [u8; 32],
    modified: Option<SystemTime>,
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
        let mut files = BTreeMap::new();
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
            let fingerprint = fingerprint(&root, path).map_err(|_| unreadable(path))?;
            files.insert(path.to_path_buf(), fingerprint);
        }
        Ok(Self { root, files })
    }

    pub(super) fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.keys().map(PathBuf::as_path)
    }

    pub(super) fn includes(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn modified(&self, path: &Path) -> Option<SystemTime> {
        self.files.get(path)?.modified
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
    crate::core::github::is_dependency_input_path(path)
        || matches!(
            path.rsplit('/').next(),
            Some("bun.lock" | "bun.lockb" | "METADATA" | "PKG-INFO" | ".npmrc" | ".yarnrc.yml")
        )
}

fn fingerprint(root: &Path, relative: &Path) -> io::Result<InputFingerprint> {
    let mut file = open_input(root, relative)?;
    let modified = file.metadata()?.modified().ok();
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    if file.metadata()?.modified().ok() != modified {
        return Err(io::Error::other(
            "Scan input changed while it was being read",
        ));
    }
    Ok(InputFingerprint {
        digest: hash.finalize().into(),
        modified,
    })
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
    #[serde(default)]
    dependencies: Vec<String>,
}

#[derive(Default)]
pub(super) struct CatalogedVersions {
    by_file: BTreeMap<PathBuf, BTreeMap<String, BTreeSet<String>>>,
}

impl CatalogedVersions {
    pub(super) fn record(&mut self, name: &str, version: &str, component: &Value) {
        for path in super::syft_locations(component)
            .filter_map(|location| location.get("value")?.as_str())
            .map(|path| Path::new(path.trim_start_matches('/')))
            .filter(|path| {
                path.components()
                    .all(|part| matches!(part, Component::Normal(_)))
            })
        {
            self.by_file
                .entry(path.to_path_buf())
                .or_default()
                .entry(name.to_owned())
                .or_default()
                .insert(version.to_owned());
        }
    }

    fn covers(&self, path: &Path, name: &str, version: &str) -> bool {
        self.by_file
            .get(path)
            .and_then(|packages| packages.get(name))
            .is_some_and(|versions| versions.contains(version))
    }
}

fn cargo_locks<'a>(
    inputs: &'a ScanInputs,
    inventory: &CatalogedVersions,
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
            .any(|package| !inventory.covers(path, &package.name, &package.version))
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
    manifests: &BTreeMap<&Path, toml::Value>,
    warnings: &mut Vec<String>,
) -> Result<(), Error> {
    let directory = path.parent().unwrap_or_else(|| Path::new(""));
    let manifest = &manifests[directory];
    let package = manifest.get("package");
    let declared_workspace = package
        .and_then(|table| table.get("workspace"))
        .and_then(toml::Value::as_str);
    let workspace = match declared_workspace {
        Some(relative) => inputs
            .root()
            .join(directory)
            .join(relative)
            .canonicalize()
            .ok()
            .and_then(|workspace| {
                workspace
                    .strip_prefix(inputs.root())
                    .ok()
                    .filter(|path| manifests.contains_key(path))
                    .map(Path::to_path_buf)
            }),
        None => directory
            .ancestors()
            .find(|dir| {
                manifests
                    .get(dir)
                    .is_some_and(|manifest| manifest.get("workspace").is_some())
            })
            .map(Path::to_path_buf),
    };
    let lock = workspace
        .as_deref()
        .or(Some(directory))
        .and_then(|dir| locks.get(dir));
    let inherited = workspace
        .as_deref()
        .and_then(|dir| manifests.get(dir))
        .and_then(|manifest| manifest.get("workspace"))
        .and_then(|workspace| workspace.get("dependencies"));
    let owner = lock.zip(package).and_then(|(lock, package)| {
        let name = package.get("name")?.as_str()?;
        let version = package.get("version").and_then(toml::Value::as_str);
        let mut candidates = lock.packages.iter().filter(|entry| {
            entry.source.is_none()
                && entry.name == name
                && version.is_none_or(|version| entry.version == version)
        });
        let owner = candidates.next()?;
        candidates.next().is_none().then_some(owner)
    });
    let targets = manifest
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .map(|(target, table)| (Some(target.as_str()), table));
    for (target, table) in std::iter::once((None, manifest)).chain(targets) {
        for group in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let legacy_group = group.replace('-', "_");
            let declarations = table
                .get(group)
                .or_else(|| table.get(&legacy_group))
                .and_then(toml::Value::as_table)
                .into_iter()
                .flatten();
            let missing = declarations
                .filter(|(name, spec)| {
                    let (spec, base) =
                        if spec.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                            (
                                inherited.and_then(|table| table.get(name)),
                                workspace.as_deref(),
                            )
                        } else {
                            (Some(*spec), Some(directory))
                        };
                    !spec.zip(base).is_some_and(|(spec, base)| {
                        cargo_dependency_covered(inputs, lock, owner, base, name, spec)
                    })
                })
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                if target.is_none() && group == "dependencies" {
                    return Err(unresolved(path));
                }
                let scope =
                    target.map_or_else(|| group.to_owned(), |target| format!("{target} {group}"));
                warnings.push(format!(
                    "Cargo.lock did not verify {scope} in {} ({})",
                    path.display(),
                    missing.join(", ")
                ));
            }
        }
    }
    Ok(())
}

fn cargo_dependency_covered(
    inputs: &ScanInputs,
    lock: Option<&CargoLock>,
    owner: Option<&CargoPackage>,
    base: &Path,
    name: &str,
    spec: &toml::Value,
) -> bool {
    if let Some(relative) = spec.get("path").and_then(toml::Value::as_str) {
        return inputs
            .root()
            .join(base)
            .join(relative)
            .join("Cargo.toml")
            .canonicalize()
            .ok()
            .is_some_and(|path| {
                path.strip_prefix(inputs.root())
                    .is_ok_and(|path| inputs.includes(path))
            });
    }
    let real_name = spec
        .get("package")
        .and_then(toml::Value::as_str)
        .unwrap_or(name);
    let requirement = spec
        .get("version")
        .and_then(toml::Value::as_str)
        .or_else(|| spec.as_str());
    let requirement = match requirement.map(semver::VersionReq::parse).transpose() {
        Ok(requirement) => requirement,
        Err(_) => return false,
    };
    let git = spec.get("git").and_then(toml::Value::as_str);
    let Some((lock, owner)) = lock.zip(owner) else {
        return false;
    };
    lock.packages.iter().any(|entry| {
        entry.name == real_name
            && requirement.as_ref().is_none_or(|requirement| {
                semver::Version::parse(&entry.version)
                    .is_ok_and(|version| requirement.matches(&version))
            })
            && git.is_none_or(|url| {
                entry
                    .source
                    .as_deref()
                    .and_then(|source| source.strip_prefix("git+"))
                    .is_some_and(|source| source.split(['?', '#']).next() == Some(url))
            })
            && owner.dependencies.iter().any(|edge| {
                let mut identity = edge.splitn(3, ' ');
                identity.next() == Some(real_name)
                    && identity
                        .next()
                        .is_none_or(|version| version == entry.version)
                    && identity.next().is_none_or(|source| {
                        source
                            .strip_prefix('(')
                            .and_then(|source| source.strip_suffix(')'))
                            == entry.source.as_deref()
                    })
            })
    })
}

pub(super) fn verify_cargo_and_python(
    inputs: &ScanInputs,
    cargo: &CatalogedVersions,
    python: &PythonCoverage,
) -> Result<Vec<String>, Error> {
    let locks = cargo_locks(inputs, cargo)?;
    let mut manifests = BTreeMap::new();
    for path in inputs.paths().filter(|path| {
        is_project_input(path) && path.file_name().is_some_and(|name| name == "Cargo.toml")
    }) {
        let text = inputs.read_text(path, super::MAX_SUPPORTED_MANIFEST_BYTES)?;
        manifests.insert(
            path.parent().unwrap_or_else(|| Path::new("")),
            toml::from_str(&text).map_err(|_| unresolved(path))?,
        );
    }
    let mut warnings = Vec::new();
    for path in inputs.paths().filter(|path| is_project_input(path)) {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        match name {
            "Cargo.toml" => {
                verify_cargo_manifest(inputs, path, &locks, &manifests, &mut warnings)?;
            }
            name if is_python_lock(name) => {
                python.verify_lock(inputs, path, name, &mut warnings)?;
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
                verify_python_groups(path, &manifest, python, &mut warnings);
            }
            name if is_requirements_file(name) => {
                let text = inputs.read_text(path, MAX_LOCKFILE_BYTES)?;
                verify_python_requirements(path, text.lines(), python)?;
            }
            _ => {}
        }
    }
    Ok(warnings)
}

#[derive(Default)]
pub(super) struct PythonCoverage {
    versions: CatalogedVersions,
    packages_by_root: BTreeMap<PathBuf, BTreeMap<String, BTreeSet<String>>>,
}

impl PythonCoverage {
    pub(super) fn record(&mut self, name: &str, version: &str, component: &Value) {
        let Some(name) = super::normalize_python_package_name(name) else {
            return;
        };
        self.versions.record(&name, version, component);
        for root in super::syft_locations(component)
            .filter_map(|location| location.get("value")?.as_str())
            .filter_map(python_resolution_root)
        {
            self.packages_by_root
                .entry(root.to_path_buf())
                .or_default()
                .entry(name.clone())
                .or_default()
                .insert(normalize_python_version(version));
        }
    }

    fn covers(&self, path: &Path, name: &str, version: Option<&str>) -> bool {
        path.parent().is_some_and(|directory| {
            directory.ancestors().any(|root| {
                self.packages_by_root
                    .get(root)
                    .and_then(|packages| packages.get(name))
                    .is_some_and(|versions| {
                        version.is_none_or(|version| versions.contains(version))
                    })
            })
        })
    }

    fn verify_lock(
        &self,
        inputs: &ScanInputs,
        path: &Path,
        name: &str,
        warnings: &mut Vec<String>,
    ) -> Result<(), Error> {
        let contents = inputs.read_text(path, MAX_LOCKFILE_BYTES)?;
        let packages = if name == "Pipfile.lock" {
            let parsed: BTreeMap<String, Value> =
                serde_json::from_str(&contents).map_err(|_| unresolved(path))?;
            parsed
                .into_iter()
                .filter(|(group, _)| group != "_meta")
                .map(|(_, group)| {
                    let group = group.as_object().ok_or_else(|| unresolved(path))?;
                    Ok(group
                        .iter()
                        .map(|(name, spec)| {
                            let version = spec
                                .get("version")
                                .and_then(Value::as_str)
                                .and_then(|version| version.strip_prefix("=="))
                                .map(str::to_owned);
                            (name.clone(), version)
                        })
                        .collect::<Vec<_>>())
                })
                .collect::<Result<Vec<_>, Error>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        } else {
            let parsed: toml::Table = toml::from_str(&contents).map_err(|_| unresolved(path))?;
            let key = if name.starts_with("pylock") {
                "packages"
            } else {
                "package"
            };
            parsed
                .get(key)
                .map(|entries| {
                    let entries = entries.as_array().ok_or_else(|| unresolved(path))?;
                    entries
                        .iter()
                        .map(|entry| {
                            let name = entry
                                .get("name")
                                .and_then(toml::Value::as_str)
                                .ok_or_else(|| unresolved(path))?;
                            let version = entry
                                .get("version")
                                .and_then(toml::Value::as_str)
                                .map(str::to_owned);
                            Ok((name.to_owned(), version))
                        })
                        .collect::<Result<Vec<_>, Error>>()
                })
                .transpose()?
                .unwrap_or_default()
        };
        let missing = packages
            .into_iter()
            .filter(|(name, version)| {
                !super::normalize_python_package_name(name)
                    .zip(version.as_ref())
                    .is_some_and(|(name, version)| self.versions.covers(path, &name, version))
            })
            .map(|(name, version)| {
                version.map_or_else(|| name.clone(), |version| format!("{name} {version}"))
            })
            .collect::<BTreeSet<_>>();
        if !missing.is_empty() {
            warnings.push(format!(
                "Syft did not verify {} resolved Python package entries from {} (including {}).",
                missing.len(),
                path.display(),
                missing
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(())
    }
}

fn is_requirements_file(name: &str) -> bool {
    name.starts_with("requirements") && name.ends_with(".txt")
}

fn is_python_lock(name: &str) -> bool {
    matches!(
        name,
        "uv.lock" | "poetry.lock" | "Pipfile.lock" | "pdm.lock" | "pylock.toml"
    ) || name.ends_with(".py.lock")
        || (name.starts_with("pylock.") && name.ends_with(".toml"))
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
    if is_requirements_file(name) || is_python_lock(name) || name == "pyproject.toml" {
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
        .any(|name| !packages.covers(path, &name, None));
    if missing {
        return Err(unresolved(path));
    }
    Ok(())
}

fn verify_python_groups(
    path: &Path,
    manifest: &toml::Value,
    packages: &PythonCoverage,
    warnings: &mut Vec<String>,
) {
    let extras = manifest
        .get("project")
        .and_then(|project| project.get("optional-dependencies"))
        .and_then(toml::Value::as_table);
    let groups = manifest
        .get("dependency-groups")
        .and_then(toml::Value::as_table);
    let group_names = groups
        .into_iter()
        .flat_map(|groups| groups.keys())
        .filter_map(|group| super::normalize_python_package_name(group))
        .collect::<BTreeSet<_>>();
    for (kind, declarations) in [("extra", extras), ("dependency group", groups)] {
        for (group, declarations) in declarations.into_iter().flatten() {
            let missing = declarations.as_array().is_none_or(|entries| {
                entries.iter().any(|entry| {
                    if let Some(requirement) = entry.as_str() {
                        return !python_requirement_covered(path, requirement, packages);
                    }
                    kind != "dependency group"
                        || !entry
                            .get("include-group")
                            .and_then(toml::Value::as_str)
                            .and_then(super::normalize_python_package_name)
                            .is_some_and(|include| group_names.contains(&include))
                })
            });
            if missing {
                warnings.push(format!("Resolved Python packages for {kind} {group} in {} could not be verified. Provide an up-to-date lockfile covering this group.", path.display()));
            }
        }
    }
    let poetry = manifest.get("tool").and_then(|tool| tool.get("poetry"));
    let poetry_dev = poetry
        .and_then(|poetry| poetry.get("dev-dependencies"))
        .map(|table| ("dev", table));
    let poetry_groups = poetry
        .and_then(|poetry| poetry.get("group"))
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .filter_map(|(name, group)| Some((name.as_str(), group.get("dependencies")?)));
    for (group, dependencies) in poetry_dev.into_iter().chain(poetry_groups) {
        if dependencies.as_table().is_none_or(|entries| {
            entries
                .keys()
                .any(|entry| !python_requirement_covered(path, entry, packages))
        }) {
            warnings.push(format!("Resolved Python packages for Poetry group {group} in {} could not be verified. Provide an up-to-date poetry.lock covering this group.", path.display()));
        }
    }
}

fn python_requirement_covered(path: &Path, requirement: &str, packages: &PythonCoverage) -> bool {
    let Some(name) = super::normalize_python_requirement(requirement) else {
        return true;
    };
    let version = requirement
        .split(';')
        .next()
        .and_then(|specifier| specifier.split_once("=="))
        .filter(|(prefix, _)| !prefix.contains(['@', '<', '>', '!', '~', ',']))
        .map(|(_, version)| {
            version
                .trim_start_matches('=')
                .split(['#', ','])
                .next()
                .unwrap_or_default()
                .trim()
        })
        .filter(|version| !version.is_empty() && !version.contains('*'))
        .map(normalize_python_version);
    packages.covers(path, &name, version.as_deref())
}

fn normalize_python_version(version: &str) -> String {
    let version = version.to_ascii_lowercase();
    if version
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        let release = version.trim_end_matches(".0");
        return if release.is_empty() {
            "0".into()
        } else {
            release.to_owned()
        };
    }
    version
}

fn unresolved(path: &Path) -> Error {
    incomplete(&format!("Could not verify the dependency inventory from {}. Check the manifest and restore its resolution before retrying.", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scan_inputs(root: &Path, files: &[(&str, &str)]) -> ScanInputs {
        let listing = files
            .iter()
            .map(|(path, text)| {
                let target = root.join(path);
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::write(target, text).unwrap();
                json!({"location":{"path": format!("/{path}")}, "metadata":{"type":"RegularFile"}})
            })
            .collect::<Vec<_>>();
        ScanInputs::capture(
            root,
            &serde_json::to_vec(&json!({"files": listing})).unwrap(),
        )
        .unwrap()
    }

    fn from_file(path: &str) -> Value {
        json!({"properties": [{"name":"syft:location:0:path", "value":path}]})
    }

    #[test]
    fn validates_inherited_cargo_dev_and_target_dependencies_against_the_owner_and_lockfile() {
        let root = tempfile::tempdir().unwrap();
        let inputs = scan_inputs(root.path(), &[
            ("Cargo.toml", "[workspace]\nmembers = ['member']\n[workspace.dependencies]\nalias = {package = 'lib', version = '=1.0.0'}\n"),
            ("member/Cargo.toml", "[package]\nname = 'member'\nversion = '0.1.0'\n[dev-dependencies]\nalias.workspace = true\n[build-dependencies]\nlib = '=2.0.0'\n[target.'cfg(unix)'.dependencies]\nlib = '=1.0.0'\n"),
            ("Cargo.lock", r#"version = 4
[[package]]
name = 'member'
version = '0.1.0'
dependencies = ['lib 1.0.0']
[[package]]
name = 'lib'
version = '1.0.0'
source = 'registry+https://github.com/rust-lang/crates.io-index'
[[package]]
name = 'lib'
version = '2.0.0'
source = 'registry+https://github.com/rust-lang/crates.io-index'
"#),
        ]);
        let mut cargo = CatalogedVersions::default();
        let python = PythonCoverage::default();
        cargo.record("lib", "1.0.0", &from_file("/Cargo.lock"));
        cargo.record("lib", "2.0.0", &from_file("/sibling/Cargo.lock"));
        assert!(verify_cargo_and_python(&inputs, &cargo, &python).is_err());
        cargo.record("lib", "2.0.0", &from_file("/Cargo.lock"));
        let warnings = verify_cargo_and_python(&inputs, &cargo, &python).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("build-dependencies"));
    }

    #[test]
    fn verifies_python_groups_and_exact_lock_entries_with_corresponding_provenance() {
        let root = tempfile::tempdir().unwrap();
        let project = r#"
[project.optional-dependencies]
feature = ["feature-lib==2.0"]
[dependency-groups]
dev = ["Dev_Lib>=1", {include-group = "lint"}]
lint = ["lint-lib==3.0.0; python_version < '3.14'"]
"#;
        let inputs = scan_inputs(
            root.path(),
            &[
                ("app/pyproject.toml", project),
                (
                    "app/uv.lock",
                    "version = 1\n[[package]]\nname = 'dev-lib'\nversion = '1.0.0'\n",
                ),
            ],
        );
        let cargo = CatalogedVersions::default();
        let mut python = PythonCoverage::default();
        python.record("dev-lib", "1.0.0", &from_file("/other/uv.lock"));
        python.record("feature-lib", "1.0.0", &from_file("/app/uv.lock"));
        let warnings = verify_cargo_and_python(&inputs, &cargo, &python).unwrap();
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        python.record("dev-lib", "1.0.0", &from_file("/app/uv.lock"));
        python.record("feature-lib", "2.0.0", &from_file("/app/uv.lock"));
        python.record("lint-lib", "3.0", &from_file("/app/uv.lock"));
        assert!(verify_cargo_and_python(&inputs, &cargo, &python)
            .unwrap()
            .is_empty());
    }
}
