//! Explicit, synthetic provider acceptance. Never part of the default suite.
use super::*;
use crate::core::provider_connections::*;

fn connection() -> ProviderConnection {
    assert_eq!(
        std::env::var("CVEC_RUN_FIX_LIVE_SMOKE").as_deref(),
        Ok("1"),
        "Explicit live-test opt-in is required"
    );
    let driver = match std::env::var("CVEC_FIX_LIVE_PROVIDER").as_deref() {
        Ok("grok") => ProviderDriverKind::Grok,
        Ok("opencode") => ProviderDriverKind::OpenCode,
        _ => panic!("Choose grok or opencode explicitly"),
    };
    ProviderConnection::builtin_defaults("live-test".into())
        .into_iter()
        .find(|value| value.driver == driver)
        .unwrap()
}

fn inventory(connection: &ProviderConnection) -> ProviderConnectionSnapshot {
    match connection.driver {
        ProviderDriverKind::Grok => probe_grok(connection),
        ProviderDriverKind::OpenCode => probe_opencode(connection),
        _ => unreachable!(),
    }
}

fn write_receipt(value: Value) {
    let target = PathBuf::from(
        std::env::var_os("CVEC_FIX_LIVE_OUTPUT").expect("Explicit local receipt path required"),
    );
    fs::write(target, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

#[test]
#[ignore = "Explicit live inventory of existing CLI connection; no inference"]
fn live_fix_provider_inventory() {
    let connection = connection();
    let snapshot = inventory(&connection);
    let requested = std::env::var("CVEC_FIX_LIVE_MODEL").ok();
    let selected = requested
        .as_deref()
        .and_then(|id| snapshot.models.iter().find(|model| model.model.id == id))
        .or_else(|| snapshot.models.iter().find(|model| model.model.is_default));
    let result = json!({"driver":connection.driver,"health":snapshot.health,"modelCount":snapshot.models.len(),"selectedModel":selected.map(|model|&model.model.id),"inferenceBoundary":selected.map(|model|model.inference_boundary),"requestedModelAvailable":requested.as_ref().map(|id|snapshot.models.iter().any(|model|&model.model.id==id)),"errorCode":snapshot.error.as_ref().map(|error|&error.code),"inferenceRequests":0});
    println!("{result}");
    write_receipt(result);
}

#[test]
#[ignore = "Explicit live model task on a disposable synthetic worktree; no production source"]
fn live_fix_provider_smoke() {
    let connection = connection();
    let model =
        std::env::var("CVEC_FIX_LIVE_MODEL").expect("An exact inventory model must be selected");
    let snapshot = inventory(&connection);
    assert_eq!(
        snapshot.health,
        ProviderHealthState::Ready,
        "Existing connection is not ready"
    );
    assert!(
        snapshot.models.iter().any(|value| value.model.id == model),
        "Selected model is not in the inventory"
    );
    let fixture = fixture();
    // Replace only the disposable fixture's initial no-op script with a real
    // assertion. Both baseline commits belong to this synthetic repository.
    let script = "node -e \"const fs=require('node:fs');const assert=require('node:assert/strict');assert.match(fs.readFileSync('app.ts','utf8'),/fixed = true/);console.log('synthetic fix assertion passed')\"";
    fs::write(
        fixture.source.join("package.json"),
        serde_json::to_string(
            &json!({"name":"cvec-live-fix-fixture","private":true,"scripts":{"test:run":script}}),
        )
        .unwrap(),
    )
    .unwrap();
    files::git(&fixture.source, &["add", "package.json"]).unwrap();
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
            "Synthetic assertion baseline",
        ],
    )
    .unwrap();
    // Reuse only public scanner database files, never a profile or credentials.
    if let Some(cache) = std::env::var_os("CVEC_FIX_LIVE_GRYPE_DB") {
        let target = fixture._temp.path().join("scanner-cache/6");
        fs::create_dir_all(&target).unwrap();
        for name in ["vulnerability.db", "import.json", "last_update_check"] {
            let source = PathBuf::from(&cache).join(name);
            if source.is_file() {
                fs::copy(source, target.join(name)).unwrap();
            }
        }
    }
    let selected = fixture
        .db
        .save_ai_connection(&ProviderConnectionDraft {
            id: connection.id.as_ref().to_owned(),
            driver: connection.driver,
            display_name: connection.display_name,
            accent_color: None,
            enabled: true,
            config: connection.config,
            server_password: None,
        })
        .unwrap();
    let initial = create(&fixture);
    let baseline = fixture
        .service
        .run_check(1, &initial.workspace.id, "npm:test:run")
        .unwrap();
    assert_eq!(baseline.workspace.checks.last().unwrap().status, "failed");
    let start = fixture.service.start(1,&initial.workspace.id,FixStartRequest {
        connection_id:selected.id.as_ref().to_owned(),connection_updated_at:selected.updated_at,
        prompt:"This is a disposable synthetic acceptance task. Inspect app.ts using the cvec_workspace tools. Change only `export const fixed = false;` to `export const fixed = true;`. Run the available npm:test:run check using cvec_workspace_check. Do not change the test or package.json. Report the actual result briefly. Do not use native tools, commit, push or create a PR.".into(),
        model:Some(model.clone()),effort:None,consent:true,
    });
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut state = fixture.service.list(1).unwrap()[0].clone();
    while start.is_ok() && is_busy(&state.status) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        state = fixture.service.list(1).unwrap()[0].clone();
    }
    if is_busy(&state.status) {
        let _ = fixture.service.stop(1, &initial.workspace.id);
    }
    let mut current = fixture.service.snapshot(1, &initial.workspace.id).unwrap();
    let content =
        fs::read_to_string(Path::new(&current.workspace.worktree_path).join("app.ts")).unwrap();
    let original = fs::read_to_string(fixture.source.join("app.ts")).unwrap();
    let model_check = current.workspace.checks.len() > 1
        && current
            .workspace
            .checks
            .last()
            .is_some_and(|check| check.status == "passed");
    let mut rescan_error = None;
    if current.workspace.status == "ready"
        && content == "export const fixed = true;\n"
        && model_check
    {
        match fixture.service.rescan(1, &initial.workspace.id) {
            Ok(value) => current = value,
            Err(error) => rescan_error = Some(error.to_string()),
        }
    }
    let passed = current.workspace.status == "ready"
        && content == "export const fixed = true;\n"
        && original == "export const fixed = false;\n"
        && model_check
        && current.workspace.rescan.is_some();
    write_receipt(
        json!({"driver":selected.driver,"model":model,"passed":passed,"startError":start.err().map(|error|error.to_string()),"workspace":current.workspace,"diff":current.diff,"originalUnchanged":original=="export const fixed = false;\n","worktreeContent":content,"modelRanPassingCheck":model_check,"rescanError":rescan_error,"productionProfileOpened":false,"productionSourceSent":false}),
    );
    fixture.service.shutdown();
    fixture.host.shutdown();
    assert!(
        passed,
        "Synthetic live fix did not pass; inspect the explicitly selected local receipt"
    );
}
