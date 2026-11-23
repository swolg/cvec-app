use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use deno_semver::npm::{parse_npm_version, parse_npm_version_req};
use serde::Deserialize;
use serde_json::Value;

use crate::Error;

use super::{
    inputs::{is_project_input, ScanInputs, MAX_LOCKFILE_BYTES},
    MAX_SUPPORTED_MANIFEST_BYTES,
};

type PackageVersions = BTreeMap<String, BTreeSet<String>>;

#[derive(Default)]
pub(super) struct NpmCoverage {
    packages_by_root: BTreeMap<PathBuf, PackageVersions>,
    unlocated_packages: PackageVersions,
}

impl NpmCoverage {
    pub(super) fn record(&mut self, name: &str, version: &str, component: &Value) {
        let mut has_location = false;
        for location in super::syft_locations(component) {
            has_location = true;
            if let Some(root) = location
                .get("value")
                .and_then(Value::as_str)
                .and_then(resolution_root)
            {
                self.packages_by_root
                    .entry(root)
                    .or_default()
                    .entry(name.to_owned())
                    .or_default()
                    .insert(version.to_owned());
            }
        }
        // Older CycloneDX exporters omit Syft's source locations.
        if !has_location {
            self.unlocated_packages
                .entry(name.to_owned())
                .or_default()
                .insert(version.to_owned());
        }
    }

    pub(super) fn verify(&self, inputs: &ScanInputs) -> Result<(), Error> {
        let pnpm = read_pnpm_resolutions(inputs)?;
        for manifest_path in inputs.paths().filter(|path| {
            is_project_input(path) && path.file_name().is_some_and(|name| name == "package.json")
        }) {
            let contents = inputs.read_text(manifest_path, MAX_SUPPORTED_MANIFEST_BYTES)?;
            let manifest: Value = serde_json::from_str(contents.trim_start_matches('\u{feff}'))
                .map_err(|_| inventory_gap(manifest_path))?;
            let directory = manifest_path.parent().unwrap_or_else(|| Path::new(""));
            let resolution = directory.ancestors().find_map(|parent| {
                let lock = pnpm.get(parent)?;
                let importer = directory.strip_prefix(parent).ok()?;
                let importer = if importer.as_os_str().is_empty() {
                    ".".to_owned()
                } else {
                    importer.to_str()?.replace('\\', "/")
                };
                Some((lock, importer))
            });
            let missing = runtime_dependencies(&manifest).any(|(name, specification)| {
                let effective = match &resolution {
                    Some((lock, importer)) => {
                        lock.effective_specifier(importer, &manifest, name, specification)
                    }
                    None => Some(specification),
                };
                let Some(effective) = effective else {
                    return false;
                };
                let Some((name, specification)) = registry_dependency(name, effective) else {
                    return false;
                };
                let requirement = parse_npm_version_req(specification).ok();
                let range = requirement
                    .as_ref()
                    .and_then(|requirement| requirement.range());
                let covers = |packages: &PackageVersions| {
                    packages.get(name).is_some_and(|versions| {
                        // Dist-tags and source URLs have no locally comparable range.
                        range.is_none_or(|range| {
                            versions.iter().any(|version| {
                                parse_npm_version(version)
                                    .is_ok_and(|version| range.satisfies(&version))
                            })
                        })
                    })
                };
                !covers(&self.unlocated_packages)
                    && !directory
                        .ancestors()
                        .any(|ancestor| self.packages_by_root.get(ancestor).is_some_and(covers))
            });
            if missing {
                return Err(inventory_gap(manifest_path));
            }
        }
        Ok(())
    }
}

fn resolution_root(path: &str) -> Option<PathBuf> {
    // Syft uses slash-prefixed paths relative to the scanned source.
    let path = Path::new(path.trim_start_matches('/'));
    let mut root = PathBuf::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        if part == "node_modules" {
            return Some(root);
        }
        root.push(part);
    }
    matches!(
        root.file_name()?.to_str()?,
        "package-lock.json"
            | "npm-shrinkwrap.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lock"
            | "bun.lockb"
    )
    .then(|| root.parent().unwrap_or_else(|| Path::new("")).to_path_buf())
}

