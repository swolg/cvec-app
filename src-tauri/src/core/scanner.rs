use crate::core::timestamps;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command as AsyncCommand;

use crate::Error;

use super::delta::build_fingerprint;
use super::models::{
    GrypeDbStatus, NormalizedFinding, NormalizedRemediationEvidence, RemediationEvidenceQuality,
    RemediationEvidenceSource, RemediationLocationKind, RemediationRelationship, RepoPackage,
    ScanOutput, ScannerAvailability, ScannerVersions, Severity, SourceSnapshot,
    REMEDIATION_SCHEMA_VERSION,
};
use super::secure_fs::read_regular_text;
#[cfg(unix)]
use super::secure_fs::AnchoredRoot;

mod inputs;
mod npm_coverage;

const TRUSTED_TOOL_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];
const SCAN_COMMAND_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_COMPANION_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ARTIFACTS: usize = 100_000;
const MAX_RELATIONSHIPS: usize = 200_000;
const MAX_LOCATIONS_PER_ARTIFACT: usize = 32;
const MAX_RELATIVE_PATH_BYTES: usize = 1_024;
const MAX_SUPPORTED_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_SUPPORTED_MANIFEST_DECLARATIONS: usize = 10_000;
#[cfg(debug_assertions)]
const QA_TRUSTED_TOOL_DIRS_ENV: &str = "CVEC_SCANNER_TOOL_DIRS";

pub fn availability() -> ScannerAvailability {
    let syft = resolve_tool("syft");
    let grype = resolve_tool("grype");

    match (syft, grype) {
        (Some(_), Some(_)) => match versions(){
            Ok(versions)=>ScannerAvailability{checked_at:Some(timestamps::now()),checking:false,ready:true,message:None,versions:Some(versions),grype_db:None},
            Err(cause)=>ScannerAvailability{checked_at:Some(timestamps::now()),checking:false,ready:false,message:Some(format!("Local scanner check failed: {cause}")),versions:None,grype_db:None},
        },
        _ => ScannerAvailability {
            checked_at: Some(timestamps::now()),
checking: false,
            ready: false,
            message: Some(
                "Install syft and grype with Homebrew, then restart cvec. Example: brew install syft grype"
                    .to_string(),
            ),
            versions: None,
            grype_db: None,
        },
    }
}

pub fn versions() -> Result<ScannerVersions, Error> {
    Ok(ScannerVersions {
        syft: version_for("syft")?,
        grype: version_for("grype")?,
    })
}

pub(crate) async fn versions_cancellable(
    cancelled: Arc<AtomicBool>,
) -> Result<ScannerVersions, Error> {
    async fn version(tool: &str, cancelled: Arc<AtomicBool>) -> Result<String, Error> {
        let mut command = async_command_for(tool, None)?;
        command.arg("--version");
        let output = super::process::output_cancellable(
            command,
            Duration::from_secs(3),
            64 * 1024,
            cancelled,
        )
        .await?;
        checked_version(tool, output)
    }
    Ok(ScannerVersions {
        syft: version("syft", cancelled.clone()).await?,
        grype: version("grype", cancelled).await?,
    })
}

pub async fn scan_repository(
    path: &Path,
    grype_db_cache_dir: Option<&Path>,
) -> Result<ScanOutput, Error> {
    let output = scan_repository_with_progress(path, grype_db_cache_dir, |_| {}).await?;
    if let Some(evidence) = &output.evidence {
        evidence.verify_source(path).await?;
    }
    Ok(output)
}

pub(crate) struct LocalScanSource {
    pub snapshot: SourceSnapshot,
    status_digest: Option<String>,
}

impl LocalScanSource {
    pub(crate) async fn verify(&self, path: &Path) -> Result<(), Error> {
        let current = capture_local_scan_source(path).await;
        let revision_changed = (self.snapshot.revision.is_some() || self.status_digest.is_some())
            && (current.snapshot.revision.is_some() || current.status_digest.is_some())
            && self.snapshot.revision != current.snapshot.revision;
        let dirty_changed = matches!(
            (self.snapshot.dirty, current.snapshot.dirty),
            (Some(before), Some(after)) if before != after
        );
        let status_changed = matches!(
            (&self.status_digest, &current.status_digest),
            (Some(before), Some(after)) if before != after
        );
        if revision_changed || dirty_changed || status_changed {
            return Err(Error::Scanner(
                "The Git checkout changed during the scan. Run the scan again.".into(),
            ));
        }
        if (self.snapshot.revision.is_some() && current.snapshot.revision.is_none())
            || (self.snapshot.dirty.is_some() && current.snapshot.dirty.is_none())
            || (self.status_digest.is_some() && current.status_digest.is_none())
        {
            return Err(Error::Scanner(
                "The Git checkout could not be checked after the scan. Run the scan again.".into(),
            ));
        }
        Ok(())
    }
}

pub async fn capture_local_source_snapshot(path: &Path) -> SourceSnapshot {
    capture_local_scan_source(path).await.snapshot
}

pub(crate) async fn capture_local_scan_source(path: &Path) -> LocalScanSource {
    // Git can walk upwards from a missing or invalid .git. Only read source
    // metadata when Git identifies this directory as the worktree root.
    let root = git_output(
        path,
        &["rev-parse", "--is-inside-work-tree", "--show-prefix"],
        4096,
    )
    .await;
    if !matches!(root.as_deref(), Some(b"true\n\n" | b"true\r\n\r\n")) {
        return LocalScanSource {
            snapshot: SourceSnapshot::default(),
            status_digest: None,
        };
    }

    // One porcelain status call samples HEAD and the index/worktree status.
    // Keep filenames and index object IDs out of the persisted source snapshot.
    let status = git_output(
        path,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--no-ahead-behind",
            "--no-renames",
            "--untracked-files=normal",
            "--ignore-submodules=dirty",
        ],
        512 * 1024,
    )
    .await;
    if let Some(source) = status.as_deref().and_then(parse_local_scan_source) {
        return source;
    }

    // Retain the earlier metadata behavior when porcelain v2 is unavailable
    // or the status exceeds the output limit.
    let revision = git_output(path, &["rev-parse", "--verify", "HEAD"], 128)
        .await
        .and_then(|bytes| parse_git_revision(&bytes));
    let dirty = git_output(
        path,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=normal",
            "--ignore-submodules=dirty",
        ],
        512 * 1024,
    )
    .await
    .map(|bytes| !bytes.is_empty());
    LocalScanSource {
        snapshot: SourceSnapshot { revision, dirty },
        status_digest: None,
    }
}

fn parse_local_scan_source(status: &[u8]) -> Option<LocalScanSource> {
    let mut headers = status
        .split_inclusive(|byte| *byte == 0)
        .take_while(|entry| entry.starts_with(b"# "));
    let first = headers.next()?;
    let oid = first.strip_prefix(b"# branch.oid ")?.strip_suffix(b"\0")?;
    let revision = if oid == b"(initial)" {
        None
    } else {
        Some(parse_git_revision(oid)?)
    };
    let header_length = first.len() + headers.map(<[u8]>::len).sum::<usize>();
    let changes = &status[header_length..];
    Some(LocalScanSource {
        snapshot: SourceSnapshot {
            revision,
            dirty: Some(!changes.is_empty()),
        },
        status_digest: Some(super::hashing::sha256_hex(changes)),
    })
}

