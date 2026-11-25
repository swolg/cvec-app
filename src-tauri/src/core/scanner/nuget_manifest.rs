use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use quick_xml::{events::Event, Reader};
use serde::{de::DeserializeOwned, Deserialize};

use super::inputs::{incomplete, is_project_input, ScanInputs, MAX_LOCKFILE_BYTES};
use super::MAX_SUPPORTED_MANIFEST_BYTES;
use crate::core::models::{NugetDeclaration, NugetProject};
use crate::Error;

#[derive(Default)]
pub(super) struct SharedManifests {
    central: BTreeMap<PathBuf, ScopedManifest>,
    build_props: BTreeMap<PathBuf, ScopedManifest>,
}

type ScopedManifest = (PathBuf, Option<MsbuildFile>);

#[derive(Deserialize, Default)]
struct MsbuildFile {
    #[serde(default, rename = "ItemGroup")]
    items: Vec<ItemGroup>,
    #[serde(default, rename = "PropertyGroup")]
    properties: Vec<PropertyGroup>,
}

#[derive(Deserialize)]
struct ItemGroup {
    #[serde(rename = "@Condition")]
    condition: Option<String>,
    #[serde(default, rename = "PackageReference")]
    references: Vec<PackageItem>,
    #[serde(default, rename = "PackageVersion")]
    versions: Vec<PackageItem>,
}

#[derive(Deserialize)]
struct PropertyGroup {
    #[serde(rename = "@Condition")]
    condition: Option<String>,
    #[serde(default, rename = "TargetFramework")]
    framework: Vec<String>,
    #[serde(default, rename = "TargetFrameworks")]
    frameworks: Vec<String>,
    #[serde(default, rename = "TargetFrameworkVersion")]
    framework_version: Vec<String>,
    #[serde(default, rename = "ManagePackageVersionsCentrally")]
    central_versions: Vec<String>,
    #[serde(default, rename = "ImportDirectoryBuildProps")]
    import_build_props: Vec<String>,
}

#[derive(Deserialize)]
struct PackageItem {
    #[serde(rename = "@Include")]
    include: Option<String>,
    #[serde(rename = "@Condition")]
    condition: Option<String>,
    #[serde(rename = "@Version")]
    version_attribute: Option<String>,
    #[serde(default, rename = "Version")]
    version_elements: Vec<VersionElement>,
    #[serde(rename = "@VersionOverride")]
    override_attribute: Option<String>,
    #[serde(default, rename = "VersionOverride")]
    override_elements: Vec<VersionElement>,
}

#[derive(Deserialize)]
struct VersionElement {
    #[serde(rename = "$text")]
    value: Option<String>,
    #[serde(rename = "@Condition")]
    condition: Option<String>,
}

impl PackageItem {
    fn names(&self) -> impl Iterator<Item = &str> {
        self.include
            .as_deref()
            .into_iter()
            .flat_map(|include| include.split(';'))
            .map(str::trim)
            .filter(|name| is_package_id(name))
    }

    fn requirement(&self) -> Option<&str> {
        let (attribute, elements) =
            if self.override_attribute.is_some() || !self.override_elements.is_empty() {
                (
                    self.override_attribute.as_deref(),
                    self.override_elements.as_slice(),
                )
            } else {
                (
                    self.version_attribute.as_deref(),
                    self.version_elements.as_slice(),
                )
            };
        match elements {
            [] => attribute,
            [element] => element.value.as_deref(),
            _ => None,
        }
        .map(str::trim)
        .filter(|version| !version.is_empty() && version.len() <= 256)
    }

    fn inherits_version(&self) -> bool {
        self.version_attribute.is_none()
            && self.version_elements.is_empty()
            && self.override_attribute.is_none()
            && self.override_elements.is_empty()
    }

    fn conditional(&self) -> bool {
        self.condition.is_some()
            || self
                .version_elements
                .iter()
                .chain(&self.override_elements)
                .any(|value| value.condition.is_some())
    }
}

impl MsbuildFile {
    fn property<'a>(
        &'a self,
        select: impl Fn(&'a PropertyGroup) -> &'a Vec<String>,
    ) -> Option<bool> {
        self.properties
            .iter()
            .filter(|group| group.condition.is_none())
            .flat_map(select)
            .next_back()
            .and_then(|value| value.trim().to_ascii_lowercase().parse().ok())
    }

    fn frameworks(&self) -> impl Iterator<Item = &str> {
        self.properties
            .iter()
            .flat_map(|group| {
                group
                    .framework
                    .iter()
                    .chain(&group.frameworks)
                    .chain(&group.framework_version)
            })
            .flat_map(|framework| framework.split(';'))
            .map(str::trim)
            .filter(|framework| {
                !framework.is_empty()
                    && framework.len() <= 64
                    && framework
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
            })
    }
}

