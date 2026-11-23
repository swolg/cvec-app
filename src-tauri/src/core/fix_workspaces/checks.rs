use super::super::delta::DetectedFindings;
use super::*;
use std::time::Duration;
use tokio::process::Command;

pub(super) fn actions(root: &Path) -> Vec<String> {
    files::SourcePolicy::new(root)
        .map(|policy| actions_with_policy(&policy))
        .unwrap_or_default()
}

pub(super) fn actions_with_policy(policy: &files::SourcePolicy) -> Vec<String> {
    let mut result = Vec::new();
    if let Ok(content) = policy.read("package.json") {
        if let Ok(manifest) = serde_json::from_str::<Value>(&content) {
            if let Some(scripts) = manifest.get("scripts").and_then(Value::as_object) {
                for name in ["test:run", "test", "check", "build", "lint", "typecheck"] {
                    if scripts.contains_key(name)
                        && !(name == "test" && scripts.contains_key("test:run"))
                    {
                        result.push(format!("npm:{name}"));
                    }
                }
            }
            result.push("npm:prepare".into());
            result.push("npm:lockfile".into());
        }
    }
    if policy.read("Cargo.toml").is_ok() {
        result.extend(
            [
                "cargo:test",
                "cargo:check",
                "cargo:clippy",
                "cargo:fetch",
                "cargo:update",
            ]
            .map(str::to_string),
        );
    }
    if policy.read("src-tauri/Cargo.toml").is_ok() {
        result.extend(
            [
                "tauri:test",
                "tauri:check",
                "tauri:clippy",
                "tauri:fetch",
                "tauri:update",
            ]
            .map(str::to_string),
        );
    }
    result
}

fn command_for(root: &Path, action: &str) -> Result<(String, Vec<String>, bool), Error> {
    if !actions(root).iter().any(|known| known == action) {
        return Err(Error::App("This check is not available in the selected worktree. Arbitrary commands are not supported.".into()));
    }
    let (kind, name) = action
        .split_once(':')
        .ok_or_else(|| Error::App("Unknown workspace check.".into()))?;
    if kind == "npm" {
        let args = match name {
            "prepare" => vec![
                if root.join("package-lock.json").exists() {
                    "ci"
                } else {
                    "install"
                },
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
            ],
            "lockfile" => vec![
                "install",
                "--package-lock-only",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
            ],
            _ => vec!["run", name],
        };
        return Ok((
            "npm".into(),
            args.into_iter().map(str::to_string).collect(),
            matches!(name, "prepare" | "lockfile"),
        ));
    }
    let mut args = vec![name.to_string()];
    if kind == "tauri" {
        args.extend(["--manifest-path".into(), "src-tauri/Cargo.toml".into()]);
    }
    if matches!(name, "test" | "check" | "clippy") {
        args.push("--offline".into());
    }
    Ok(("cargo".into(), args, matches!(name, "fetch" | "update")))
}

