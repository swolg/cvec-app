use super::*;
use crate::core::{companion_host::CompanionHostConfig, repo_discovery::DiscoveredRepo};
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};

mod live;

#[cfg(unix)]
#[test]
fn snapshot_retains_saved_evidence_when_verification_inventory_fails() {
    const NAME: &str = "core::fix_workspaces::tests::snapshot_retains_saved_evidence_when_verification_inventory_fails";
    if std::env::var_os("CVEC_WARNING_INVENTORY_TEST").is_none() {
        use std::os::unix::fs::PermissionsExt;
        let tools = tempfile::tempdir().unwrap();
        let wrapper = tools.path().join("git");
        fs::write(
            &wrapper,
            r#"#!/bin/sh
case " $* " in
 *" ls-files --stage -z "*)
  if [ -f "$CVEC_WARNING_INVENTORY_TEST" ]; then
   count=$(cat "$CVEC_WARNING_INVENTORY_TEST")
   count=$((count + 1))
   printf '%s' "$count" > "$CVEC_WARNING_INVENTORY_TEST"
   if [ $((count % 3)) -eq 0 ]; then exit 128; fi
  fi;;
esac
exec /usr/bin/git "$@"
"#,
        )
        .unwrap();
        fs::set_permissions(wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        let mut paths = vec![tools.path().to_path_buf()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([NAME, "--exact", "--nocapture"])
            .env("CVEC_WARNING_INVENTORY_TEST", tools.path().join("armed"))
            .env("PATH", std::env::join_paths(paths).unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let fixture = fixture();
    let initial = create(&fixture);
    let session = fixture.service.session(1, &initial.workspace.id).unwrap();
    session.data.lock().unwrap().messages.push(
        serde_json::from_value(json!({"role":"assistant","text":"Retained saved explanation"}))
            .unwrap(),
    );
    session.publish().unwrap();
    fs::write(
        PathBuf::from(std::env::var_os("CVEC_WARNING_INVENTORY_TEST").unwrap()),
        "0",
    )
    .unwrap();
    let snapshot = fixture
        .service
        .snapshot(1, &initial.workspace.id)
        .expect("A verification probe failure must not hide saved evidence");
    assert!(snapshot.source_error.is_some());
    assert!(snapshot.diff.is_none());
    assert!(snapshot.actions.is_empty());
    assert_eq!(
        snapshot.workspace.messages[0].text,
        "Retained saved explanation"
    );
    let archived = fixture
        .service
        .set_archived(1, &initial.workspace.id, true)
        .expect("Archiving must remain available when source verification fails");
    assert!(archived.workspace.archived_at.is_some());
    assert!(archived.source_error.is_some());
    assert_eq!(
        archived.workspace.messages[0].text,
        "Retained saved explanation"
    );
}

#[test]
fn missing_submodule_alias_cannot_be_replaced_by_a_source_file() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    let mut accepted = Vec::new();
    for (name, alias) in [("library", "LIBRARY"), ("caf\u{e9}", "cafe\u{301}")] {
        fs::create_dir(root.join(name)).unwrap();
        let aliases = root.join(alias).is_dir();
        fs::remove_dir(root.join(name)).unwrap();
        files::git(
            root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},{name}", snapshot.workspace.base_revision),
            ],
        )
        .unwrap();
        if aliases && files::write(root, alias, "excluded source").is_ok() {
            accepted.push(alias);
        }
    }
    assert!(
        accepted.is_empty(),
        "Created source files at missing submodule aliases: {accepted:?}"
    );
}

#[test]
fn source_policy_reserves_canonical_names_before_creating_any_parents() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    for name in ["nested/library", "stra\u{df}e"] {
        files::git(
            root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},{name}", snapshot.workspace.base_revision),
            ],
        )
        .unwrap();
    }
    for path in [
        "nested//LIBRARY",
        "nested/./LiBrArY/child.ts",
        "STRASSE",
        ".aw\u{17f}/credentials",
        ".doc\u{212a}er/config.json",
        ".gitattribute\u{17f}",
    ] {
        assert!(
            files::write(root, path, "forbidden").is_err(),
            "Created excluded alias {path}"
        );
    }
    assert!(!root.join("nested").exists());
    assert!(!root.join(".aws").exists());
    files::write(root, "nested-library/allowed.ts", "source").unwrap();
    assert_eq!(
        files::read(root, "nested-library/allowed.ts").unwrap(),
        "source"
    );
}

#[cfg(unix)]
#[test]
fn anchored_submodule_exclusion_checks_the_opened_directory_identity() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    fs::create_dir(root.join("library")).unwrap();
    fs::write(root.join("library/module.ts"), "preserved").unwrap();
    let relative = if root.join("LIBRARY").is_dir() {
        "LIBRARY/module.ts"
    } else {
        "library/module.ts"
    };
    let anchor = super::secure_fs::AnchoredRoot::open(root).unwrap();
    assert!(anchor
        .entry_excluding(Path::new(relative), false, &[PathBuf::from("library")])
        .is_err());
    assert_eq!(
        fs::read_to_string(root.join("library/module.ts")).unwrap(),
        "preserved"
    );
}

#[cfg(unix)]
#[test]
fn source_policy_does_not_follow_a_symlink_workspace_root() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    let alias = fixture._temp.path().join("root-alias");
    std::os::unix::fs::symlink(root, &alias).unwrap();
    assert!(
        files::read(&alias, "app.ts").is_err(),
        "Source policy followed a symlink root"
    );
    assert!(files::write(&alias, "created-through-alias.ts", "forbidden").is_err());
    assert!(!root.join("created-through-alias.ts").exists());
}

#[test]
fn submodule_aliases_cannot_read_write_delete_or_create_source() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    let revision = &snapshot.workspace.base_revision;
    for (name, alias) in [("library", "LIBRARY"), ("caf\u{e9}", "cafe\u{301}")] {
        files::git(
            root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{revision},{name}"),
            ],
        )
        .unwrap();
        fs::create_dir(root.join(name)).unwrap();
        let original = "export const protectedSource = 1;\n";
        fs::write(root.join(name).join("module.ts"), original).unwrap();
        assert!(files::read(root, &format!("{name}/module.ts")).is_err());
        if root.join(alias).is_dir() {
            let path = format!("{alias}/module.ts");
            assert!(files::read(root, &path).is_err(), "read through {alias}");
            assert!(
                files::write(root, &path, "changed").is_err(),
                "write through {alias}"
            );
            assert!(
                files::delete(root, &path).is_err(),
                "delete through {alias}"
            );
            assert!(files::SourcePolicy::new(root)
                .unwrap()
                .safe_path(&path, false)
                .is_err());
            assert_eq!(
                fs::read_to_string(root.join(name).join("module.ts")).unwrap(),
                original
            );
            fs::remove_dir_all(root.join(name)).unwrap();
            assert!(files::write(root, &format!("{alias}/new.ts"), "forbidden").is_err());
            assert!(!root.join(name).join("new.ts").exists());
        }
    }
    files::write(root, "root-source.ts", "export const safe = 1;\n").unwrap();
    assert!(files::read(root, "root-source.ts").is_ok());
}

#[cfg(unix)]
#[test]
fn source_inventory_git_processes_are_independent_of_matches() {
    const NAME: &str =
        "core::fix_workspaces::tests::source_inventory_git_processes_are_independent_of_matches";
    if std::env::var_os("CVEC_FILE_INVENTORY_TEST_CHILD").is_none() {
        use std::os::unix::fs::PermissionsExt;
        let tools = tempfile::tempdir().unwrap();
        let log = tools.path().join("git.log");
        let wrapper = tools.path().join("git");
        fs::write(&wrapper, "#!/bin/sh\nprintf 'git\\n' >> \"$CVEC_FILE_INVENTORY_TEST_LOG\"\nexec /usr/bin/git \"$@\"\n").unwrap();
        fs::set_permissions(wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        let mut paths = vec![tools.path().to_path_buf()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([NAME, "--exact", "--nocapture"])
            .env("CVEC_FILE_INVENTORY_TEST_CHILD", "1")
            .env("CVEC_FILE_INVENTORY_TEST_LOG", &log)
            .env("PATH", std::env::join_paths(paths).unwrap())
            .output()
            .unwrap();
        println!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    {
        let mut data = session.data.lock().unwrap();
        data.status = "working".into();
        data.provider_thread_id = Some("synthetic-provider".into());
        data.turn_id = Some("synthetic-turn".into());
    }
    let log = PathBuf::from(std::env::var_os("CVEC_FILE_INVENTORY_TEST_LOG").unwrap());
    let mut counts = Vec::new();
    for count in [200, 450] {
        for index in 0..count {
            fs::write(
                root.join(format!("inventory-{index:04}.ts")),
                "export const value = 1;\n",
            )
            .unwrap();
        }
        fs::write(&log, "").unwrap();
        let started = Instant::now();
        let result = FixSink::call(
            session.as_ref(),
            "cvec_workspace_list_files",
            &json!({"query":"inventory-"}),
            "synthetic-provider",
            "synthetic-turn",
        )
        .unwrap();
        let processes = fs::read_to_string(&log).unwrap().lines().count();
        println!(
            "inventory matches={count} Git_processes={processes} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        assert_eq!(result["files"].as_array().unwrap().len(), count.min(300));
        assert_eq!(result["omittedCount"], count.saturating_sub(300));
        assert!(
            processes <= 20,
            "{processes} Git processes for {count} matches"
        );
        counts.push(processes);
    }
    assert_eq!(counts[0], counts[1]);
}

#[test]
fn clean_repository_with_submodule_keeps_source_evidence_available() {
    let fixture = fixture();
    let revision = files::git_text(&fixture.source, &["rev-parse", "HEAD"]).unwrap();
    fs::write(
        fixture.source.join(".gitmodules"),
        "[submodule \"library\"]\n\tpath = library\n\turl = https://example.invalid/library.git\n",
    )
    .unwrap();
    fs::create_dir(fixture.source.join("library")).unwrap();
    files::git(&fixture.source, &["add", ".gitmodules"]).unwrap();
    files::git(
        &fixture.source,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{revision},library"),
        ],
    )
    .unwrap();
    files::git(
        &fixture.source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "Synthetic submodule",
        ],
    )
    .unwrap();
    assert!(files::git(&fixture.source, &["status", "--porcelain"])
        .unwrap()
        .is_empty());
    let snapshot = create(&fixture);
    assert!(
        snapshot.diff.is_some(),
        "Source evidence unavailable: {:?}",
        snapshot.source_error
    );
    assert!(snapshot
        .verification_warning
        .as_deref()
        .unwrap()
        .contains("Submodules"));
    assert!(snapshot.actions.is_empty());
    let root = Path::new(&snapshot.workspace.worktree_path);
    let before = files::digest(root).unwrap();
    assert!(files::write(root, "library/unsafe.ts", "unsafe").is_err());
    assert!(fixture
        .service
        .run_check(1, &snapshot.workspace.id, "npm:test:run")
        .is_err());
    assert!(fixture.service.rescan(1, &snapshot.workspace.id).is_err());
    files::write(root, "app.ts", "export const fixed = true;\n").unwrap();
    assert_ne!(before, files::digest(root).unwrap());
    let next = fixture.service.snapshot(1, &snapshot.workspace.id).unwrap();
    assert!(next
        .diff
        .unwrap()
        .files
        .iter()
        .any(|file| file.path == "app.ts"));
}

#[test]
fn file_to_directory_refactor_keeps_source_evidence_available() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    let before = files::digest(root).unwrap();
    files::delete(root, "app.ts").unwrap();
    files::write(root, "app.ts/module.ts", "export const fixed = true;\n").unwrap();
    let snapshot = fixture.service.snapshot(1, &snapshot.workspace.id).unwrap();
    assert!(
        snapshot.diff.is_some(),
        "Source evidence unavailable: {:?}",
        snapshot.source_error
    );
    let diff = snapshot.diff.unwrap();
    assert_ne!(before, diff.digest);
    assert!(diff
        .files
        .iter()
        .any(|file| file.path == "app.ts" && file.patch.contains("-export const fixed = false;")));
    assert!(diff.files.iter().any(|file| file.path == "app.ts/module.ts"
        && file.patch.contains("+export const fixed = true;")));
    let previous = diff.digest;
    files::write(root, "app.ts/module.ts", "export const fixed = 2;\n").unwrap();
    assert_ne!(previous, files::digest(root).unwrap());
}

#[test]
fn parallel_cached_rescans_finish_through_the_ipc_worker_lane() {
    const NAME: &str =
        "core::fix_workspaces::tests::parallel_cached_rescans_finish_through_the_ipc_worker_lane";
    if std::env::var_os("CVEC_FIX_RESCAN_TEST_RESCAN_CHILD").is_none() {
        let tools = tempfile::tempdir().unwrap();
        let log = tools.path().join("syft.log");
        let syft = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CVEC_FIX_RESCAN_TEST_SYFT_LOG"
if [ "$1" = "--version" ]; then printf 'syft 1.54.1\n'; exit 0; fi
if [ "$1" = "config" ]; then printf 'exclude: []\n'; exit 0; fi
for arg in "$@"; do
 case "$arg" in
  --override-default-catalogers=file-metadata-cataloger) printf '%s' '{"files":[]}'; exit 0;;
  cyclonedx-json=*) target=${arg#cyclonedx-json=}; printf '%s' '{"bomFormat":"CycloneDX","specVersion":"1.6","components":[]}' > "$target";;
  syft-json=*) target=${arg#syft-json=}; printf '%s' '{"schema":{"version":"16.0.0"},"artifacts":[]}' > "$target";;
 esac
done
"#;
        let grype = "#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'fixture-grype'; exit 0; fi\ncat >/dev/null\nprintf '%s' '{\"matches\":[]}'\n";
        for (name, script) in [("syft", syft), ("grype", grype)] {
            let path = tools.path().join(name);
            fs::write(&path, script).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args([NAME, "--exact", "--nocapture"])
            .env("CVEC_FIX_RESCAN_TEST_RESCAN_CHILD", "1")
            .env("CVEC_SCANNER_TOOL_DIRS", tools.path())
            .env("CVEC_FIX_RESCAN_TEST_SYFT_LOG", &log)
            .output()
            .unwrap();
        println!("{}", String::from_utf8_lossy(&output.stdout));
        println!("{}", String::from_utf8_lossy(&output.stderr));
        assert!(
            output.status.success(),
            "parallel cached rescans stalled in the IPC worker lane"
        );
        return;
    }
    let fixture = fixture();
    let first = create(&fixture).workspace.id;
    let second = create(&fixture).workspace.id;
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let control_service = fixture.service.clone();
    let control_id = first.clone();
    let control = executor
        .block_on(crate::runtime::run_rescan(move || {
            control_service.rescan(1, &control_id)
        }))
        .unwrap()
        .unwrap();
    assert!(control.workspace.rescan.is_some());
    fs::write(
        std::env::var_os("CVEC_FIX_RESCAN_TEST_SYFT_LOG").unwrap(),
        "",
    )
    .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let service_a = fixture.service.clone();
    let service_b = fixture.service.clone();
    let id_a = first.clone();
    let id_b = second.clone();
    let barrier_a = barrier.clone();
    let barrier_b = barrier.clone();
    let completed = executor.block_on(async {
        let a = crate::runtime::run_rescan(move || {
            barrier_a.wait();
            service_a.rescan(1, &id_a)
        });
        let b = crate::runtime::run_rescan(move || {
            barrier_b.wait();
            service_b.rescan(1, &id_b)
        });
        let joined = futures::future::join(a, b);
        tokio::pin!(joined);
        // Allow Git, SQLite and the scanner subprocesses to make progress on a
        // shared runner. A blocked worker still exercises the Stop path below.
        match tokio::time::timeout(Duration::from_secs(15), &mut joined).await {
            Ok((a, b)) => {
                assert!(a.unwrap().unwrap().workspace.rescan.is_some());
                assert!(b.unwrap().unwrap().workspace.rescan.is_some());
                true
            }
            Err(_) => {
                println!(
                    "PARALLEL_STATES {:?}",
                    fixture
                        .service
                        .list(1)
                        .unwrap()
                        .iter()
                        .map(|w| (&w.id, &w.status))
                        .collect::<Vec<_>>()
                );
                println!(
                    "SYFT_BEFORE_STOP {}",
                    fs::read_to_string(std::env::var_os("CVEC_FIX_RESCAN_TEST_SYFT_LOG").unwrap())
                        .unwrap()
                );
                fixture.service.stop(1, &first).unwrap();
                fixture.service.stop(1, &second).unwrap();
                let _cancelled = tokio::time::timeout(Duration::from_secs(3), &mut joined)
                    .await
                    .expect("Stop must release both stalled operations");
                false
            }
        }
    });
    fixture.service.shutdown();
    fixture.host.shutdown();
    assert!(
        completed,
        "a single cached rescan succeeded, but two simultaneous IPC rescans stalled until Stop"
    );
}

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    service: FixWorkspaces,
    host: CompanionHost,
    db: Database,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    fixture_with_temp(temp)
}