fn parse_git_revision(bytes: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(bytes).ok()?.trim();
    (matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

async fn git_output(path: &Path, args: &[&str], max_bytes: usize) -> Option<Vec<u8>> {
    let output = super::git::run(path, args, Duration::from_secs(5), max_bytes)
        .await
        .ok()?;
    output.status.success().then_some(output.stdout)
}

pub async fn scan_repository_with_progress<F>(
    path: &Path,
    grype_db_cache_dir: Option<&Path>,
    mut on_stage: F,
) -> Result<ScanOutput, Error>
where
    F: FnMut(super::models::ScanActivityStage),
{
    let trace = env::var_os("CVEC_PERF_TRACE").is_some();
    let inventory_started = trace.then(std::time::Instant::now);
    on_stage(super::models::ScanActivityStage::InventoryingPackages);
    let syft = resolve_tool("syft").ok_or_else(|| {
        Error::Scanner("syft is not installed or not reachable from the app".into())
    })?;
    let ScanInventory {
        cyclonedx,
        companion,
        packages,
        evidence,
    } = inventory_repository(path, &syft, grype_db_cache_dir).await?;
    if let Some(started) = inventory_started {
        eprintln!(
            "cvec-perf scanner cyclonedx_bytes={} evidence_bytes={} inventory_ms={:.3}",
            cyclonedx.len(),
            companion.as_ref().map(Vec::len).unwrap_or(0),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    let matching_started = trace.then(std::time::Instant::now);
    on_stage(super::models::ScanActivityStage::MatchingVulnerabilities);
    let mut grype_command = async_command_for("grype", grype_db_cache_dir)?;
    // Read the report over IPC pipes regardless of standalone CLI output
    // and severity-threshold settings. Keep built-in matcher exclusions.
    grype_command
        .args(["--file=", "--fail-on=", "-o", "json"])
        .env("GRYPE_INCLUDE_MATCHER_SUPPRESSIONS", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = super::process::output(
        grype_command,
        Some(&cyclonedx),
        SCAN_COMMAND_TIMEOUT,
        MAX_COMPANION_BYTES as usize,
    )
    .await?;
    if let Some(started) = matching_started {
        eprintln!(
            "cvec-perf scanner grype_bytes={} matching_ms={:.3}",
            output.stdout.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    if !output.status.success() {
        return Err(Error::Scanner(format!(
            "grype failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let path = path.to_path_buf();
    crate::runtime::run(move || -> Result<ScanOutput, Error> {
        let findings = parse_grype_json(&output.stdout)?;
        let mut remediation_evidence = companion
            .as_deref()
            .and_then(|bytes| parse_syft_remediation_evidence(bytes, &path).ok())
            .map(|artifacts| correlate_remediation_evidence(&findings, &artifacts))
            .unwrap_or_else(|| unavailable_remediation_evidence(&findings));
        if evidence.inputs.includes(Path::new("pyproject.toml")) {
            apply_supported_manifest_evidence(&path, &findings, &mut remediation_evidence);
        }

        Ok(ScanOutput {
            findings,
            packages,
            remediation_evidence,
            evidence: Some(evidence),
        })
    })
    .await
    .map_err(Error::Scanner)?
}

async fn active_syft_configuration(tool: &Path) -> Option<Vec<u8>> {
    syft_configuration(tool, true).await
}

async fn syft_configuration(tool: &Path, active: bool) -> Option<Vec<u8>> {
    let mut command = syft_command_for_path(tool);
    command.arg("config");
    if active {
        command.arg("--load");
    }
    let output = super::process::output(command, None, Duration::from_secs(3), 512 * 1024)
        .await
        .ok()?;
    output.status.success().then_some(output.stdout)
}

#[derive(Deserialize)]
struct SyftFileExclusions {
    #[serde(default)]
    exclude: Vec<String>,
}

async fn cache_fingerprint(
    path: &Path,
    tool: &Path,
    active: &[u8],
    defaults: &[u8],
) -> Option<String> {
    // Reuse only tested Syft versions with their default configuration.
    let mut version = syft_command_for_path(tool);
    version.arg("--version");
    let version = super::process::output(version, None, Duration::from_secs(3), 64 * 1024)
        .await
        .ok()?;
    if !version.status.success()
        || !matches!(
            String::from_utf8_lossy(&version.stdout).trim(),
            "syft 1.54.1" | "syft 1.57.1"
        )
    {
        return None;
    }
    if super::inventory_cache::default_configuration(defaults)
        != super::inventory_cache::default_configuration(active)
    {
        return None;
    }
    inventory_fingerprint(path, tool, active).await
}

async fn inventory_fingerprint(path: &Path, tool: &Path, config: &[u8]) -> Option<String> {
    let (path, tool, config) = (path.to_path_buf(), tool.to_path_buf(), config.to_vec());
    crate::runtime::run_io(move || super::inventory_cache::fingerprint(&path, &tool, &config))
        .await
        .ok()?
        .ok()
}

async fn cache_inputs_unchanged(path: &Path, tool: &Path, config: &[u8], key: &str) -> bool {
    let (current_config, current_key) = tokio::join!(
        active_syft_configuration(tool),
        inventory_fingerprint(path, tool, config),
    );
    current_config.as_deref() == Some(config) && current_key.as_deref() == Some(key)
}

struct ScanInventory {
    cyclonedx: Vec<u8>,
    companion: Option<Vec<u8>>,
    packages: Vec<RepoPackage>,
    evidence: ScanEvidence,
}

impl ScanInventory {
    fn parse(
        cyclonedx: Vec<u8>,
        companion: Option<Vec<u8>>,
        evidence: ScanEvidence,
    ) -> Result<Self, Error> {
        let (packages, npm_coverage, python_coverage) = parse_cyclonedx_inventory(&cyclonedx)?;
        npm_coverage.verify(&evidence.inputs)?;
        inputs::verify_cargo_and_python(&evidence.inputs, &packages, &python_coverage)?;
        Ok(Self {
            cyclonedx,
            companion,
            packages,
            evidence,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanEvidence {
    pub(crate) scope: ScanScope,
    pub(crate) default_scope: bool,
    tool: PathBuf,
    exclusions: Vec<String>,
    inputs: std::sync::Arc<inputs::ScanInputs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScanScope {
    configuration: String,
    exclusions: BTreeSet<String>,
}

impl ScanScope {
    pub(crate) fn covers(&self, previous: &str) -> bool {
        serde_json::from_str::<Self>(previous).is_ok_and(|previous| {
            self.configuration == previous.configuration
                && self.exclusions.is_subset(&previous.exclusions)
        })
    }
}

impl ScanEvidence {
    pub(crate) async fn verify_source(&self, root: &Path) -> Result<(), Error> {
        let (config, current) = tokio::join!(
            active_syft_configuration(&self.tool),
            scan_source_inputs(root, &self.tool, &self.exclusions),
        );
        let current_scope = config.as_deref().map(syft_scan_scope).transpose()?;
        if !current_scope.is_some_and(|(scope, _)| scope == self.scope) || *self.inputs != current?
        {
            return Err(inputs::incomplete(
                "Dependency inputs or Syft settings changed during the scan. Retry the scan.",
            ));
        }
        Ok(())
    }
}

fn syft_scan_scope(config: &[u8]) -> Result<(ScanScope, Vec<String>), Error> {
    let exclusions = serde_saphyr::from_slice::<SyftFileExclusions>(config)
        .map_err(|_| inputs::incomplete("Could not read Syft's file exclusions."))?
        .exclude;
    let mut config: BTreeMap<String, Value> = serde_saphyr::from_slice(config)
        .map_err(|_| inputs::incomplete("Could not read Syft's effective configuration."))?;
    // These options affect rendering or execution, not cataloging coverage.
    for key in [
        "log",
        "output",
        "format",
        "config",
        "check-for-app-update",
        "parallelism",
        "exclude",
    ] {
        config.remove(key);
    }
    let scope = ScanScope {
        configuration: super::hashing::sha256_hex(&serde_json::to_vec(&config)?),
        exclusions: exclusions
            .iter()
            .map(|pattern| super::hashing::sha256_hex(pattern.as_bytes()))
            .collect(),
    };
    Ok((scope, exclusions))
}

async fn scan_source_inputs(
    path: &Path,
    tool: &Path,
    exclusions: &[String],
) -> Result<inputs::ScanInputs, Error> {
    // Query the directory resolver used by Syft itself. Metadata retains the
    // logical path of a symlink, including when its target has another name.
    let mut command = syft_scan_command(path, tool, Some(exclusions));
    command.env("SYFT_FILE_METADATA_SELECTION", "all").args([
        "--override-default-catalogers=file-metadata-cataloger",
        "--select-catalogers=-file-content-cataloger,-file-digest-cataloger,-file-executable-cataloger",
        "-o",
        "syft-json",
    ]);
    let report = super::process::output(
        command,
        None,
        SCAN_COMMAND_TIMEOUT,
        MAX_COMPANION_BYTES as usize,
    )
    .await?;
    if !report.status.success() {
        return Err(inputs::incomplete(
            "Syft could not list the files covered by the scan.",
        ));
    }
    let root = path.to_path_buf();
    crate::runtime::run_io(move || inputs::ScanInputs::capture(&root, &report.stdout))
        .await
        .map_err(Error::Scanner)?
}

async fn inventory_repository(
    path: &Path,
    tool: &Path,
    grype_cache: Option<&Path>,
) -> Result<ScanInventory, Error> {
    let (config, defaults) = tokio::join!(
        active_syft_configuration(tool),
        syft_configuration(tool, false)
    );
    let config = config.as_deref().ok_or_else(|| {
        inputs::incomplete("Could not load Syft's effective configuration. Check Syft and retry.")
    })?;
    let (scope, exclusions) = syft_scan_scope(config)?;
    let default_scope = defaults
        .as_deref()
        .map(syft_scan_scope)
        .transpose()?
        .is_some_and(|(defaults, _)| scope == defaults);
    let inputs = scan_source_inputs(path, tool, &exclusions).await?;
    let evidence = ScanEvidence {
        default_scope,
        scope,
        tool: tool.to_path_buf(),
        exclusions,
        inputs: std::sync::Arc::new(inputs),
    };
    let directory = grype_cache
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(|parent| parent.join("inventory"));
    let key = match (&directory, defaults.as_deref()) {
        (Some(_), Some(defaults)) => cache_fingerprint(path, tool, config, defaults).await,
        _ => None,
    };
    if let (Some(directory), Some(key)) = (&directory, &key) {
        let directory = directory.clone();
        let cached_key = key.clone();
        let evidence = evidence.clone();
        let cached = crate::runtime::run_io(move || {
            let (cyclonedx, companion) = super::inventory_cache::read(&directory, &cached_key)?;
            ScanInventory::parse(cyclonedx, companion, evidence).ok()
        })
        .await
        .ok()
        .flatten();
        if let Some(cached) = cached {
            if cache_inputs_unchanged(path, tool, config, key).await {
                if std::env::var_os("CVEC_PERF_TRACE").is_some() {
                    eprintln!("cvec-perf inventory cache=hit");
                }
                return Ok(cached);
            }
        }
    }
    let (cyclonedx, companion) = capture_syft_outputs_with_tool(
        path,
        tool,
        SCAN_COMMAND_TIMEOUT,
        None,
        Some(&evidence.exclusions),
    )
    .await?;
    if std::env::var_os("CVEC_PERF_TRACE").is_some() {
        eprintln!("cvec-perf inventory cache=miss");
    }
    let cache_entry = match directory.zip(key) {
        Some((directory, key))
            if companion.is_some() && cache_inputs_unchanged(path, tool, config, &key).await =>
        {
            Some((directory, key))
        }
        _ => None,
    };
    crate::runtime::run_io(move || {
        let inventory = ScanInventory::parse(cyclonedx, companion, evidence)?;
        if let (Some((directory, key)), Some(companion)) = (cache_entry, &inventory.companion) {
            let _ =
                super::inventory_cache::write(&directory, &key, &inventory.cyclonedx, companion);
        }
        Ok(inventory)
    })
    .await
    .map_err(Error::Scanner)?
}

async fn capture_syft_outputs_with_tool(
    path: &Path,
    syft_path: &Path,
    command_timeout: Duration,
    temp_parent: Option<&Path>,
    exclusions: Option<&[String]>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), Error> {
    let mut temp_builder = tempfile::Builder::new();
    temp_builder.prefix("cvec-remediation-");
    let temp_dir = match temp_parent {
        Some(parent) => temp_builder.tempdir_in(parent)?,
        None => temp_builder.tempdir()?,
    };
    let cyclonedx_path = temp_dir.path().join("inventory.cdx.json");
    let companion_path = temp_dir.path().join("inventory.syft.json");

    let mut syft = syft_scan_command(path, syft_path, exclusions);
    syft.arg("-o")
        .arg(format!("cyclonedx-json={}", cyclonedx_path.display()))
        .arg("-o")
        .arg(format!("syft-json={}", companion_path.display()));
    let output = super::process::output(syft, None, command_timeout, 1024 * 1024).await?;

    if output.status.success() {
        let (cyclonedx, companion) = crate::runtime::run(move || -> Result<_, Error> {
            let cyclonedx = read_bounded_output(&cyclonedx_path, MAX_COMPANION_BYTES)?;
            let companion = read_bounded_output(&companion_path, MAX_COMPANION_BYTES).ok();
            Ok((cyclonedx, companion))
        })
        .await
        .map_err(Error::Scanner)??;
        return Ok((cyclonedx, companion));
    }

    // Older Syft versions do not understand multi-output destinations. Preserve
    // the established CycloneDX stdout path so optional evidence cannot fail a scan.
    let mut legacy = syft_scan_command(path, syft_path, exclusions);
    legacy.args(["-o", "cyclonedx-json"]);
    let legacy_output =
        super::process::output(legacy, None, command_timeout, MAX_COMPANION_BYTES as usize).await?;
    if !legacy_output.status.success() {
        return Err(Error::Scanner(format!(
            "syft failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&legacy_output.stderr).trim()
        )));
    }
    Ok((legacy_output.stdout, None))
}

fn syft_scan_command(path: &Path, tool: &Path, exclusions: Option<&[String]>) -> AsyncCommand {
    let mut command = syft_command_for_path(tool);
    command.arg(path).arg("--base-path").arg(path);
    // Freeze the loaded exclusions for both the catalog and file list.
    if let Some(exclusions) = exclusions {
        if exclusions.is_empty() {
            command.arg("--exclude=");
        }
        for exclusion in exclusions {
            command.arg("--exclude").arg(exclusion);
        }
    }
    command
}

fn syft_command_for_path(path: &Path) -> AsyncCommand {
    let mut command = AsyncCommand::new(path);
    command.kill_on_drop(true).env("PATH", joined_path());
    command
}

pub(crate) fn read_bounded_output(path: &Path, max_bytes: u64) -> Result<Vec<u8>, Error> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(Error::Scanner(
            "scanner output exceeded the supported size".into(),
        ));
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(Error::Scanner(
            "scanner output exceeded the supported size".into(),
        ));
    }
    Ok(bytes)
}

#[derive(Debug, Clone)]
struct SyftArtifactEvidence {
    name: String,
    version: Option<String>,
    package_type: Option<String>,
    package_url: Option<String>,
    locations: Vec<(String, RemediationLocationKind)>,
    relationship: RemediationRelationship,
    parent_package: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SupportedManifestDeclaration {
    normalized_package_name: String,
    relative_path: String,
}

fn parse_syft_remediation_evidence(
    bytes: &[u8],
    source_root: &Path,
) -> Result<Vec<SyftArtifactEvidence>, Error> {
    let value: Value = serde_json::from_slice(bytes)?;
    let schema_version = value
        .get("schema")
        .and_then(|schema| schema.get("version"))
        .and_then(Value::as_str)
        .or_else(|| value.get("schemaVersion").and_then(Value::as_str));
    if schema_version.is_none() {
        return Err(Error::Scanner("unsupported Syft evidence schema".into()));
    }

    let raw_artifacts = value
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Scanner("Syft evidence has no artifacts".into()))?;
    if raw_artifacts.len() > MAX_ARTIFACTS {
        return Err(Error::Scanner(
            "Syft evidence has too many artifacts".into(),
        ));
    }

    let mut artifacts = Vec::with_capacity(raw_artifacts.len());
    let mut artifact_ids = BTreeMap::new();
    for artifact in raw_artifacts {
        let name = artifact
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unknown")
            .to_string();
        let version = artifact
            .get("version")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        let package_type = artifact
            .get("type")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        let package_url = artifact
            .get("purl")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        let mut locations = artifact
            .get("locations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_LOCATIONS_PER_ARTIFACT)
            .filter_map(|location| {
                location
                    .get("path")
                    .and_then(Value::as_str)
                    .or_else(|| location.get("accessPath").and_then(Value::as_str))
                    .and_then(|candidate| normalize_scanner_path(source_root, candidate))
            })
            .map(|relative_path| {
                let kind = classify_remediation_path(&relative_path);
                (relative_path, kind)
            })
            .collect::<Vec<_>>();
        locations.sort_by(|left, right| rank_location(left).cmp(&rank_location(right)));
        locations.dedup_by(|left, right| left.0 == right.0);

        let index = artifacts.len();
        if let Some(id) = artifact.get("id").and_then(Value::as_str) {
            artifact_ids.insert(id.to_string(), index);
        }
        artifacts.push(SyftArtifactEvidence {
            name,
            version,
            package_type,
            package_url,
            locations,
            relationship: RemediationRelationship::Unknown,
            parent_package: None,
        });
    }

    let relationships = value
        .get("artifactRelationships")
        .or_else(|| value.get("relationships"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if relationships.len() > MAX_RELATIONSHIPS {
        return Err(Error::Scanner(
            "Syft evidence has too many relationships".into(),
        ));
    }
    for relationship in relationships {
        if relationship.get("type").and_then(Value::as_str) != Some("dependency-of") {
            continue;
        }
        // Syft models `parent dependency-of child`; the child is the package
        // depending on the parent. This proves an upstream package relationship,
        // but not a direct declaration in a manifest.
        let dependency_id = relationship.get("parent").and_then(Value::as_str);
        let dependent_id = relationship.get("child").and_then(Value::as_str);
        let (Some(dependency_index), Some(dependent_index)) = (
            dependency_id.and_then(|id| artifact_ids.get(id)).copied(),
            dependent_id.and_then(|id| artifact_ids.get(id)).copied(),
        ) else {
            continue;
        };
        if dependency_index == dependent_index {
            continue;
        }
        let parent_name = artifacts[dependent_index].name.clone();
        artifacts[dependency_index].relationship = RemediationRelationship::Transitive;
        artifacts[dependency_index].parent_package = Some(parent_name);
    }

    Ok(artifacts)
}

fn normalize_scanner_path(source_root: &Path, candidate: &str) -> Option<String> {
    if candidate.is_empty()
        || candidate.as_bytes().contains(&0)
        || candidate.len() > MAX_RELATIVE_PATH_BYTES
    {
        return None;
    }
    let raw = Path::new(candidate);
    let relative = if raw.is_absolute() {
        raw.strip_prefix(source_root).ok()?
    } else {
        raw
    };
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return None;
    }
    let normalized = relative.to_str()?.replace('\\', "/");
    (!normalized.is_empty() && normalized.len() <= MAX_RELATIVE_PATH_BYTES).then_some(normalized)
}

fn classify_remediation_path(path: &str) -> RemediationLocationKind {
    let file_name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if matches!(
        file_name.as_str(),
        "package.json"
            | "cargo.toml"
            | "pyproject.toml"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "go.mod"
            | "gemfile"
            | "composer.json"
            | "requirements.txt"
    ) {
        RemediationLocationKind::Manifest
    } else if file_name.ends_with(".lock")
        || matches!(
            file_name.as_str(),
            "package-lock.json"
                | "npm-shrinkwrap.json"
                | "yarn.lock"
                | "pnpm-lock.yaml"
                | "go.sum"
                | "composer.lock"
        )
    {
        RemediationLocationKind::Lockfile
    } else {
        RemediationLocationKind::Other
    }
}

fn rank_location(location: &(String, RemediationLocationKind)) -> (u8, usize, &str) {
    let rank = match location.1 {
        RemediationLocationKind::Manifest => 0,
        RemediationLocationKind::Lockfile => 1,
        RemediationLocationKind::Other => 2,
    };
    (rank, location.0.len(), location.0.as_str())
}

fn apply_supported_manifest_evidence(
    source_root: &Path,
    findings: &[NormalizedFinding],
    evidence: &mut [NormalizedRemediationEvidence],
) {
    let declarations = parse_supported_manifest_declarations(source_root);
    if declarations.is_empty() {
        return;
    }

    let mut paths_by_package = BTreeMap::<String, Vec<String>>::new();
    for declaration in declarations {
        paths_by_package
            .entry(declaration.normalized_package_name)
            .or_default()
            .push(declaration.relative_path);
    }
    for paths in paths_by_package.values_mut() {
        paths.sort_by(|left, right| (left.len(), left).cmp(&(right.len(), right)));
        paths.dedup();
        paths.truncate(MAX_LOCATIONS_PER_ARTIFACT);
    }

    for (finding, item) in findings.iter().zip(evidence.iter_mut()) {
        if !is_python_finding(finding) {
            continue;
        }
        let Some(normalized_name) = normalize_python_package_name(&finding.package_name) else {
            continue;
        };
        let Some(paths) = paths_by_package.get(&normalized_name) else {
            continue;
        };
        let Some(best_path) = paths.first() else {
            continue;
        };

        let existing_is_distinct = item
            .relative_path
            .as_ref()
            .is_some_and(|path| !paths.contains(path));
        item.relative_path = Some(best_path.clone());
        item.location_kind = Some(RemediationLocationKind::Manifest);
        item.relationship = RemediationRelationship::Direct;
        item.parent_package = None;
        item.source = RemediationEvidenceSource::SupportedManifest;
        item.quality = RemediationEvidenceQuality::Current;
        item.location_count = (paths.len() + usize::from(existing_is_distinct))
            .min(MAX_LOCATIONS_PER_ARTIFACT) as u32;
    }
}

fn is_python_finding(finding: &NormalizedFinding) -> bool {
    finding.package_type.as_deref().is_some_and(|package_type| {
        package_type.eq_ignore_ascii_case("python") || package_type.eq_ignore_ascii_case("pypi")
    }) || finding
        .package_url
        .as_deref()
        .is_some_and(|package_url| package_url.starts_with("pkg:pypi/"))
}

fn parse_supported_manifest_declarations(source_root: &Path) -> Vec<SupportedManifestDeclaration> {
    let relative_path = "pyproject.toml";
    #[cfg(unix)]
    let manifest = AnchoredRoot::open(source_root)
        .and_then(|root| root.entry(Path::new(relative_path), false))
        .and_then(|entry| entry.open());
    #[cfg(not(unix))]
    let manifest = {
        let path = source_root.join(relative_path);
        fs::symlink_metadata(&path).and_then(|metadata| {
            if !metadata.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Expected a regular pyproject.toml",
                ));
            }
            fs::File::open(path)
        })
    };
    let Ok(contents) =
        manifest.and_then(|file| read_regular_text(file, MAX_SUPPORTED_MANIFEST_BYTES))
    else {
        return Vec::new();
    };
    let Some(requirements) = parse_pep621_project_dependencies(&contents) else {
        return Vec::new();
    };

    let mut declarations = requirements
        .into_iter()
        .filter_map(|requirement| {
            normalize_python_requirement(&requirement).map(|normalized_package_name| {
                SupportedManifestDeclaration {
                    normalized_package_name,
                    relative_path: relative_path.to_string(),
                }
            })
        })
        .take(MAX_SUPPORTED_MANIFEST_DECLARATIONS)
        .collect::<Vec<_>>();
    declarations.sort_by(|left, right| {
        (&left.normalized_package_name, &left.relative_path)
            .cmp(&(&right.normalized_package_name, &right.relative_path))
    });
    declarations.dedup();
    declarations
}

fn parse_pep621_project_dependencies(contents: &str) -> Option<Vec<String>> {
    let mut project_table = String::new();
    let mut in_project_table = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            let header = trimmed.split('#').next().unwrap_or(trimmed).trim();
            if header == "[project]" {
                in_project_table = true;
                continue;
            }
            if in_project_table {
                break;
            }
        }
        if in_project_table {
            project_table.push_str(line);
            project_table.push('\n');
        }
    }
    if !in_project_table {
        return None;
    }

    let mut offset = 0;
    for line in project_table.split_inclusive('\n') {
        let leading_bytes = line.len() - line.trim_start().len();
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with('#') {
            offset += line.len();
            continue;
        }
        let Some(equals_index) = trimmed_start.find('=') else {
            offset += line.len();
            continue;
        };
        if trimmed_start[..equals_index].trim() != "dependencies" {
            offset += line.len();
            continue;
        }
        let after_equals = &trimmed_start[equals_index + 1..];
        let array_padding = after_equals.len() - after_equals.trim_start().len();
        if !after_equals.trim_start().starts_with('[') {
            return None;
        }
        let array_start = offset + leading_bytes + equals_index + 1 + array_padding;
        return parse_toml_string_array(&project_table[array_start..]);
    }
    None
}

fn parse_toml_string_array(input: &str) -> Option<Vec<String>> {
    let mut chars = input.chars().peekable();
    if chars.next()? != '[' {
        return None;
    }
    let mut values = Vec::new();
    loop {
        skip_toml_array_whitespace_and_comments(&mut chars);
        if chars.peek().copied() == Some(']') {
            chars.next();
            return Some(values);
        }
        if values.len() >= MAX_SUPPORTED_MANIFEST_DECLARATIONS {
            return None;
        }
        let quote = chars.next()?;
        if quote != '\'' && quote != '"' {
            return None;
        }
        if chars.peek().copied() == Some(quote) {
            return None;
        }
        let mut value = String::new();
        loop {
            let character = chars.next()?;
            if character == quote {
                break;
            }
            if character == '\\' || character == '\n' || character == '\r' {
                return None;
            }
            value.push(character);
        }
        values.push(value);
        skip_toml_array_whitespace_and_comments(&mut chars);
        match chars.next()? {
            ',' => continue,
            ']' => return Some(values),
            _ => return None,
        }
    }
}

fn skip_toml_array_whitespace_and_comments(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    loop {
        while chars
            .peek()
            .is_some_and(|character| character.is_whitespace())
        {
            chars.next();
        }
        if chars.peek().copied() != Some('#') {
            return;
        }
        for character in chars.by_ref() {
            if character == '\n' {
                break;
            }
        }
    }
}

fn normalize_python_requirement(requirement: &str) -> Option<String> {
    let trimmed = requirement.trim();
    let name_length = trimmed
        .char_indices()
        .take_while(|(_, character)| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        .last()
        .map(|(index, character)| index + character.len_utf8())?;
    let name = &trimmed[..name_length];
    if !name.chars().next()?.is_ascii_alphanumeric() {
        return None;
    }
    if let Some(next) = trimmed[name_length..].chars().next() {
        if !next.is_whitespace() && !matches!(next, '[' | '<' | '>' | '=' | '!' | '~' | ';' | '@') {
            return None;
        }
    }
    normalize_python_package_name(name)
}

pub(super) fn normalize_python_package_name(name: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut previous_separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
            previous_separator = false;
        } else if matches!(character, '-' | '_' | '.') {
            if !previous_separator {
                normalized.push('-');
                previous_separator = true;
            }
        } else {
            return None;
        }
    }
    if normalized.is_empty() || normalized.ends_with('-') {
        None
    } else {
        Some(normalized)
    }
}

fn correlate_remediation_evidence(
    findings: &[NormalizedFinding],
    artifacts: &[SyftArtifactEvidence],
) -> Vec<NormalizedRemediationEvidence> {
    let mut by_purl = std::collections::HashMap::<&str, Vec<&SyftArtifactEvidence>>::new();
    let mut by_coordinate = std::collections::HashMap::new();
    for artifact in artifacts {
        if let Some(purl) = artifact.package_url.as_deref() {
            by_purl.entry(purl).or_default().push(artifact);
        }
        by_coordinate
            .entry((
                artifact.name.as_str(),
                artifact.version.as_deref(),
                artifact.package_type.as_deref(),
            ))
            .or_insert_with(Vec::new)
            .push(artifact);
    }
    findings
        .iter()
        .map(|finding| {
            let candidates = finding
                .package_url
                .as_deref()
                .and_then(|purl| by_purl.get(purl))
                .or_else(|| {
                    by_coordinate.get(&(
                        finding.package_name.as_str(),
                        finding.package_version.as_deref(),
                        finding.package_type.as_deref(),
                    ))
                });
            let candidates = candidates.map(Vec::as_slice).unwrap_or_default();
            let matched = (candidates.len() == 1)
                .then(|| candidates.first().copied())
                .flatten();
            evidence_for_finding(finding, matched)
        })
        .collect()
}

fn evidence_for_finding(
    finding: &NormalizedFinding,
    artifact: Option<&SyftArtifactEvidence>,
) -> NormalizedRemediationEvidence {
    let best_location = artifact.and_then(|value| value.locations.first());
    let relationship = artifact
        .map(|value| value.relationship)
        .unwrap_or(RemediationRelationship::Unknown);
    let quality = match (artifact, best_location, relationship) {
        (None, _, _) => RemediationEvidenceQuality::Unavailable,
        (
            Some(_),
            Some(_),
            RemediationRelationship::Direct | RemediationRelationship::Transitive,
        ) => RemediationEvidenceQuality::Current,
        (Some(_), _, _) => RemediationEvidenceQuality::Partial,
    };
    let source = match (best_location, relationship) {
        (_, RemediationRelationship::Direct | RemediationRelationship::Transitive) => {
            RemediationEvidenceSource::SyftRelationship
        }
        (Some(_), _) => RemediationEvidenceSource::SyftLocation,
        _ => RemediationEvidenceSource::None,
    };
    NormalizedRemediationEvidence {
        schema_version: REMEDIATION_SCHEMA_VERSION,
        fingerprint: finding.fingerprint.clone(),
        package_name: finding.package_name.clone(),
        package_version: finding.package_version.clone(),
        package_type: finding.package_type.clone(),
        package_url: finding.package_url.clone(),
        relative_path: best_location.map(|value| value.0.clone()),
        location_kind: best_location.map(|value| value.1),
        relationship,
        parent_package: artifact.and_then(|value| value.parent_package.clone()),
        source,
        quality,
        location_count: artifact
            .map(|value| value.locations.len() as u32)
            .unwrap_or(0),
    }
}

fn unavailable_remediation_evidence(
    findings: &[NormalizedFinding],
) -> Vec<NormalizedRemediationEvidence> {
    findings
        .iter()
        .map(|finding| evidence_for_finding(finding, None))
        .collect()
}

pub fn grype_db_local_status(cache_dir: &Path) -> Result<GrypeDbStatus, Error> {
    let mut command = grype_command(cache_dir)?;
    command.args(["db", "status", "-o", "json"]);
    let output = super::process::output_sync(&mut command, Duration::from_secs(3), 1024 * 1024)?;
    let status: RawGrypeDbStatus = parse_json_output("grype db status", &output)?;
    Ok(GrypeDbStatus {
        present: status.valid || status.built.is_some(),
        valid: status.valid,
        path: status.path,
        schema_version: non_empty(status.schema_version),
        built_at: status.built,
        source_url: status.from,
        update_available: None,
        latest_schema_version: None,
        latest_built_at: None,
        error: status.error,
        update_check_error: None,
    })
}
pub fn grype_db_update_check(mut status: GrypeDbStatus, cache_dir: &Path) -> GrypeDbStatus {
    let checked = (|| -> Result<RawGrypeDbCheck, Error> {
        let mut command = grype_check_command(cache_dir)?;
        command.args(["db", "check", "-o", "json"]);
        let output =
            super::process::output_sync(&mut command, Duration::from_secs(5), 1024 * 1024)?;
        parse_json_output("grype db check", &output)
    })();
    match checked {
        Ok(check) => {
            status.update_available = Some(check.update_available);
            status.latest_schema_version = check
                .candidate_db
                .as_ref()
                .map(|candidate| candidate.schema_version.clone());
            status.latest_built_at = check.candidate_db.map(|candidate| candidate.built);
            status.update_check_error = None;
        }
        Err(cause) => {
            status.update_available = None;
            status.update_check_error = Some(cause.to_string());
        }
    }
    status
}
pub fn grype_db_status(cache_dir: &Path) -> Result<GrypeDbStatus, Error> {
    Ok(grype_db_update_check(
        grype_db_local_status(cache_dir)?,
        cache_dir,
    ))
}

pub fn update_grype_db(grype_db_cache_dir: &Path) -> Result<(), Error> {
    let mut command = grype_command(grype_db_cache_dir)?;
    command.arg("db").arg("update");
    let output = super::process::output_sync(&mut command, Duration::from_secs(120), 1024 * 1024)?;

    if !output.status.success() {
        return Err(command_failed("grype db update", &output));
    }

    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrypeReport {
    matches: Vec<GrypeMatch>,
    ignored_matches: Option<Vec<GrypeMatch>>,
}

#[derive(Deserialize)]
struct GrypeMatch {
    vulnerability: GrypeVulnerability,
    artifact: GrypeArtifact,
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
struct GrypeArtifact {
    name: String,
    version: Option<String>,
    #[serde(rename = "type")]
    package_type: Option<String>,
    purl: Option<String>,
}

impl GrypeMatch {
    fn into_finding(self) -> Result<Option<NormalizedFinding>, Error> {
        let Self {
            vulnerability,
            artifact,
        } = self;
        if vulnerability.id.trim().is_empty() || artifact.name.trim().is_empty() {
            return Err(Error::Scanner(
                "Grype match is missing its advisory or package identity".into(),
            ));
        }
        let reported_severity = vulnerability.severity.trim();
        let Some(severity) = Severity::parse(reported_severity) else {
            if ["low", "negligible", "unknown"]
                .iter()
                .any(|value| reported_severity.eq_ignore_ascii_case(value))
            {
                return Ok(None);
            }
            return Err(Error::Scanner(
                "Grype match has an unrecognized severity".into(),
            ));
        };
        let fingerprint = build_fingerprint(
            &vulnerability.id,
            artifact.package_type.as_deref(),
            artifact.purl.as_deref(),
            &artifact.name,
            artifact.version.as_deref(),
        );
        Ok(Some(NormalizedFinding {
            fingerprint,
            advisory_id: vulnerability.id,
            package_name: artifact.name,
            package_version: artifact.version,
            package_type: artifact.package_type,
            severity,
            fix_version: vulnerability
                .fix
                .and_then(|fix| fix.versions)
                .and_then(|versions| versions.into_iter().next()),
            title: vulnerability.description,
            source_url: vulnerability.data_source,
            package_url: artifact.purl,
            advisory_details: None,
        }))
    }
}

fn parse_grype_json(bytes: &[u8]) -> Result<Vec<NormalizedFinding>, Error> {
    let report: GrypeReport = serde_json::from_slice(bytes).map_err(|cause| {
        Error::Scanner(format!("Grype returned an invalid JSON report: {cause}"))
    })?;
    let mut findings = BTreeMap::new();
    for entry in report.matches {
        if let Some(finding) = entry.into_finding()? {
            findings
                .entry(finding.fingerprint.clone())
                .or_insert(finding);
        }
    }
    for entry in report.ignored_matches.into_iter().flatten() {
        if let Some(finding) = entry.into_finding()? {
            if !findings
                .get(&finding.fingerprint)
                .is_some_and(|included| included.severity == finding.severity)
            {
                return Err(Error::Scanner(
                    "Grype filters suppressed findings tracked by CVEC. Review Grype's ignore, fix-state, or VEX settings and retry the scan.".into(),
                ));
            }
        }
    }
    Ok(findings.into_values().collect())
}

fn parse_cyclonedx_inventory(
    bytes: &[u8],
) -> Result<
    (
        Vec<RepoPackage>,
        npm_coverage::NpmCoverage,
        inputs::PythonCoverage,
    ),
    Error,
> {
    let value: Value = serde_json::from_slice(bytes).map_err(|cause| {
        Error::Scanner(format!("Syft returned invalid CycloneDX JSON: {cause}"))
    })?;
    let invalid_inventory =
        || Error::Scanner("Syft output is not a complete CycloneDX inventory".into());
    if value.get("bomFormat").and_then(Value::as_str) != Some("CycloneDX")
        || value
            .get("specVersion")
            .and_then(Value::as_str)
            .is_none_or(|version| version.trim().is_empty())
    {
        return Err(invalid_inventory());
    }
    let components = match value.get("components") {
        Some(components) => components
            .as_array()
            .ok_or_else(invalid_inventory)?
            .as_slice(),
        // Syft omits the array for an empty scan and includes the source component.
        None if value
            .pointer("/metadata/component/name")
            .and_then(Value::as_str)
            .is_some_and(|name| !name.trim().is_empty()) =>
        {
            &[]
        }
        None => return Err(invalid_inventory()),
    };

    let mut packages = BTreeMap::new();
    let mut npm_coverage = npm_coverage::NpmCoverage::default();
    let mut python_coverage = inputs::PythonCoverage::default();
    for component in components {
        let component_type = component
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| !kind.trim().is_empty())
            .ok_or_else(invalid_inventory)?;
        let name = component
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(invalid_inventory)?;
        if !matches!(component_type, "library" | "framework") {
            continue;
        }
        let version = component
            .get("version")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        let package_url = component
            .get("purl")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        let package_type = package_url
            .as_deref()
            .and_then(package_type_from_purl)
            .or_else(|| {
                component
                    .get("properties")
                    .and_then(Value::as_array)
                    .and_then(|properties| package_type_from_properties(properties))
            });
        match (package_type.as_deref(), version.as_deref()) {
            (Some("npm"), Some(version)) => npm_coverage.record(name, version, component),
            (Some("pypi" | "python"), Some(_)) => python_coverage.record(name, component),
            _ => {}
        }

        let key = package_identity(
            package_url.as_deref(),
            name,
            version.as_deref(),
            package_type.as_deref(),
        );
        packages.entry(key).or_insert(RepoPackage {
            name: name.to_string(),
            version,
            package_type,
            package_url,
            highest_severity: None,
            finding_count: 0,
            fix_available: false,
        });
    }

    Ok((
        packages.into_values().collect(),
        npm_coverage,
        python_coverage,
    ))
}

fn syft_locations(component: &Value) -> impl Iterator<Item = &Value> {
    component
        .get("properties")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|property| {
            property
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|key| key.starts_with("syft:location:") && key.ends_with(":path"))
        })
}

fn package_type_from_purl(value: &str) -> Option<String> {
    value
        .strip_prefix("pkg:")
        .and_then(|rest| rest.split('/').next())
        .filter(|package_type| !package_type.is_empty())
        .map(str::to_string)
}

fn package_type_from_properties(properties: &[Value]) -> Option<String> {
    for property in properties {
        let name = property.get("name").and_then(Value::as_str)?;
        if name != "syft:package:type" {
            continue;
        }
        return property
            .get("value")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
    }
    None
}

pub fn package_identity(
    package_url: Option<&str>,
    name: &str,
    version: Option<&str>,
    package_type: Option<&str>,
) -> String {
    package_url.map(str::to_string).unwrap_or_else(|| {
        format!(
            "{}|{}|{}",
            package_type.unwrap_or("unknown"),
            name,
            version.unwrap_or("unknown")
        )
    })
}

fn version_for(tool: &str) -> Result<String, Error> {
    let mut command = command_for(tool, None)?;
    command.arg("--version");
    let output = super::process::output_sync(&mut command, Duration::from_secs(3), 64 * 1024)?;
    checked_version(tool, output)
}

fn checked_version(tool: &str, output: Output) -> Result<String, Error> {
    if !output.status.success() {
        return Err(Error::Scanner(format!(
            "{} --version failed: {}",
            tool,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn command_for(tool: &str, grype_db_cache_dir: Option<&Path>) -> Result<Command, Error> {
    let path = resolve_tool(tool).ok_or_else(|| {
        Error::Scanner(format!(
            "{} is not installed or not reachable from the app",
            tool
        ))
    })?;
    let mut command = Command::new(path);
    command.env("PATH", joined_path());
    if tool == "grype" {
        if let Some(cache_dir) = grype_db_cache_dir {
            fs::create_dir_all(cache_dir)?;
            command.env("GRYPE_DB_CACHE_DIR", cache_dir);
        }
        command.env("GRYPE_CHECK_FOR_APP_UPDATE", "false");
    }
    Ok(command)
}

fn async_command_for(tool: &str, grype_db_cache_dir: Option<&Path>) -> Result<AsyncCommand, Error> {
    let path = resolve_tool(tool).ok_or_else(|| {
        Error::Scanner(format!(
            "{} is not installed or not reachable from the app",
            tool
        ))
    })?;
    let mut command = AsyncCommand::new(path);
    command.kill_on_drop(true).env("PATH", joined_path());
    if tool == "grype" {
        if let Some(cache_dir) = grype_db_cache_dir {
            fs::create_dir_all(cache_dir)?;
            command.env("GRYPE_DB_CACHE_DIR", cache_dir);
        }
        command.env("GRYPE_CHECK_FOR_APP_UPDATE", "false");
    }
    Ok(command)
}

fn grype_command(grype_db_cache_dir: &Path) -> Result<Command, Error> {
    command_for("grype", Some(grype_db_cache_dir))
}

fn grype_check_command(grype_db_cache_dir: &Path) -> Result<Command, Error> {
    let mut command = grype_command(grype_db_cache_dir)?;
    command.env("GRYPE_DB_UPDATE_AVAILABLE_TIMEOUT", "3s");
    Ok(command)
}

fn resolve_tool(tool: &str) -> Option<PathBuf> {
    let current_path = env::var_os("PATH").unwrap_or_default();
    resolve_tool_from_paths(tool, env::split_paths(&current_path))
}

fn resolve_tool_from_paths<I>(tool: &str, path_dirs: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    resolve_tool_from_path_sets(tool, path_dirs, trusted_tool_dirs())
}

fn trusted_tool_dirs() -> Vec<PathBuf> {
    #[cfg(debug_assertions)]
    let qa_override = env::var_os(QA_TRUSTED_TOOL_DIRS_ENV);

    #[cfg(not(debug_assertions))]
    let qa_override: Option<OsString> = None;

    resolve_trusted_tool_dirs(qa_override)
}

fn resolve_trusted_tool_dirs(qa_override: Option<OsString>) -> Vec<PathBuf> {
    let qa_dirs = qa_override
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|path| is_safe_path_dir(path))
        .collect::<Vec<_>>();

    if qa_dirs.is_empty() {
        TRUSTED_TOOL_DIRS.iter().map(PathBuf::from).collect()
    } else {
        qa_dirs
    }
}

fn resolve_tool_from_path_sets<I, J>(tool: &str, path_dirs: I, trusted_dirs: J) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
    J: IntoIterator<Item = PathBuf>,
{
    trusted_dirs
        .into_iter()
        .filter_map(|base| executable_in_dir(tool, &base))
        .chain(
            path_dirs
                .into_iter()
                .filter(|path| is_safe_path_dir(path))
                .filter_map(|path| executable_in_dir(tool, &path)),
        )
        .next()
}

fn executable_in_dir(tool: &str, dir: &Path) -> Option<PathBuf> {
    let candidate = dir.join(tool);
    is_executable_file(&candidate).then_some(candidate)
}

fn is_safe_path_dir(path: &Path) -> bool {
    path.is_absolute()
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn joined_path() -> OsString {
    let mut paths = trusted_tool_dirs();
    let current_path = env::var_os("PATH").unwrap_or_default();

    for path in env::split_paths(&current_path).filter(|path| is_safe_path_dir(path)) {
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    }

    env::join_paths(paths).unwrap_or_default()
}

fn parse_json_output<T: for<'de> Deserialize<'de>>(
    command_name: &str,
    output: &Output,
) -> Result<T, Error> {
    if output.stdout.is_empty() {
        return Err(command_failed(command_name, output));
    }

    serde_json::from_slice(&output.stdout).map_err(Error::from)
}

fn command_failed(command_name: &str, output: &Output) -> Error {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let details = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("exit status {}", output.status)
    };

    Error::Scanner(format!("{command_name} failed: {details}"))
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGrypeDbStatus {
    #[serde(default)]
    schema_version: String,
    #[serde(rename = "from")]
    #[serde(default)]
    from: Option<String>,
    built: Option<String>,
    path: String,
    valid: bool,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGrypeDbCheck {
    #[serde(rename = "currentDB")]
    _current_db: Option<RawGrypeDbVersion>,
    candidate_db: Option<RawGrypeDbVersion>,
    update_available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGrypeDbVersion {
    schema_version: String,
    built: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn malformed_primary_outputs_never_become_clean_results() {
        for bytes in [
            b"{}".as_slice(),
            br#"{"matches":null}"#,
            br#"{"matches":[null]}"#,
            br#"{"matches":[{"vulnerability":{"id":"CVE-1"},"artifact":{"name":"example"}}]}"#,
            br#"{"matches":[{"vulnerability":{"id":"CVE-1","severity":""},"artifact":{"name":"example"}}]}"#,
            br#"{"matches":[{"vulnerability":{"id":"CVE-1","severity":"HIGH"},"artifact":{}}]}"#,
            br#"{"matches":[{"vulnerability":{"id":"","severity":"Low"},"artifact":{"name":"example"}}]}"#,
            br#"{"matches":[{"vulnerability":{"id":"CVE-1","severity":"Low"},"artifact":{"name":""}}]}"#,
            br#"{"matches":[{"vulnerability":{"id":"CVE-1","severity":"Criticals"},"artifact":{"name":"example"}}]}"#,
            br#"{"matches":[{"vulnerability":{"id":"CVE-1","severity":"HIGH"},"artifact":{"name":"example","version":1}}]}"#,
            br#"{"matches":[],"matches":[]}"#,
        ] {
            assert!(parse_grype_json(bytes).is_err());
        }
        for bytes in [
            br#"{"matches":[]}"#.as_slice(),
            br#"{"matches":[{"vulnerability":{"id":"CVE-1","severity":"Low"},"artifact":{"name":"example"}}],"ignoredMatches":null}"#,
            br#"{"matches":[{"vulnerability":{"id":"CVE-1","severity":"Unknown"},"artifact":{"name":"example"}}]}"#,
        ] {
            assert!(parse_grype_json(bytes).unwrap().is_empty());
        }
    }

    #[test]
    fn grype_filters_cannot_turn_tracked_findings_into_absence() {
        let critical = r#"{"vulnerability":{"id":"CVE-1","severity":"Critical"},"artifact":{"name":"example","version":"1.0.0","type":"npm"},"appliedIgnoreRules":[{"fix-state":"fixed"}]}"#;
        let low = r#"{"vulnerability":{"id":"CVE-2","severity":"Low"},"artifact":{"name":"example"},"appliedIgnoreRules":[{"fix-state":"fixed"}]}"#;
        let report = format!(r#"{{"matches":[],"ignoredMatches":[{critical}]}}"#);
        let error = parse_grype_json(report.as_bytes()).unwrap_err().to_string();
        assert!(
            error.contains("Grype filters suppressed findings"),
            "{error}"
        );

        let report = format!(r#"{{"matches":[{critical}],"ignoredMatches":[{critical},{low}]}}"#);
        assert_eq!(parse_grype_json(report.as_bytes()).unwrap().len(), 1);
        let report = format!(r#"{{"matches":[],"ignoredMatches":[{low}]}}"#);
        assert!(parse_grype_json(report.as_bytes()).unwrap().is_empty());
        assert!(parse_grype_json(br#"{"matches":[],"ignoredMatches":{}}"#).is_err());
    }

    #[test]
    fn validates_cyclonedx_inventory_before_it_can_replace_saved_results() {
        for bytes in [
            b"{}".as_slice(),
            br#"{"components":[]}"#,
            br#"{"bomFormat":"CycloneDX","specVersion":"","components":[]}"#,
            br#"{"bomFormat":"CycloneDX","specVersion":"1.7","components":null}"#,
            br#"{"bomFormat":"CycloneDX","specVersion":"1.7","metadata":{}}"#,
            br#"{"bomFormat":"CycloneDX","specVersion":"1.7","metadata":{"component":{}}}"#,
            br#"{"bomFormat":"CycloneDX","specVersion":"1.7","components":[null]}"#,
            br#"{"bomFormat":"CycloneDX","specVersion":"1.7","components":[{"type":"library","version":"1.0.0"}]}"#,
            br#"{"bomFormat":"CycloneDX","specVersion":"1.7","components":[{"name":"example","version":"1.0.0"}]}"#,
        ] {
            assert!(parse_cyclonedx_inventory(bytes).is_err());
        }
        for bytes in [
            br#"{"bomFormat":"CycloneDX","specVersion":"1.6","components":[]}"#.as_slice(),
            br#"{"bomFormat":"CycloneDX","specVersion":"1.7","metadata":{"component":{"name":"empty repo","type":"file"}}}"#,
            br#"{"bomFormat":"CycloneDX","specVersion":"1.7","components":[{"type":"file","name":"/README.md"}]}"#,
        ] {
            assert!(parse_cyclonedx_inventory(bytes).unwrap().0.is_empty());
        }
    }

    #[test]
    fn rejects_missing_declared_npm_packages_even_with_other_inventory() {
        let checkout = tempfile::tempdir().unwrap();
        let root = checkout.path();
        let app = root.join("apps/frontend");
        fs::create_dir_all(&app).unwrap();
        fs::write(
            app.join("package.json"),
            r#"{"name":"frontend","dependencies":{"lodash":"4.17.20","alias":"npm:@example/actual@^1","local":"workspace:*","optional":"1"},"optionalDependencies":{"optional":"1"},"peerDependencies":{"react":"^19"}}"#,
        )
        .unwrap();

        let component = |name, version, location| {
            serde_json::json!({
                "type": "library", "name":name, "version":version,
                "properties":[
                    {"name":"syft:package:type","value":"npm"},
                    {"name":"syft:location:0:path","value":location}
                ]
            })
        };
        let review_inventory = |components| {
            let cdx = serde_json::to_vec(&serde_json::json!({
                "bomFormat":"CycloneDX", "specVersion":"1.6", "components": components
            }))
            .unwrap();
            let inputs = inputs::ScanInputs::capture(
                root,
                br#"{"files":[{"location":{"path":"/apps/frontend/package.json"},"metadata":{"type":"RegularFile"}}]}"#,
            )
            .unwrap();
            parse_cyclonedx_inventory(&cdx).unwrap().1.verify(&inputs)
        };

        // An unrelated ecosystem, an app's own package, and a copy installed
        // for a different sibling must not stand in for its missing dependency.
        let unrelated = vec![
            serde_json::json!({"type":"library","name":"unrelated","purl":"pkg:cargo/unrelated@1","version":"1"}),
            component("frontend", "1.2.3", "/apps/frontend/package.json"),
            component("lodash", "4.17.20", "/apps/other/package-lock.json"),
            component("@example/actual", "1.2.3", "/pnpm-lock.yaml"),
        ];
        let cause = review_inventory(unrelated.clone()).unwrap_err().to_string();
        assert!(cause.starts_with("Incomplete npm inventory:"));
        assert!(cause.contains("apps/frontend/package.json"));
        assert!(!cause.contains(root.to_str().unwrap()));
        assert!(review_inventory(Vec::<Value>::new()).is_err());

        for source in [
            "/apps/frontend/package-lock.json",
            "/node_modules/lodash/package.json",
        ] {
            let mut incompatible = unrelated.clone();
            incompatible.push(component("lodash", "4.17.21", source));
            assert!(review_inventory(incompatible).is_err());

            let mut recovered = unrelated.clone();
            recovered.push(component("lodash", "4.17.20", source));
            review_inventory(recovered).unwrap();
        }

        // npm aliases carry the target's node-semver range. The root lockfile
        // can resolve them for a workspace, including disjunctive ranges.
        fs::write(
            app.join("package.json"),
            r#"{"dependencies":{"alias":"npm:@example/actual@1.x || 3.0.0-rc.1"}}"#,
        )
        .unwrap();
        for (version, accepted) in [("1.2.3", true), ("2.0.0", false), ("3.0.0-rc.1", true)] {
            let mut evidence = vec![component("@example/actual", version, "/pnpm-lock.yaml")];
            assert_eq!(review_inventory(evidence.clone()).is_ok(), accepted);
            evidence[0].as_object_mut().unwrap().remove("properties");
            evidence[0]["purl"] = serde_json::json!("pkg:npm/%40example/actual");
            assert_eq!(review_inventory(evidence).is_ok(), accepted);
        }

        fs::write(
            app.join("package.json"),
            r#"{"dependencies":{"lodash":">=4.17.21 <4.17.20"}}"#,
        )
        .unwrap();
        let contradictory = vec![component("lodash", "4.17.21", "/package-lock.json")];
        assert!(review_inventory(contradictory).is_err());

        // Removing an actual dependency declaration still permits an empty scan.
        fs::write(app.join("package.json"), r#"{"name":"frontend"}"#).unwrap();
        review_inventory(Vec::<Value>::new()).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn npm_verification_uses_syfts_active_exclusions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let excluded = root.join("tests/fixtures/invalid");
        fs::create_dir_all(&excluded).unwrap();
        fs::write(excluded.join("package.json"), "{").unwrap();
        let ignored_manifest = root.join("example/package.json");
        fs::create_dir(ignored_manifest.parent().unwrap()).unwrap();
        fs::write(&ignored_manifest, "{").unwrap();
        let tool = write_executable_with_content(
            temp.path().join("syft"),
            br#"#!/bin/sh
if [ "$1" = config ]; then
  [ "$2" = --load ] || exit 1
  cat <<'EOF'
exclude:
  - '**/fixtures/**'
  - './example/package.json'
EOF
  exit 0
fi
printf '%s\n' "$@" > "$0.args"
for arg in "$@"; do
  case "$arg" in
    --override-default-catalogers=file-metadata-cataloger)
      if [ -f "$1/package.json" ]; then
        printf '%s' '{"files":[{"location":{"path":"/package.json"},"metadata":{"type":"RegularFile"}}]}'
      else
        printf '%s' '{"files":[]}'
      fi
      exit 0;;
    cyclonedx-json=*) printf '%s' '{"bomFormat":"CycloneDX","specVersion":"1.6","components":[]}' > "${arg#cyclonedx-json=}";;
  esac
done
"#,
        );
        let inventory = inventory_repository(&root, &tool, None).await.unwrap();
        assert!(inventory.packages.is_empty());
        let arguments = fs::read_to_string(temp.path().join("syft.args")).unwrap();
        assert!(arguments.contains("--exclude\n**/fixtures/**\n"));
        assert!(arguments.contains("--exclude\n./example/package.json\n"));

        fs::write(root.join("package.json"), "{").unwrap();
        let cause = inventory_repository(&root, &tool, None)
            .await
            .err()
            .unwrap();
        assert!(cause.to_string().contains("dependencies in package.json"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn invalid_cached_inventory_is_replaced_by_a_valid_scan() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        let cache_dir = temp.path().join("cache/grype/db");
        let inventory_dir = temp.path().join("cache/inventory");
        let tool = write_executable_with_content(
            temp.path().join("syft"),
            br#"#!/bin/sh
case "$1" in
  --version) printf 'syft 1.57.1\n'; exit 0;;
  config) printf 'exclude: []\n'; exit 0;;
esac
for arg in "$@"; do
  case "$arg" in
    --override-default-catalogers=file-metadata-cataloger) printf '%s' '{"files":[]}'; exit 0;;
    cyclonedx-json=*) printf 'cataloged\n' >> "$0.calls"; printf '%s' '{"bomFormat":"CycloneDX","specVersion":"1.6","components":[{"type":"library","name":"example","version":"1.0.0"}]}' > "${arg#cyclonedx-json=}";;
    syft-json=*) printf '%s' '{"schema":{"version":"16.0.0"},"artifacts":[]}' > "${arg#syft-json=}";;
  esac
done
"#,
        );
        let configuration = active_syft_configuration(&tool).await.unwrap();
        let key = cache_fingerprint(&root, &tool, &configuration, &configuration)
            .await
            .unwrap();
        super::super::inventory_cache::write(
            &inventory_dir,
            &key,
            br#"{"bomFormat":"CycloneDX","specVersion":"1.6","components":[null]}"#,
            br#"{"schema":{"version":"16.0.0"},"artifacts":[]}"#,
        )
        .unwrap();

        let repaired = inventory_repository(&root, &tool, Some(&cache_dir))
            .await
            .unwrap();
        assert_eq!(repaired.packages.len(), 1);
        assert_eq!(repaired.packages[0].name, "example");
        let reused = inventory_repository(&root, &tool, Some(&cache_dir))
            .await
            .unwrap();
        assert_eq!(reused.packages, repaired.packages);
        assert_eq!(
            fs::read_to_string(temp.path().join("syft.calls")).unwrap(),
            "cataloged\n"
        );
    }

    #[test]
    fn parses_medium_findings_from_grype_json() {
        let payload = br#"
        {
          "matches": [
            {
              "vulnerability": {
                "id": "CVE-2026-0001",
                "severity": "MEDIUM",
                "fix": { "versions": ["1.2.3"] },
                "description": "demo issue",
                "dataSource": "https://example.test/advisory"
              },
              "artifact": {
                "name": "demo-package",
                "version": "1.0.0",
                "type": "npm",
                "purl": "pkg:npm/demo-package@1.0.0"
              }
            }
          ]
        }
        "#;

        let findings = parse_grype_json(payload).expect("grype json parses");

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Medium);
        assert_eq!(findings[0].advisory_id, "CVE-2026-0001");
    }

    #[test]
    fn parses_cyclonedx_components_to_packages() {
        let payload = br#"
        {
          "bomFormat": "CycloneDX",
          "specVersion": "1.7",
          "components": [
            {
              "type": "library",
              "name": "demo-package",
              "version": "1.0.0",
              "purl": "pkg:npm/demo-package@1.0.0"
            },
            {
              "type": "application",
              "name": "root-app",
              "version": "0.1.0"
            },
            {
              "type": "library",
              "name": "left-pad",
              "properties": [
                { "name": "syft:package:type", "value": "npm" }
              ]
            }
          ]
        }
        "#;

        let (packages, ..) = parse_cyclonedx_inventory(payload).expect("cyclonedx JSON parses");
        let demo = packages
            .iter()
            .find(|package| package.name == "demo-package")
            .expect("demo package");
        let left_pad = packages
            .iter()
            .find(|package| package.name == "left-pad")
            .expect("left-pad package");

        assert_eq!(packages.len(), 2);
        assert_eq!(demo.package_type.as_deref(), Some("npm"));
        assert_eq!(left_pad.package_type.as_deref(), Some("npm"));
    }

    #[test]
    fn parses_and_ranks_bounded_syft_remediation_evidence() {
        let root = tempfile::TempDir::new().expect("source root");
        let payload = format!(
            r#"{{
              "schema": {{ "version": "16.0.0" }},
              "artifacts": [
                {{ "id": "pkg-a", "name": "router", "version": "1.0.0", "type": "npm", "purl": "pkg:npm/router@1.0.0",
                   "locations": [{{ "path": "node_modules/router/index.js" }}, {{ "path": "package-lock.json" }}, {{ "path": "package.json" }}, {{ "path": "../secret" }}] }},
                {{ "id": "pkg-root", "name": "app", "version": "1.0.0", "type": "npm", "purl": "pkg:npm/app@1.0.0", "locations": [{{ "path": "{}" }}] }}
              ],
              "artifactRelationships": [{{ "parent": "pkg-a", "child": "pkg-root", "type": "dependency-of" }}]
            }}"#,
            root.path().join("package.json").display()
        );
        let artifacts = parse_syft_remediation_evidence(payload.as_bytes(), root.path())
            .expect("Syft evidence");
        let router = &artifacts[0];
        assert_eq!(router.locations[0].0, "package.json");
        assert_eq!(router.locations[0].1, RemediationLocationKind::Manifest);
        assert_eq!(router.locations.len(), 3);
        assert_eq!(router.relationship, RemediationRelationship::Transitive);
        assert_eq!(router.parent_package.as_deref(), Some("app"));
    }

    #[test]
    fn classifies_nested_npm_manifests_and_lockfiles_and_caps_locations() {
        let root = tempfile::TempDir::new().expect("source root");
        let mut locations = vec![
            serde_json::json!({ "path": "workspace/apps/web/npm-shrinkwrap.json" }),
            serde_json::json!({ "path": "workspace/apps/web/package-lock.json" }),
            serde_json::json!({ "path": "workspace/apps/web/package.json" }),
            serde_json::json!({ "path": "workspace/pnpm-lock.yaml" }),
            serde_json::json!({ "path": "workspace/yarn.lock" }),
        ];
        locations
            .extend((0..40).map(
                |index| serde_json::json!({ "path": format!("node_modules/pkg/{index}.js") }),
            ));
        let payload = serde_json::to_vec(&serde_json::json!({
            "schema": { "version": "16.0.0" },
            "artifacts": [{
                "id": "pkg-a", "name": "router", "version": "1.0.0", "type": "npm",
                "locations": locations
            }]
        }))
        .expect("serialize fixture");

        let artifacts = parse_syft_remediation_evidence(&payload, root.path()).expect("evidence");
        let locations = &artifacts[0].locations;
        assert_eq!(locations.len(), MAX_LOCATIONS_PER_ARTIFACT);
        assert_eq!(locations[0].0, "workspace/apps/web/package.json");
        assert_eq!(locations[0].1, RemediationLocationKind::Manifest);
        assert!(locations.iter().any(|(path, kind)| {
            path == "workspace/apps/web/package-lock.json"
                && *kind == RemediationLocationKind::Lockfile
        }));
        assert!(locations.iter().any(|(path, kind)| {
            path == "workspace/pnpm-lock.yaml" && *kind == RemediationLocationKind::Lockfile
        }));
        assert!(locations.iter().any(|(path, kind)| {
            path == "workspace/apps/web/npm-shrinkwrap.json"
                && *kind == RemediationLocationKind::Lockfile
        }));
    }

    #[test]
    fn rejects_unsafe_scanner_paths_and_unsupported_evidence() {
        let root = tempfile::TempDir::new().expect("source root");
        assert_eq!(normalize_scanner_path(root.path(), "../secret"), None);
        assert_eq!(normalize_scanner_path(root.path(), "/etc/passwd"), None);
        assert_eq!(
            normalize_scanner_path(root.path(), "safe/package.json"),
            Some("safe/package.json".into())
        );
        assert!(parse_syft_remediation_evidence(br#"{"artifacts": []}"#, root.path()).is_err());
        assert!(parse_syft_remediation_evidence(b"not json", root.path()).is_err());
    }

    #[test]
    fn rejects_oversized_syft_graphs_before_normalization() {
        let root = tempfile::TempDir::new().expect("source root");
        let artifact_payload = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": "16.0.0",
            "artifacts": vec![Value::Null; MAX_ARTIFACTS + 1]
        }))
        .expect("serialize artifacts");
        assert!(parse_syft_remediation_evidence(&artifact_payload, root.path()).is_err());

        let relationship_payload = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": "16.0.0",
            "artifacts": [{ "id": "pkg", "name": "pkg" }],
            "relationships": vec![Value::Null; MAX_RELATIONSHIPS + 1]
        }))
        .expect("serialize relationships");
        assert!(parse_syft_remediation_evidence(&relationship_payload, root.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn stores_symlink_like_scanner_locations_only_as_untrusted_relative_evidence() {
        use std::os::unix::fs::symlink;

        let root = tempfile::TempDir::new().expect("source root");
        let outside = tempfile::TempDir::new().expect("outside root");
        fs::write(outside.path().join("package.json"), "{}").expect("outside manifest");
        symlink(outside.path(), root.path().join("linked")).expect("symlink");

        assert_eq!(
            normalize_scanner_path(root.path(), "linked/package.json"),
            Some("linked/package.json".into())
        );
        assert_eq!(
            normalize_scanner_path(
                root.path(),
                outside
                    .path()
                    .join("package.json")
                    .to_string_lossy()
                    .as_ref()
            ),
            None
        );
    }

    #[test]
    fn correlates_by_purl_and_keeps_ambiguous_identity_unavailable() {
        let findings = parse_grype_json(br#"{
          "matches": [{"vulnerability":{"id":"CVE-1","severity":"HIGH"},"artifact":{"name":"router","version":"1.0.0","type":"npm","purl":"pkg:npm/router@1.0.0"}}]
        }"#).expect("finding");
        let artifact = SyftArtifactEvidence {
            name: "router".into(),
            version: Some("1.0.0".into()),
            package_type: Some("npm".into()),
            package_url: Some("pkg:npm/router@1.0.0".into()),
            locations: vec![("package.json".into(), RemediationLocationKind::Manifest)],
            relationship: RemediationRelationship::Unknown,
            parent_package: None,
        };
        let evidence = correlate_remediation_evidence(&findings, std::slice::from_ref(&artifact));
        assert_eq!(evidence[0].relative_path.as_deref(), Some("package.json"));
        assert_eq!(evidence[0].quality, RemediationEvidenceQuality::Partial);
        assert_eq!(evidence[0].relationship, RemediationRelationship::Unknown);

        let duplicate = SyftArtifactEvidence {
            locations: vec![(
                "nested/package.json".into(),
                RemediationLocationKind::Manifest,
            )],
            ..artifact.clone()
        };
        let ambiguous = correlate_remediation_evidence(&findings, &[artifact, duplicate]);
        assert_eq!(
            ambiguous[0].quality,
            RemediationEvidenceQuality::Unavailable
        );
        assert_eq!(ambiguous[0].relative_path, None);
    }

    #[test]
    fn duplicate_fallback_identities_remain_unavailable_without_a_purl() {
        let findings = parse_grype_json(br#"{
          "matches": [{"vulnerability":{"id":"CVE-1","severity":"HIGH"},"artifact":{"name":"router","version":"1.0.0","type":"npm"}}]
        }"#).expect("finding");
        let artifact = SyftArtifactEvidence {
            name: "router".into(),
            version: Some("1.0.0".into()),
            package_type: Some("npm".into()),
            package_url: Some("pkg:npm/router@1.0.0?workspace=one".into()),
            locations: vec![(
                "workspace/one/package.json".into(),
                RemediationLocationKind::Manifest,
            )],
            relationship: RemediationRelationship::Unknown,
            parent_package: None,
        };
        let duplicate = SyftArtifactEvidence {
            package_url: Some("pkg:npm/router@1.0.0?workspace=two".into()),
            locations: vec![(
                "workspace/two/package.json".into(),
                RemediationLocationKind::Manifest,
            )],
            ..artifact.clone()
        };

        let evidence = correlate_remediation_evidence(&findings, &[artifact, duplicate]);
        assert_eq!(evidence[0].quality, RemediationEvidenceQuality::Unavailable);
        assert_eq!(evidence[0].relative_path, None);
    }

    #[test]
    fn proves_flock_litellm_is_direct_from_pep621_manifest() {
        let root = tempfile::TempDir::new().expect("source root");
        fs::write(
            root.path().join("pyproject.toml"),
            r#"[project]
name = "flock"
dependencies = [
  "dspy==3.2.1",
  "litellm==1.83.14",
]
"#,
        )
        .expect("manifest");
        let findings = parse_grype_json(br#"{
          "matches": [{"vulnerability":{"id":"GHSA-4xpc-pv4p-pm3w","severity":"CRITICAL","fix":{"versions":["1.84.0"]}},"artifact":{"name":"litellm","version":"1.83.14","type":"python","purl":"pkg:pypi/litellm@1.83.14"}}]
        }"#).expect("finding");
        let mut evidence = unavailable_remediation_evidence(&findings);

        apply_supported_manifest_evidence(root.path(), &findings, &mut evidence);

        assert_eq!(evidence[0].relative_path.as_deref(), Some("pyproject.toml"));
        assert_eq!(
            evidence[0].location_kind,
            Some(RemediationLocationKind::Manifest)
        );
        assert_eq!(evidence[0].relationship, RemediationRelationship::Direct);
        assert_eq!(
            evidence[0].source,
            RemediationEvidenceSource::SupportedManifest
        );
        assert_eq!(evidence[0].quality, RemediationEvidenceQuality::Current);
        assert_eq!(evidence[0].location_count, 1);
    }

    #[test]
    fn normalizes_pep503_names_but_does_not_guess_from_other_tables() {
        let root = tempfile::TempDir::new().expect("source root");
        fs::write(
            root.path().join("pyproject.toml"),
            r#"[project]
dependencies = ["My_Package[cli] >= 1.0"]

[tool.poetry.dependencies]
litellm = "1.83.14"
"#,
        )
        .expect("manifest");
        let declarations = parse_supported_manifest_declarations(root.path());

        assert_eq!(
            declarations,
            vec![SupportedManifestDeclaration {
                normalized_package_name: "my-package".into(),
                relative_path: "pyproject.toml".into(),
            }]
        );
        assert_eq!(
            normalize_python_package_name("my.package"),
            Some("my-package".into())
        );
    }

    #[test]
    fn malformed_pep621_dependencies_remain_unproven() {
        let root = tempfile::TempDir::new().expect("source root");
        fs::write(
            root.path().join("pyproject.toml"),
            "[project]\ndependencies = [litellm]\n",
        )
        .expect("manifest");
        assert!(parse_supported_manifest_declarations(root.path()).is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn old_syft_falls_back_once_and_removes_private_outputs() {
        let source = tempfile::TempDir::new().expect("source");
        let temp_parent = tempfile::TempDir::new().expect("temp parent");
        let tool = write_executable_with_content(
            source.path().join("syft-old"),
            b"#!/bin/sh\nfor arg in \"$@\"; do\n  case \"$arg\" in syft-json=*) exit 2;; esac\ndone\nprintf '%s' '{\"components\":[]}'\n",
        );

        let (cyclonedx, companion) = capture_syft_outputs_with_tool(
            source.path(),
            &tool,
            Duration::from_secs(2),
            Some(temp_parent.path()),
            None,
        )
        .await
        .expect("legacy fallback");
        assert_eq!(cyclonedx, br#"{"components":[]}"#);
        assert_eq!(companion, None);
        assert_directory_empty(temp_parent.path());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn private_syft_outputs_are_removed_after_success_failure_and_timeout() {
        let source = tempfile::TempDir::new().expect("source");
        let temp_parent = tempfile::TempDir::new().expect("temp parent");
        let success = write_executable_with_content(
            source.path().join("syft-success"),
            b"#!/bin/sh\nfor arg in \"$@\"; do\n case \"$arg\" in\n  cyclonedx-json=*) target=${arg#cyclonedx-json=}; printf '%s' '{\"components\":[]}' > \"$target\";;\n  syft-json=*) target=${arg#syft-json=}; printf '%s' '{\"schemaVersion\":\"16.0.0\",\"artifacts\":[]}' > \"$target\";;\n esac\ndone\n",
        );
        let (_, companion) = capture_syft_outputs_with_tool(
            source.path(),
            &success,
            Duration::from_secs(2),
            Some(temp_parent.path()),
            None,
        )
        .await
        .expect("multi-output success");
        assert!(companion.is_some());
        assert_directory_empty(temp_parent.path());

        let failure = write_executable_with_content(
            source.path().join("syft-failure"),
            b"#!/bin/sh\nexit 3\n",
        );
        assert!(capture_syft_outputs_with_tool(
            source.path(),
            &failure,
            Duration::from_secs(2),
            Some(temp_parent.path()),
            None,
        )
        .await
        .is_err());
        assert_directory_empty(temp_parent.path());

        let slow =
            write_executable_with_content(source.path().join("syft-slow"), b"#!/bin/sh\nsleep 2\n");
        assert!(capture_syft_outputs_with_tool(
            source.path(),
            &slow,
            Duration::from_millis(25),
            Some(temp_parent.path()),
            None,
        )
        .await
        .is_err());
        assert_directory_empty(temp_parent.path());
    }

    #[tokio::test]
    async fn captures_only_bounded_git_revision_and_dirty_state() {
        let repo = tempfile::TempDir::new().expect("git repo");
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "qa@example.invalid"],
            vec!["config", "user.name", "CVEC QA"],
        ] {
            assert!(Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(args)
                .status()
                .expect("git")
                .success());
        }
        let unborn = capture_local_scan_source(repo.path()).await;
        assert_eq!(unborn.snapshot.revision, None);
        assert_eq!(unborn.snapshot.dirty, Some(false));
        fs::write(repo.path().join("manifest-with-sensitive-name.json"), "{}").expect("file");
        assert!(Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["add", "."])
            .status()
            .expect("add")
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["commit", "--no-gpg-sign", "-q", "-m", "initial"])
            .status()
            .expect("commit")
            .success());
        let initial = capture_local_scan_source(repo.path()).await;
        assert!(initial.snapshot.revision.is_some());
        assert_eq!(initial.snapshot.dirty, Some(false));
        assert!(unborn.verify(repo.path()).await.is_err());
        initial
            .verify(repo.path())
            .await
            .expect("unchanged checkout");

        assert!(Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args([
                "commit",
                "--allow-empty",
                "--no-gpg-sign",
                "-q",
                "-m",
                "new revision"
            ])
            .status()
            .expect("commit")
            .success());
        let clean = capture_local_scan_source(repo.path()).await;
        assert_eq!(initial.snapshot.dirty, clean.snapshot.dirty);
        assert!(initial.verify(repo.path()).await.is_err());

        fs::write(
            repo.path().join("manifest-with-sensitive-name.json"),
            "changed",
        )
        .expect("change");
        let dirty = capture_local_source_snapshot(repo.path()).await;
        assert_eq!(dirty.dirty, Some(true));
        assert!(clean.verify(repo.path()).await.is_err());
        let serialized = serde_json::to_string(&dirty).expect("serialize snapshot");
        assert!(!serialized.contains("manifest-with-sensitive-name"));
        assert!(!serialized.contains(repo.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn resolve_tool_prefers_trusted_directory_over_path() {
        let trusted = tempfile::TempDir::new().expect("trusted tempdir");
        let path = tempfile::TempDir::new().expect("path tempdir");
        let tool_name = "cvec-test-tool";
        let trusted_tool = write_executable(trusted.path().join(tool_name));
        let path_tool = write_executable(path.path().join(tool_name));

        let resolved = resolve_tool_from_path_sets(
            tool_name,
            vec![path.path().to_path_buf()],
            vec![trusted.path().to_path_buf()],
        );

        assert_eq!(resolved.as_deref(), Some(trusted_tool.as_path()));
        assert_ne!(resolved.as_deref(), Some(path_tool.as_path()));
    }

    #[test]
    fn qa_trusted_tool_dirs_require_an_explicit_absolute_override() {
        let defaults = TRUSTED_TOOL_DIRS
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();

        assert_eq!(resolve_trusted_tool_dirs(None), defaults);
        assert_eq!(
            resolve_trusted_tool_dirs(Some(OsString::from("relative-bin"))),
            defaults
        );
        assert_eq!(
            resolve_trusted_tool_dirs(Some(OsString::from("/tmp/cvec-no-scanners"))),
            vec![PathBuf::from("/tmp/cvec-no-scanners")]
        );
    }

    #[test]
    fn resolve_tool_ignores_relative_path_entries() {
        let absolute = tempfile::TempDir::new().expect("absolute tempdir");
        let tool_name = "cvec-test-tool";
        let absolute_tool = write_executable(absolute.path().join(tool_name));

        let resolved = resolve_tool_from_path_sets(
            tool_name,
            vec![PathBuf::from("relative-bin"), absolute.path().to_path_buf()],
            Vec::<PathBuf>::new(),
        );

        assert_eq!(resolved.as_deref(), Some(absolute_tool.as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_tool_rejects_non_executable_files() {
        let path = tempfile::TempDir::new().expect("path tempdir");
        let tool_name = "cvec-test-tool";
        fs::write(path.path().join(tool_name), b"not executable").expect("write tool");

        let resolved = resolve_tool_from_path_sets(
            tool_name,
            vec![path.path().to_path_buf()],
            Vec::<PathBuf>::new(),
        );

        assert!(resolved.is_none());
    }

    fn write_executable(path: PathBuf) -> PathBuf {
        write_executable_with_content(path, b"#!/bin/sh\nexit 0\n")
    }

    fn write_executable_with_content(path: PathBuf, contents: &[u8]) -> PathBuf {
        fs::write(&path, contents).expect("write tool");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&path).expect("tool metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("set executable bit");
        }

        path
    }

    fn assert_directory_empty(path: &Path) {
        assert!(fs::read_dir(path)
            .expect("read temp parent")
            .next()
            .is_none());
    }
}