pub(super) fn sandbox_command(
    root: &Path,
    source: &Path,
    runtime: &Path,
    program: &str,
    args: &[String],
    network: bool,
) -> Result<Command, Error> {
    if !cfg!(target_os = "macos") || !Path::new("/usr/bin/sandbox-exec").is_file() {
        return Err(Error::App("Workspace checks require the macOS execution sandbox. No unsandboxed command was started.".into()));
    }
    let parent = runtime
        .parent()
        .ok_or_else(|| Error::App("Invalid workspace runtime root.".into()))?;
    let name = Path::new(
        runtime
            .file_name()
            .ok_or_else(|| Error::App("Invalid workspace runtime name.".into()))?,
    );
    let parent = parent.canonicalize()?;
    let runtime = parent.join(name);
    let anchor = super::secure_fs::AnchoredRoot::open(&parent)?.directory(name, true)?;
    anchor.directory(Path::new("home"), true)?;
    for config in ["npm-user.config", "npm-global.config"] {
        anchor.entry(Path::new(config), false)?.replace(b"", None)?;
    }
    let quote = |path: &Path| serde_json::to_string(&path.to_string_lossy()).unwrap_or_default();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::App("The local home directory is unavailable.".into()))?;
    let git_common = files::git_text(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let git_dir = files::git_text(root, &["rev-parse", "--path-format=absolute", "--git-dir"])?;
    // Closed by default, including native IPC and signalling other apps. Limit
    // process/OS allowances to what local npm/Cargo children require. See the
    // upstream Codex seatbelt_base_policy.sbpl for the same process boundary.
    let mut profile=format!("(version 1) (deny default) (allow process-exec process-fork) (allow signal (target same-sandbox)) (allow process-info* (target same-sandbox)) (allow sysctl-read mach-host* ipc-posix-sem) (allow mach-lookup (global-name \"com.apple.system.opendirectoryd.libinfo\") (global-name \"com.apple.PowerManagement.control\")) (allow file-ioctl (subpath \"/dev\")) (allow file-read-metadata) (allow file-read* (literal \"/\") (subpath \"/System\") (subpath \"/Library\") (subpath \"/Applications\") (subpath \"/usr\") (subpath \"/bin\") (subpath \"/sbin\") (subpath \"/opt\") (subpath \"/dev\") (subpath \"/private/etc\") (subpath \"/private/var/db\") (subpath {}) (subpath {}) (subpath {})) (allow file-write* (subpath {}) (subpath {}) (literal \"/dev/null\")) (deny file-write* (literal {}) (subpath {}) (subpath {}) (subpath {}) (literal {}) (literal {}) (literal {}))",
        quote(root),quote(&runtime),quote(&home.join(".rustup").join("toolchains")),quote(root),quote(&runtime),quote(&root.join(".git")),quote(Path::new(&git_common)),quote(Path::new(&git_dir)),quote(&root.join(".cargo")),quote(&root.join(".npmrc")),quote(&root.join(".gitmodules")),quote(&root.join(".gitattributes")));
    // npm and Cargo can fetch dependencies only through named preparation/update actions.
    if network {
        profile.push_str(" (allow network*) (allow mach-lookup (global-name \"com.apple.TrustEvaluationAgent\"))");
    } else {
        profile.push_str(" (deny network*)");
    }
    // Toolchain installation is never part of a workspace check.
    profile.push_str(&format!(
        " (allow file-read* (subpath {}) (literal {}))",
        quote(&home.join(".cargo").join("bin")),
        quote(&home.join(".rustup").join("settings.toml"))
    ));
    // Read only the selected installed toolchain, including Node's npm library.
    // This also supports installations outside Homebrew without opening the user's home.
    let mut path_entries = super::super::toolchain::search_dirs();
    for tool in if program == "npm" {
        vec!["node", "npm"]
    } else {
        vec![program]
    } {
        if let Some(binary) = super::super::toolchain::find_in(&path_entries, tool)
            .and_then(|candidate| candidate.canonicalize().ok())
        {
            profile.push_str(&format!(" (allow file-read* (literal {}))", quote(&binary)));
            if tool == "node" {
                if let Some(bin) = binary.parent() {
                    profile.push_str(&format!(" (allow file-read* (subpath {}))", quote(bin)));
                    path_entries.insert(0, bin.to_path_buf());
                    if let Some(prefix) = bin.parent() {
                        let lib = prefix.join("lib");
                        if lib.is_dir() {
                            profile.push_str(&format!(
                                " (allow file-read* (subpath {}))",
                                quote(&lib)
                            ));
                        }
                    }
                }
            }
            if tool == "npm" && binary.file_name().is_some_and(|name| name == "npm-cli.js") {
                if let Some(package) = binary.parent().and_then(Path::parent) {
                    profile.push_str(&format!(" (allow file-read* (subpath {}))", quote(package)));
                }
            }
        }
    }
    let mut command = Command::new("/usr/bin/sandbox-exec");
    profile.push_str(&format!(
        " (deny file-read-data (subpath {}))",
        quote(source)
    ));
    // Child processes can mutate contents, never replace the roots which a
    // subsequent operation will trust. Prevent native UI/IPC escape through Apple events.
    profile.push_str(&format!(
        " (deny file-write-unlink (literal {}) (literal {})) (deny appleevent-send)",
        quote(root),
        quote(&runtime)
    ));
    command
        .args(["-p", &profile, "/usr/bin/env", program])
        .args(args)
        .current_dir(root)
        .env_clear()
        .env(
            "PATH",
            std::env::join_paths(path_entries)
                .map_err(|_| Error::App("The toolchain search path is invalid.".into()))?,
        )
        .env("HOME", runtime.join("home"))
        .env("TMPDIR", &runtime)
        .env("CI", "1")
        .env("NO_COLOR", "1")
        .env("npm_config_cache", runtime.join("npm-cache"))
        .env("npm_config_userconfig", runtime.join("npm-user.config"))
        .env("npm_config_globalconfig", runtime.join("npm-global.config"))
        .env("npm_config_update_notifier", "false")
        .env(
            "npm_config_ignore_scripts",
            if network { "true" } else { "false" },
        )
        .env("CARGO_HOME", runtime.join("cargo-home"))
        .env("RUSTUP_HOME", home.join(".rustup"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0");
    Ok(command)
}

impl FixSession {
    pub(super) fn execute_check(&self, action: &str) -> Result<FixCheck, Error> {
        let root = self.root()?;
        files::require_verifiable_source(&root)?;
        let before = files::digest(&root)?;
        let (program, args, network) = command_for(&root, action)?;
        if network {
            self.invalidate_evidence();
        }
        self.activity(&format!("Running {action} in the worktree…"))?;
        let owner = self.owner()?;
        let id = self.data.lock().expect("fix record").id.clone();
        let source = PathBuf::from(self.data.lock().expect("fix record").source_path.clone());
        let result = (|| {
            let command = sandbox_command(
                &root,
                &source,
                &owner.root.join(format!(".runtime-{id}")),
                &program,
                &args,
                network,
            )?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let output = runtime.block_on(super::super::process::output_cancellable(
                command,
                Duration::from_secs(600),
                128 * 1024,
                self.stopped.clone(),
            ))?;
            let success = output.status.success();
            let output_exit_code = output.status.code();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let unavailable = text.starts_with("sandbox-exec:") || text.contains("sandbox_apply:");
            let output = if super::super::companion_context::contains_secret(&text) {
                "Output withheld by the secret-exclusion policy.".into()
            } else {
                bounded(&text, 128 * 1024)
            };
            Ok::<_, Error>((
                if unavailable {
                    "unavailable"
                } else if success {
                    "passed"
                } else {
                    "failed"
                }
                .to_string(),
                output,
                output_exit_code,
            ))
        })();
        let after = files::digest(&root)?;
        let (status, output, exit_code) = match result {
            Ok(value) => value,
            Err(error) => (
                if self.stopped.load(Ordering::SeqCst) {
                    "interrupted"
                } else {
                    "unavailable"
                }
                .into(),
                bounded(&error.to_string(), 4096),
                None,
            ),
        };
        let check = FixCheck {
            action: action.into(),
            status,
            exit_code,
            output,
            digest: before.clone(),
            stale: before != after,
            finished_at: now(),
        };
        {
            let mut data = self.data.lock().expect("fix record");
            data.checks.push(check.clone());
            if data.checks.len() > 20 {
                data.checks.remove(0);
            }
        }
        self.publish()?;
        Ok(check)
    }

    pub(super) fn execute_rescan(&self) -> Result<FixRescan, Error> {
        let root = self.root()?;
        files::require_verifiable_source(&root)?;
        let before = files::digest(&root)?;
        self.activity("Scanning the isolated worktree with Syft and Grype…")?;
        let owner = self.owner()?;
        let cancelled = self.stopped.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (versions,output)=runtime.block_on(async {
            let versions=super::super::scanner::versions_cancellable(cancelled.clone()).await?;
            if cancelled.load(Ordering::SeqCst) { return Err(Error::App("Workspace rescan stopped.".into())); }
            let output=tokio::select! {
                result=super::super::scanner::scan_repository(&root,Some(&owner.scanner_cache))=>result,
                _=async {loop {if cancelled.load(Ordering::SeqCst){break;}tokio::time::sleep(Duration::from_millis(50)).await;}}=>Err(Error::App("Workspace rescan stopped.".into())),
            }?;
            Ok::<_,Error>((versions,output))
        })?;
        let after = files::digest(&root)?;
        let data = self.data.lock().expect("fix record");
        let comparable = match (data.baseline_scan_id, output.evidence.as_ref()) {
            (Some(run_id), Some(evidence)) if data.baseline_matched => owner
                .db
                .scan_scope_matches(data.repository_id, run_id, evidence)?,
            _ => false,
        };
        let detected = DetectedFindings::new(&output.findings);
        let contains = |finding: &FixFinding| {
            detected.contains(
                &finding.fingerprint,
                &finding.advisory_id,
                &finding.package_name,
                finding.package_type.as_deref(),
                finding.package_url.as_deref(),
            )
        };
        let not_detected = comparable.then(|| {
            data.baseline_findings
                .iter()
                .filter(|finding| !contains(finding))
                .count()
        });
        let target_detected = data
            .baseline_findings
            .iter()
            .find(|finding| Some(&finding.fingerprint) == data.finding_fingerprint.as_ref())
            .map(contains)
            .filter(|detected| *detected || comparable);
        drop(data);
        let scan = FixRescan {
            finding_count: output.findings.len(),
            findings: output
                .findings
                .into_iter()
                .take(500)
                .map(|finding| FixFinding {
                    fingerprint: finding.fingerprint,
                    advisory_id: finding.advisory_id,
                    package_name: finding.package_name,
                    version: finding.package_version,
                    package_type: finding.package_type,
                    package_url: finding.package_url,
                    fix_version: finding.fix_version,
                    severity: finding.severity.as_str().into(),
                })
                .collect(),
            not_detected,
            target_detected,
            digest: before.clone(),
            stale: before != after,
            scanned_at: now(),
            scanner_versions: format!("{} · {}", versions.syft, versions.grype),
        };
        self.data.lock().expect("fix record").rescan = Some(scan.clone());
        self.publish()?;
        Ok(scan)
    }
}