fn fixture_with_temp(temp: tempfile::TempDir) -> Fixture {
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    files::git(&source, &["init", "-q"]).unwrap();
    fs::write(source.join("app.ts"), "export const fixed = false;\n").unwrap();
    fs::write(
        source.join("package.json"),
        "{\"scripts\":{\"test:run\":\"node -e \\\"process.exit(0)\\\"\"}}",
    )
    .unwrap();
    fs::write(source.join(".gitignore"), "node_modules\ntarget\n").unwrap();
    files::git(&source, &["add", "--all"]).unwrap();
    // Synthetic fixture history only; no user-repository commits are created.
    files::git(
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-q",
            "-m",
            "Synthetic baseline",
        ],
    )
    .unwrap();
    let db = Database::new(temp.path().join("cvec.db")).unwrap();
    db.sync_repositories(&[DiscoveredRepo {
        name: "fixture".into(),
        path: source.clone(),
        remote_url: None,
        remote_host: None,
        provider: None,
    }])
    .unwrap();
    let python = Command::new("/usr/bin/which")
        .arg("python3")
        .output()
        .unwrap();
    let python = String::from_utf8(python.stdout).unwrap();
    let sandbox = temp.path().join("sandbox");
    fs::create_dir(&sandbox).unwrap();
    let host = CompanionHost::with_event_sink(
        db.clone(),
        CompanionHostConfig {
            executable_override: Some(PathBuf::from(python.trim())),
            executable_prefix_args: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/fake_codex_app_server.py")
                .to_string_lossy()
                .into_owned()],
            environment: Vec::new(),
            sandbox_root: sandbox,
            idle_timeout: Duration::from_secs(600),
            request_timeout: Duration::from_secs(3),
        },
        |_| {},
    );
    let service = FixWorkspaces::new(
        db.clone(),
        temp.path().join("workspaces"),
        temp.path().join("scanner-cache"),
        host.clone(),
        |_| {},
    )
    .unwrap();
    Fixture {
        _temp: temp,
        source,
        service,
        host,
        db,
    }
}

#[test]
#[ignore = "Opt-in real macOS execution sandbox check; no native app or provider is started"]
fn macos_sandbox_allows_worktree_and_denies_source_git_credentials_and_network() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let fixture = fixture_with_temp(tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap());
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    let runtime = fixture._temp.path().join("private-runtime");
    let source = fixture.source.join("app.ts");
    let private = fixture._temp.path().join("private-credential-fixture");
    fs::write(&private, "synthetic-private-value").unwrap();
    // All interpolated paths are generated fixture paths, never user shell input.
    let script=format!("printf allowed > sandbox-result; if printf bad > '{}' 2>/dev/null; then exit 41; fi; if printf bad > .git 2>/dev/null; then exit 42; fi; if cat '{}' >/dev/null 2>/dev/null; then exit 43; fi; if cat '{}' >/dev/null 2>/dev/null; then exit 44; fi; if ln '{}' sandbox-hardlink 2>/dev/null; then exit 45; fi; printf sandbox-ok",source.display(),source.display(),private.display(),private.display());
    let command = checks::sandbox_command(
        root,
        &fixture.source,
        &runtime,
        "/bin/sh",
        &["-c".into(), script],
        false,
    )
    .unwrap();
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let output = executor
        .block_on(crate::core::process::output_cancellable(
            command,
            Duration::from_secs(5),
            8192,
            Arc::new(AtomicBool::new(false)),
        ))
        .unwrap();
    assert!(
        output.status.success(),
        "status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("sandbox-ok"));
    assert_eq!(
        fs::read_to_string(source).unwrap(),
        "export const fixed = false;\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("sandbox-result")).unwrap(),
        "allowed"
    );
    let code="import socket,ctypes\ntry:\n socket.socket().bind(('127.0.0.1',0))\nexcept PermissionError:\n print('network-denied')\nelse:\n raise RuntimeError('network unexpectedly allowed')\nlib=ctypes.CDLL('/usr/lib/libSystem.B.dylib')\nbootstrap=ctypes.c_uint.in_dll(lib,'bootstrap_port')\nport=ctypes.c_uint()\nresult=lib.bootstrap_look_up(bootstrap,b'com.apple.cfprefsd.agent',ctypes.byref(port))\nif result==0:\n raise RuntimeError('private preferences IPC unexpectedly allowed')\nprint('ipc-denied')";
    let command = checks::sandbox_command(
        root,
        &fixture.source,
        &runtime,
        "/usr/bin/python3",
        &["-c".into(), code.into()],
        false,
    )
    .unwrap();
    let output = executor
        .block_on(crate::core::process::output_cancellable(
            command,
            Duration::from_secs(5),
            8192,
            Arc::new(AtomicBool::new(false)),
        ))
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("network-denied"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ipc-denied"));
    let checked = fixture
        .service
        .run_check(1, &snapshot.workspace.id, "npm:test:run")
        .unwrap();
    assert_eq!(
        checked.workspace.checks.last().unwrap().status,
        "passed",
        "{}",
        checked.workspace.checks.last().unwrap().output
    );
    files::write(
        root,
        "Cargo.toml",
        "[package]\nname = \"sandbox-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n",
    )
    .unwrap();
    files::write(
        root,
        "src/lib.rs",
        "#[test]\nfn fixture_works() { assert_eq!(2 + 2, 4); }\n",
    )
    .unwrap();
    let checked = fixture
        .service
        .run_check(1, &snapshot.workspace.id, "cargo:test")
        .unwrap();
    assert_eq!(
        checked.workspace.checks.last().unwrap().status,
        "passed",
        "{}",
        checked.workspace.checks.last().unwrap().output
    );
}

fn create(fixture: &Fixture) -> FixSnapshot {
    fixture.service.create(1, None, true, None).unwrap()
}
fn wait_ready(fixture: &Fixture, id: &str) {
    let data = wait_settled(fixture, id);
    assert_eq!(data.status, "ready", "{}", data.activity);
}

fn wait_settled(fixture: &Fixture, id: &str) -> FixWorkspace {
    let started = Instant::now();
    loop {
        let data = fixture
            .service
            .list(1)
            .unwrap()
            .into_iter()
            .find(|data| data.id == id)
            .unwrap();
        if !is_busy(&data.status) {
            return data;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{}",
            data.activity
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_saved_workspace(fixture: &Fixture, expected: &FixWorkspace) -> FixWorkspace {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let saved = fixture
            .db
            .load_fix_workspaces(expected.repository_id)
            .unwrap()
            .into_iter()
            .find(|workspace| workspace.id == expected.id)
            .unwrap();
        if saved.status == expected.status
            && saved.activity == expected.activity
            && saved.revision >= expected.revision
        {
            return saved;
        }
        // A terminal in-memory transition precedes publication's SQLite write.
        // Tests that require durable evidence must observe that write explicitly.
        assert!(Instant::now() < deadline, "workspace state was not saved");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn review_literal_git_paths_never_expand_into_protected_files() {
    let fixture = fixture();
    let magic_name = ":(glob)**";
    fs::write(fixture.source.join(magic_name), "benign baseline\n").unwrap();
    fs::write(fixture.source.join(".env"), "old synthetic sentinel\n").unwrap();
    files::git(&fixture.source, &["add", "--all"]).unwrap();
    files::git(
        &fixture.source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "Synthetic pathspec baseline",
        ],
    )
    .unwrap();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    files::write(root, magic_name, "eligible change\n").unwrap();
    fs::write(root.join(".env"), "CVEC_EXCLUDED_FILE_CANARY\n").unwrap();
    let diff = fixture
        .service
        .snapshot(1, &snapshot.workspace.id)
        .unwrap()
        .diff
        .unwrap();
    assert!(diff.omitted_files >= 1);
    assert!(diff.files.iter().any(|file| file.path == magic_name));
    assert!(diff
        .files
        .iter()
        .all(|file| !file.patch.contains("CVEC_EXCLUDED_FILE_CANARY")
            && !file.patch.contains("a/.env")));
}

#[test]
#[cfg(unix)]
fn review_executable_mode_changes_invalidate_evidence() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    let digest = snapshot.diff.as_ref().unwrap().digest.clone();
    {
        let mut data = session.data.lock().unwrap();
        data.checks.push(FixCheck {
            action: "npm:test:run".into(),
            status: "passed".into(),
            exit_code: Some(0),
            output: "synthetic prior check".into(),
            digest: digest.clone(),
            stale: false,
            finished_at: now(),
        });
        data.rescan = Some(FixRescan {
            findings: vec![],
            finding_count: 0,
            not_detected: None,
            target_detected: None,
            digest,
            stale: false,
            scanned_at: now(),
            scanner_versions: "fixture".into(),
        });
    }
    let path = root.join("app.ts");
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    fs::set_permissions(&path, fs::Permissions::from_mode(mode ^ 0o111)).unwrap();
    let changed = fixture.service.snapshot(1, &snapshot.workspace.id).unwrap();
    assert!(changed.workspace.checks[0].stale);
    assert!(changed.workspace.rescan.unwrap().stale);
    assert!(changed
        .diff
        .unwrap()
        .files
        .iter()
        .any(|file| file.path == "app.ts" && file.patch.contains("mode")));
}

#[test]
fn review_invalid_saved_records_do_not_hide_valid_workspaces() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let connection = rusqlite::Connection::open(fixture.db.path()).unwrap();
    let mut originals = Vec::new();
    for variant in 0..5 {
        let mut invalid = snapshot.workspace.clone();
        invalid.id = format!("{:024x}", variant + 1);
        match variant {
            0 => invalid.status = "published".into(),
            1 => invalid.messages.push(FixMessage {
                provider: None,
                role: "tool".into(),
                text: "unsupported saved role".into(),
            }),
            2 => invalid.revision = 9_007_199_254_740_992,
            3 => invalid.activity = "x".repeat(4097),
            _ => invalid.messages.push(FixMessage {
                provider: None,
                role: "assistant".into(),
                text: "x".repeat(32_001),
            }),
        }
        let original = serde_json::to_string(&invalid).unwrap();
        connection.execute("INSERT INTO fix_workspaces(id,repository_id,updated_at,record_json) VALUES (?1,1,'now',?2)", rusqlite::params![invalid.id, original]).unwrap();
        originals.push((invalid.id, original));
    }
    let recovered = FixWorkspaces::new(
        fixture.db.clone(),
        fixture.service.0.root.clone(),
        fixture.service.0.scanner_cache.clone(),
        fixture.host.clone(),
        |_| {},
    )
    .unwrap();
    assert_eq!(recovered.list(1).unwrap().len(), 1);
    for (id, original) in originals {
        assert_eq!(
            connection
                .query_row(
                    "SELECT record_json FROM fix_workspaces WHERE id=?1",
                    [&id],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            original
        );
    }
}

#[test]
fn review_invalid_updates_preserve_the_previous_saved_record() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let mut invalid = snapshot.workspace.clone();
    invalid.checks.push(FixCheck {
        action: "npm:test:run".into(),
        status: "successful".into(),
        exit_code: Some(0),
        output: "unsupported check state".into(),
        digest: snapshot.diff.unwrap().digest,
        stale: false,
        finished_at: now(),
    });
    assert!(fixture.db.save_fix_workspace(&invalid).is_err());
    let saved = fixture.db.load_fix_workspaces(1).unwrap();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].checks.is_empty());
}