impl SharedManifests {
    pub(super) fn capture(inputs: &ScanInputs, warnings: &mut Vec<String>) -> Self {
        let mut shared = Self::default();
        for path in inputs.paths().filter(|path| is_project_input(path)) {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let files = if name.eq_ignore_ascii_case("Directory.Packages.props") {
                &mut shared.central
            } else if name.eq_ignore_ascii_case("Directory.Build.props") {
                &mut shared.build_props
            } else {
                continue;
            };
            let manifest = read_xml(inputs, path, "Project", MAX_SUPPORTED_MANIFEST_BYTES)
                .inspect_err(|error| warnings.push(error.to_string()))
                .ok();
            let folder = path.parent().unwrap_or_else(|| Path::new(""));
            files
                .entry(folder.to_path_buf())
                .and_modify(|(_, doc)| *doc = None)
                .or_insert_with(|| (path.to_path_buf(), manifest));
        }
        shared
    }

    pub(super) fn project(&self, inputs: &ScanInputs, path: &Path) -> Result<NugetProject, Error> {
        let project: MsbuildFile = read_xml(inputs, path, "Project", MAX_SUPPORTED_MANIFEST_BYTES)?;
        let directory = path.parent().unwrap_or_else(|| Path::new(""));
        let central = closest_manifest(directory, &self.central);
        let build_props = closest_manifest(directory, &self.build_props)
            .filter(|_| project.property(|group| &group.import_build_props) != Some(false));
        let uses_central_versions = project
            .property(|group| &group.central_versions)
            .or_else(|| central.and_then(|(_, doc)| doc.property(|group| &group.central_versions)))
            .or_else(|| {
                build_props.and_then(|(_, doc)| doc.property(|group| &group.central_versions))
            });
        let central = central.filter(|_| uses_central_versions == Some(true));
        let mut central_versions = BTreeMap::<String, BTreeSet<(&str, bool)>>::new();
        if let Some((_, doc)) = central {
            for group in &doc.items {
                for item in &group.versions {
                    if let Some(requirement) = item.requirement() {
                        for name in item.names() {
                            central_versions
                                .entry(name.to_ascii_lowercase())
                                .or_default()
                                .insert((
                                    requirement,
                                    group.condition.is_some() || item.conditional(),
                                ));
                        }
                    }
                }
            }
        }

        let mut declarations = BTreeSet::new();
        for (source, doc) in std::iter::once((path, &project)).chain(build_props) {
            for group in &doc.items {
                for item in &group.references {
                    for name in item.names() {
                        let direct = item
                            .requirement()
                            .map(|requirement| (requirement, source, false));
                        let shared =
                            central
                                .filter(|_| item.inherits_version())
                                .and_then(|(path, _)| {
                                    let choices =
                                        central_versions.get(&name.to_ascii_lowercase())?;
                                    if choices.len() != 1 {
                                        return None;
                                    }
                                    let (requirement, conditional) = choices.first()?;
                                    Some((*requirement, path, *conditional))
                                });
                        let requirement = direct.or(shared);
                        declarations.insert(NugetDeclaration {
                            name: name.to_owned(),
                            requirement: requirement.map(|(value, _, _)| value.to_owned()),
                            requirement_path: requirement
                                .map(|(_, path, _)| path.display().to_string()),
                            conditional: group.condition.is_some()
                                || item.conditional()
                                || requirement.is_some_and(|(_, _, conditional)| conditional),
                        });
                    }
                }
            }
        }
        let frameworks = project
            .frameworks()
            .chain(
                build_props
                    .into_iter()
                    .flat_map(|(_, doc)| doc.frameworks()),
            )
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        Ok(NugetProject {
            path: path.display().to_string(),
            frameworks: frameworks.into_iter().take(32).collect(),
            declarations: declarations.into_iter().collect(),
            resolution_path: None,
        })
    }
}

fn closest_manifest<'a>(
    directory: &Path,
    files: &'a BTreeMap<PathBuf, ScopedManifest>,
) -> Option<(&'a Path, &'a MsbuildFile)> {
    let (path, doc) = directory.ancestors().find_map(|folder| files.get(folder))?;
    Some((path.as_path(), doc.as_ref()?))
}

#[derive(Deserialize)]
struct LegacyPackages {
    #[serde(default, rename = "package")]
    packages: Vec<LegacyPackage>,
}

#[derive(Deserialize)]
struct LegacyPackage {
    #[serde(rename = "@id")]
    name: String,
    #[serde(rename = "@version")]
    version: String,
}

pub(super) fn legacy_packages(
    inputs: &ScanInputs,
    path: &Path,
) -> Result<Vec<(String, String)>, Error> {
    let doc: LegacyPackages = read_xml(inputs, path, "packages", MAX_LOCKFILE_BYTES)?;
    Ok(doc
        .packages
        .into_iter()
        .map(|package| (package.name, package.version))
        .collect())
}

pub(super) fn is_package_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn read_xml<T: DeserializeOwned>(
    inputs: &ScanInputs,
    path: &Path,
    root: &str,
    limit: u64,
) -> Result<T, Error> {
    let fail = || {
        incomplete(&format!(
            "Could not read NuGet metadata from {}.",
            path.display()
        ))
    };
    let contents = inputs.read_text(path, limit).map_err(|_| fail())?;
    let contents = contents.trim_start_matches('\u{feff}');
    let mut reader = Reader::from_str(contents);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event().map_err(|_| fail())? {
            Event::Start(tag) | Event::Empty(tag)
                if tag.local_name().as_ref() == root.as_bytes() =>
            {
                break
            }
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
            _ => return Err(fail()),
        }
    }
    quick_xml::de::from_str(contents).map_err(|_| fail())
}