fn runtime_dependencies(manifest: &Value) -> impl Iterator<Item = (&str, &str)> {
    // Peer and optional dependencies can be absent from a complete install.
    manifest
        .get("dependencies")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(move |(name, specification)| {
            let specification = specification.as_str()?.trim();
            manifest
                .get("optionalDependencies")
                .and_then(|optional| optional.get(name))
                .is_none()
                .then_some((name.as_str(), specification))
        })
}

fn registry_dependency<'a>(name: &'a str, specification: &'a str) -> Option<(&'a str, &'a str)> {
    if ["file:", "link:", "portal:", "workspace:"]
        .iter()
        .any(|protocol| specification.starts_with(protocol))
        || specification.starts_with(['.', '/'])
    {
        return None;
    }
    Some(match specification.strip_prefix("npm:") {
        Some(alias) => alias
            .rsplit_once('@')
            .filter(|(target, _)| !target.is_empty())
            .unwrap_or((alias, "latest")),
        None => (name, specification),
    })
}

#[derive(Deserialize, Default)]
struct PnpmWorkspace {
    overrides: Option<PackageSpecifiers>,
    #[serde(default)]
    catalog: PackageSpecifiers,
    #[serde(default)]
    catalogs: BTreeMap<String, PackageSpecifiers>,
}

type PackageSpecifiers = BTreeMap<String, String>;

#[derive(Deserialize)]
struct PnpmLock {
    #[serde(default)]
    overrides: BTreeMap<String, String>,
    #[serde(default)]
    importers: BTreeMap<String, PnpmImporter>,
}

#[derive(Deserialize)]
struct PnpmImporter {
    #[serde(default)]
    dependencies: BTreeMap<String, PnpmDependency>,
}

#[derive(Deserialize)]
struct PnpmDependency {
    specifier: String,
}

impl PnpmLock {
    fn effective_specifier<'a>(
        &'a self,
        importer: &str,
        manifest: &Value,
        name: &str,
        declared: &'a str,
    ) -> Option<&'a str> {
        let Some(importer) = self.importers.get(importer) else {
            return Some(declared);
        };
        let resolved = importer
            .dependencies
            .get(name)
            .map(|entry| entry.specifier.as_str());
        let authorized = self.overrides.iter().any(|(selector, substitute)| {
            let matches = match pnpm_parent_selector(selector) {
                Some((parent, target)) => {
                    manifest
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| {
                            let version = manifest
                                .get("version")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            selector_applies(parent, name, version)
                        })
                        && selector_applies(target, name, declared)
                }
                None => selector_applies(selector, name, declared),
            };
            matches
                && (Some(substitute.as_str()) == resolved
                    || (substitute == "-" && resolved.is_none()))
        });
        // An importer cannot authorize a new requirement on its own. The
        // resolved specifier must be backed by a configured, applicable override.
        if authorized {
            resolved
        } else {
            Some(declared)
        }
    }
}

fn pnpm_parent_selector(selector: &str) -> Option<(&str, &str)> {
    // A '>' after '@', a space or '|' belongs to a version range.
    let (delimiter, _) = selector.match_indices('>').find(|(index, _)| {
        *index > 0 && !matches!(selector.as_bytes()[index - 1], b'@' | b' ' | b'|')
    })?;
    Some((&selector[..delimiter], &selector[delimiter + 1..]))
}

fn selector_applies(selector: &str, name: &str, declared: &str) -> bool {
    if selector == name {
        return true;
    }
    selector
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('@'))
        .is_some_and(|selector| {
            selector == declared
                || parse_npm_version_req(selector)
                    .ok()
                    .zip(parse_npm_version_req(declared).ok())
                    .is_some_and(|(selector, declared)| selector.intersects(&declared))
        })
}