#[test]
fn worktree_changes_and_actual_diff_preserve_source_and_head() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    assert_ne!(root, fixture.source);
    files::write(root, "app.ts", "export const fixed = true;\n").unwrap();
    files::write(root, "new.ts", "export const added = true;\n").unwrap();
    let snapshot = fixture.service.snapshot(1, &snapshot.workspace.id).unwrap();
    assert_eq!(snapshot.diff.as_ref().unwrap().files.len(), 2);
    assert!(snapshot
        .diff
        .as_ref()
        .unwrap()
        .files
        .iter()
        .any(|file| file.path == "new.ts" && file.patch.contains("/dev/null")));
    assert_eq!(
        fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
        "export const fixed = false;\n"
    );
    assert_eq!(
        files::git_text(root, &["rev-parse", "HEAD"]).unwrap(),
        snapshot.workspace.base_revision
    );
    assert!(files::git(&fixture.source, &["status", "--porcelain"])
        .unwrap()
        .is_empty());
}

#[test]
fn git_filters_cannot_execute_during_creation_inspection_or_diff() {
    let fixture = fixture();
    fs::write(
        fixture.source.join(".gitattributes"),
        "*.ts filter=fixture\n",
    )
    .unwrap();
    files::git(&fixture.source, &["add", ".gitattributes"]).unwrap();
    files::git(
        &fixture.source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "Synthetic filter attributes",
        ],
    )
    .unwrap();
    let marker = fixture._temp.path().join("filter-ran");
    let script = format!("touch '{}'; cat", marker.display());
    for key in ["filter.fixture.clean", "filter.fixture.smudge"] {
        files::git(&fixture.source, &["config", key, &script]).unwrap();
    }
    files::git(
        &fixture.source,
        &["config", "filter.fixture.required", "true"],
    )
    .unwrap();
    let snapshot = create(&fixture);
    files::write(
        Path::new(&snapshot.workspace.worktree_path),
        "app.ts",
        "export const fixed = true;\n",
    )
    .unwrap();
    let inspected = fixture.service.snapshot(1, &snapshot.workspace.id).unwrap();
    assert_eq!(inspected.diff.unwrap().files.len(), 1);
    assert!(!marker.exists());
}

#[test]
fn creation_requires_confirmation_clean_local_checkout_and_matching_repository() {
    let fixture = fixture();
    assert!(fixture.service.create(1, None, false, None).is_err());
    fs::write(fixture.source.join("uncommitted.ts"), "preserve this").unwrap();
    let error = fixture
        .service
        .create(1, None, true, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("uncommitted"));
    assert_eq!(
        fs::read_to_string(fixture.source.join("uncommitted.ts")).unwrap(),
        "preserve this"
    );
    fs::remove_file(fixture.source.join("uncommitted.ts")).unwrap();
    let snapshot = create(&fixture);
    assert!(fixture.service.snapshot(2, &snapshot.workspace.id).is_err());
    assert!(fixture
        .service
        .create(1, Some("missing-finding".into()), true, None)
        .is_err());
}

#[test]
fn file_policy_denies_secrets_metadata_traversal_and_symlinks() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    for path in [
        "../outside.txt",
        "/tmp/outside.txt",
        ".git",
        ".git/config",
        ".env",
        ".npmrc",
        ".NPMRC",
        ".CaRgO/config.toml",
        ".cargo/config.toml",
        ".agents/rules",
        "a/../../outside",
        "node_modules/package/index.js",
    ] {
        assert!(files::write(root, path, "bad").is_err(), "{path}");
    }
    fs::write(
        root.join("secret.ts"),
        "const api_key = 'abcdefghijklmnop';",
    )
    .unwrap();
    assert!(files::read(root, "secret.ts").is_err());
    let diff = files::diff(root, &snapshot.workspace.base_revision).unwrap();
    assert!(diff.omitted_files > 0);
    assert!(!serde_json::to_string(&diff)
        .unwrap()
        .contains("abcdefghijklmnop"));
    #[cfg(unix)]
    {
        let outside = fixture._temp.path().join("outside.txt");
        fs::write(&outside, "keep").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link.ts")).unwrap();
        assert!(files::write(root, "link.ts", "overwrite").is_err());
        assert!(files::read(root, "link.ts").is_err());
        fs::hard_link(&outside, root.join("hardlink.ts")).unwrap();
        assert!(files::read(root, "hardlink.ts").is_err());
        assert!(files::write(root, "hardlink.ts", "overwrite").is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn runtime_symlinks_cannot_truncate_or_redirect_external_files() {
    use std::os::unix::fs::symlink;
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    let runtime = fixture._temp.path().join("private-runtime");
    checks::sandbox_command(
        root,
        &fixture.source,
        &runtime,
        "npm",
        &["--version".into()],
        false,
    )
    .unwrap();
    let outside = fixture._temp.path().join("outside-private");
    fs::create_dir(&outside).unwrap();
    let victim = outside.join("npm-user.config");
    fs::write(&victim, "preserve outside contents").unwrap();
    fs::remove_file(runtime.join("npm-user.config")).unwrap();
    symlink(&victim, runtime.join("npm-user.config")).unwrap();
    checks::sandbox_command(
        root,
        &fixture.source,
        &runtime,
        "npm",
        &["--version".into()],
        false,
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(&victim).unwrap(),
        "preserve outside contents"
    );
    assert!(!fs::symlink_metadata(runtime.join("npm-user.config"))
        .unwrap()
        .file_type()
        .is_symlink());
    fs::rename(&runtime, fixture._temp.path().join("retained-runtime")).unwrap();
    symlink(&outside, &runtime).unwrap();
    assert!(checks::sandbox_command(
        root,
        &fixture.source,
        &runtime,
        "npm",
        &["--version".into()],
        false
    )
    .is_err());
    assert_eq!(
        fs::read_to_string(victim).unwrap(),
        "preserve outside contents"
    );
}

#[cfg(unix)]
#[test]
fn anchored_access_survives_parent_replacement_without_following_symlinks() {
    use std::{io::Read, os::unix::fs::symlink};
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/file.ts"), "original worktree source").unwrap();
    let outside = fixture._temp.path().join("outside-source");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("file.ts"), "outside must stay private").unwrap();
    let entry = super::secure_fs::AnchoredRoot::open(root)
        .unwrap()
        .entry(Path::new("nested/file.ts"), false)
        .unwrap();
    fs::rename(root.join("nested"), root.join("retained-nested")).unwrap();
    symlink(&outside, root.join("nested")).unwrap();
    let mut content = String::new();
    entry.open().unwrap().read_to_string(&mut content).unwrap();
    assert_eq!(content, "original worktree source");
    entry.replace(b"updated worktree source", None).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("retained-nested/file.ts")).unwrap(),
        "updated worktree source"
    );
    entry.remove().unwrap();
    assert!(!root.join("retained-nested/file.ts").exists());
    assert_eq!(
        fs::read_to_string(outside.join("file.ts")).unwrap(),
        "outside must stay private"
    );
    assert!(files::write(root, "nested/file.ts", "bad").is_err());
}

#[cfg(unix)]
#[test]
fn source_digest_never_follows_a_replaced_tracked_parent() {
    use std::os::unix::fs::symlink;
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/file.ts"), "source").unwrap();
    files::git(root, &["add", "-N", "nested/file.ts"]).unwrap();
    let outside = fixture._temp.path().join("outside-source");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("file.ts"), "private").unwrap();
    fs::rename(root.join("nested"), root.join("retained-nested")).unwrap();
    symlink(&outside, root.join("nested")).unwrap();
    assert!(files::digest(root).is_err());
}

#[test]
fn check_results_become_stale_for_tracked_and_untracked_changes() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let root = Path::new(&snapshot.workspace.worktree_path);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    session.data.lock().unwrap().checks.push(FixCheck {
        action: "npm:test:run".into(),
        status: "passed".into(),
        exit_code: Some(0),
        output: "fixture result".into(),
        digest: snapshot.diff.as_ref().unwrap().digest.clone(),
        stale: false,
        finished_at: now(),
    });
    session.publish().unwrap();
    assert!(
        !fixture
            .service
            .snapshot(1, &snapshot.workspace.id)
            .unwrap()
            .workspace
            .checks[0]
            .stale
    );
    files::write(root, "untracked.ts", "new source").unwrap();
    assert!(
        fixture
            .service
            .snapshot(1, &snapshot.workspace.id)
            .unwrap()
            .workspace
            .checks[0]
            .stale
    );
    fs::remove_file(root.join("untracked.ts")).unwrap();
    fs::remove_file(root.join("app.ts")).unwrap();
    let changed = fixture.service.snapshot(1, &snapshot.workspace.id).unwrap();
    assert!(changed.workspace.checks[0].stale);
    assert!(changed
        .diff
        .unwrap()
        .files
        .iter()
        .any(|file| file.patch.contains("--- a/app.ts") && file.patch.contains("+++ /dev/null")));
}

#[test]
fn restart_and_missing_worktree_preserve_local_results() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    {
        let mut data = session.data.lock().unwrap();
        data.status = "working".into();
        data.messages.push(FixMessage {
            provider: None,
            role: "assistant".into(),
            text: "Saved answer".into(),
        });
    }
    session.publish().unwrap();
    let root = fixture.service.0.root.clone();
    let scanner_cache = fixture.service.0.scanner_cache.clone();
    drop(session);
    drop(fixture.service);
    let recovered = FixWorkspaces::new(
        fixture.db.clone(),
        root,
        scanner_cache,
        fixture.host.clone(),
        |_| {},
    )
    .unwrap();
    assert!(recovered.0.sessions.lock().unwrap().is_empty());
    assert_eq!(recovered.list(1).unwrap()[0].status, "interrupted");
    fs::rename(
        &snapshot.workspace.worktree_path,
        fixture._temp.path().join("moved-worktree"),
    )
    .unwrap();
    let saved = recovered.snapshot(1, &snapshot.workspace.id).unwrap();
    assert!(saved.diff.is_none());
    assert!(saved.source_error.is_some());
    assert_eq!(saved.workspace.messages[0].text, "Saved answer");
}

#[cfg(target_os = "macos")]
#[test]
fn workspace_check_finishes_when_a_helper_inherits_its_output() {
    for (exit_code, expected) in [(0, "passed"), (7, "failed")] {
        let fixture = fixture();
        let snapshot = create(&fixture);
        let root = Path::new(&snapshot.workspace.worktree_path);
        fs::write(
            root.join("package.json"),
            r#"{"scripts":{"test:run":"node inherited-check.cjs"}}"#,
        )
        .unwrap();
        fs::write(
            root.join("inherited-check.cjs"),
            format!(
                "const {{spawn}} = require('node:child_process');\nconst helper = spawn(process.execPath, ['-e', 'setInterval(() => {{}}, 1000)'], {{stdio: 'inherit'}});\nhelper.unref();\nrequire('node:fs').writeSync(1, 'synthetic check complete\\n');\nprocess.exit({exit_code});\n"
            ),
        )
        .unwrap();
        let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
        let stopped = session.stopped.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            if finished.recv_timeout(Duration::from_secs(5)).is_err() {
                stopped.store(true, Ordering::SeqCst);
            }
        });
        let result = fixture
            .service
            .run_check(1, &snapshot.workspace.id, "npm:test:run");
        let _ = done.send(());
        watchdog.join().unwrap();
        let result = result.unwrap();
        let check = result.workspace.checks.last().unwrap();
        assert_eq!(check.status, expected, "{}", check.output);
        assert_eq!(check.exit_code, Some(exit_code));
        assert!(check.output.contains("synthetic check complete"));
        assert!(!check.stale);
        assert_eq!(
            fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
            "export const fixed = false;\n"
        );
        fixture.service.shutdown();
        fixture.host.shutdown();
    }
}

#[test]
fn arbitrary_commands_and_inactive_or_wrong_turn_tools_are_denied() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    assert!(session
        .call(
            "cvec_workspace_write_file",
            &json!({"path":"app.ts","content":"bad"}),
            "provider-current",
            "turn-1"
        )
        .is_err());
    {
        let mut data = session.data.lock().unwrap();
        data.status = "working".into();
        data.turn_id = Some("turn-1".into());
        data.provider_thread_id = Some("provider-current".into());
    }
    assert!(session
        .call("git_push", &json!({}), "provider-current", "turn-1")
        .is_err());
    assert!(session
        .call(
            "cvec_workspace_write_file",
            &json!({"path":"app.ts","content":"bad"}),
            "provider-current",
            "old-turn"
        )
        .is_err());
    assert!(session
        .call(
            "cvec_workspace_check",
            &json!({"action":"git:push"}),
            "provider-current",
            "turn-1"
        )
        .is_err());
    session.stopped.store(true, Ordering::SeqCst);
    assert!(session
        .call(
            "cvec_workspace_write_file",
            &json!({"path":"app.ts","content":"bad"}),
            "provider-current",
            "turn-1"
        )
        .is_err());
}

