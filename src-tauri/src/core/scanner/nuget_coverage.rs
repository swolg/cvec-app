use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use super::inputs::{incomplete, is_project_input, ScanInputs, MAX_LOCKFILE_BYTES};
use super::nuget_manifest::{is_package_id, legacy_packages, SharedManifests};
use crate::core::models::{NugetDeclaration, NugetProject};
use crate::Error;

const MAX_REPORTED_PROJECTS: usize = 100;
const MAX_REPORTED_DECLARATIONS: usize = 2_000;
type NugetPackages = BTreeSet<(String, String)>;

#[derive(Default)]
pub(super) struct NugetCoverage {
    by_file: BTreeMap<PathBuf, BTreeSet<(String, String)>>,
}

#[derive(Deserialize)]
struct PackagesLock {
    dependencies: BTreeMap<String, BTreeMap<String, LockedPackage>>,
}

#[derive(Deserialize)]
struct LockedPackage {
    #[serde(rename = "type")]
    kind: String,
    resolved: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectAssets {
    version: u32,
    project: AssetsProject,
    targets: BTreeMap<String, BTreeMap<String, AssetLibrary>>,
    libraries: BTreeMap<String, AssetLibrary>,
    #[serde(default)]
    logs: Vec<AssetLog>,
}

#[derive(Deserialize)]
struct AssetsProject {
    restore: ProjectRestore,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectRestore {
    project_path: PathBuf,
    output_path: PathBuf,
    #[serde(default)]
    config_file_paths: Vec<PathBuf>,
    frameworks: BTreeMap<String, RestoreFramework>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreFramework {
    #[serde(default)]
    project_references: BTreeMap<PathBuf, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetLibrary {
    #[serde(rename = "type")]
    kind: String,
    msbuild_project: Option<PathBuf>,
}

#[derive(Deserialize)]
struct AssetLog {
    level: String,
}

#[derive(Default)]
pub(super) struct NugetResolution {
    pub(super) warnings: Vec<String>,
    pub(super) artifacts: Vec<Value>,
    pub(super) projects: Vec<NugetProject>,
}

impl NugetCoverage {
    pub(super) fn record(&mut self, name: &str, version: &str, component: &Value) {
        for path in super::syft_locations(component)
            .filter_map(|property| property.get("value").and_then(Value::as_str))
        {
            self.by_file
                .entry(PathBuf::from(path.trim_start_matches('/')))
                .or_default()
                .insert((name.to_ascii_lowercase(), version.to_owned()));
        }
    }

    pub(super) fn resolve(
        &self,
        inputs: &ScanInputs,
        native: bool,
    ) -> Result<NugetResolution, Error> {
        let mut resolution = NugetResolution::default();
        let mut projects = BTreeMap::<&Path, Vec<&Path>>::new();
        let mut assets_by_project = BTreeMap::<PathBuf, Vec<_>>::new();
        let mut verified_files = BTreeMap::new();
        let mut supplemental_locks = BTreeMap::<&Path, Vec<Value>>::new();
        let mut restored_artifacts = Vec::new();
        for path in inputs.paths().filter(|path| is_project_input(path)) {
            let name = file_name(path);
            if is_lockfile(&name) {
                let packages = lock_packages(inputs, path)?;
                let supplemental = (native && name != "packages.lock.json")
                    .then_some("cvec-nuget-project-lockfile");
                match packages.filter(|packages| {
                    self.catalog_packages(
                        path,
                        packages,
                        supplemental,
                        supplemental_locks.entry(path).or_default(),
                    )
                }) {
                    Some(packages) => {
                        verified_files.insert(path.to_path_buf(), packages);
                    }
                    None => resolution.warnings.push(format!(
                        "Could not verify the resolved NuGet packages in {}",
                        path.display()
                    )),
                }
            } else if is_project(&name) {
                projects.entry(directory(path)).or_default().push(path);
            } else if name == "project.assets.json" && native {
                if let Ok(text) = inputs.read_text(path, MAX_LOCKFILE_BYTES) {
                    if let Ok(assets) =
                        serde_json::from_str::<ProjectAssets>(text.trim_start_matches('\u{feff}'))
                    {
                        if let Some(project) =
                            local_relative(inputs, &assets.project.restore.project_path)
                        {
                            assets_by_project
                                .entry(project)
                                .or_default()
                                .push((path, assets));
                        }
                    }
                }
            } else if name == "packages.config" {
                let packages =
                    legacy_packages(inputs, path).map(|packages| packages.into_iter().collect());
                let supplemental = native.then_some("cvec-nuget-packages-config");
                match packages.ok().filter(|packages| {
                    self.catalog_packages(
                        path,
                        packages,
                        supplemental,
                        supplemental_locks.entry(path).or_default(),
                    )
                }) {
                    Some(packages) => {
                        verified_files.insert(path.to_path_buf(), packages);
                    }
                    None => resolution.warnings.push(format!(
                        "No resolved NuGet packages were cataloged from {}",
                        path.display()
                    )),
                }
            }
        }
        let shared = if projects.is_empty() {
            SharedManifests::default()
        } else {
            SharedManifests::capture(inputs, &mut resolution.warnings)
        };
        let mut left = MAX_REPORTED_DECLARATIONS;
        let mut report_truncated = false;
        for (folder, projects) in projects {
            for project in &projects {
                let mut details = shared.project(inputs, project).unwrap_or_else(|error| {
                    resolution.warnings.push(error.to_string());
                    NugetProject {
                        path: project.display().to_string(),
                        frameworks: Vec::new(),
                        declarations: Vec::new(),
                        resolution_path: None,
                    }
                });
                let named_lock = folder.join(format!(
                    "packages.{}.lock.json",
                    project.file_stem().unwrap_or_default().to_string_lossy()
                ));
                let standard_files = (projects.len() == 1).then(|| {
                    [
                        folder.join("packages.lock.json"),
                        folder.join("packages.config"),
                    ]
                });
                let project_file = std::iter::once(named_lock)
                    .chain(standard_files.into_iter().flatten())
                    .find(|path| inputs.includes(path));
                let mut resolved = project_file
                    .as_deref()
                    .and_then(|path| verified_files.get(path).map(|packages| (path, packages)));
                let restored;
                if project_file.is_none() {
                    if let Some([(path, assets)]) =
                        assets_by_project.get(*project).map(Vec::as_slice)
                    {
                        restored = assets.packages(inputs, project, path);
                        match &restored {
                            Ok(packages) => resolved = Some((path, packages)),
                            Err(reason) => resolution.warnings.push(format!(
                                "NuGet dependency coverage for {} could not be verified from {} ({reason}). Run restore or provide packages.lock.json.",
                                project.display(), path.display()
                            )),
                        }
                    } else {
                        resolution.warnings.push(format!(
                            "NuGet dependency coverage for {} requires packages.lock.json or a current project.assets.json for this project directory.",
                            project.display()
                        ));
                    }
                }
                if let Some((path, packages)) = resolved {
                    let missing = details.declarations.iter().find(|declaration| {
                        !declaration.conditional && !declaration_covered(declaration, packages)
                    });
                    if let Some(missing) = missing {
                        let warning = format!("NuGet resolution in {} does not cover {} declared by {}. Refresh the restore metadata.", path.display(), missing.name, project.display());
                        // A cataloged stale graph would still be sent to Grype.
                        if self.by_file.contains_key(path) {
                            return Err(incomplete(&warning));
                        }
                        supplemental_locks.remove(path);
                        resolution.warnings.push(warning);
                    } else if project_file.is_some()
                        || self.catalog_packages(
                            path,
                            packages,
                            Some("cvec-nuget-project-assets"),
                            &mut restored_artifacts,
                        )
                    {
                        details.resolution_path = Some(path.display().to_string());
                    } else {
                        resolution.warnings.push(format!(
                            "Could not verify the resolved NuGet package identities in {}.",
                            path.display()
                        ));
                    }
                }
                if resolution.projects.len() == MAX_REPORTED_PROJECTS {
                    report_truncated = true;
                    continue;
                }
                report_truncated |= details.declarations.len() > left;
                details.declarations.truncate(left);
                left -= details.declarations.len();
                resolution.projects.push(details);
            }
        }
        if report_truncated {
            resolution
                .warnings
                .push("The NuGet declaration report reached its size limit.".into());
        }
        resolution.artifacts = supplemental_locks
            .into_values()
            .flatten()
            .chain(restored_artifacts)
            .collect();
        Ok(resolution)
    }

    fn catalog_packages(
        &self,
        path: &Path,
        packages: &NugetPackages,
        supplemental: Option<&str>,
        artifacts: &mut Vec<Value>,
    ) -> bool {
        if packages
            .iter()
            .any(|(name, version)| !valid_package(name, version))
        {
            return false;
        }
        let already_cataloged = self.by_file.get(path);
        let mut uncataloged = packages
            .iter()
            .filter(|(name, version)| {
                !already_cataloged.is_some_and(|entries| {
                    entries.contains(&(name.to_ascii_lowercase(), version.clone()))
                })
            })
            .peekable();
        if let Some(cataloger) = supplemental {
            artifacts.extend(
                uncataloged.map(|(name, version)| nuget_artifact(name, version, path, cataloger)),
            );
            return true;
        }
        uncataloged.peek().is_none()
    }
}

fn lock_packages(inputs: &ScanInputs, path: &Path) -> Result<Option<NugetPackages>, Error> {
    let contents = inputs.read_text(path, MAX_LOCKFILE_BYTES)?;
    let packages = serde_json::from_str::<PackagesLock>(contents.trim_start_matches('\u{feff}'))
        .ok()
        .and_then(|lock| {
            lock.dependencies
                .into_values()
                .flatten()
                .filter(|(_, package)| !package.kind.eq_ignore_ascii_case("project"))
                .map(|(name, package)| Some((name, package.resolved?)))
                .collect()
        });
    Ok(packages)
}

fn declaration_covered(declaration: &NugetDeclaration, packages: &NugetPackages) -> bool {
    packages.iter().any(|(name, version)| {
        if !name.eq_ignore_ascii_case(&declaration.name) {
            return false;
        }
        let Some(requirement) = declaration.requirement.as_deref() else {
            return true;
        };
        meets_literal_requirement(version, requirement).unwrap_or(true)
    })
}

fn meets_literal_requirement(version: &str, requirement: &str) -> Option<bool> {
    let requirement = requirement.trim();
    if !requirement.starts_with(['[', '(']) {
        return compare_nuget_versions(version, requirement).map(Ordering::is_ge);
    }
    if !requirement.ends_with([']', ')']) {
        return None;
    }
    let limits = requirement.get(1..requirement.len() - 1)?.trim();
    let Some((lower, upper)) = limits.split_once(',') else {
        return if requirement.starts_with('[') && requirement.ends_with(']') {
            compare_nuget_versions(version, limits).map(Ordering::is_eq)
        } else {
            None
        };
    };
    let (lower, upper) = (lower.trim(), upper.trim());
    let lower_bound = if lower.is_empty() {
        true
    } else {
        let order = compare_nuget_versions(version, lower)?;
        order.is_gt() || (order.is_eq() && requirement.starts_with('['))
    };
    let upper_bound = if upper.is_empty() {
        true
    } else {
        let order = compare_nuget_versions(version, upper)?;
        order.is_lt() || (order.is_eq() && requirement.ends_with(']'))
    };
    Some(lower_bound && upper_bound)
}

fn compare_nuget_versions(left: &str, right: &str) -> Option<Ordering> {
    let (left, left_label) = version_release(left);
    let (right, right_label) = version_release(right);
    let release = numeric_release(left)?.cmp(&numeric_release(right)?);
    if !release.is_eq() {
        return Some(release);
    }
    let left_label = semver::Prerelease::new(&left_label.to_ascii_lowercase()).ok()?;
    let right_label = semver::Prerelease::new(&right_label.to_ascii_lowercase()).ok()?;
    Some(left_label.cmp(&right_label))
}

fn version_release(version: &str) -> (&str, &str) {
    let without_metadata = version
        .split_once('+')
        .map_or(version, |(release, _)| release);
    without_metadata
        .split_once('-')
        .unwrap_or((without_metadata, ""))
}

fn numeric_release(version: &str) -> Option<[u32; 4]> {
    let mut release = [0; 4];
    for (index, part) in version.split('.').enumerate() {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *release.get_mut(index)? = part.parse().ok()?;
    }
    Some(release)
}

impl ProjectAssets {
    fn packages(
        &self,
        inputs: &ScanInputs,
        project: &Path,
        path: &Path,
    ) -> Result<NugetPackages, &'static str> {
        let restore = &self.project.restore;
        if !matches!(self.version, 3 | 4)
            || self
                .logs
                .iter()
                .any(|log| log.level.eq_ignore_ascii_case("error"))
            || self.targets.is_empty()
            || restore.frameworks.is_empty()
            || !restore
                .frameworks
                .keys()
                .all(|framework| self.targets.contains_key(framework))
            || local_relative(inputs, &restore.output_path).as_deref() != Some(directory(path))
        {
            return Err("incomplete restore or unrecognized output location");
        }
        let restored = inputs
            .modified(path)
            .ok_or("restore timestamp is unavailable")?;
        let mut referenced = BTreeSet::from([project.to_path_buf()]);
        for reference in restore
            .frameworks
            .values()
            .flat_map(|framework| framework.project_references.keys())
        {
            referenced.insert(
                local_relative(inputs, reference)
                    .filter(|path| inputs.includes(path))
                    .ok_or("project reference is outside the scan")?,
            );
        }
        for library in self
            .libraries
            .values()
            .filter(|library| library.kind == "project")
        {
            let relative = library
                .msbuild_project
                .as_ref()
                .ok_or("project reference is missing a path")?;
            let reference = inputs.root().join(directory(project)).join(relative);
            referenced.insert(
                local_relative(inputs, &reference)
                    .filter(|path| inputs.includes(path))
                    .ok_or("project reference is outside the scan")?,
            );
        }
        let configs = restore
            .config_file_paths
            .iter()
            .filter_map(|path| {
                local_relative(inputs, path)
                    .or_else(|| path.strip_prefix(inputs.root()).ok().map(Path::to_path_buf))
            })
            .collect::<Vec<_>>();
        let nearby_build_inputs = inputs.paths().filter(|path| {
            let name = file_name(path);
            let is_build_input = name.ends_with(".props")
                || name.ends_with(".targets")
                || matches!(name.as_str(), "nuget.config" | "global.json");
            is_project_input(path)
                && is_build_input
                && !path
                    .components()
                    .any(|part| part.as_os_str() == "obj" || part.as_os_str() == "bin")
        });
        if referenced
            .iter()
            .map(PathBuf::as_path)
            .chain(nearby_build_inputs)
            .chain(configs.iter().map(PathBuf::as_path))
            .any(|input| {
                inputs
                    .modified(input)
                    .is_none_or(|modified| modified > restored)
            })
        {
            return Err("dependency inputs are newer than the restore or unavailable");
        }
        let resolved = self
            .targets
            .values()
            .flat_map(|target| target.iter())
            .filter(|(_, library)| library.kind == "package")
            .map(|(identity, _)| identity.as_str())
            .collect::<BTreeSet<_>>();
        let expected = self
            .libraries
            .iter()
            .filter(|(_, library)| library.kind == "package")
            .map(|(identity, _)| identity.as_str())
            .collect::<BTreeSet<_>>();
        if resolved != expected {
            return Err("target packages disagree with the restore inventory");
        }
        resolved
            .into_iter()
            .map(|identity| {
                let (name, version) = identity
                    .split_once('/')
                    .ok_or("invalid resolved package identity")?;
                Ok((name.to_owned(), version.to_owned()))
            })
            .collect()
    }
}

fn valid_package(name: &str, version: &str) -> bool {
    let (release, build) = version
        .split_once('+')
        .map_or((version, None), |(release, build)| (release, Some(build)));
    let (numeric, prerelease) = release
        .split_once('-')
        .map_or((release, None), |(numeric, prerelease)| {
            (numeric, Some(prerelease))
        });
    is_package_id(name)
        && version.len() <= 256
        && numeric_release(numeric).is_some()
        && [build, prerelease].into_iter().flatten().all(|suffix| {
            suffix.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        })
}

fn nuget_artifact(name: &str, version: &str, path: &Path, cataloger: &str) -> Value {
    let location = format!("/{}", path.display());
    let id = crate::core::hashing::sha256_hex(
        format!("{cataloger}\0{location}\0{name}/{version}").as_bytes(),
    );
    json!({
        "id": id,
        "name": name,
        "version": version,
        "purl": format!("pkg:nuget/{name}@{}", version.replace('+', "%2B")),
        "type": "dotnet",
        "language": "dotnet",
        "foundBy": cataloger,
        "locations": [{"path": location, "accessPath": location, "annotations": {"evidence": "primary"}}]
    })
}

fn local_relative(inputs: &ScanInputs, path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    path.canonicalize()
        .ok()?
        .strip_prefix(inputs.root())
        .ok()
        .map(Path::to_path_buf)
}

fn directory(path: &Path) -> &Path {
    path.parent().unwrap_or_else(|| Path::new(""))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_project(name: &str) -> bool {
    [".csproj", ".fsproj", ".vbproj"]
        .iter()
        .any(|extension| name.ends_with(extension))
}

fn is_lockfile(name: &str) -> bool {
    name == "packages.lock.json" || (name.starts_with("packages.") && name.ends_with(".lock.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_list(paths: &[&str]) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "files": paths.iter().map(|path| json!({
                "location": { "path": format!("/{path}") },
                "metadata": { "type": "RegularFile" }
            })).collect::<Vec<_>>()
        }))
        .unwrap()
    }

    #[test]
    fn requires_resolved_nuget_packages_from_the_correct_project_lockfile() {
        let root = tempfile::tempdir().unwrap();
        let project = "app/Fixture.csproj";
        let lock = "app/packages.lock.json";
        std::fs::create_dir(root.path().join("app")).unwrap();
        std::fs::write(root.path().join(project), "<Project/>").unwrap();
        let inputs = ScanInputs::capture(root.path(), &input_list(&[project])).unwrap();
        let mut coverage = NugetCoverage::default();
        assert!(coverage.resolve(&inputs, true).unwrap().warnings[0].contains(project));
        std::fs::write(
            root.path().join(lock),
            r#"{"dependencies":{"net8.0":{
            "Example":{"type":"Direct","resolved":"1.0.0"},
            "Sibling":{"type":"Project"}
        }}}"#,
        )
        .unwrap();
        let inputs = ScanInputs::capture(root.path(), &input_list(&[project, lock])).unwrap();
        coverage.record(
            "Example",
            "1.0.0",
            &json!({ "properties": [{
                "name": "syft:location:0:path", "value": "/other/packages.lock.json"
            }]}),
        );
        assert!(coverage.resolve(&inputs, true).unwrap().warnings[0].contains(lock));
        coverage.record(
            "Example",
            "1.0.0",
            &json!({ "properties": [{
                "name": "syft:location:0:path", "value": "/app/packages.lock.json"
            }]}),
        );
        assert!(coverage.resolve(&inputs, true).unwrap().warnings.is_empty());
    }

    #[test]
    fn adds_only_current_assets_tied_to_the_scanned_project_and_output() {
        let root = tempfile::tempdir().unwrap();
        let project = "Fixture.csproj";
        let assets = "obj/project.assets.json";
        std::fs::create_dir(root.path().join("obj")).unwrap();
        std::fs::write(root.path().join(project), "<Project/>").unwrap();
        let scan = |project_path, library: &str, output: &Path| {
            let entry = |identity: &str| json!({identity: {"type":"package"}});
            std::fs::write(
                root.path().join(assets),
                json!({
                    "version": 3,
                    "project": {"restore": {
                        "projectPath": project_path,
                        "outputPath": output,
                        "frameworks": {"net8.0": {}}
                    }},
                    "libraries": entry(library),
                    "targets": {"net8.0": entry("Example/1.0.0")}
                })
                .to_string(),
            )
            .unwrap();
            ScanInputs::capture(root.path(), &input_list(&[project, assets])).unwrap()
        };
        let coverage = NugetCoverage::default();
        let output = root.path().join("obj");
        let project_path = root.path().join(project);
        for inputs in [
            scan(root.path().join("Foreign.csproj"), "Example/1.0.0", &output),
            scan(project_path.clone(), "Example/2.0.0", &output),
            scan(project_path.clone(), "Example/1.0.0", root.path()),
        ] {
            let resolution = coverage.resolve(&inputs, true).unwrap();
            assert!(resolution.artifacts.is_empty());
            assert_eq!(resolution.warnings.len(), 1);
        }

        for requirement in ["[2.0.0]", "[2.0.0,3.0.0)", "(1.0.0,2.0.0]"] {
            std::fs::write(&project_path, format!("<Project><ItemGroup><PackageReference Include=\"Example\" Version=\"{requirement}\" /></ItemGroup></Project>")).unwrap();
            let inputs = scan(project_path.clone(), "Example/1.0.0", &output);
            let resolution = coverage.resolve(&inputs, true).unwrap();
            assert!(resolution.artifacts.is_empty());
            assert!(resolution.projects[0].resolution_path.is_none());
            assert!(resolution.warnings[0].contains("does not cover Example"));
        }

        std::fs::write(&project_path, r#"<Project><ItemGroup><PackageReference Include="Example" Version="[1.0.0.0,2.0.0)" /></ItemGroup></Project>"#).unwrap();
        let inputs = scan(project_path.clone(), "Example/1.0.0", &output);
        let resolution = coverage.resolve(&inputs, true).unwrap();
        assert_eq!(resolution.artifacts[0]["purl"], "pkg:nuget/Example@1.0.0");
        assert!(resolution.warnings.is_empty());
        std::fs::File::open(project_path)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
            .unwrap();
        let inputs = ScanInputs::capture(root.path(), &input_list(&[project, assets])).unwrap();
        let resolution = coverage.resolve(&inputs, true).unwrap();
        assert!(resolution.artifacts.is_empty());
        assert!(resolution.warnings[0].contains("newer"));
    }

    #[test]
    fn rejects_a_cataloged_nuget_lock_that_conflicts_with_its_project() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("App.csproj"), r#"<Project><ItemGroup><PackageReference Include="Example" Version="[2.0.0]" /></ItemGroup></Project>"#).unwrap();
        std::fs::write(
            root.path().join("packages.lock.json"),
            r#"{"dependencies":{"net8.0":{"Example":{"type":"Direct","resolved":"1.0.0"}}}}"#,
        )
        .unwrap();
        let inputs = ScanInputs::capture(
            root.path(),
            &input_list(&["App.csproj", "packages.lock.json"]),
        )
        .unwrap();
        let mut coverage = NugetCoverage::default();
        coverage.record(
            "Example",
            "1.0.0",
            &json!({"properties":[{"name":"syft:location:0:path","value":"/packages.lock.json"}]}),
        );
        let Err(error) = coverage.resolve(&inputs, true) else {
            panic!("accepted a lockfile that contradicts its project");
        };
        assert!(error.to_string().contains("does not cover Example"));
    }

    #[test]
    fn shows_literal_projects_and_central_constraints_before_a_restore() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("tool")).unwrap();
        std::fs::write(
            root.path().join("tool/App.csproj"),
            r#"
<Project xmlns="http://schemas.microsoft.com/developer/msbuild/2003">
  <ItemGroup><PackageReference Include="Central.Lib" /></ItemGroup>
  <PropertyGroup><TargetFrameworks>net8.0;net9.0</TargetFrameworks></PropertyGroup>
  <ItemGroup Condition="'$(TargetFramework)' == 'net9.0'">
    <PackageReference Include="Optional.Lib"><Version>[3.1.0]</Version></PackageReference>
    <PackageReference Update="Metadata.Only" Version="2.0.0" />
  </ItemGroup>
</Project>"#,
        )
        .unwrap();
        std::fs::write(root.path().join("Directory.Packages.props"), r#"
<Project>
  <PropertyGroup><ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally></PropertyGroup>
  <ItemGroup>
    <PackageVersion Include="Central.Lib" Version="2.3.4" />
    <PackageVersion Include="Unused.Lib" Version="1.0.0" />
  </ItemGroup>
</Project>"#).unwrap();
        let inputs = ScanInputs::capture(
            root.path(),
            &input_list(&["tool/App.csproj", "Directory.Packages.props"]),
        )
        .unwrap();
        let resolution = NugetCoverage::default().resolve(&inputs, true).unwrap();
        assert!(resolution.artifacts.is_empty());
        assert_eq!(resolution.warnings.len(), 1, "{:?}", resolution.warnings);
        let project = &resolution.projects[0];
        assert_eq!(project.frameworks, ["net8.0", "net9.0"]);
        assert_eq!(project.path, "tool/App.csproj");
        assert!(project.resolution_path.is_none());
        assert_eq!(
            project.declarations,
            vec![
                NugetDeclaration {
                    name: "Central.Lib".into(),
                    requirement: Some("2.3.4".into()),
                    requirement_path: Some("Directory.Packages.props".into()),
                    conditional: false,
                },
                NugetDeclaration {
                    name: "Optional.Lib".into(),
                    requirement: Some("[3.1.0]".into()),
                    requirement_path: Some("tool/App.csproj".into()),
                    conditional: true,
                },
            ]
        );
    }

    #[test]
    fn catalogs_named_lockfiles_and_legacy_pins_for_the_projects_they_belong_to() {
        let root = tempfile::tempdir().unwrap();
        let inputs = [
            "App.csproj",
            "Helper.fsproj",
            "packages.App.lock.json",
            "legacy/Old.vbproj",
            "legacy/packages.config",
        ];
        std::fs::create_dir(root.path().join("legacy")).unwrap();
        std::fs::write(root.path().join(inputs[0]), r#"<Project><ItemGroup><PackageReference Include="Library" Version="2.0" /></ItemGroup></Project>"#).unwrap();
        for path in [inputs[1], inputs[3]] {
            std::fs::write(root.path().join(path), "<Project/>").unwrap();
        }
        std::fs::write(
            root.path().join(inputs[2]),
            r#"{"dependencies":{"net8.0":{"Library":{"type":"Direct","resolved":"2.1.0"}}}}"#,
        )
        .unwrap();
        std::fs::write(root.path().join(inputs[4]), r#"<?xml version="1.0"?><packages><package id="Legacy.Lib" version="1.2.3" /></packages>"#).unwrap();
        let capture = || ScanInputs::capture(root.path(), &input_list(&inputs)).unwrap();
        let coverage = NugetCoverage::default();
        let resolution = coverage.resolve(&capture(), true).unwrap();
        assert_eq!(
            resolution.artifacts[0]["purl"],
            "pkg:nuget/Legacy.Lib@1.2.3"
        );
        assert_eq!(resolution.artifacts[1]["purl"], "pkg:nuget/Library@2.1.0");
        assert_eq!(
            resolution.projects[0].resolution_path.as_deref(),
            Some(inputs[2])
        );
        assert!(resolution.projects[1].resolution_path.is_none());
        assert_eq!(
            resolution.projects[2].resolution_path.as_deref(),
            Some(inputs[4])
        );
        assert_eq!(resolution.warnings.len(), 1, "{:?}", resolution.warnings);

        std::fs::write(root.path().join(inputs[0]), r#"<Project><ItemGroup><PackageReference Include="Library" Version="[3.0]" /></ItemGroup></Project>"#).unwrap();
        let stale = coverage.resolve(&capture(), true).unwrap();
        assert!(stale.projects[0].resolution_path.is_none());
        assert_eq!(stale.artifacts.len(), 1);
        assert_eq!(stale.artifacts[0]["purl"], "pkg:nuget/Legacy.Lib@1.2.3");
        assert!(stale
            .warnings
            .iter()
            .any(|warning| warning.contains("does not cover Library")));
    }
}
