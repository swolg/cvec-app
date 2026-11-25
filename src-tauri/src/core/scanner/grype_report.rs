use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use super::npm_coverage::resolution_root;
use super::{
    invalid_report, non_empty, package_identity, package_type_from_purl, syft_locations,
    MAX_RELATIVE_PATH_BYTES,
};
use crate::core::delta::build_fingerprint;
use crate::core::models::{
    BundledToolEvidence, FindingMatchEvidence, FindingMatchKind, NormalizedFinding, RepoPackage,
    Severity,
};
use crate::Error;

const GO_MODULE_FILE_CATALOGER: &str = "go-module-file-cataloger";
const GO_MODULE_BINARY_CATALOGER: &str = "go-module-binary-cataloger";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrypeReport {
    matches: Vec<GrypeMatch>,
    ignored_matches: Option<Vec<GrypeMatch>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrypeMatch {
    vulnerability: GrypeVulnerability,
    artifact: Artifact,
    #[serde(default)]
    match_details: Vec<MatchDetail>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrypeVulnerability {
    id: String,
    severity: String,
    description: Option<String>,
    data_source: Option<String>,
    fix: Option<GrypeFix>,
}

#[derive(Deserialize)]
struct GrypeFix {
    versions: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct MatchDetail {
    #[serde(rename = "type")]
    kind: String,
    found: Option<Found>,
    fix: Option<SuggestedFix>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Found {
    #[serde(default)]
    matched_symbols: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuggestedFix {
    suggested_version: Option<String>,
}

#[derive(Deserialize)]
struct Artifact {
    id: Option<String>,
    name: String,
    version: Option<String>,
    #[serde(rename = "type")]
    package_type: Option<String>,
    purl: Option<String>,
}

impl Artifact {
    fn is_go_module(&self) -> bool {
        self.package_type.as_deref() == Some("go-module")
            || self
                .purl
                .as_deref()
                .and_then(package_type_from_purl)
                .as_deref()
                == Some("golang")
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativePackage {
    #[serde(flatten)]
    artifact: Artifact,
    #[serde(default)]
    found_by: String,
    #[serde(default)]
    locations: Vec<Location>,
}

#[derive(Deserialize)]
struct Location {
    path: String,
}

#[derive(Deserialize)]
struct NativeSbom {
    artifacts: Vec<NativePackage>,
}

pub(super) struct SourceInventory {
    packages: BTreeMap<String, NativePackage>,
    javascript_locks: BTreeMap<PathBuf, BTreeSet<String>>,
}

pub(super) enum Verification<'a> {
    Native(&'a SourceInventory),
    Artifact(&'a SourceInventory),
    CycloneDx {
        packages: &'a [RepoPackage],
        source: &'a SourceInventory,
    },
}

impl SourceInventory {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let sbom: NativeSbom = serde_json::from_slice(bytes)
            .map_err(|error| invalid_report("native Syft inventory", &error))?;
        Self::from_packages(sbom.artifacts)
    }

    pub(super) fn parse_cyclonedx(bytes: &[u8]) -> Result<Self, Error> {
        let sbom: Value = serde_json::from_slice(bytes)
            .map_err(|error| invalid_report("Syft CycloneDX", &error))?;
        let components = sbom.get("components").and_then(Value::as_array);
        Self::from_packages(
            components
                .into_iter()
                .flatten()
                .filter_map(NativePackage::cyclonedx_source),
        )
    }

    fn from_packages(artifacts: impl IntoIterator<Item = NativePackage>) -> Result<Self, Error> {
        let mut packages = BTreeMap::new();
        let mut javascript_locks: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
        for package in artifacts {
            let id = package.artifact.id.as_ref().filter(|id| !id.is_empty());
            let id = id
                .ok_or_else(|| Error::Scanner("Syft package is missing its artifact ID.".into()))?;
            if package.found_by == "javascript-lock-cataloger" {
                for root in package.relative_paths().filter_map(resolution_root) {
                    javascript_locks
                        .entry(root)
                        .or_default()
                        .insert(package.identity());
                }
            }
            if packages.insert(id.clone(), package).is_some() {
                return Err(Error::Scanner(
                    "Syft returned a duplicate package artifact ID.".into(),
                ));
            }
        }
        Ok(Self {
            packages,
            javascript_locks,
        })
    }

    fn find(&self, artifact: &Artifact) -> Option<&NativePackage> {
        artifact
            .id
            .as_ref()
            .and_then(|id| self.packages.get(id))
            .filter(|package| {
                let source = &package.artifact;
                source.name == artifact.name
                    && present(&source.version) == present(&artifact.version)
                    && present(&source.package_type) == present(&artifact.package_type)
                    && present(&source.purl) == present(&artifact.purl)
            })
    }

    fn is_local_javascript_installation(&self, package: &NativePackage) -> bool {
        if package.artifact.package_type.as_deref() != Some("npm")
            || package.found_by == "javascript-lock-cataloger"
        {
            return false;
        }
        let mut paths = package.relative_paths().peekable();
        if paths.peek().is_none()
            || !package
                .relative_paths()
                .all(|path| path.split('/').any(|part| part == "node_modules"))
        {
            return false;
        }
        let identity = package.identity();
        !paths.filter_map(resolution_root).any(|root| {
            root.ancestors()
                .find_map(|ancestor| self.javascript_locks.get(ancestor))
                .is_some_and(|packages| packages.contains(&identity))
        })
    }

    pub(super) fn include_packages(&self, packages: &mut Vec<RepoPackage>) {
        let mut identities = packages
            .iter()
            .map(|package| {
                package_identity(
                    package.package_url.as_deref(),
                    &package.name,
                    package.version.as_deref(),
                    package.package_type.as_deref(),
                )
            })
            .collect::<BTreeSet<_>>();
        for source in self.packages.values() {
            let artifact = &source.artifact;
            let purl = present(&artifact.purl);
            let version = present(&artifact.version);
            let kind = purl.and_then(package_type_from_purl);
            let kind = kind.as_deref().or(present(&artifact.package_type));
            if identities.insert(package_identity(purl, &artifact.name, version, kind)) {
                packages.push(RepoPackage {
                    name: artifact.name.clone(),
                    version: version.map(str::to_owned),
                    package_type: kind.map(str::to_owned),
                    package_url: purl.map(str::to_owned),
                    highest_severity: None,
                    finding_count: 0,
                    fix_available: false,
                });
            }
        }
    }
}

impl NativePackage {
    fn cyclonedx_source(component: &Value) -> Option<Self> {
        let properties = component.get("properties")?.as_array()?;
        let property = |name| {
            properties
                .iter()
                .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))?
                .get("value")?
                .as_str()
        };
        let field = |key| {
            component
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        };
        Some(Self {
            artifact: Artifact {
                id: Some(field("bom-ref")?.to_owned()),
                name: field("name")?.to_owned(),
                version: field("version").map(str::to_owned),
                package_type: Some(property("syft:package:type")?.to_owned()),
                purl: field("purl").map(str::to_owned),
            },
            found_by: property("syft:package:foundBy")?.to_owned(),
            locations: syft_locations(component)
                .filter_map(|entry| entry.get("value")?.as_str())
                .map(|path| Location {
                    path: path.to_owned(),
                })
                .collect(),
        })
    }

    fn identity(&self) -> String {
        package_identity(
            present(&self.artifact.purl),
            &self.artifact.name,
            present(&self.artifact.version),
            present(&self.artifact.package_type),
        )
    }

    fn relative_paths(&self) -> impl Iterator<Item = &str> {
        self.locations
            .iter()
            .filter_map(|location| relative_path(&location.path))
    }

    fn has_repository_go_manifest(&self) -> bool {
        self.found_by == GO_MODULE_FILE_CATALOGER
            && self.relative_paths().any(|path| {
                matches!(path.rsplit('/').next(), Some("go.mod" | "go.sum"))
                    && !path.split('/').any(|part| part == "node_modules")
            })
    }
}

fn relative_path(path: &str) -> Option<&str> {
    let path = path.trim_start_matches('/');
    (!path.is_empty()
        && path.len() <= MAX_RELATIVE_PATH_BYTES
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, "" | "." | "..")))
    .then_some(path)
}

fn present(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn unmatched_artifact() -> Error {
    Error::Scanner("Grype returned a package identity or version that is absent from the Syft input. Previous scan results were retained.".into())
}

fn verify_cyclonedx_artifact(artifact: &Artifact, packages: &[RepoPackage]) -> Result<(), Error> {
    if packages.iter().any(|package| {
        package.name == artifact.name
            && present(&package.version) == present(&artifact.version)
            && match (present(&package.package_url), present(&artifact.purl)) {
                (Some(source), Some(matched)) => source == matched,
                (None, None) => present(&package.package_type) == present(&artifact.package_type),
                _ => false,
            }
    }) {
        return Ok(());
    }
    Err(unmatched_artifact())
}

impl GrypeMatch {
    fn into_finding(
        self,
        verification: Option<&Verification<'_>>,
    ) -> Result<NormalizedFinding, Error> {
        let Self {
            vulnerability,
            artifact,
            match_details,
        } = self;
        let advisory_id = non_empty(vulnerability.id).ok_or_else(|| {
            Error::Scanner("Grype match is missing its advisory identity.".into())
        })?;
        if artifact.name.trim().is_empty() {
            return Err(unmatched_artifact());
        }
        let (package, local_installation) = match verification {
            Some(Verification::Native(source)) => {
                let package = source.find(&artifact).ok_or_else(unmatched_artifact)?;
                (
                    Some(package),
                    source.is_local_javascript_installation(package),
                )
            }
            Some(Verification::Artifact(source)) => (
                Some(source.find(&artifact).ok_or_else(unmatched_artifact)?),
                false,
            ),
            Some(Verification::CycloneDx { packages, source }) => {
                verify_cyclonedx_artifact(&artifact, packages)?;
                let package = source.find(&artifact);
                (
                    package,
                    package.is_some_and(|package| source.is_local_javascript_installation(package)),
                )
            }
            None => (None, false),
        };
        let uses_cyclonedx = matches!(verification, Some(Verification::CycloneDx { .. }));
        let go_origin_unverified = uses_cyclonedx
            && artifact.is_go_module()
            && !package.is_some_and(NativePackage::has_repository_go_manifest);
        let severity = Severity::parse(&vulnerability.severity)
            .ok_or_else(|| Error::Scanner("Grype match has an unrecognized severity".into()))?;
        let mut evidence = match_evidence(
            &artifact,
            &match_details,
            package,
            local_installation,
            go_origin_unverified,
        );
        if uses_cyclonedx && evidence.relative_path.is_none() && !evidence.requires_review() {
            evidence.kind = FindingMatchKind::Unverified;
        }
        let mut fix_version = reported_fix(vulnerability.fix, &match_details);
        if evidence.bundled_tool.is_some() {
            evidence.normalize_bundled_go_fix(&mut fix_version);
        } else if verification.is_some() && evidence.requires_review() {
            fix_version = None;
        }
        let version = artifact.version.and_then(non_empty);
        let package_type = artifact.package_type.and_then(non_empty);
        let purl = artifact.purl.and_then(non_empty);
        Ok(NormalizedFinding {
            fingerprint: build_fingerprint(
                &advisory_id,
                package_type.as_deref(),
                purl.as_deref(),
                &artifact.name,
                version.as_deref(),
            ),
            advisory_id,
            package_name: artifact.name,
            package_version: version,
            package_type,
            severity,
            fix_version,
            title: vulnerability.description,
            source_url: vulnerability.data_source,
            package_url: purl,
            advisory_details: None,
            match_evidence: Some(evidence),
        })
    }
}

fn match_evidence(
    artifact: &Artifact,
    details: &[MatchDetail],
    source: Option<&NativePackage>,
    local_installation: bool,
    go_origin_unverified: bool,
) -> FindingMatchEvidence {
    use FindingMatchKind::*;
    let mut path = source.and_then(|source| source.relative_paths().next());
    let mut bundled_tool = None;
    if let Some(source) = source.filter(|_| artifact.is_go_module()) {
        if let Some(standalone) = source
            .relative_paths()
            .find(|path| BundledToolEvidence::from_node_modules_path(path).is_none())
        {
            path = Some(standalone);
        } else {
            bundled_tool = path.and_then(BundledToolEvidence::from_node_modules_path);
        }
    }
    let nested_workflow = source.is_some_and(|source| {
        artifact.package_type.as_deref() == Some("github-action")
            && path.is_some()
            && source
                .relative_paths()
                .all(|path| !path.starts_with(".github/"))
    });
    let go_version_only = go_origin_unverified
        || source.is_some_and(|package| package.found_by == GO_MODULE_BINARY_CATALOGER);
    let kind = if nested_workflow {
        NestedWorkflow
    } else if local_installation {
        LocalInstallation
    } else if present(&artifact.version).is_none() {
        Unverified
    } else if !go_origin_unverified
        && details.iter().any(|detail| {
            detail
                .found
                .as_ref()
                .is_some_and(|found| !found.matched_symbols.is_empty())
        })
    {
        GoSymbol
    } else if go_version_only
        && details.iter().any(|detail| {
            matches!(
                detail.kind.as_str(),
                "exact-direct-match" | "exact-indirect-match"
            )
        })
    {
        GoModule
    } else if details
        .iter()
        .any(|detail| detail.kind == "exact-direct-match")
    {
        PackageVersion
    } else if details
        .iter()
        .any(|detail| detail.kind == "exact-indirect-match")
    {
        Upstream
    } else if details.iter().any(|detail| detail.kind == "cpe-match") {
        Cpe
    } else {
        Unverified
    };
    FindingMatchEvidence {
        kind,
        relative_path: path.map(str::to_owned),
        bundled_tool,
    }
}

fn reported_fix(fix: Option<GrypeFix>, details: &[MatchDetail]) -> Option<String> {
    let suggested = details
        .iter()
        .filter_map(|detail| {
            detail
                .fix
                .as_ref()
                .and_then(|fix| present(&fix.suggested_version))
        })
        .collect::<BTreeSet<_>>();
    if suggested.len() == 1 {
        return suggested.first().map(|version| (*version).to_owned());
    }
    if !suggested.is_empty() {
        return None;
    }
    let versions = fix?
        .versions?
        .into_iter()
        .filter_map(non_empty)
        .collect::<BTreeSet<_>>();
    (versions.len() == 1)
        .then(|| versions.into_iter().next())
        .flatten()
}

pub(super) fn parse(
    bytes: &[u8],
    verification: Option<Verification<'_>>,
) -> Result<Vec<NormalizedFinding>, Error> {
    let report: GrypeReport =
        serde_json::from_slice(bytes).map_err(|error| invalid_report("Grype", &error))?;
    let mut findings: BTreeMap<String, NormalizedFinding> = BTreeMap::new();
    for entry in report.matches {
        let finding = entry.into_finding(verification.as_ref())?;
        match findings.entry(finding.fingerprint.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(finding);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let rank = |item: &NormalizedFinding| {
                    (
                        item.match_evidence
                            .as_ref()
                            .is_some_and(|evidence| !evidence.requires_review()),
                        item.match_evidence.as_ref().map(|evidence| evidence.kind),
                        item.severity,
                    )
                };
                match rank(&finding).cmp(&rank(entry.get())) {
                    std::cmp::Ordering::Greater => {
                        entry.insert(finding);
                    }
                    std::cmp::Ordering::Equal => {
                        let merged = entry.get_mut();
                        if merged.fix_version != finding.fix_version {
                            merged.fix_version = None;
                        }
                        let tool = merged
                            .match_evidence
                            .as_mut()
                            .and_then(|e| e.bundled_tool.as_mut());
                        let incoming = finding
                            .match_evidence
                            .as_ref()
                            .and_then(|e| e.bundled_tool.as_ref());
                        if let (Some(tool), Some(incoming)) = (tool, incoming) {
                            if tool.component_fix_version != incoming.component_fix_version {
                                tool.component_fix_version = None;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for entry in report.ignored_matches.into_iter().flatten() {
        let finding = entry.into_finding(verification.as_ref())?;
        if !findings.get(&finding.fingerprint).is_some_and(|included| {
            included.severity == finding.severity
                && included.match_evidence == finding.match_evidence
        }) {
            return Err(Error::Scanner(
                "Grype filters suppressed findings tracked by CVEC. Review Grype's ignore, fix-state, or VEX settings and retry the scan.".into(),
            ));
        }
    }
    Ok(findings.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn package() -> serde_json::Value {
        json!({
            "id": "fixture-id",
            "name": "example",
            "version": "1.0.0",
            "type": "npm",
            "purl": "pkg:npm/example@1.0.0",
            "locations": [{ "path": "/package-lock.json" }]
        })
    }

    fn source() -> SourceInventory {
        SourceInventory::parse(&serde_json::to_vec(&json!({ "artifacts": [package()] })).unwrap())
            .unwrap()
    }

    fn report(artifact: serde_json::Value, severity: &str, kind: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "matches": [{
                "vulnerability": {
                    "id": "CVE-FIXTURE",
                    "severity": severity,
                    "fix": { "versions": ["1.0.1", "2.0.1"] }
                },
                "artifact": artifact,
                "matchDetails": [{
                    "type": kind,
                    "fix": { "suggestedVersion": "1.0.1" }
                }]
            }]
        }))
        .unwrap()
    }

    #[test]
    fn persists_all_grype_severities_and_uses_the_fix_for_the_matched_branch() {
        let source = source();
        for severity in Severity::ALL {
            let result = parse(
                &report(package(), severity.as_str(), "exact-direct-match"),
                Some(Verification::Native(&source)),
            )
            .unwrap();
            assert_eq!(result[0].severity, severity);
            assert_eq!(result[0].fix_version.as_deref(), Some("1.0.1"));
            assert_eq!(
                result[0]
                    .match_evidence
                    .as_ref()
                    .unwrap()
                    .relative_path
                    .as_deref(),
                Some("package-lock.json")
            );
        }
    }

    #[test]
    fn rejects_existing_ids_with_conflicting_versions_or_ecosystems() {
        let source = source();
        for (field, replacement) in [
            ("id", "foreign-artifact"),
            ("version", "0.0.9"),
            ("type", "python"),
            ("purl", "pkg:npm/%40other/example@1.0.0"),
        ] {
            let mut artifact = package();
            artifact[field] = json!(replacement);
            let error = parse(
                &report(artifact, "High", "exact-direct-match"),
                Some(Verification::Native(&source)),
            )
            .unwrap_err();
            assert!(error.to_string().contains("absent from the Syft input"));
        }
    }

    #[test]
    fn keeps_a_cpe_observation_without_treating_it_as_a_supported_fix() {
        let source = source();
        let result = parse(
            &report(package(), "High", "cpe-match"),
            Some(Verification::Native(&source)),
        )
        .unwrap();
        let evidence = result[0].match_evidence.as_ref().unwrap();
        assert_eq!(evidence.kind, FindingMatchKind::Cpe);
        assert!(evidence.kind.requires_review());
        assert!(result[0].fix_version.is_none());
    }

    #[test]
    fn go_symbols_from_an_npm_tool_keep_the_component_fix_and_source() {
        let tool_path = "node_modules/.bun/cli@1.0.0/node_modules/@example/cli/lib/tool";
        let binary = json!({
            "id": "binary-module",
            "name": "example.com/module",
            "version": "1.0.0",
            "type": "go-module",
            "purl": "pkg:golang/example.com/module@1.0.0",
            "foundBy": "go-module-binary-cataloger",
            "locations": [{ "path": format!("/{tool_path}") }]
        });
        let source =
            SourceInventory::parse(&serde_json::to_vec(&json!({ "artifacts": [binary] })).unwrap())
                .unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&report(binary.clone(), "High", "exact-direct-match")).unwrap();
        value["matches"][0]["matchDetails"][0]["found"] =
            json!({ "matchedSymbols": ["example.com/module.Fixture"] });
        let grype = serde_json::to_vec(&value).unwrap();
        for verification in [
            Verification::Native(&source),
            Verification::Artifact(&source),
        ] {
            let findings = parse(&grype, Some(verification)).unwrap();
            let finding = &findings[0];
            let evidence = finding.match_evidence.as_ref().unwrap();
            assert_eq!(finding.severity, Severity::High);
            assert_eq!(finding.fix_version, None);
            assert_eq!(evidence.kind, FindingMatchKind::GoSymbol);
            assert_eq!(evidence.relative_path.as_deref(), Some(tool_path));
            assert!(evidence.requires_review());
            let tool = evidence.bundled_tool.as_ref().unwrap();
            assert_eq!(tool.package_name.as_deref(), Some("@example/cli"));
            assert_eq!(tool.component_fix_version.as_deref(), Some("1.0.1"));
        }

        let mut module = binary;
        module["id"] = json!("manifest-module");
        module["foundBy"] = json!("go-module-file-cataloger");
        module["locations"] = json!([{ "path": "/go.mod" }]);
        let source = SourceInventory::parse(
            &serde_json::to_vec(&json!({ "artifacts": [value["matches"][0]["artifact"], module] }))
                .unwrap(),
        )
        .unwrap();
        let mut direct: serde_json::Value =
            serde_json::from_slice(&report(module, "High", "exact-direct-match")).unwrap();
        let direct = direct["matches"].as_array_mut().unwrap().remove(0);
        let bundled = value["matches"].as_array_mut().unwrap().remove(0);
        for matches in [[bundled.clone(), direct.clone()], [direct, bundled]] {
            let report = serde_json::to_vec(&json!({ "matches": matches })).unwrap();
            let findings = parse(&report, Some(Verification::Native(&source))).unwrap();
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].fix_version.as_deref(), Some("1.0.1"));
            let evidence = findings[0].match_evidence.as_ref().unwrap();
            assert_eq!(evidence.relative_path.as_deref(), Some("go.mod"));
            assert!(evidence.bundled_tool.is_none());
        }
    }

    #[test]
    fn same_go_component_needs_manifest_evidence_for_a_cyclonedx_fix() {
        let purl = "pkg:golang/example.com/module@1.0.0";
        let path = "node_modules/@example/cli/lib/tool";
        let artifact = |id| {
            json!({
                "id": id, "name": "example.com/module", "version": "1.0.0", "type": "go-module", "purl": purl
            })
        };
        let mut native_artifact = artifact("binary");
        native_artifact["foundBy"] = json!(GO_MODULE_BINARY_CATALOGER);
        native_artifact["locations"] = json!([{ "path": format!("/{path}") }]);
        let mut native_manifest = artifact("manifest");
        native_manifest["foundBy"] = json!(GO_MODULE_FILE_CATALOGER);
        native_manifest["locations"] = json!([{ "path": "/go.mod" }]);
        let native = SourceInventory::parse(
            &serde_json::to_vec(&json!({ "artifacts": [native_artifact, native_manifest] }))
                .unwrap(),
        )
        .unwrap();
        let mut native_report: Value =
            serde_json::from_slice(&report(artifact("binary"), "High", "exact-direct-match"))
                .unwrap();
        native_report["matches"][0]["matchDetails"][0]["found"] =
            json!({ "matchedSymbols": ["example.com/module.Fixture"] });
        let native_findings = parse(
            &serde_json::to_vec(&native_report).unwrap(),
            Some(Verification::Native(&native)),
        )
        .unwrap();
        assert_eq!(
            native_findings[0].match_evidence.as_ref().unwrap().kind,
            FindingMatchKind::GoSymbol
        );

        let component = |id, cataloger, path: &str| {
            json!({
                "bom-ref": id, "type": "library", "name": "example.com/module", "version": "1.0.0", "purl": purl,
                "properties": [
                    { "name": "syft:package:foundBy", "value": cataloger },
                    { "name": "syft:package:type", "value": "go-module" },
                    { "name": "syft:location:0:path", "value": format!("/{path}") }
                ]
            })
        };
        let cyclonedx = serde_json::to_vec(&json!({
            "bomFormat": "CycloneDX", "specVersion": "1.6", "components": [
                component("cdx-binary", GO_MODULE_BINARY_CATALOGER, path),
                component("cdx-manifest", GO_MODULE_FILE_CATALOGER, "go.mod"),
            ]
        }))
        .unwrap();
        let source = SourceInventory::parse_cyclonedx(&cyclonedx).unwrap();
        let (packages, ..) = super::super::parse_cyclonedx_inventory(&cyclonedx).unwrap();
        for (id, kind, location, fix) in [
            ("cdx-binary", FindingMatchKind::GoModule, Some(path), None),
            (
                "cdx-manifest",
                FindingMatchKind::PackageVersion,
                Some("go.mod"),
                Some("1.0.1"),
            ),
            ("unidentified", FindingMatchKind::GoModule, None, None),
        ] {
            let findings = parse(
                &report(artifact(id), "High", "exact-direct-match"),
                Some(Verification::CycloneDx {
                    packages: &packages,
                    source: &source,
                }),
            )
            .unwrap();
            let observation = &findings[0];
            let evidence = observation.match_evidence.as_ref().unwrap();
            assert_eq!(observation.fingerprint, native_findings[0].fingerprint);
            assert_eq!(observation.severity, Severity::High);
            assert_eq!(evidence.kind, kind);
            assert_eq!(evidence.relative_path.as_deref(), location);
            assert_eq!(observation.fix_version.as_deref(), fix);
            if id == "cdx-binary" {
                let tool = evidence.bundled_tool.as_ref().unwrap();
                assert_eq!(tool.package_name.as_deref(), Some("@example/cli"));
                assert_eq!(tool.component_fix_version.as_deref(), Some("1.0.1"));
            }
        }
    }

    #[test]
    fn a_stale_installation_does_not_inherit_lock_evidence_from_another_project_or_version() {
        let mut installed = package();
        installed["foundBy"] = json!("javascript-package-cataloger");
        installed["locations"] = json!([{ "path": "/app/node_modules/example/package.json" }]);
        for (lock_path, version, expected) in [
            (
                "/other/package-lock.json",
                "1.0.0",
                FindingMatchKind::LocalInstallation,
            ),
            (
                "/app/package-lock.json",
                "1.0.1",
                FindingMatchKind::LocalInstallation,
            ),
            (
                "/app/package-lock.json",
                "1.0.0",
                FindingMatchKind::PackageVersion,
            ),
        ] {
            let mut locked = package();
            locked["id"] = json!("lock-artifact");
            locked["foundBy"] = json!("javascript-lock-cataloger");
            locked["version"] = json!(version);
            locked["purl"] = json!(format!("pkg:npm/example@{version}"));
            locked["locations"] = json!([{ "path": lock_path }]);
            let sbom = json!({ "artifacts": [installed, locked] });
            let source = SourceInventory::parse(&serde_json::to_vec(&sbom).unwrap()).unwrap();
            let components = sbom["artifacts"].as_array().unwrap().iter().map(|artifact| json!({
                "bom-ref": artifact["id"], "type": "library", "name": artifact["name"],
                "purl": artifact["purl"], "version": artifact["version"], "properties": [
                    {"name": "syft:package:type", "value": "npm"},
                    {"name": "syft:package:foundBy", "value": artifact["foundBy"]},
                    {"name": "syft:location:0:path", "value": artifact["locations"][0]["path"]},
                ]
            })).collect::<Vec<_>>();
            let cyclonedx = serde_json::to_vec(&json!({ "bomFormat": "CycloneDX", "specVersion": "1.6", "components": components })).unwrap();
            let cdx_source = SourceInventory::parse_cyclonedx(&cyclonedx).unwrap();
            let (packages, ..) = super::super::parse_cyclonedx_inventory(&cyclonedx).unwrap();
            for verification in [
                Verification::Native(&source),
                Verification::CycloneDx {
                    packages: &packages,
                    source: &cdx_source,
                },
            ] {
                let result = parse(
                    &report(installed.clone(), "High", "exact-direct-match"),
                    Some(verification),
                )
                .unwrap();
                assert_eq!(result[0].match_evidence.as_ref().unwrap().kind, expected);
                assert_eq!(result[0].fix_version.is_none(), expected.requires_review());
            }
        }
    }
}