#[test]
fn fake_codex_can_fix_only_the_registered_worktree_and_cannot_escape() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    assert!(fixture
        .service
        .start(
            1,
            &snapshot.workspace.id,
            FixStartRequest {
                connection_updated_at: fixture
                    .db
                    .ai_connection("codex_default")
                    .unwrap()
                    .updated_at,
                connection_id: "codex_default".into(),
                prompt: "[[FIX_WRITE]]".into(),
                model: Some("fake-model".into()),
                effort: None,
                consent: false
            }
        )
        .is_err());
    fixture
        .service
        .start(
            1,
            &snapshot.workspace.id,
            FixStartRequest {
                connection_updated_at: fixture
                    .db
                    .ai_connection("codex_default")
                    .unwrap()
                    .updated_at,
                connection_id: "codex_default".into(),
                prompt: "[[FIX_WRITE]]".into(),
                model: Some("fake-model".into()),
                effort: None,
                consent: true,
            },
        )
        .unwrap();
    wait_ready(&fixture, &snapshot.workspace.id);
    assert_eq!(
        fs::read_to_string(Path::new(&snapshot.workspace.worktree_path).join("app.ts")).unwrap(),
        "export const fixed = true;\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
        "export const fixed = false;\n"
    );
    fixture
        .service
        .start(
            1,
            &snapshot.workspace.id,
            FixStartRequest {
                connection_updated_at: fixture
                    .db
                    .ai_connection("codex_default")
                    .unwrap()
                    .updated_at,
                connection_id: "codex_default".into(),
                prompt: "[[FIX_ESCAPE]]".into(),
                model: Some("fake-model".into()),
                effort: None,
                consent: true,
            },
        )
        .unwrap();
    wait_ready(&fixture, &snapshot.workspace.id);
    assert!(!fixture.service.0.root.join("outside.txt").exists());
    fixture.host.shutdown();
}

#[test]
fn stopping_a_codex_turn_preserves_work_and_allows_a_later_task() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    fixture
        .service
        .start(
            1,
            &snapshot.workspace.id,
            FixStartRequest {
                connection_updated_at: fixture
                    .db
                    .ai_connection("codex_default")
                    .unwrap()
                    .updated_at,
                connection_id: "codex_default".into(),
                prompt: "[[WAIT]]".into(),
                model: Some("fake-model".into()),
                effort: None,
                consent: true,
            },
        )
        .unwrap();
    let previous_provider = fixture.service.list(1).unwrap()[0]
        .provider_thread_id
        .clone();
    fixture.service.stop(1, &snapshot.workspace.id).unwrap();
    let started = Instant::now();
    while fixture
        .service
        .session(1, &snapshot.workspace.id)
        .unwrap()
        .agent_active
        .load(Ordering::SeqCst)
    {
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(10));
    }
    fixture
        .service
        .start(
            1,
            &snapshot.workspace.id,
            FixStartRequest {
                connection_updated_at: fixture
                    .db
                    .ai_connection("codex_default")
                    .unwrap()
                    .updated_at,
                connection_id: "codex_default".into(),
                prompt: "Follow-up".into(),
                model: Some("fake-model".into()),
                effort: None,
                consent: true,
            },
        )
        .unwrap();
    wait_ready(&fixture, &snapshot.workspace.id);
    assert_ne!(
        fixture.service.list(1).unwrap()[0].provider_thread_id,
        previous_provider
    );
    fixture.host.shutdown();
}

#[test]
fn stale_provider_events_cannot_finish_or_append_to_a_new_request() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    session.agent_active.store(true, Ordering::SeqCst);
    {
        let mut data = session.data.lock().unwrap();
        data.status = "working".into();
        data.provider_thread_id = Some("new-provider".into());
        data.turn_id = Some("new-turn".into());
    }
    for (provider, turn) in [("old-provider", "old-turn"), ("new-provider", "old-turn")] {
        notify(
            &session,
            "turn/started",
            &json!({"threadId":provider,"turn":{"id":turn}}),
        );
        notify(
            &session,
            "item/completed",
            &json!({"threadId":provider,"turnId":turn,"item":{"type":"agentMessage","text":"Stale answer"}}),
        );
        notify(
            &session,
            "turn/completed",
            &json!({"threadId":provider,"turn":{"id":turn,"status":"completed"}}),
        );
    }
    notify(
        &session,
        "cvec/interrupted",
        &json!({"threadId":"old-provider"}),
    );
    {
        let data = session.data.lock().unwrap();
        assert_eq!(data.status, "working");
        assert_eq!(data.turn_id.as_deref(), Some("new-turn"));
        assert!(data.messages.is_empty());
    }
    assert!(session.agent_active.load(Ordering::SeqCst));
    assert!(!session.stopped.load(Ordering::SeqCst));
    notify(
        &session,
        "turn/completed",
        &json!({"threadId":"new-provider","turn":{"id":"new-turn","status":"completed"}}),
    );
    let revision = session.data.lock().unwrap().revision;
    notify(
        &session,
        "turn/completed",
        &json!({"threadId":"new-provider","turn":{"id":"new-turn","status":"completed"}}),
    );
    assert_eq!(session.data.lock().unwrap().revision, revision);
    assert_eq!(session.data.lock().unwrap().status, "ready");
    // A late process failure must not interrupt an independent manual check.
    {
        let mut data = session.data.lock().unwrap();
        data.status = "testing".into();
        data.turn_id = Some("stopped-turn".into());
    }
    notify(
        &session,
        "turn/completed",
        &json!({"threadId":"new-provider","turn":{"id":"stopped-turn","status":"interrupted"}}),
    );
    notify(
        &session,
        "cvec/interrupted",
        &json!({"threadId":"new-provider"}),
    );
    assert_eq!(session.data.lock().unwrap().status, "testing");
}

#[test]
fn pending_start_rejects_a_followup_even_after_local_stop() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    session.start_pending.store(true, Ordering::SeqCst);
    fixture.service.stop(1, &snapshot.workspace.id).unwrap();
    let error = fixture
        .service
        .start(
            1,
            &snapshot.workspace.id,
            FixStartRequest {
                connection_updated_at: fixture
                    .db
                    .ai_connection("codex_default")
                    .unwrap()
                    .updated_at,
                connection_id: "codex_default".into(),
                prompt: "Follow-up".into(),
                model: Some("fake-model".into()),
                effort: None,
                consent: true,
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("still settling"));
    assert!(session.data.lock().unwrap().messages.is_empty());
    let error = fixture
        .service
        .run_check(1, &snapshot.workspace.id, "npm:test:run")
        .unwrap_err();
    assert!(error.to_string().contains("start"), "{error}");
    let error = fixture
        .service
        .rescan(1, &snapshot.workspace.id)
        .unwrap_err();
    assert!(error.to_string().contains("start"), "{error}");
}

struct RecordingRuntime {
    id: &'static str,
    stops: Mutex<Vec<Option<String>>>,
}
impl RecordingRuntime {
    fn new(id: &'static str) -> Arc<Self> {
        Arc::new(Self {
            id,
            stops: Mutex::new(Vec::new()),
        })
    }
}
impl FixRuntime for RecordingRuntime {
    fn id(&self) -> &str {
        self.id
    }
    fn start(&self, _: String, _: Option<&str>, _: Option<&str>) -> Result<String, Error> {
        Err(Error::App("Recording runtime does not infer.".into()))
    }
    fn stop(&self, turn: Option<&str>) -> Result<(), Error> {
        self.stops.lock().unwrap().push(turn.map(str::to_owned));
        Ok(())
    }
}

#[test]
fn review_delayed_stop_cancels_only_the_runtime_captured_before_publication() {
    let mut fixture = fixture();
    let target = Arc::new(Mutex::new(Weak::<FixSession>::new()));
    let replacement = RecordingRuntime::new("replacement-runtime");
    let replacement_sink = replacement.clone();
    let target_sink = target.clone();
    Arc::get_mut(&mut fixture.service.0).unwrap().sink = Arc::new(move |data| {
        if data.activity != "Local actions stopped. Workspace changes are preserved." {
            return;
        }
        let session = target_sink.lock().unwrap().upgrade().unwrap();
        // Deterministic scheduling: the next submission replaces the runtime
        // after Stop's publication and before the old cancellation is sent.
        *session.runtime.lock().unwrap() = Some(replacement_sink.clone());
        let mut data = session.data.lock().unwrap();
        data.provider_thread_id = Some("replacement-runtime".into());
        data.turn_id = Some("replacement-turn".into());
        data.status = "working".into();
        session.stopped.store(false, Ordering::SeqCst);
        session.agent_active.store(true, Ordering::SeqCst);
    });
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    *target.lock().unwrap() = Arc::downgrade(&session);
    let original = RecordingRuntime::new("original-runtime");
    *session.runtime.lock().unwrap() = Some(original.clone());
    {
        let mut data = session.data.lock().unwrap();
        data.status = "working".into();
        data.provider_thread_id = Some("original-runtime".into());
        data.turn_id = Some("original-turn".into());
    }
    fixture.service.stop(1, &snapshot.workspace.id).unwrap();
    assert_eq!(
        *original.stops.lock().unwrap(),
        vec![Some("original-turn".into())]
    );
    assert!(replacement.stops.lock().unwrap().is_empty());
    assert_eq!(session.data.lock().unwrap().status, "working");
}

#[test]
fn review_stop_still_cancels_the_provider_if_persisting_status_fails() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    let runtime = RecordingRuntime::new("original-runtime");
    *session.runtime.lock().unwrap() = Some(runtime.clone());
    session.data.lock().unwrap().turn_id = Some("original-turn".into());
    let db = rusqlite::Connection::open(fixture._temp.path().join("cvec.db")).unwrap();
    db.execute_batch("CREATE TRIGGER fixture_reject_updates BEFORE UPDATE ON fix_workspaces BEGIN SELECT RAISE(ABORT,'fixture storage failure'); END;").unwrap();
    assert!(fixture.service.stop(1, &snapshot.workspace.id).is_err());
    assert_eq!(
        *runtime.stops.lock().unwrap(),
        vec![Some("original-turn".into())]
    );
    assert!(session.stopped.load(Ordering::SeqCst));
}

#[test]
fn review_manual_completion_reports_final_storage_failure_and_keeps_saved_check() {
    let mut fixture = fixture();
    let published = Arc::new(Mutex::new(Vec::<FixWorkspace>::new()));
    let output = published.clone();
    Arc::get_mut(&mut fixture.service.0).unwrap().sink =
        Arc::new(move |data| output.lock().unwrap().push(data));
    let snapshot = create(&fixture);
    let db = rusqlite::Connection::open(fixture._temp.path().join("cvec.db")).unwrap();
    db.execute_batch("CREATE TRIGGER fixture_reject_completion BEFORE UPDATE ON fix_workspaces WHEN json_extract(NEW.record_json,'$.status')='ready' BEGIN SELECT RAISE(ABORT,'fixture completion storage failure'); END;").unwrap();
    let result = fixture
        .service
        .run_check(1, &snapshot.workspace.id, "npm:test:run");
    let saved = fixture.db.load_fix_workspaces(1).unwrap();
    assert!(
        result.is_err(),
        "a failed final status write must not report success"
    );
    assert_eq!(saved[0].checks.len(), 1);
    let expected_check = if cfg!(target_os = "macos") {
        "passed"
    } else {
        "unavailable"
    };
    assert_eq!(saved[0].checks[0].status, expected_check);
    assert_eq!(saved[0].status, "testing");
    let visible = published.lock().unwrap().last().unwrap().clone();
    assert_eq!(visible.status, "failed");
    assert!(visible.activity.contains("could not be saved"));
    assert_eq!(visible.checks[0].status, expected_check);
    db.execute_batch("DROP TRIGGER fixture_reject_completion")
        .unwrap();
    let root = fixture.service.0.root.clone();
    let cache = fixture.service.0.scanner_cache.clone();
    drop(fixture.service);
    let recovered = FixWorkspaces::new(
        fixture.db.clone(),
        root,
        cache,
        fixture.host.clone(),
        |_| {},
    )
    .unwrap();
    let restored = recovered.snapshot(1, &snapshot.workspace.id).unwrap();
    assert_eq!(restored.workspace.status, "interrupted");
    assert_eq!(restored.workspace.checks[0].status, expected_check);
    assert!(!restored.workspace.checks[0].stale);
}

#[test]
fn review_provider_completion_storage_failure_is_visible_without_losing_saved_state() {
    let mut fixture = fixture();
    let published = Arc::new(Mutex::new(Vec::<FixWorkspace>::new()));
    let output = published.clone();
    Arc::get_mut(&mut fixture.service.0).unwrap().sink =
        Arc::new(move |data| output.lock().unwrap().push(data));
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    {
        let mut data = session.data.lock().unwrap();
        data.status = "working".into();
        data.provider_thread_id = Some("recording-session".into());
        data.turn_id = Some("recording-turn".into());
        session.agent_active.store(true, Ordering::SeqCst);
    }
    session.publish().unwrap();
    let db = rusqlite::Connection::open(fixture._temp.path().join("cvec.db")).unwrap();
    db.execute_batch("CREATE TRIGGER fixture_reject_completion BEFORE UPDATE ON fix_workspaces WHEN json_extract(NEW.record_json,'$.status')='ready' BEGIN SELECT RAISE(ABORT,'fixture completion storage failure'); END;").unwrap();
    session.event(FixEvent::Finished {
        session: "recording-session".into(),
        turn: "recording-turn".into(),
        status: "completed".into(),
        error: None,
    });
    let visible = published.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        visible.status, "failed",
        "the UI must not remain stuck on working after a failed save"
    );
    assert!(visible.activity.contains("could not be saved"));
    assert_eq!(
        fixture.db.load_fix_workspaces(1).unwrap()[0].status,
        "working"
    );
    assert!(session.stopped.load(Ordering::SeqCst));
}