fn read_pnpm_resolutions(inputs: &ScanInputs) -> Result<BTreeMap<PathBuf, PnpmLock>, Error> {
    let mut resolutions = BTreeMap::new();
    for path in inputs.paths().filter(|path| {
        is_project_input(path)
            && path
                .file_name()
                .is_some_and(|name| name == "pnpm-lock.yaml")
    }) {
        let root = path.parent().unwrap_or_else(|| Path::new(""));
        let Some(configured) = pnpm_overrides(inputs, root)? else {
            continue;
        };
        if configured.is_empty() {
            continue;
        }
        let lock: PnpmLock = serde_saphyr::from_str(&inputs.read_text(path, MAX_LOCKFILE_BYTES)?)
            .map_err(|_| inventory_gap(path))?;
        if lock.overrides != configured {
            return Err(inventory_gap(path));
        }
        resolutions.insert(root.to_path_buf(), lock);
    }
    Ok(resolutions)
}

fn pnpm_overrides(inputs: &ScanInputs, root: &Path) -> Result<Option<PackageSpecifiers>, Error> {
    let manifest_path = root.join("package.json");
    let manifest: Value = if inputs.includes(&manifest_path) {
        let contents = inputs.read_text(&manifest_path, MAX_SUPPORTED_MANIFEST_BYTES)?;
        serde_json::from_str(contents.trim_start_matches('\u{feff}'))
            .map_err(|_| inventory_gap(&manifest_path))?
    } else {
        Value::Null
    };
    let workspace_path = root.join("pnpm-workspace.yaml");
    let workspace: PnpmWorkspace = if inputs.includes(&workspace_path) {
        let contents = inputs.read_text(&workspace_path, MAX_SUPPORTED_MANIFEST_BYTES)?;
        serde_saphyr::from_str::<Option<PnpmWorkspace>>(&contents)
            .map_err(|_| inventory_gap(&workspace_path))?
            .unwrap_or_default()
    } else {
        PnpmWorkspace::default()
    };
    // pnpm's legacy package.json settings take precedence when present.
    let legacy = manifest
        .pointer("/pnpm/overrides")
        .cloned()
        .map(serde_json::from_value::<PackageSpecifiers>)
        .transpose()
        .map_err(|_| inventory_gap(&manifest_path))?;
    let Some(mut overrides) = legacy.or(workspace.overrides) else {
        return Ok(None);
    };
    for (selector, specification) in &mut overrides {
        if let Some(reference) = specification.strip_prefix('$') {
            let referenced = ["optionalDependencies", "dependencies", "devDependencies"]
                .iter()
                .find_map(|field| manifest.get(field)?.get(reference)?.as_str());
            *specification = referenced
                .ok_or_else(|| inventory_gap(&manifest_path))?
                .to_owned();
        }
        if let Some(catalog_name) = specification.strip_prefix("catalog:") {
            let catalog = if catalog_name.is_empty() {
                Some(&workspace.catalog)
            } else {
                workspace.catalogs.get(catalog_name)
            };
            let target =
                pnpm_parent_selector(selector).map_or(selector.as_str(), |(_, target)| target);
            let name = target
                .match_indices('@')
                .find(|(index, _)| *index > 0)
                .map_or(target, |(index, _)| &target[..index]);
            *specification = catalog
                .and_then(|catalog| catalog.get(name))
                .cloned()
                .ok_or_else(|| inventory_gap(&workspace_path))?;
        }
    }
    Ok(Some(overrides))
}

fn inventory_gap(manifest: &Path) -> Error {
    Error::Scanner(format!(
        "Incomplete npm inventory: could not verify the dependencies in {}. Make sure the lockfile or installed dependencies match the manifest, then retry.",
        manifest.display()
    ))
}