#[test]
fn review_restart_keeps_saved_results_readable_when_recovery_cannot_be_saved() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    {
        let mut data = session.data.lock().unwrap();
        data.status = "working".into();
        data.messages.push(FixMessage {
            provider: None,
            role: "assistant".into(),
            text: "Previously saved result".into(),
        });
    }
    session.publish().unwrap();
    let db = rusqlite::Connection::open(fixture._temp.path().join("cvec.db")).unwrap();
    db.execute_batch("CREATE TRIGGER fixture_reject_updates BEFORE UPDATE ON fix_workspaces BEGIN SELECT RAISE(ABORT,'fixture recovery storage failure'); END;").unwrap();
    let root = fixture.service.0.root.clone();
    let cache = fixture.service.0.scanner_cache.clone();
    drop(session);
    drop(fixture.service);
    let recovered = FixWorkspaces::new(
        fixture.db.clone(),
        root,
        cache,
        fixture.host.clone(),
        |_| {},
    )
    .unwrap();
    let saved = recovered
        .snapshot(1, &snapshot.workspace.id)
        .expect("readable history must survive a failed recovery write");
    assert_eq!(saved.workspace.status, "interrupted");
    assert!(saved.workspace.activity.contains("could not be saved"));
    assert_eq!(saved.workspace.messages[0].text, "Previously saved result");
    assert!(saved.diff.is_some());
    assert_eq!(
        fixture.db.load_fix_workspaces(1).unwrap()[0].status,
        "working"
    );
    assert!(recovered
        .start(
            1,
            &snapshot.workspace.id,
            FixStartRequest {
                connection_id: "codex_default".into(),
                connection_updated_at: fixture
                    .db
                    .ai_connection("codex_default")
                    .unwrap()
                    .updated_at,
                prompt: "Do not submit before saving".into(),
                model: Some("fake-model".into()),
                effort: None,
                consent: true,
            }
        )
        .is_err());
    assert_eq!(fixture.host.status().active_turns, 0);
}

#[test]
fn review_provider_message_storage_failure_revokes_tools_and_cancels_the_captured_runtime() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    let runtime = RecordingRuntime::new("recording-session");
    *session.runtime.lock().unwrap() = Some(runtime.clone());
    {
        let mut data = session.data.lock().unwrap();
        data.status = "working".into();
        data.provider_thread_id = Some("recording-session".into());
        data.turn_id = Some("recording-turn".into());
        session.agent_active.store(true, Ordering::SeqCst);
    }
    session.publish().unwrap();
    let db = rusqlite::Connection::open(fixture._temp.path().join("cvec.db")).unwrap();
    db.execute_batch("CREATE TRIGGER fixture_reject_updates BEFORE UPDATE ON fix_workspaces BEGIN SELECT RAISE(ABORT,'fixture storage failure'); END;").unwrap();
    session.event(FixEvent::Message {
        session: "recording-session".into(),
        turn: "recording-turn".into(),
        text: "Retain this response in memory.".into(),
    });
    assert!(session.stopped.load(Ordering::SeqCst));
    assert!(!session.agent_active.load(Ordering::SeqCst));
    assert!(session
        .call(
            "cvec_workspace_write_file",
            &json!({"path":"app.ts","content":"bad"}),
            "recording-session",
            "recording-turn"
        )
        .is_err());
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.stops.lock().unwrap().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        *runtime.stops.lock().unwrap(),
        vec![Some("recording-turn".into())]
    );
    assert_eq!(
        session.data.lock().unwrap().messages.last().unwrap().text,
        "Retain this response in memory."
    );
    assert!(fixture.db.load_fix_workspaces(1).unwrap()[0]
        .messages
        .is_empty());
}

#[test]
fn review_codex_reader_survives_storage_failure_while_cancelling_its_turn() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let db = rusqlite::Connection::open(fixture._temp.path().join("cvec.db")).unwrap();
    db.execute_batch("CREATE TRIGGER fixture_reject_answer BEFORE UPDATE ON fix_workspaces WHEN json_array_length(NEW.record_json,'$.messages')>1 BEGIN SELECT RAISE(ABORT,'fixture answer storage failure'); END;").unwrap();
    let _ = fixture.service.start(
        1,
        &snapshot.workspace.id,
        FixStartRequest {
            connection_id: "codex_default".into(),
            connection_updated_at: fixture
                .db
                .ai_connection("codex_default")
                .unwrap()
                .updated_at,
            prompt: "[[MESSAGE_WAIT]]".into(),
            model: Some("fake-model".into()),
            effort: None,
            consent: true,
        },
    );
    let data = wait_settled(&fixture, &snapshot.workspace.id);
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture.host.status().active_turns != 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(data.status, "failed");
    assert!(data.activity.contains("could not be saved"));
    assert_eq!(fixture.host.status().active_turns, 0);
    assert!(
        fixture.host.account_status().is_ok(),
        "the shared reader must remain responsive"
    );
    fixture.host.shutdown();
}

#[test]
fn review_stop_cancels_scanner_version_preflight_without_starting_the_scan() {
    const NAME: &str = "core::fix_workspaces::tests::review_stop_cancels_scanner_version_preflight_without_starting_the_scan";
    if std::env::var_os("CVEC_FIX_SCAN_CANCEL_CHILD").is_none() {
        for stage in ["syft", "grype"] {
            let tools = tempfile::tempdir().unwrap();
            for tool in ["syft", "grype"] {
                let script = format!("#!/bin/sh\nif [ \"$1\" = --version ]; then\n if [ \"$CVEC_FIX_SCAN_CANCEL_STAGE\" = {tool} ]; then\n  printf '%s' \"$$\" > \"$CVEC_FIX_SCAN_CANCEL_MARKER\"\n  exec /bin/sleep 20\n fi\n printf fixture-{tool}\n exit 0\nfi\nprintf unexpected > \"$CVEC_FIX_SCAN_CANCEL_MARKER.scan\"\nexit 1\n");
                let path = tools.path().join(tool);
                fs::write(&path, script).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
                }
            }
            let output = Command::new(std::env::current_exe().unwrap())
                .args([NAME, "--exact", "--nocapture"])
                .env("CVEC_FIX_SCAN_CANCEL_CHILD", "1")
                .env("CVEC_FIX_SCAN_CANCEL_STAGE", stage)
                .env("CVEC_FIX_SCAN_CANCEL_MARKER", tools.path().join("started"))
                .env("CVEC_SCANNER_TOOL_DIRS", tools.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{stage}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    }
    use std::sync::mpsc;
    let fixture = fixture();
    let snapshot = create(&fixture);
    let service = fixture.service.clone();
    let id = snapshot.workspace.id.clone();
    let marker = PathBuf::from(std::env::var_os("CVEC_FIX_SCAN_CANCEL_MARKER").unwrap());
    let (send, receive) = mpsc::channel();
    let worker = std::thread::spawn(move || send.send(service.rescan(1, &id)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = fs::read_to_string(&marker).expect("owned scanner probe started");
    let alive = || {
        Command::new("/bin/kill")
            .args(["-0", &pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    fixture.service.stop(1, &snapshot.workspace.id).unwrap();
    let result = receive.recv_timeout(Duration::from_secs(1));
    if result.is_err() {
        let _ = Command::new("/bin/kill").args(["-KILL", &pid]).status();
    }
    worker.join().unwrap();
    assert!(
        result.is_ok(),
        "Stop must reap a blocked scanner version probe promptly"
    );
    assert!(result.unwrap().is_err());
    assert!(!alive());
    assert!(!marker.with_extension("scan").exists());
    let current = fixture.service.list(1).unwrap().remove(0);
    assert_eq!(current.status, "interrupted");
    assert!(current.rescan.is_none());
}

#[test]
fn review_concurrent_creation_fails_promptly_without_queuing_another_worktree() {
    use std::sync::mpsc;
    let fixture = fixture();
    let creation = fixture.service.0.creation.lock().unwrap();
    let service = fixture.service.clone();
    let (send, receive) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        send.send(service.create(1, None, true, None).map(|_| ()))
            .unwrap()
    });
    let response = receive.recv_timeout(Duration::from_millis(500));
    drop(creation);
    worker.join().unwrap();
    assert!(
        response.is_ok(),
        "a competing creation must return while the first creation owns its lock"
    );
    assert!(response.unwrap().is_err());
    assert!(fixture.service.list(1).unwrap().is_empty());
}

#[test]
fn review_shutdown_cancels_active_codex_and_rejects_subsequent_writes() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let request = || FixStartRequest {
        connection_id: "codex_default".into(),
        connection_updated_at: fixture
            .db
            .ai_connection("codex_default")
            .unwrap()
            .updated_at,
        prompt: "[[WAIT]]".into(),
        model: Some("fake-model".into()),
        effort: None,
        consent: true,
    };
    fixture
        .service
        .start(1, &snapshot.workspace.id, request())
        .unwrap();
    assert_eq!(fixture.host.status().active_turns, 1);
    fixture.service.shutdown();
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture.host.status().active_turns != 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fixture.host.status().active_turns, 0);
    assert!(fixture
        .service
        .start(1, &snapshot.workspace.id, request())
        .is_err());
    assert!(fixture.service.create(1, None, true, None).is_err());
    assert!(fixture
        .service
        .run_check(1, &snapshot.workspace.id, "npm:test:run")
        .is_err());
    assert!(fixture.service.rescan(1, &snapshot.workspace.id).is_err());
    assert_eq!(
        fixture
            .service
            .snapshot(1, &snapshot.workspace.id)
            .unwrap()
            .workspace
            .status,
        "interrupted"
    );
    fixture.host.shutdown();
}

#[test]
fn review_codex_runtime_stopped_before_start_does_not_submit_a_turn() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    let connection = fixture.db.ai_connection("codex_default").unwrap();
    let runtime = fix_runtime::create(
        &connection,
        &fixture.host,
        fixture._temp.path(),
        tool_specs(),
        Arc::new(WeakFixSink(Arc::downgrade(&session))),
        Some("fake-model"),
    )
    .unwrap();
    runtime.stop(None).unwrap();
    let result = runtime.start("[[WAIT]]".into(), Some("fake-model"), None);
    let active = fixture.host.status().active_turns;
    fixture.host.shutdown();
    assert!(result.is_err());
    assert_eq!(active, 0);
}

#[test]
fn review_start_cancels_codex_if_persisting_its_accepted_turn_fails() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let db = rusqlite::Connection::open(fixture._temp.path().join("cvec.db")).unwrap();
    db.execute_batch("CREATE TRIGGER fixture_reject_turn BEFORE UPDATE ON fix_workspaces WHEN json_extract(NEW.record_json,'$.turnId') IS NOT NULL BEGIN SELECT RAISE(ABORT,'fixture accepted turn storage failure'); END;").unwrap();
    let error = fixture
        .service
        .start(
            1,
            &snapshot.workspace.id,
            FixStartRequest {
                connection_id: "codex_default".into(),
                connection_updated_at: fixture
                    .db
                    .ai_connection("codex_default")
                    .unwrap()
                    .updated_at,
                prompt: "[[WAIT]]".into(),
                model: Some("fake-model".into()),
                effort: None,
                consent: true,
            },
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("fixture accepted turn storage failure"));
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture.host.status().active_turns != 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fixture.host.status().active_turns, 0);
    let after = fixture.service.list(1).unwrap()[0].clone();
    assert_eq!(after.status, "failed");
    assert!(after.turn_id.is_none());
    fixture.host.shutdown();
}

#[test]
fn unreadable_and_future_records_do_not_block_recovery_or_get_overwritten() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let connection = rusqlite::Connection::open(fixture._temp.path().join("cvec.db")).unwrap();
    let mut future = snapshot.workspace.clone();
    future.id = "f".repeat(24);
    future.schema_version = 99;
    let future_json = serde_json::to_string(&future).unwrap();
    for (id, json) in [
        ("d".repeat(24), "{broken".to_string()),
        (future.id.clone(), future_json.clone()),
    ] {
        connection.execute("INSERT INTO fix_workspaces(id, repository_id, updated_at, record_json) VALUES (?1, 1, 'now', ?2)", rusqlite::params![id, json]).unwrap();
    }
    let recovered = FixWorkspaces::new(
        fixture.db.clone(),
        fixture.service.0.root.clone(),
        fixture.service.0.scanner_cache.clone(),
        fixture.host.clone(),
        |_| {},
    )
    .unwrap();
    assert!(recovered.0.sessions.lock().unwrap().is_empty());
    assert_eq!(recovered.list(1).unwrap().len(), 1);
    assert!(recovered
        .snapshot(1, &snapshot.workspace.id)
        .unwrap()
        .diff
        .is_some());
    let saved: String = connection
        .query_row(
            "SELECT record_json FROM fix_workspaces WHERE id=?1",
            [&future.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(saved, future_json);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM fix_workspaces", [], |row| row
                .get::<_, u32>(0))
            .unwrap(),
        3
    );
}

#[test]
fn a_second_instance_can_read_but_cannot_mutate_or_interrupt_owned_workspaces() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    session.data.lock().unwrap().status = "working".into();
    session.publish().unwrap();
    let other = FixWorkspaces::new(
        fixture.db.clone(),
        fixture.service.0.root.clone(),
        fixture.service.0.scanner_cache.clone(),
        fixture.host.clone(),
        |_| {},
    )
    .unwrap();
    assert_eq!(other.list(1).unwrap()[0].status, "working");
    assert!(other
        .snapshot(1, &snapshot.workspace.id)
        .unwrap()
        .diff
        .is_some());
    assert!(other.create(1, None, true, None).is_err());
    assert!(other.stop(1, &snapshot.workspace.id).is_err());
    assert!(other
        .run_check(1, &snapshot.workspace.id, "npm:test:run")
        .is_err());
    other.shutdown();
    assert_eq!(
        fixture.db.load_fix_workspaces(1).unwrap()[0].status,
        "working"
    );
    session.data.lock().unwrap().activity = "Still owned by the first instance".into();
    session.publish().unwrap();
    assert_eq!(
        other.list(1).unwrap()[0].activity,
        "Still owned by the first instance"
    );
}

#[test]
fn worktree_rescan_uses_scanner_pipeline_without_mutating_canonical_findings() {
    const NAME:&str="core::fix_workspaces::tests::worktree_rescan_uses_scanner_pipeline_without_mutating_canonical_findings";
    if std::env::var_os("CVEC_FIX_RESCAN_TEST_CHILD").is_none() {
        // Resolve fake scanners only in a child test process, without changing
        // environment variables shared by concurrently running scanner tests.
        let tools = tempfile::tempdir().unwrap();
        let syft = r#"#!/bin/sh
if [ "$1" = "--version" ]; then printf 'fixture-syft'; exit 0; fi
if [ "$1" = "config" ]; then printf 'exclude: []\n'; exit 0; fi
for arg in "$@"; do
 case "$arg" in
  --override-default-catalogers=file-metadata-cataloger) printf '%s' '{"files":[]}'; exit 0;;
  cyclonedx-json=*) target=${arg#cyclonedx-json=}; printf '%s' '{"bomFormat":"CycloneDX","specVersion":"1.7","components":[]}' > "$target";;
  syft-json=*) target=${arg#syft-json=}; printf '%s' '{"schema":{"version":"16.0.0"},"artifacts":[]}' > "$target";;
 esac
done
"#;
        let grype="#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'fixture-grype'; exit 0; fi\ncat >/dev/null\nprintf '%s' '{\"matches\":[]}'\n";
        for (name, script) in [("syft", syft), ("grype", grype)] {
            let path = tools.path().join(name);
            fs::write(&path, script).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args([NAME, "--exact", "--nocapture"])
            .env("CVEC_FIX_RESCAN_TEST_CHILD", "1")
            .env("CVEC_SCANNER_TOOL_DIRS", tools.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use crate::core::models::{NormalizedFinding, ScannerVersions, Severity, SourceSnapshot};
    let fixture = fixture();
    let base = files::git_text(&fixture.source, &["rev-parse", "HEAD"]).unwrap();
    let findings = vec![NormalizedFinding {
        fingerprint: "fixture-fingerprint".into(),
        advisory_id: "CVE-fixture".into(),
        package_name: "fixture-package".into(),
        package_version: Some("1.0.0".into()),
        package_type: Some("npm".into()),
        severity: Severity::High,
        fix_version: Some("2.0.0".into()),
        title: None,
        source_url: None,
        package_url: Some("pkg:npm/fixture-package@1.0.0".into()),
        advisory_details: None,
    }];
    let versions = ScannerVersions {
        syft: "fixture".into(),
        grype: "fixture".into(),
    };
    let run = fixture.db.insert_scan_start(1, &versions).unwrap();
    let delta = crate::core::delta::compute_delta(&[], &findings);
    fixture
        .db
        .complete_scan_with_remediation(crate::core::db::ScanCompletion {
            repo_id: 1,
            run_id: run,
            findings: &findings,
            packages: &[],
            remediation_evidence: &[],
            source_snapshot: &SourceSnapshot {
                revision: Some(base),
                dirty: Some(false),
            },
            delta: &delta,
        })
        .unwrap();
    // Finding inventory remains tied to the last successful scan, even if
    // later attempts fail beyond the repository detail's bounded history.
    for _ in 0..13 {
        let failed = fixture.db.insert_scan_start(1, &versions).unwrap();
        fixture
            .db
            .fail_scan(1, failed, "Synthetic later failure", None)
            .unwrap();
    }
    let workspace = fixture
        .service
        .create(1, Some("fixture-fingerprint".into()), true, None)
        .unwrap();
    assert!(workspace.workspace.baseline_matched);
    assert_eq!(workspace.workspace.baseline_scan_id, Some(run));
    let result = fixture.service.rescan(1, &workspace.workspace.id).unwrap();
    let scan = result.workspace.rescan.unwrap();
    assert_eq!(scan.finding_count, 0);
    assert_eq!(scan.not_detected, Some(1));
    assert!(!scan.stale);
    assert!(scan.scanner_versions.contains("fixture-syft"));
    let canonical = fixture.db.repo_detail(1).unwrap();
    assert_ne!(canonical.latest_scan.unwrap().id, run);
    assert_eq!(canonical.scan_history.len(), 12);
    assert_eq!(canonical.findings.len(), 1);
    assert_ne!(
        canonical.findings[0].state,
        crate::core::models::RepoFindingState::Resolved
    );
}

fn notify(session: &FixSession, method: &str, params: &Value) {
    if let Some(event) = crate::core::fix_runtime::codex::event(method, params) {
        session.event(event);
    }
}

#[cfg(unix)]
fn configure_fix_provider(
    fixture: &Fixture,
    driver: crate::core::provider_connections::ProviderDriverKind,
) -> String {
    use crate::core::provider_connections::*;
    use std::os::unix::fs::PermissionsExt;
    let (name, id) = match driver {
        ProviderDriverKind::Grok => ("grok", "grok_default"),
        ProviderDriverKind::OpenCode => ("opencode", "opencode_default"),
        _ => unreachable!(),
    };
    let python = Command::new("/usr/bin/which")
        .arg("python3")
        .output()
        .unwrap();
    let python = String::from_utf8(python.stdout).unwrap();
    let binary = fixture._temp.path().join(format!("fake-{name}"));
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_fix_runtime.py");
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\nexec '{}' '{}' '{}' \"$@\"\n",
            python.trim(),
            script.display(),
            name
        ),
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let config = if driver == ProviderDriverKind::Grok {
        ProviderConnectionConfig::Grok(GrokConnectionConfig {
            binary_path: Some(binary.to_string_lossy().into_owned()),
            ..Default::default()
        })
    } else {
        ProviderConnectionConfig::OpenCode(OpenCodeConnectionConfig {
            binary_path: Some(binary.to_string_lossy().into_owned()),
            ..Default::default()
        })
    };
    fixture
        .db
        .save_ai_connection(&ProviderConnectionDraft {
            id: id.into(),
            driver,
            display_name: name.into(),
            accent_color: None,
            enabled: true,
            config,
            server_password: None,
        })
        .unwrap();
    id.into()
}

#[cfg(unix)]
#[test]
fn stopping_during_provider_probes_reaps_processes_without_starting_the_next_phase() {
    use crate::core::provider_connections::ProviderDriverKind;
    assert_stopped_provider_stages(
        &[
            (ProviderDriverKind::Grok, "grok-inspect"),
            (ProviderDriverKind::OpenCode, "opencode-version"),
            (ProviderDriverKind::OpenCode, "opencode-server"),
            (ProviderDriverKind::OpenCode, "opencode-agent"),
            (ProviderDriverKind::OpenCode, "opencode-models"),
            (ProviderDriverKind::OpenCode, "opencode-completion"),
        ],
        false,
    );
}

#[cfg(unix)]
#[test]
fn stopping_provider_probe_reaps_owned_helpers() {
    use crate::core::provider_connections::ProviderDriverKind;
    assert_stopped_provider_stages(&[(ProviderDriverKind::Grok, "grok-inspect")], true);
}

#[cfg(unix)]
#[test]
fn stopping_private_opencode_server_reaps_owned_helpers() {
    use crate::core::provider_connections::ProviderDriverKind;
    assert_stopped_provider_stages(&[(ProviderDriverKind::OpenCode, "opencode-server")], true);
}

#[cfg(unix)]
fn block_fix_provider_at(
    config: &crate::core::provider_connections::ProviderConnectionConfig,
    stage: &str,
    marker: &Path,
    helpers: bool,
) {
    use crate::core::provider_connections::ProviderConnectionConfig;
    let binary = match config {
        ProviderConnectionConfig::Grok(config) => config.binary_path.as_deref().unwrap(),
        ProviderConnectionConfig::OpenCode(config) => config.binary_path.as_deref().unwrap(),
        _ => unreachable!(),
    };
    let original = fs::read_to_string(binary).unwrap();
    let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
    let environment = format!(
        "#!/bin/sh\n\
         export CVEC_FAKE_FIX_BLOCK_STAGE={}\n\
         export CVEC_FAKE_FIX_MARKER={}\n\
         export CVEC_FAKE_FIX_WITH_DESCENDANT={}\n",
        quote(stage),
        quote(&marker.to_string_lossy()),
        u8::from(helpers),
    );
    fs::write(binary, original.replacen("#!/bin/sh\n", &environment, 1)).unwrap();
}

#[cfg(unix)]
fn assert_stopped_provider_stages(
    stages: &[(crate::core::provider_connections::ProviderDriverKind, &str)],
    helpers: bool,
) {
    for &(driver, stage) in stages {
        let fixture = fixture();
        let snapshot = create(&fixture);
        let id = configure_fix_provider(&fixture, driver);
        let connection = fixture.db.ai_connection(&id).unwrap();
        let marker = fixture._temp.path().join("blocked-probe");
        block_fix_provider_at(&connection.config, stage, &marker, helpers);
        fixture
            .service
            .start(
                1,
                &snapshot.workspace.id,
                FixStartRequest {
                    connection_id: id,
                    connection_updated_at: connection.updated_at,
                    prompt: "Fix this fixture".into(),
                    model: Some("fixture/model".into()),
                    effort: None,
                    consent: true,
                },
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid: i32 = fs::read_to_string(&marker).expect(stage).parse().unwrap();
        let server = fs::read_to_string(marker.with_extension("server"))
            .ok()
            .map(|value| value.parse::<i32>().unwrap());
        let helper = helpers.then(|| {
            fs::read_to_string(marker.with_extension("helper"))
                .unwrap()
                .parse::<i32>()
                .unwrap()
        });
        let alive = |pid: i32| {
            assert!(pid > 1);
            Command::new("/bin/ps")
                .args(["-p", &pid.to_string(), "-o", "stat="])
                .stderr(std::process::Stdio::null())
                .output()
                .is_ok_and(|output| {
                    let state = String::from_utf8_lossy(&output.stdout);
                    output.status.success()
                        && !state.trim().is_empty()
                        && !state.trim().starts_with('Z')
                })
        };
        assert!(alive(pid), "{stage}");
        fixture.service.stop(1, &snapshot.workspace.id).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while (alive(pid) || server.is_some_and(alive) || helper.is_some_and(alive))
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let helper_survived = helper.is_some_and(alive);
        if let Some(pid) = helper.filter(|_| helper_survived) {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        }
        assert!(!alive(pid), "{stage}: blocked probe survived Stop");
        assert!(
            !server.is_some_and(alive),
            "{stage}: private server survived Stop"
        );
        assert_eq!(
            marker.with_extension("run").exists(),
            stage == "opencode-completion",
            "{stage}: unexpected model phase"
        );
        assert_eq!(
            fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
            "export const fixed = false;\n"
        );
        assert_eq!(fixture.service.list(1).unwrap()[0].status, "interrupted");
        fixture.service.shutdown();
        assert!(!helper_survived, "{stage}: owned helper survived Stop");
    }
}

#[cfg(unix)]
#[test]
fn opencode_confirms_missing_finish_events_and_rejects_incomplete_turns() {
    use crate::core::provider_connections::ProviderDriverKind;
    for (prompt, expected) in [
        ("[[NATIVE_FINISH_GAP]]", "ready"),
        ("[[NATIVE_FINISH_GAP]] [[FAILED_COMPLETION]]", "failed"),
        ("[[INCOMPLETE_FINISH]]", "failed"),
        ("[[NO_OUTPUT]]", "failed"),
        ("[[NATIVE_FINISH_GAP]] [[EXIT_FAILURE]]", "failed"),
    ] {
        let fixture = fixture();
        let snapshot = create(&fixture);
        let id = configure_fix_provider(&fixture, ProviderDriverKind::OpenCode);
        fixture
            .service
            .start(
                1,
                &snapshot.workspace.id,
                FixStartRequest {
                    connection_updated_at: fixture.db.ai_connection(&id).unwrap().updated_at,
                    connection_id: id,
                    prompt: prompt.into(),
                    model: Some("fixture/model".into()),
                    effort: None,
                    consent: true,
                },
            )
            .unwrap();
        let current = wait_settled(&fixture, &snapshot.workspace.id);
        assert_eq!(current.status, expected, "{prompt}: {}", current.activity);
        assert_eq!(
            fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
            "export const fixed = false;\n"
        );
        fixture.service.shutdown();
    }
}

#[cfg(unix)]
fn assert_opencode_session_identity_is_required(prompt: &str) {
    use crate::core::provider_connections::ProviderDriverKind;
    let fixture = fixture();
    let snapshot = create(&fixture);
    let id = configure_fix_provider(&fixture, ProviderDriverKind::OpenCode);
    fixture
        .service
        .start(
            1,
            &snapshot.workspace.id,
            FixStartRequest {
                connection_updated_at: fixture.db.ai_connection(&id).unwrap().updated_at,
                connection_id: id,
                prompt: prompt.into(),
                model: Some("fixture/model".into()),
                effort: None,
                consent: true,
            },
        )
        .unwrap();
    let current = wait_settled(&fixture, &snapshot.workspace.id);
    let saved = wait_saved_workspace(&fixture, &current);
    fixture.service.shutdown();
    assert_eq!(current.status, "failed", "{}", current.activity);
    assert_eq!(saved.status, "failed");
    assert_eq!(
        fs::read_to_string(Path::new(&current.worktree_path).join("app.ts")).unwrap(),
        "export const fixed = true;\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
        "export const fixed = false;\n"
    );
}

#[cfg(unix)]
#[test]
fn opencode_without_session_identity_cannot_report_a_successful_fix() {
    assert_opencode_session_identity_is_required("[[NO_SESSION_ID]]");
}

#[cfg(unix)]
#[test]
fn opencode_with_malformed_session_identity_cannot_report_a_successful_fix() {
    assert_opencode_session_identity_is_required("[[INVALID_SESSION_ID]]");
}

#[cfg(unix)]
#[test]
fn grok_and_opencode_share_worktree_tools_and_keep_provider_provenance() {
    use crate::core::provider_connections::ProviderDriverKind;
    let fixture = fixture();
    let snapshot = create(&fixture);
    for driver in [ProviderDriverKind::Grok, ProviderDriverKind::OpenCode] {
        let id = configure_fix_provider(&fixture, driver);
        fixture
            .service
            .start(
                1,
                &snapshot.workspace.id,
                FixStartRequest {
                    connection_updated_at: fixture.db.ai_connection(&id).unwrap().updated_at,
                    connection_id: id.clone(),
                    prompt: "Fix this fixture".into(),
                    model: Some("fixture/model".into()),
                    effort: None,
                    consent: true,
                },
            )
            .unwrap();
        wait_ready(&fixture, &snapshot.workspace.id);
        let current = fixture.service.snapshot(1, &snapshot.workspace.id).unwrap();
        assert_eq!(
            current.workspace.status, "ready",
            "{}",
            current.workspace.activity
        );
        assert_eq!(
            current.workspace.provider.as_ref().unwrap().connection_id,
            id
        );
        assert_eq!(
            current
                .workspace
                .messages
                .last()
                .unwrap()
                .provider
                .as_ref()
                .unwrap()
                .driver,
            driver
        );
        assert_eq!(
            fs::read_to_string(Path::new(&current.workspace.worktree_path).join("app.ts")).unwrap(),
            "export const fixed = true;\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
            "export const fixed = false;\n"
        );
        assert!(!fixture.service.0.root.join("outside.txt").exists());
    }
    let saved = fixture.db.load_fix_workspaces(1).unwrap();
    assert_eq!(
        saved[0].messages[0].provider.as_ref().unwrap().driver,
        ProviderDriverKind::Grok
    );
    assert_eq!(
        saved[0].messages[2].provider.as_ref().unwrap().driver,
        ProviderDriverKind::OpenCode
    );
    fixture.service.shutdown();
}

#[cfg(unix)]
#[test]
fn stopping_new_providers_revokes_old_access_and_allows_retry() {
    use crate::core::provider_connections::ProviderDriverKind;
    for driver in [ProviderDriverKind::Grok, ProviderDriverKind::OpenCode] {
        let fixture = fixture();
        let snapshot = create(&fixture);
        let id = configure_fix_provider(&fixture, driver);
        let connection = fixture.db.ai_connection(&id).unwrap();
        let marker = fixture._temp.path().join("active-fix");
        block_fix_provider_at(&connection.config, "active-fix", &marker, false);
        fixture
            .service
            .start(
                1,
                &snapshot.workspace.id,
                FixStartRequest {
                    connection_updated_at: connection.updated_at,
                    connection_id: id.clone(),
                    prompt: "[[WAIT]]".into(),
                    model: Some("fixture/model".into()),
                    effort: None,
                    consent: true,
                },
            )
            .unwrap();
        // The fake provider signals once all setup tool requests have returned.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.is_file() {
            assert!(
                Instant::now() < deadline,
                "{driver:?} did not reach the active fix stop point"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let before = fixture.service.list(1).unwrap()[0].clone();
        fixture.service.stop(1, &snapshot.workspace.id).unwrap();
        let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
        assert!(session
            .call(
                "cvec_workspace_write_file",
                &json!({"path":"app.ts","content":"bad"}),
                before.provider_thread_id.as_deref().unwrap(),
                before.turn_id.as_deref().unwrap()
            )
            .is_err());
        fixture
            .service
            .start(
                1,
                &snapshot.workspace.id,
                FixStartRequest {
                    connection_updated_at: fixture.db.ai_connection(&id).unwrap().updated_at,
                    connection_id: id,
                    prompt: "Retry".into(),
                    model: Some("fixture/model".into()),
                    effort: None,
                    consent: true,
                },
            )
            .unwrap();
        wait_ready(&fixture, &snapshot.workspace.id);
        let after = fixture.service.list(1).unwrap()[0].clone();
        assert_eq!(after.status, "ready", "{}", after.activity);
        assert_ne!(before.provider_thread_id, after.provider_thread_id);
        assert_eq!(
            after
                .messages
                .iter()
                .filter(|message| message.role == "assistant")
                .count(),
            1
        );
        fixture.service.shutdown();
    }
}

#[test]
fn historical_codex_records_and_connection_failures_preserve_workspaces() {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let mut old = serde_json::to_value(&snapshot.workspace).unwrap();
    old["schemaVersion"] = json!(1);
    old.as_object_mut().unwrap().remove("provider");
    let old: FixWorkspace = serde_json::from_value(old).unwrap();
    assert!(old.is_supported_record());
    assert!(old.provider.is_none());
    for connection_id in ["missing", "bad/connection"] {
        assert!(fixture
            .service
            .start(
                1,
                &snapshot.workspace.id,
                FixStartRequest {
                    connection_updated_at: fixture
                        .db
                        .ai_connection("codex_default")
                        .unwrap()
                        .updated_at,
                    connection_id: connection_id.into(),
                    prompt: "Task".into(),
                    model: Some("fake-model".into()),
                    effort: None,
                    consent: true
                }
            )
            .is_err());
    }
    assert!(fixture.service.list(1).unwrap()[0].messages.is_empty());
}

#[cfg(unix)]
#[test]
fn stale_provider_consent_and_unsupported_effort_do_not_submit_a_task() {
    use crate::core::provider_connections::ProviderDriverKind;
    let fixture = fixture();
    let snapshot = create(&fixture);
    let id = configure_fix_provider(&fixture, ProviderDriverKind::Grok);
    let updated_at = fixture.db.ai_connection(&id).unwrap().updated_at;
    for (revision, effort) in [("outdated".into(), None), (updated_at, Some("high".into()))] {
        assert!(fixture
            .service
            .start(
                1,
                &snapshot.workspace.id,
                FixStartRequest {
                    connection_id: id.clone(),
                    connection_updated_at: revision,
                    prompt: "Task".into(),
                    model: Some("fixture/model".into()),
                    effort,
                    consent: true,
                }
            )
            .is_err());
    }
    let after = fixture.service.list(1).unwrap()[0].clone();
    assert!(after.messages.is_empty());
    assert_eq!(after.status, "ready");
    assert!(after.provider.is_none());
}

#[cfg(unix)]
#[test]
fn opencode_process_failure_cannot_report_a_successful_fix() {
    use crate::core::provider_connections::ProviderDriverKind;
    let fixture = fixture();
    let snapshot = create(&fixture);
    let id = configure_fix_provider(&fixture, ProviderDriverKind::OpenCode);
    fixture
        .service
        .start(
            1,
            &snapshot.workspace.id,
            FixStartRequest {
                connection_updated_at: fixture.db.ai_connection(&id).unwrap().updated_at,
                connection_id: id,
                prompt: "[[EXIT_FAILURE]]".into(),
                model: Some("fixture/model".into()),
                effort: None,
                consent: true,
            },
        )
        .unwrap();
    wait_settled(&fixture, &snapshot.workspace.id);
    let after = fixture.service.list(1).unwrap()[0].clone();
    assert_eq!(after.status, "failed");
    assert!(after.activity.contains("exited unsuccessfully"));
    assert_eq!(
        fs::read_to_string(Path::new(&after.worktree_path).join("app.ts")).unwrap(),
        "export const fixed = true;\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
        "export const fixed = false;\n"
    );
    fixture.service.shutdown();
}

#[cfg(unix)]
#[test]
fn opencode_fix_preserves_useful_errors_and_redacts_authentication_diagnostics() {
    use crate::core::provider_connections::ProviderDriverKind;
    for (prompt, expected) in [
        (
            "[[PROVIDER_ERROR]]",
            "Connection refused by the configured model endpoint.",
        ),
        ("[[AUTH_ERROR]]", "provider diagnostics were redacted"),
    ] {
        let fixture = fixture();
        let snapshot = create(&fixture);
        let id = configure_fix_provider(&fixture, ProviderDriverKind::OpenCode);
        fixture
            .service
            .start(
                1,
                &snapshot.workspace.id,
                FixStartRequest {
                    connection_updated_at: fixture.db.ai_connection(&id).unwrap().updated_at,
                    connection_id: id,
                    prompt: prompt.into(),
                    model: Some("fixture/model".into()),
                    effort: None,
                    consent: true,
                },
            )
            .unwrap();
        let after = wait_settled(&fixture, &snapshot.workspace.id);
        assert_eq!(after.status, "failed");
        assert!(after.activity.contains(expected), "{}", after.activity);
        assert!(!after.activity.contains("synthetic-sensitive-value"));
        assert!(!after.activity.contains("Private diagnostic"));
        fixture.service.shutdown();
    }
}

#[cfg(unix)]
#[test]
fn opencode_inherited_stdout_retains_completion_and_the_real_exit_outcome() {
    use crate::core::provider_connections::ProviderDriverKind;
    for (prompt, expected) in [
        ("[[STDOUT_HELPER]]", "ready"),
        ("[[STDOUT_HELPER]] [[EXIT_FAILURE]]", "failed"),
    ] {
        let fixture = fixture();
        let snapshot = create(&fixture);
        let id = configure_fix_provider(&fixture, ProviderDriverKind::OpenCode);
        fixture
            .service
            .start(
                1,
                &snapshot.workspace.id,
                FixStartRequest {
                    connection_updated_at: fixture.db.ai_connection(&id).unwrap().updated_at,
                    connection_id: id,
                    prompt: prompt.into(),
                    model: Some("fixture/model".into()),
                    effort: None,
                    consent: true,
                },
            )
            .unwrap();
        let after = wait_settled(&fixture, &snapshot.workspace.id);
        assert_eq!(after.status, expected, "{}", after.activity);
        assert!(after.messages.iter().any(|message| {
            message.role == "assistant"
                && message.text.contains("Applied the focused worktree change")
        }));
        if expected == "failed" {
            assert!(after.activity.contains("exited unsuccessfully"));
        }
        assert_eq!(
            fs::read_to_string(Path::new(&after.worktree_path).join("app.ts")).unwrap(),
            "export const fixed = true;\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
            "export const fixed = false;\n"
        );
        fixture.service.shutdown();
        fixture.host.shutdown();
    }
}

#[test]
fn codex_fix_redacts_terminal_diagnostics_before_display_and_storage() {
    for (prompt, expected) in [
        (
            "[[PROVIDER_ERROR]]",
            "Connection refused by the configured model endpoint.",
        ),
        ("[[AUTH_ERROR]]", "provider diagnostics were redacted"),
    ] {
        let fixture = fixture();
        let snapshot = create(&fixture);
        fixture
            .service
            .start(
                1,
                &snapshot.workspace.id,
                FixStartRequest {
                    connection_updated_at: fixture
                        .db
                        .ai_connection("codex_default")
                        .unwrap()
                        .updated_at,
                    connection_id: "codex_default".into(),
                    prompt: prompt.into(),
                    model: Some("fake-model".into()),
                    effort: None,
                    consent: true,
                },
            )
            .unwrap();
        let after = wait_settled(&fixture, &snapshot.workspace.id);
        let saved = wait_saved_workspace(&fixture, &after);
        for workspace in [after, saved] {
            assert_eq!(workspace.status, "failed");
            assert!(
                workspace.activity.contains(expected),
                "{}",
                workspace.activity
            );
            assert!(!workspace.activity.contains("synthetic-sensitive-value"));
            assert!(!workspace.activity.contains("Private diagnostic"));
        }
        fixture.service.shutdown();
        fixture.host.shutdown();
    }
}

#[cfg(unix)]
#[test]
fn grok_prompt_rejection_keeps_the_worktree_and_allows_explicit_retry() {
    use crate::core::provider_connections::ProviderDriverKind;
    let fixture = fixture();
    let snapshot = create(&fixture);
    let id = configure_fix_provider(&fixture, ProviderDriverKind::Grok);
    let request = |prompt: &str| FixStartRequest {
        connection_updated_at: fixture.db.ai_connection(&id).unwrap().updated_at,
        connection_id: id.clone(),
        prompt: prompt.into(),
        model: Some("fixture/model".into()),
        effort: None,
        consent: true,
    };
    fixture
        .service
        .start(1, &snapshot.workspace.id, request("[[LICENSE_ERROR]]"))
        .unwrap();
    let rejected = wait_settled(&fixture, &snapshot.workspace.id);
    assert_eq!(rejected.status, "failed");
    assert!(rejected.activity.contains("rejected session/prompt"));
    let session = fixture.service.session(1, &snapshot.workspace.id).unwrap();
    assert!(session.stopped.load(Ordering::SeqCst));
    assert!(!session.agent_active.load(Ordering::SeqCst));
    assert_eq!(
        fs::read_to_string(Path::new(&rejected.worktree_path).join("app.ts")).unwrap(),
        "export const fixed = false;\n"
    );
    assert_eq!(wait_saved_workspace(&fixture, &rejected).status, "failed");
    // The fixture now accepts a later explicit task. No account, credential or
    // real license is changed; this tests recovery from a rejected ACP prompt.
    fixture
        .service
        .start(
            1,
            &snapshot.workspace.id,
            request("Retry the synthetic fix"),
        )
        .unwrap();
    wait_ready(&fixture, &snapshot.workspace.id);
    assert_eq!(
        fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
        "export const fixed = false;\n"
    );
    fixture.service.shutdown();
    fixture.host.shutdown();
}

fn assert_codex_fix_rpc_diagnostics_are_redacted(stage: &str) {
    let fixture = fixture();
    let snapshot = create(&fixture);
    let request = FixStartRequest {
        connection_updated_at: fixture
            .db
            .ai_connection("codex_default")
            .unwrap()
            .updated_at,
        connection_id: "codex_default".into(),
        prompt: match stage {
            "start" => "[[START_RPC_ERROR]]",
            "stop" => "[[STOP_RPC_ERROR]] [[WAIT]]",
            _ => "Create the synthetic provider session",
        }
        .into(),
        model: Some(
            if stage == "create" {
                "fixture-create-rpc-error"
            } else {
                "fake-model"
            }
            .into(),
        ),
        effort: None,
        consent: true,
    };
    let start = fixture.service.start(1, &snapshot.workspace.id, request);
    let error = if stage == "stop" {
        start.unwrap();
        fixture.service.stop(1, &snapshot.workspace.id).unwrap_err()
    } else {
        start.unwrap_err()
    };
    let saved = fixture.db.load_fix_workspaces(1).unwrap().remove(0);
    fixture.service.shutdown();
    fixture.host.shutdown();
    let text = error.to_string();
    assert!(
        !text.contains("synthetic-sensitive-value"),
        "{stage}: {text}"
    );
    assert!(!text.contains("Private diagnostic"), "{stage}: {text}");
    assert!(
        text.contains("provider diagnostics were redacted"),
        "{stage}: {text}"
    );
    assert!(!saved.activity.contains("synthetic-sensitive-value"));
    assert!(!saved.activity.contains("Private diagnostic"));
    assert_eq!(
        saved.status,
        if stage == "stop" {
            "interrupted"
        } else {
            "failed"
        }
    );
    assert_eq!(
        fs::read_to_string(fixture.source.join("app.ts")).unwrap(),
        "export const fixed = false;\n"
    );
}

#[test]
fn codex_fix_rpc_creation_diagnostics_are_redacted() {
    assert_codex_fix_rpc_diagnostics_are_redacted("create");
}

#[test]
fn codex_fix_rpc_start_diagnostics_are_redacted() {
    assert_codex_fix_rpc_diagnostics_are_redacted("start");
}

#[test]
fn codex_fix_rpc_stop_diagnostics_are_redacted() {
    assert_codex_fix_rpc_diagnostics_are_redacted("stop");
}

#[test]
fn archive_preserves_worktree_evidence_and_restarts_without_caching_archived_records() {
    let fixture = fixture();
    let initial = create(&fixture);
    let id = initial.workspace.id.clone();
    let root = PathBuf::from(&initial.workspace.worktree_path);
    files::write(&root, "app.ts", "export const fixed = true;\n").unwrap();
    let session = fixture.service.session(1, &id).unwrap();
    {
        let mut data = session.data.lock().unwrap();
        data.messages.push(FixMessage {
            role: "user".into(),
            text: "Retain this task".into(),
            provider: None,
        });
        data.checks.push(FixCheck {
            action: "npm:test:run".into(),
            status: "passed".into(),
            exit_code: Some(0),
            output: "retained check".into(),
            digest: files::digest(&root).unwrap(),
            stale: false,
            finished_at: now(),
        });
        data.rescan = Some(FixRescan {
            findings: Vec::new(),
            finding_count: 0,
            not_detected: None,
            target_detected: None,
            digest: files::digest(&root).unwrap(),
            stale: false,
            scanned_at: now(),
            scanner_versions: "Synthetic retained evidence".into(),
        });
    }
    session.publish().unwrap();
    let before = fixture.service.snapshot(1, &id).unwrap();
    let archived = fixture.service.set_archived(1, &id, true).unwrap();
    assert_eq!(archived.workspace.schema_version, 3);
    assert!(archived.workspace.archived_at.is_some());
    assert!(archived.actions.is_empty());
    assert!(fixture.service.list(1).unwrap().is_empty());
    assert!(!fixture.service.0.sessions.lock().unwrap().contains_key(&id));
    assert!(fixture.service.run_check(1, &id, "npm:test:run").is_err());
    assert!(fixture.service.rescan(1, &id).is_err());
    assert!(fixture
        .service
        .start(
            1,
            &id,
            FixStartRequest {
                connection_id: "codex_default".into(),
                connection_updated_at: fixture
                    .db
                    .ai_connection("codex_default")
                    .unwrap()
                    .updated_at,
                prompt: "forbidden archived task".into(),
                model: Some("fixture-model".into()),
                effort: None,
                consent: true
            }
        )
        .is_err());
    assert!(fixture.service.stop(1, &id).is_err());
    // A previously captured session cannot write an archived record after eviction.
    assert!(session.publish().is_err());
    let read_only = FixWorkspaces::new(
        fixture.db.clone(),
        fixture.service.0.root.clone(),
        fixture.service.0.scanner_cache.clone(),
        fixture.host.clone(),
        |_| {},
    )
    .unwrap();
    assert!(read_only.0.write_lease.is_none());
    assert!(read_only.set_archived(1, &id, false).is_err());
    let selected = read_only.snapshot(1, &id).unwrap();
    assert_eq!(
        serde_json::to_value(&selected.workspace.messages).unwrap(),
        serde_json::to_value(&before.workspace.messages).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&selected.workspace.checks).unwrap(),
        serde_json::to_value(&before.workspace.checks).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&selected.workspace.rescan).unwrap(),
        serde_json::to_value(&before.workspace.rescan).unwrap()
    );
    assert_eq!(
        selected.diff.as_ref().unwrap().digest,
        before.diff.as_ref().unwrap().digest
    );
    assert!(read_only.0.sessions.lock().unwrap().is_empty());
    let restored = fixture.service.set_archived(1, &id, false).unwrap();
    assert_eq!(restored.workspace.schema_version, 3);
    assert!(restored.workspace.archived_at.is_none());
    assert_eq!(restored.workspace.checks[0].output, "retained check");
    assert_eq!(
        files::git_text(&root, &["symbolic-ref", "--short", "HEAD"]).unwrap(),
        initial.workspace.branch
    );
    assert_eq!(
        fs::read_to_string(root.join("app.ts")).unwrap(),
        "export const fixed = true;\n"
    );
    assert_eq!(fixture.service.list(1).unwrap().len(), 1);
    assert_eq!(read_only.list(1).unwrap().len(), 1);
    fixture.service.set_archived(1, &id, true).unwrap();
    assert!(read_only.list(1).unwrap().is_empty());
    assert_eq!(
        fixture.service.archived(1, None, None).unwrap().entries[0].title,
        "Retain this task"
    );
    assert!(fixture.service.snapshot(2, &id).is_err());
    let workspace_root = fixture.service.0.root.clone();
    let cache = fixture.service.0.scanner_cache.clone();
    fixture.service.shutdown();
    drop(fixture.service);
    let restarted = FixWorkspaces::new(
        fixture.db.clone(),
        workspace_root,
        cache,
        fixture.host.clone(),
        |_| {},
    )
    .unwrap();
    assert!(restarted.0.write_lease.is_some());
    assert!(restarted.list(1).unwrap().is_empty());
    assert_eq!(
        restarted.snapshot(1, &id).unwrap().workspace.checks[0].output,
        "retained check"
    );
    assert!(restarted.0.sessions.lock().unwrap().is_empty());
    let restored = restarted.set_archived(1, &id, false).unwrap();
    assert_eq!(restored.workspace.schema_version, 3);
    assert_eq!(restored.workspace.messages[0].text, "Retain this task");
}

#[test]
fn archive_refuses_busy_pending_and_failed_storage_without_changing_capacity() {
    let fixture = fixture();
    let initial = create(&fixture);
    let id = &initial.workspace.id;
    let session = fixture.service.session(1, id).unwrap();
    session.start_pending.store(true, Ordering::SeqCst);
    assert!(fixture.service.set_archived(1, id, true).is_err());
    session.start_pending.store(false, Ordering::SeqCst);
    session.agent_active.store(true, Ordering::SeqCst);
    assert!(fixture.service.set_archived(1, id, true).is_err());
    session.agent_active.store(false, Ordering::SeqCst);
    session.data.lock().unwrap().status = "testing".into();
    assert!(fixture.service.set_archived(1, id, true).is_err());
    session.data.lock().unwrap().status = "ready".into();
    let db = rusqlite::Connection::open(fixture.db.path()).unwrap();
    db.execute_batch("CREATE TRIGGER fixture_reject_archive BEFORE UPDATE ON fix_workspaces BEGIN SELECT RAISE(ABORT,'fixture storage failure'); END;").unwrap();
    assert!(fixture.service.set_archived(1, id, true).is_err());
    assert!(session.data.lock().unwrap().archived_at.is_none());
    assert_eq!(fixture.db.active_fix_workspace_count(1).unwrap(), 1);
    assert!(fixture
        .service
        .archived(1, None, None)
        .unwrap()
        .entries
        .is_empty());
    assert!(fixture
        .db
        .load_fix_workspace(1, id)
        .unwrap()
        .archived_at
        .is_none());
}

#[test]
fn archived_inventory_is_paged_and_capacity_admission_is_shared_with_creation() {
    let fixture = fixture();
    let initial = create(&fixture);
    // Synthetic historical records exercise paging without creating unrelated worktrees.
    for number in 1..=45 {
        let mut data = initial.workspace.clone();
        data.id = format!("{number:024x}");
        data.schema_version = 3;
        data.archived_at = Some(format!("2026-11-10T00:00:{number:02}Z"));
        fixture.db.save_fix_workspace(&data).unwrap();
    }
    let mut cursor = None;
    let mut ids = HashSet::new();
    let mut lengths = Vec::new();
    loop {
        let page = fixture.service.archived(1, cursor, None).unwrap();
        lengths.push(page.entries.len());
        for entry in page.entries {
            assert!(ids.insert(entry.id));
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(lengths, vec![20, 20, 5]);
    assert_eq!(ids.len(), 45);
    assert_eq!(fixture.service.0.sessions.lock().unwrap().len(), 1);
    assert!(fixture
        .service
        .archived(2, None, None)
        .unwrap()
        .entries
        .is_empty());
    assert!(fixture
        .service
        .archived(1, Some("bad cursor".into()), None)
        .is_err());
    for number in 100..119 {
        let mut data = initial.workspace.clone();
        data.id = format!("{number:024x}");
        fixture.db.save_fix_workspace(&data).unwrap();
    }
    assert_eq!(fixture.db.active_fix_workspace_count(1).unwrap(), 20);
    assert!(fixture
        .service
        .create(1, None, true, None)
        .unwrap_err()
        .to_string()
        .contains("Archive"));
    assert!(fixture
        .service
        .set_archived(1, &format!("{:024x}", 1), false)
        .is_err());
    fixture
        .service
        .set_archived(1, &initial.workspace.id, true)
        .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let restore_service = fixture.service.clone();
    let restore_barrier = barrier.clone();
    let id = initial.workspace.id.clone();
    let restore = std::thread::spawn(move || {
        restore_barrier.wait();
        restore_service.set_archived(1, &id, false)
    });
    let create_service = fixture.service.clone();
    let create = std::thread::spawn(move || {
        barrier.wait();
        create_service.create(1, None, true, None)
    });
    let successes = [
        restore.join().unwrap().is_ok(),
        create.join().unwrap().is_ok(),
    ]
    .into_iter()
    .filter(|ok| *ok)
    .count();
    assert_eq!(successes, 1);
    assert_eq!(fixture.db.active_fix_workspace_count(1).unwrap(), 20);
}

#[test]
fn archive_migration_preserves_legacy_and_future_record_bytes() {
    let fixture = fixture();
    let mut old = create(&fixture).workspace;
    old.schema_version = 1;
    let database = Database::new(fixture._temp.path().join("legacy.db")).unwrap();
    database
        .sync_repositories(&[DiscoveredRepo {
            name: "fixture".into(),
            path: fixture.source.clone(),
            remote_url: None,
            remote_host: None,
            provider: None,
        }])
        .unwrap();
    let mut future = old.clone();
    future.schema_version = 99;
    future.id = "f".repeat(24);
    let connection = rusqlite::Connection::open(database.path()).unwrap();
    connection.execute_batch("CREATE TABLE fix_workspaces (id TEXT PRIMARY KEY, repository_id INTEGER NOT NULL REFERENCES repos(id), updated_at TEXT NOT NULL, record_json TEXT NOT NULL);").unwrap();
    let old_json = serde_json::to_string(&old).unwrap();
    let future_json = serde_json::to_string(&future).unwrap();
    for (data, json) in [(&old, &old_json), (&future, &future_json)] {
        connection
            .execute(
                "INSERT INTO fix_workspaces VALUES (?1,1,?2,?3)",
                rusqlite::params![data.id, data.updated_at, json],
            )
            .unwrap();
    }
    let service = FixWorkspaces::new(
        database,
        fixture._temp.path().join("legacy-workspaces"),
        fixture._temp.path().join("legacy-cache"),
        fixture.host.clone(),
        |_| {},
    )
    .unwrap();
    assert_eq!(service.list(1).unwrap().len(), 1);
    for (data, original) in [(&old, old_json), (&future, future_json)] {
        let saved: String = connection
            .query_row(
                "SELECT record_json FROM fix_workspaces WHERE id=?1",
                [&data.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, original);
    }
    assert!(service.archived(1, None, None).unwrap().entries.is_empty());
}

#[test]
fn restore_transition_cannot_create_a_second_active_session() {
    let fixture = fixture();
    let initial = create(&fixture);
    fixture
        .service
        .set_archived(1, &initial.workspace.id, true)
        .unwrap();
    // Simulate the interval between restore's durable write and map insertion.
    let mut data = fixture
        .db
        .load_fix_workspace(1, &initial.workspace.id)
        .unwrap();
    data.archived_at = None;
    fixture.db.save_fix_workspace(&data).unwrap();
    let error = fixture
        .service
        .run_check(1, &data.id, "npm:test:run")
        .unwrap_err();
    assert!(error.to_string().contains("inventory is changing"));
    assert!(fixture.service.0.sessions.lock().unwrap().is_empty());
    assert!(fixture
        .db
        .load_fix_workspace(1, &data.id)
        .unwrap()
        .checks
        .is_empty());
}
