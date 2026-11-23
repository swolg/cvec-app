use crate::core::timestamps;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

use crate::core::{
    assets,
    db::{Database, ScanReviewChanges},
    external_intel, github, github_advisory,
    github_auth::{GithubLoginPollResult, GithubLoginStart, GithubRepositoryOption},
    models::{
        AppNotificationKind, AppNotificationTone, AppSnapshot, ExternalIntelDetail,
        ExternalIntelFeed, GrypeDbStatus, GrypeDbUpdateOutcome, GrypeDbUpdateSummary,
        LocalEntityResult, OnboardingStatus, RepoRecord, RepoSourceType, ReviewActionBasis,
        ReviewPage, ReviewQuery, ReviewUpdate, RiskTrendPoint, ScanActivity, ScanActivityStage,
        ScanActivityStatus, ScanAllResult, ScanAllSummary, ScanOutput, ScanRepositoryOutcome,
        ScannerVersions, ScheduleInterval, SnoozeFindingInput, SourceSnapshot,
    },
    repo_discovery, scanner, url_policy,
};
use crate::state::{ScanGuard, SharedState};
use crate::{refresh_tray, refresh_tray_from_snapshot, set_review_badge, Error};

#[tauri::command]
pub async fn bootstrap(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<AppSnapshot, String> {
    let started = std::time::Instant::now();
    let shared = state.inner().clone();
    let cached = shared.clone();
    let snapshot = crate::runtime::run(move || refresh_snapshot(&cached))
        .await
        .map_err(|cause| cause.to_string())?
        .map_err(|cause| cause.to_string())?;
    if std::env::var_os("CVEC_PERF_TRACE").is_some() {
        eprintln!(
            "cvec-perf bootstrap persisted_ms={:.3} payload_bytes={}",
            started.elapsed().as_secs_f64() * 1000.0,
            serde_json::to_vec(&snapshot)
                .map(|value| value.len())
                .unwrap_or(0)
        );
    }
    set_review_badge(&app, snapshot.review_summary.needs_review);

    // Return persisted data first. Directory enumeration can wait on a slow or
    // unavailable filesystem and must never occupy AppKit's UI thread.
    if let Some(guard) = shared.begin_bootstrap_refresh() {
        tauri::async_runtime::spawn(async move {
            let _guard = guard;
            let discovery_state = shared.clone();
            let discovery_app = app.clone();
            let discovery = async move {
                let work = discovery_state.clone();
                match crate::runtime::run_io(move || sync_repositories(&work)).await {
                    Ok(Ok(())) => {
                        publish_persisted_snapshot(&discovery_app, &discovery_state).await
                    }
                    other => eprintln!("Background repository refresh failed: {other:?}"),
                }
            };
            let scanners = async move {
                let local = shared.clone();
                let _ = crate::runtime::run_io(move || local.refresh_scanner_local_status()).await;
                publish_persisted_snapshot(&app, &shared).await;
                let network = shared.clone();
                let _ =
                    crate::runtime::run_io(move || network.refresh_scanner_update_status()).await;
                publish_persisted_snapshot(&app, &shared).await;
            };
            tokio::join!(discovery, scanners);
        });
    }
    Ok(snapshot)
}

async fn publish_persisted_snapshot(app: &AppHandle, shared: &SharedState) {
    if let Ok(snapshot) = load_persisted_snapshot(shared).await {
        set_review_badge(app, snapshot.review_summary.needs_review);
        let _ = app.emit("cvec://state", snapshot);
    }
}

async fn load_persisted_snapshot(shared: &SharedState) -> Result<AppSnapshot, Error> {
    let work = shared.clone();
    crate::runtime::run(move || refresh_snapshot(&work))
        .await
        .map_err(Error::App)?
}

#[tauri::command]
pub async fn get_snapshot(state: State<'_, SharedState>) -> Result<AppSnapshot, String> {
    load_persisted_snapshot(state.inner())
        .await
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn get_repo_overview(
    state: State<'_, SharedState>,
    repo_id: i64,
) -> Result<crate::core::models::RepoOverview, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .repo_overview(repo_id)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn list_repository_assets(
    state: State<'_, SharedState>,
    repo_id: i64,
) -> Result<Vec<crate::core::models::RepoAsset>, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .list_repository_assets(repo_id)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn list_repository_dependencies(
    state: State<'_, SharedState>,
    repo_id: i64,
    query: crate::core::models::DependencyQuery,
) -> Result<crate::core::models::DependencyPage, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .dependency_page(repo_id, &query)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn get_repo_scan_comparison(
    state: State<'_, SharedState>,
    repo_id: i64,
    run_id: i64,
) -> Result<Option<crate::core::models::RepoScanComparison>, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .repo_scan_comparison(repo_id, run_id)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn list_risk_trend(
    state: State<'_, SharedState>,
    days: Option<u32>,
) -> Result<Vec<RiskTrendPoint>, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .list_risk_trend(days.unwrap_or(7))
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn list_review_findings(
    app: AppHandle,
    state: State<'_, SharedState>,
    query: ReviewQuery,
    preserve_selection: Option<String>,
    loaded_rows: Option<u32>,
) -> Result<ReviewPage, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        let page = shared
            .0
            .db
            .list_review_findings_preserving(&query, preserve_selection.as_deref(), loaded_rows)
            .map_err(|cause| cause.to_string())?;
        if let Some(summary) = &page.review_summary {
            set_review_badge(&app, summary.needs_review);
        }
        Ok(page)
    })
    .await?
}

#[tauri::command]
pub async fn get_review_finding_detail(
    state: State<'_, SharedState>,
    repo_id: i64,
    fingerprint: String,
    evidence_scan_run_id: i64,
    resolved_in_scan_run_id: Option<i64>,
) -> Result<crate::core::models::ReviewFinding, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .review_finding_detail_with_scope(
                repo_id,
                &fingerprint,
                evidence_scan_run_id,
                resolved_in_scan_run_id,
            )
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn get_finding_remediation(
    state: State<'_, SharedState>,
    repo_id: i64,
    fingerprint: String,
    evidence_scan_run_id: i64,
) -> Result<crate::core::models::FindingRemediationProjection, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .finding_remediation(repo_id, &fingerprint, evidence_scan_run_id)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn open_finding_manifest(
    state: State<'_, SharedState>,
    repo_id: i64,
    fingerprint: String,
    evidence_scan_run_id: i64,
) -> Result<(), String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        let target = shared
            .0
            .db
            .finding_manifest_target(repo_id, &fingerprint, evidence_scan_run_id)
            .map_err(|cause| cause.to_string())?;
        open_local_file(&target).map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn mark_finding_reviewed(
    app: AppHandle,
    state: State<'_, SharedState>,
    repo_id: i64,
    fingerprint: String,
    evidence: ReviewActionBasis,
) -> Result<Option<ReviewUpdate>, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .mark_finding_reviewed_with_evidence(repo_id, &fingerprint, Some(&evidence))
            .map_err(|cause| cause.to_string())?;
        Ok(update_review_summary(&app, &shared))
    })
    .await?
}

#[tauri::command]
pub async fn snooze_finding(
    app: AppHandle,
    state: State<'_, SharedState>,
    input: SnoozeFindingInput,
) -> Result<Option<ReviewUpdate>, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .snooze_finding_with_evidence(
                input.repo_id,
                &input.fingerprint,
                &input.until,
                input.note,
                Some(&input.evidence),
            )
            .map_err(|cause| cause.to_string())?;
        Ok(update_review_summary(&app, &shared))
    })
    .await?
}

#[tauri::command]
pub async fn return_finding_to_review(
    app: AppHandle,
    state: State<'_, SharedState>,
    repo_id: i64,
    fingerprint: String,
    evidence: ReviewActionBasis,
) -> Result<Option<ReviewUpdate>, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .clear_finding_review_with_evidence(repo_id, &fingerprint, Some(&evidence))
            .map_err(|cause| cause.to_string())?;
        Ok(update_review_summary(&app, &shared))
    })
    .await?
}

fn update_review_summary(app: &AppHandle, shared: &SharedState) -> Option<ReviewUpdate> {
    match shared.0.db.review_summary_with_revision() {
        Ok(update) => {
            set_review_badge(app, update.review_summary.needs_review);
            Some(update)
        }
        Err(cause) => {
            // The write already committed. A failed counter read must not undo it in the UI.
            eprintln!("Failed to refresh review summary: {cause}");
            None
        }
    }
}

#[tauri::command]
pub async fn search_local_entities(
    state: State<'_, SharedState>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<LocalEntityResult>, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .search_local_entities(&query, limit.unwrap_or(30))
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub fn get_scan_activity(state: State<'_, SharedState>) -> Option<ScanActivity> {
    state.inner().scan_activity()
}

#[tauri::command]
pub async fn clear_notification(
    state: State<'_, SharedState>,
    notification_id: i64,
) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .clear_notification(notification_id)
            .map_err(|cause| cause.to_string())?;
        refresh_snapshot(&shared).map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn clear_all_notifications(state: State<'_, SharedState>) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .clear_all_notifications()
            .map_err(|cause| cause.to_string())?;
        refresh_snapshot(&shared).map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn complete_onboarding(state: State<'_, SharedState>) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .set_onboarding_status(
                OnboardingStatus::Completed,
                env!("CARGO_PKG_VERSION").to_string(),
            )
            .map_err(|cause| cause.to_string())?;
        refresh_snapshot(&shared).map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn skip_onboarding(state: State<'_, SharedState>) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .set_onboarding_status(
                OnboardingStatus::Skipped,
                env!("CARGO_PKG_VERSION").to_string(),
            )
            .map_err(|cause| cause.to_string())?;
        refresh_snapshot(&shared).map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn list_external_intel_feed(
    state: State<'_, SharedState>,
) -> Result<ExternalIntelFeed, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .external_intel_feed()
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn get_external_intel_detail(
    state: State<'_, SharedState>,
    canonical_key: String,
    intel_revision: u64,
    inventory_revision: u64,
) -> Result<Option<ExternalIntelDetail>, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .external_intel_detail(&canonical_key, intel_revision, inventory_revision)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    open_url_in_system_browser(&url).map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn set_external_intel_topics(
    state: State<'_, SharedState>,
    topic_keys: Vec<String>,
) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .set_external_intel_topics(topic_keys)
            .map_err(|cause| cause.to_string())?;
        refresh_snapshot(&shared).map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn refresh_external_intel(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<AppSnapshot, String> {
    let token = state
        .inner()
        .0
        .github_auth
        .usable_token()
        .await
        .unwrap_or(None);
    external_intel::refresh(&state.inner().0.db, token)
        .await
        .map_err(|cause| cause.to_string())?;
    state
        .inner()
        .0
        .db
        .reconcile_finding_reviews()
        .map_err(|cause| cause.to_string())?;
    let snapshot = refresh_snapshot(state.inner()).map_err(|cause| cause.to_string())?;
    set_review_badge(&app, snapshot.review_summary.needs_review);
    Ok(snapshot)
}

#[tauri::command]
pub async fn configure_github_app(
    state: State<'_, SharedState>,
    client_id: Option<String>,
    install_url: Option<String>,
) -> Result<crate::core::github_auth::GithubAuthSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run_io(move || {
        shared
            .0
            .github_auth
            .configure_app(&shared.0.db, client_id, install_url)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn load_saved_github_auth(
    state: State<'_, SharedState>,
) -> Result<crate::core::github_auth::GithubAuthSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run_io(move || shared.0.github_auth.load_saved_auth()).await
}

#[tauri::command]
pub async fn start_github_login(state: State<'_, SharedState>) -> Result<GithubLoginStart, String> {
    state
        .inner()
        .0
        .github_auth
        .start_login()
        .await
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn poll_github_login(
    state: State<'_, SharedState>,
    login_id: String,
) -> Result<GithubLoginPollResult, String> {
    state
        .inner()
        .0
        .github_auth
        .poll_login(&login_id)
        .await
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn logout_github(state: State<'_, SharedState>) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .github_auth
            .logout()
            .map_err(|cause| cause.to_string())?;
        refresh_snapshot(&shared).map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn list_github_repositories(
    state: State<'_, SharedState>,
) -> Result<Vec<GithubRepositoryOption>, String> {
    state
        .inner()
        .0
        .github_auth
        .list_repositories()
        .await
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn scan_all(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<ScanAllResult, String> {
    run_scan_all(app, state.inner().clone(), false)
        .await
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn scan_repo(
    app: AppHandle,
    state: State<'_, SharedState>,
    repo_id: i64,
) -> Result<AppSnapshot, String> {
    scan_repo_internal(app, state.inner().clone(), repo_id)
        .await
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn add_github_repository(
    app: AppHandle,
    state: State<'_, SharedState>,
    url: String,
) -> Result<AppSnapshot, String> {
    add_github_repository_internal(app, state.inner().clone(), url)
        .await
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn remove_remote_repository(
    app: AppHandle,
    state: State<'_, SharedState>,
    repo_id: i64,
) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        if shared
            .0
            .scan_running
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(
                "A scan is currently running. Let it finish before removing a repository.".into(),
            );
        }

        shared
            .0
            .db
            .remove_remote_repository(repo_id)
            .map_err(|cause| cause.to_string())?;
        let snapshot = refresh_snapshot(&shared).map_err(|cause| cause.to_string())?;
        let _ = refresh_tray(&app, &shared);
        let _ = app.emit("cvec://state", &snapshot);
        Ok(snapshot)
    })
    .await?
}

#[tauri::command]
pub async fn set_schedule(
    app: AppHandle,
    state: State<'_, SharedState>,
    interval: ScheduleInterval,
) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .set_schedule(interval)
            .map_err(|cause| cause.to_string())?;
        shared.reschedule(app.clone());
        let snapshot = refresh_snapshot(&shared).map_err(|cause| cause.to_string())?;
        let _ = refresh_tray(&app, &shared);
        Ok(snapshot)
    })
    .await?
}

#[tauri::command]
pub async fn choose_discovery_root(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<AppSnapshot, String> {
    let current_root = state.inner().discovery_root();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_directory(&current_root)
        .pick_folder(move |selected| {
            let _ = sender.send(selected.and_then(|path| path.into_path().ok()));
        });
    let selected = receiver
        .await
        .map_err(|_| "Folder selection was interrupted".to_string())?;
    let shared = state.inner().clone();
    if let Some(path) = selected {
        let work = shared.clone();
        crate::runtime::run_io(move || -> Result<(), Error> {
            work.set_discovery_root(path)?;
            sync_repositories(&work)
        })
        .await?
        .map_err(|cause| cause.to_string())?;
    }
    let snapshot = load_persisted_snapshot(&shared)
        .await
        .map_err(|cause| cause.to_string())?;
    let _ = refresh_tray_from_snapshot(&app, &snapshot);
    let _ = app.emit("cvec://state", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn refresh_local_repositories(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();
    let work = shared.clone();
    crate::runtime::run_io(move || sync_repositories(&work))
        .await?
        .map_err(|cause| cause.to_string())?;
    let snapshot = load_persisted_snapshot(&shared)
        .await
        .map_err(|cause| cause.to_string())?;
    let _ = refresh_tray_from_snapshot(&app, &snapshot);
    let _ = app.emit("cvec://state", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn check_vulnerability_db(
    state: State<'_, SharedState>,
    check_updates: bool,
) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();
    let work = shared.clone();
    crate::runtime::run_io(move || {
        let _guard = work.try_acquire_scan()?;
        work.refresh_scanner_local_status();
        if check_updates {
            work.refresh_scanner_update_status();
        }
        Ok::<_, Error>(())
    })
    .await?
    .map_err(|cause| cause.to_string())?;
    load_persisted_snapshot(&shared)
        .await
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn update_vulnerability_db(state: State<'_, SharedState>) -> Result<AppSnapshot, String> {
    let shared = state.inner().clone();

    crate::runtime::run_io(move || update_vulnerability_db_internal(shared))
        .await
        .map_err(|cause| format!("failed to update vulnerability DB: {cause}"))?
        .map_err(|cause| cause.to_string())
}

fn update_vulnerability_db_internal(shared: SharedState) -> Result<AppSnapshot, Error> {
    let operation_guard = shared.try_acquire_scan().map_err(|_| {
        Error::App(
            "A scan or vulnerability DB update is already running. Let it finish first.".into(),
        )
    })?;

    let grype_db_cache_dir = shared.grype_db_cache_dir();
    let before = scanner::grype_db_status(&grype_db_cache_dir).ok();
    scanner::update_grype_db(&grype_db_cache_dir)?;
    let after = scanner::grype_db_status(&grype_db_cache_dir)?;
    let summary = summarize_grype_db_update(before.as_ref(), &after);
    let summary_json = serde_json::to_string(&summary)?;
    shared
        .0
        .db
        .set_setting("last_grype_db_update", summary_json)?;
    let (title, message, tone) = grype_db_notification_content(&summary);
    shared.0.db.insert_notification(
        AppNotificationKind::DbUpdate,
        tone,
        &title,
        &message,
        None,
        None,
    )?;
    drop(operation_guard);
    shared.refresh_scanner_status();
    refresh_snapshot(&shared)
}

pub async fn scan_all_internal(app: AppHandle, shared: SharedState) -> Result<(), Error> {
    run_scan_all(app, shared, true).await.map(|_| ())
}

async fn run_scan_all(
    app: AppHandle,
    shared: SharedState,
    broadcast: bool,
) -> Result<ScanAllResult, Error> {
    let guard = shared.try_acquire_scan()?;
    let started_at = shared.begin_scan_activity(None, 0).started_at;
    publish_scan_activity(&app, &shared);

    let repos = match repositories_for_batch_scan(&shared).await {
        Ok(repos) => repos,
        Err(cause) => {
            finish_scan_failure(&app, &shared, &cause, guard).await;
            return Err(cause);
        }
    };
    let mut notifications_sent = 0usize;
    if !repos.is_empty() {
        update_scan_activity(&app, &shared, |activity| {
            activity.total_repositories = repos.len() as u32;
        });
        let versions = match scanner_versions().await {
            Ok(versions) => versions,
            Err(cause) => {
                finish_scan_failure(&app, &shared, &cause, guard).await;
                return Err(cause);
            }
        };
        for repo in &repos {
            // Each attempt publishes its own outcome, including setup errors.
            if let Ok(sent) = scan_repository_record(&app, &shared, repo, &versions).await {
                notifications_sent += usize::from(sent);
            }
        }
    }

    let finished_at = timestamps::now();
    let (completed_activity, snapshot) = finish_scan(&app, &shared, guard, &finished_at).await;
    if broadcast {
        if let Ok(snapshot) = &snapshot {
            let _ = app.emit("cvec://state", snapshot);
        }
    }

    Ok(ScanAllResult {
        summary: ScanAllSummary {
            started_at,
            finished_at,
            repos_attempted: repos.len(),
            repos_scanned: completed_activity.completed_repositories as usize,
            repos_failed: completed_activity.failed_repositories as usize,
            alerts_created: notifications_sent,
        },
        snapshot: snapshot.ok(),
    })
}

pub fn emit_background_error(app: &AppHandle, message: String) {
    eprintln!("{message}");
    if let Err(cause) = app.emit("cvec://error", message) {
        eprintln!("Failed to emit background error: {cause}");
    }
}

pub async fn scan_repo_internal(
    app: AppHandle,
    shared: SharedState,
    repo_id: i64,
) -> Result<AppSnapshot, Error> {
    let guard = shared.try_acquire_scan()?;
    shared.begin_scan_activity(Some(repo_id), 1);
    publish_scan_activity(&app, &shared);

    let repo = match repository_for_scan(&shared, repo_id).await {
        Ok(repo) => repo,
        Err(cause) => {
            finish_scan_failure(&app, &shared, &cause, guard).await;
            return Err(cause);
        }
    };
    update_scan_activity(&app, &shared, |activity| {
        activity.current_repo_id = Some(repo.id);
        activity.current_repo_name = Some(repo.name.clone());
    });
    let versions = match scanner_versions().await {
        Ok(versions) => versions,
        Err(cause) => {
            record_scan_outcome(&app, &shared, &repo, None, None, Some(&cause));
            finish_scan_failure(&app, &shared, &cause, guard).await;
            return Err(cause);
        }
    };
    if let Err(cause) = scan_repository_record(&app, &shared, &repo, &versions).await {
        finish_scan_failure(&app, &shared, &cause, guard).await;
        return Err(cause);
    }
    finish_scan_and_get_snapshot(&app, &shared, guard).await
}

async fn add_github_repository_internal(
    app: AppHandle,
    shared: SharedState,
    url: String,
) -> Result<AppSnapshot, Error> {
    let guard = shared.try_acquire_scan()?;
    shared.begin_scan_activity(None, 1);
    publish_scan_activity(&app, &shared);
    let versions = match scanner_versions().await {
        Ok(versions) => versions,
        Err(cause) => {
            finish_scan_failure(&app, &shared, &cause, guard).await;
            return Err(cause);
        }
    };
    let repo_ref = match github::parse_github_url(&url) {
        Ok(repo_ref) => repo_ref,
        Err(cause) => {
            finish_scan_failure(&app, &shared, &cause, guard).await;
            return Err(cause);
        }
    };
    let token = match shared.0.github_auth.usable_token().await {
        Ok(token) => token,
        Err(cause) => {
            finish_scan_failure(&app, &shared, &cause, guard).await;
            return Err(cause);
        }
    };
    let materialized = match github::materialize_repository(
        &repo_ref,
        token,
        Some(&shared.github_blob_cache_dir()),
    )
    .await
    {
        Ok(materialized) => materialized,
        Err(cause) => {
            finish_scan_failure(&app, &shared, &cause, guard).await;
            return Err(cause);
        }
    };
    let repo_id = match shared.0.db.upsert_github_repository(&materialized.info) {
        Ok(repo_id) => repo_id,
        Err(cause) => {
            finish_scan_failure(&app, &shared, &cause, guard).await;
            return Err(cause);
        }
    };
    let repo = match repository_for_scan(&shared, repo_id).await {
        Ok(repo) => repo,
        Err(cause) => {
            finish_scan_failure(&app, &shared, &cause, guard).await;
            return Err(cause);
        }
    };

    update_scan_activity(&app, &shared, |activity| {
        activity.scope_repo_id = Some(repo_id);
        activity.current_repo_id = Some(repo_id);
        activity.current_repo_name = Some(repo.name.clone());
        activity.stage = ScanActivityStage::PreparingSource;
    });

    let prepared = PreparedScanRoot::Github(materialized);
    let run_id = match insert_scan_start(&shared, repo.id, &versions).await {
        Ok(run_id) => run_id,
        Err(cause) => {
            record_scan_outcome(&app, &shared, &repo, None, None, Some(&cause));
            finish_scan_failure(&app, &shared, &cause, guard).await;
            return Err(cause);
        }
    };
    if let Err(cause) = scan_prepared_repository(&app, &shared, &repo, run_id, prepared).await {
        finish_scan_failure(&app, &shared, &cause, guard).await;
        return Err(cause);
    }
    finish_scan_and_get_snapshot(&app, &shared, guard).await
}

async fn scanner_versions() -> Result<ScannerVersions, Error> {
    crate::runtime::run_io(scanner::versions)
        .await
        .map_err(Error::App)?
}

async fn repositories_for_batch_scan(shared: &SharedState) -> Result<Vec<RepoRecord>, Error> {
    let work = shared.clone();
    crate::runtime::run_io(move || {
        discover_and_sync_repositories(&work)?;
        work.0.db.list_active_repositories()
    })
    .await
    .map_err(Error::App)?
}

async fn repository_for_scan(shared: &SharedState, repo_id: i64) -> Result<RepoRecord, Error> {
    let db = shared.0.db.clone();
    crate::runtime::run(move || db.active_repository(repo_id))
        .await
        .map_err(Error::App)?
}

enum PreparedScanRoot {
    Local {
        path: std::path::PathBuf,
        source: scanner::LocalScanSource,
    },
    Github(github::MaterializedGithubRepository),
}

impl PreparedScanRoot {
    fn path(&self) -> &std::path::Path {
        match self {
            PreparedScanRoot::Local { path, .. } => path.as_path(),
            PreparedScanRoot::Github(materialized) => materialized.root(),
        }
    }

    fn source_snapshot(&self) -> SourceSnapshot {
        match self {
            Self::Local { source, .. } => source.snapshot.clone(),
            Self::Github(materialized) => SourceSnapshot {
                revision: Some(materialized.info.commit_sha.clone()),
                dirty: Some(false),
            },
        }
    }
}

async fn scan_repository_record(
    app: &AppHandle,
    shared: &SharedState,
    repo: &RepoRecord,
    versions: &ScannerVersions,
) -> Result<bool, Error> {
    update_scan_activity(app, shared, |activity| {
        activity.current_repo_id = Some(repo.id);
        activity.current_repo_name = Some(repo.name.clone());
        activity.stage = ScanActivityStage::PreparingSource;
    });
    let run_id = match insert_scan_start(shared, repo.id, versions).await {
        Ok(run_id) => run_id,
        Err(cause) => {
            record_scan_outcome(app, shared, repo, None, None, Some(&cause));
            return Err(cause);
        }
    };
    let prepared = match prepare_scan_root(shared, repo).await {
        Ok(prepared) => prepared,
        Err(cause) => {
            record_failed_scan(app, shared, repo, run_id, &cause, None).await;
            return Err(cause);
        }
    };

    scan_prepared_repository(app, shared, repo, run_id, prepared).await
}

async fn insert_scan_start(
    shared: &SharedState,
    repo_id: i64,
    versions: &ScannerVersions,
) -> Result<i64, Error> {
    let db = shared.0.db.clone();
    let versions = versions.clone();
    crate::runtime::run(move || db.insert_scan_start(repo_id, &versions))
        .await
        .map_err(Error::App)?
}

async fn prepare_scan_root(
    shared: &SharedState,
    repo: &RepoRecord,
) -> Result<PreparedScanRoot, Error> {
    match repo.source_type {
        RepoSourceType::Local => {
            prepare_local_scan_root(std::path::PathBuf::from(&repo.path)).await
        }
        RepoSourceType::Github => {
            let source_key = repo.source_key.as_deref().ok_or_else(|| {
                Error::App(format!(
                    "GitHub repository {} is missing a source key",
                    repo.name
                ))
            })?;
            let repo_ref = github_ref_from_source_key(source_key)?;
            let token = shared.0.github_auth.usable_token().await?;
            Ok(PreparedScanRoot::Github(
                github::materialize_repository(
                    &repo_ref,
                    token,
                    Some(&shared.github_blob_cache_dir()),
                )
                .await?,
            ))
        }
        RepoSourceType::Unknown => Err(Error::App(format!(
            "repository {} has an unsupported source type",
            repo.name
        ))),
    }
}

async fn prepare_local_scan_root(path: std::path::PathBuf) -> Result<PreparedScanRoot, Error> {
    // Stored repositories can outlive an unavailable drive or checkout.
    let checkout = path.clone();
    crate::runtime::run_io(move || {
        if checkout.join(".git").try_exists()? {
            return Ok(());
        }
        Err(Error::App(format!(
            "The Git checkout at {} is unavailable. Restore the folder and retry the scan.",
            checkout.display()
        )))
    })
    .await
    .map_err(Error::App)??;
    let source = scanner::capture_local_scan_source(&path).await;
    if source.snapshot.revision.is_none() && source.snapshot.dirty.is_none() {
        return Err(Error::App(format!(
            "The Git checkout at {} could not be verified. Check the folder and retry the scan.",
            path.display()
        )));
    }
    Ok(PreparedScanRoot::Local { path, source })
}

async fn record_failed_scan(
    app: &AppHandle,
    shared: &SharedState,
    repo: &RepoRecord,
    run_id: i64,
    cause: &Error,
    source: Option<SourceSnapshot>,
) {
    let db = shared.0.db.clone();
    let repo_id = repo.id;
    let message = cause.to_string();
    let result =
        crate::runtime::run(move || db.fail_scan(repo_id, run_id, &message, source.as_ref()))
            .await
            .map_err(Error::App)
            .and_then(|result| result);
    let recorded_run = match result {
        Ok(()) => Some(run_id),
        Err(storage_error) => {
            publish_scan_warning(
                app,
                shared,
                format!("Could not persist failed scan #{run_id}: {storage_error}"),
            );
            None
        }
    };
    record_scan_outcome(app, shared, repo, recorded_run, None, Some(cause));
}

fn publish_scan_warning(app: &AppHandle, shared: &SharedState, message: String) {
    update_scan_activity(app, shared, |activity| activity.warnings.push(message));
}

async fn scan_prepared_repository(
    app: &AppHandle,
    shared: &SharedState,
    repo: &RepoRecord,
    run_id: i64,
    prepared: PreparedScanRoot,
) -> Result<bool, Error> {
    let source = prepared.source_snapshot();
    let result = scan_prepared_repository_inner(app, shared, repo, run_id, prepared).await;
    if let Err(cause) = &result {
        record_failed_scan(app, shared, repo, run_id, cause, Some(source)).await;
    }
    result
}

async fn scan_prepared_repository_inner(
    app: &AppHandle,
    shared: &SharedState,
    repo: &RepoRecord,
    run_id: i64,
    prepared: PreparedScanRoot,
) -> Result<bool, Error> {
    let cache_dir = shared.grype_db_cache_dir();
    let output = scanner::scan_repository_with_progress(
        prepared.path(),
        Some(cache_dir.as_path()),
        |stage| {
            update_scan_activity(app, shared, |activity| activity.stage = stage);
        },
    )
    .await?;

    update_scan_activity(app, shared, |activity| {
        activity.stage = ScanActivityStage::SavingResults
    });

    let (changes, enrich) =
        persist_prepared_scan(shared.0.db.clone(), repo.clone(), run_id, prepared, output).await?;
    let repo_id = repo.id;

    if let Some(warning) = &changes.review_warning {
        publish_scan_warning(app, shared, warning.clone());
    }
    record_scan_outcome(app, shared, repo, Some(run_id), Some(&changes), None);
    if enrich {
        defer_advisory_enrichment(app.clone(), shared.clone(), repo.id, run_id);
    }

    let new_issue_count = changes.alerts;
    if new_issue_count > 0 {
        let title = format!("New vulnerabilities in {}", repo.name);
        let message = format!(
            "{} new or worsened HIGH / CRITICAL findings need review.",
            new_issue_count
        );
        let db = shared.0.db.clone();
        let name = repo.name.clone();
        let stored_title = title.clone();
        let stored_message = message.clone();
        if let Err(cause) = crate::runtime::run(move || {
            db.insert_notification(
                AppNotificationKind::ScanAlert,
                AppNotificationTone::Warning,
                &stored_title,
                &stored_message,
                Some((repo_id, &name)),
                Some(run_id),
            )
        })
        .await
        .map_err(Error::App)
        .and_then(|result| result)
        {
            publish_scan_warning(
                app,
                shared,
                format!(
                    "Scanner results were saved, but the scan alert could not be stored: {cause}"
                ),
            );
            return Ok(false);
        }
        let _ = app
            .notification()
            .builder()
            .title(title)
            .body(message)
            .show();
        return Ok(true);
    }

    Ok(false)
}

async fn persist_prepared_scan(
    db: Database,
    repo: RepoRecord,
    run_id: i64,
    prepared: PreparedScanRoot,
    output: ScanOutput,
) -> Result<(ScanReviewChanges, bool), Error> {
    let repo_id = repo.id;
    let (assets, prepared) = crate::runtime::run_io(move || {
        let assets = assets::extract_repo_assets_from_root(&repo, prepared.path());
        (assets, prepared)
    })
    .await
    .map_err(Error::App)?;
    if let Some(evidence) = &output.evidence {
        evidence.verify_source(prepared.path()).await?;
    }
    let source_snapshot = prepared.source_snapshot();
    let remote = match prepared {
        PreparedScanRoot::Local { path, source } => {
            source.verify(&path).await?;
            None
        }
        PreparedScanRoot::Github(materialized) => Some((materialized.info, materialized.files)),
    };
    crate::runtime::run(move || {
        let enrich = output.findings.iter().any(|finding| {
            github_advisory::exact_github_advisory_id(
                &finding.advisory_id,
                finding.source_url.as_deref(),
            )
            .is_some()
        });
        let persist_started = std::env::var_os("CVEC_PERF_TRACE")
            .is_some()
            .then(std::time::Instant::now);
        let changes = db.complete_repository_scan(
            repo_id,
            run_id,
            &output,
            &source_snapshot,
            &assets,
            remote
                .as_ref()
                .map(|(info, files)| (info, files.as_slice())),
        )?;
        if let Some(started) = persist_started {
            eprintln!(
                "cvec-perf scan sqlite_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        Ok((changes, enrich))
    })
    .await
    .map_err(Error::App)?
}

fn defer_advisory_enrichment(app: AppHandle, shared: SharedState, repo_id: i64, run_id: i64) {
    use std::sync::{Arc, OnceLock};
    static QUEUE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    static JOBS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let queued = QUEUE
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(32)))
        .clone()
        .try_acquire_owned();
    tauri::async_runtime::spawn(async move {
        let outcome = if let Ok(_queued) = queued {
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                let _job = JOBS
                    .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| Error::App("Advisory worker queue unavailable".into()))?;
                let token = match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    shared.0.github_auth.advisory_token(),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(Error::App(
                        "Advisory authentication deadline exceeded".into(),
                    )),
                };
                github_advisory::enrich_completed_scan(shared.0.db.clone(), repo_id, run_id, token)
                    .await
            })
            .await
        } else {
            Ok(Err(Error::App("Advisory background queue is full".into())))
        };
        let (changed,reopened,warnings)=match outcome {
            Ok(Ok(result))=>result,
            _=>(0,0,vec!["Optional advisory details were delayed or unavailable; local scanner results remain complete.".into()]),
        };
        if changed == 0 && warnings.is_empty() {
            return;
        }
        if !warnings.is_empty() {
            let saved = warnings.clone();
            let db = shared.0.db.clone();
            let result = crate::runtime::run(move || db.record_scan_warnings(run_id, &saved)).await;
            if !matches!(result, Ok(Ok(()))) {
                eprintln!("Optional advisory warning could not be recorded");
            }
        }
        if let Some(activity) = shared.record_scan_enrichment(run_id, reopened, warnings) {
            let _ = app.emit("cvec://scan-activity", activity);
        }
        publish_persisted_snapshot(&app, &shared).await;
    });
}

fn record_scan_outcome(
    app: &AppHandle,
    shared: &SharedState,
    repo: &RepoRecord,
    run_id: Option<i64>,
    changes: Option<&ScanReviewChanges>,
    failure: Option<&Error>,
) {
    let (new_findings, resolved_findings, worsened_findings) = changes
        .map(|changes| {
            (
                changes.new_findings,
                changes.resolved_findings,
                changes.worsened_findings,
            )
        })
        .unwrap_or((0, 0, 0));
    update_scan_activity(app, shared, |activity| {
        let succeeded = failure.is_none();
        if succeeded {
            activity.completed_repositories += 1;
        } else {
            activity.failed_repositories += 1;
        }
        activity.new_findings += new_findings;
        activity.resolved_findings += resolved_findings;
        activity.worsened_findings += worsened_findings;
        activity.outcomes.push(ScanRepositoryOutcome {
            repo_id: repo.id,
            repo_name: repo.name.clone(),
            scan_run_id: run_id,
            succeeded,
            error: failure.map(ToString::to_string),
            new_findings,
            resolved_findings,
            worsened_findings,
        });
    });
}

async fn finish_scan_and_get_snapshot(
    app: &AppHandle,
    shared: &SharedState,
    guard: ScanGuard,
) -> Result<AppSnapshot, Error> {
    let (_, snapshot) = finish_scan(app, shared, guard, &timestamps::now()).await;
    let snapshot = snapshot?;
    let _ = app.emit("cvec://state", &snapshot);
    Ok(snapshot)
}

async fn finish_scan(
    app: &AppHandle,
    shared: &SharedState,
    guard: ScanGuard,
    finished_at: &str,
) -> (ScanActivity, Result<AppSnapshot, Error>) {
    let work = shared.clone();
    let scheduled = crate::runtime::run(move || work.schedule_next_from_now())
        .await
        .map_err(Error::App)
        .and_then(|result| result);
    if let Err(cause) = scheduled {
        publish_scan_warning(
            app,
            shared,
            format!("Could not save the next scan time: {cause}"),
        );
    }
    let mut snapshot = load_persisted_snapshot(shared).await;
    match &mut snapshot {
        Ok(snapshot) => {
            snapshot.workspace_status.scan_running = false;
            if let Err(cause) = refresh_tray_from_snapshot(app, snapshot) {
                publish_scan_warning(
                    app,
                    shared,
                    format!("Could not update the menu bar: {cause}"),
                );
            }
        }
        Err(cause) => publish_scan_warning(
            app,
            shared,
            format!("Could not reload scan results: {cause}"),
        ),
    }
    let activity = shared
        .update_scan_activity(|activity| {
            activity.status = match (
                activity.completed_repositories,
                activity.failed_repositories,
            ) {
                (_, 0) => ScanActivityStatus::Completed,
                (0, _) => ScanActivityStatus::Failed,
                _ => ScanActivityStatus::PartialFailure,
            };
            activity.stage = if activity.status == ScanActivityStatus::Failed {
                ScanActivityStage::Failed
            } else {
                ScanActivityStage::Completed
            };
            activity.finished_at = Some(finished_at.to_string());
        })
        .expect("scan activity initialized");
    drop(guard);
    let _ = app.emit("cvec://scan-activity", &activity);
    let snapshot = snapshot
        .map(|mut snapshot| {
            snapshot.scan_activity = Some(activity.clone());
            snapshot
        })
        .map_err(|cause| {
            Error::App(format!(
                "Scan results were saved. Could not reload the workspace: {cause}"
            ))
        });
    (activity, snapshot)
}

async fn finish_scan_failure(
    app: &AppHandle,
    shared: &SharedState,
    cause: &Error,
    guard: ScanGuard,
) {
    let activity = shared.update_scan_activity(|activity| {
        activity.status = ScanActivityStatus::Failed;
        activity.stage = ScanActivityStage::Failed;
        activity.finished_at = Some(timestamps::now());
        activity.terminal_error = Some(cause.to_string());
    });
    drop(guard);
    if let Some(activity) = activity {
        let _ = app.emit("cvec://scan-activity", activity);
    }
    if let Ok(snapshot) = load_persisted_snapshot(shared).await {
        let _ = refresh_tray_from_snapshot(app, &snapshot);
        let _ = app.emit("cvec://state", snapshot);
    }
}

fn github_ref_from_source_key(source_key: &str) -> Result<github::GithubRepoRef, Error> {
    let value = source_key
        .strip_prefix("github:")
        .ok_or_else(|| Error::App(format!("unsupported remote source key: {source_key}")))?;
    let (owner, repo) = value
        .split_once('/')
        .ok_or_else(|| Error::App(format!("invalid GitHub source key: {source_key}")))?;
    Ok(github::GithubRepoRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

pub fn refresh_snapshot(shared: &SharedState) -> Result<AppSnapshot, Error> {
    shared.0.db.snapshot(
        shared.schedule(),
        shared.scanner_status(),
        shared.discovery_root().display().to_string(),
        shared
            .0
            .scan_running
            .load(std::sync::atomic::Ordering::SeqCst),
        env!("CARGO_PKG_VERSION").to_string(),
        shared.0.github_auth.cached_snapshot(),
        shared.scan_activity(),
        shared.next_scheduled_scan_at(),
    )
}

fn publish_scan_activity(app: &AppHandle, shared: &SharedState) {
    if let Some(activity) = shared.scan_activity() {
        let _ = app.emit("cvec://scan-activity", activity);
    }
}

fn update_scan_activity(
    app: &AppHandle,
    shared: &SharedState,
    update: impl FnOnce(&mut ScanActivity),
) {
    if let Some(activity) = shared.update_scan_activity(update) {
        let _ = app.emit("cvec://scan-activity", activity);
    }
}

fn sync_repositories(shared: &SharedState) -> Result<(), Error> {
    discover_and_sync_repositories(shared)?;
    for repo in shared.0.db.list_active_repositories()? {
        if repo.source_type != RepoSourceType::Local {
            continue;
        }
        let assets = assets::extract_repo_assets(&repo);
        shared.0.db.replace_repo_assets(repo.id, &assets)?;
    }
    Ok(())
}

fn discover_and_sync_repositories(shared: &SharedState) -> Result<(), Error> {
    let root = shared.discovery_root();
    let report = repo_discovery::discover_repositories_resilient(&root);
    for warning in &report.warnings {
        eprintln!("Repository discovery warning: {warning}");
    }
    {
        let selected_root = shared
            .0
            .discovery_root
            .lock()
            .expect("discovery_root lock poisoned");
        if *selected_root != root {
            return Ok(());
        }
        shared
            .0
            .db
            .sync_repositories_with_completeness(&report.repositories, report.is_complete())?;
    }
    Ok(())
}

fn open_url_in_system_browser(url: &str) -> Result<(), Error> {
    let external_url = url_policy::validate_external_url(url)?;

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("/usr/bin/open");
        command.args(["--", &external_url]);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer.exe");
        command.arg(&external_url);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(&external_url);
        command
    };

    command
        .spawn()
        .map_err(|cause| Error::App(format!("failed to open external URL: {cause}")))?;
    Ok(())
}

pub(crate) fn open_local_file(path: &std::path::Path) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("/usr/bin/open");
        command.arg("--");
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer.exe");

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");

    command
        .arg(path)
        .spawn()
        .map_err(|_| Error::App("Could not open the verified local path.".into()))?;
    Ok(())
}

fn summarize_grype_db_update(
    before: Option<&GrypeDbStatus>,
    after: &GrypeDbStatus,
) -> GrypeDbUpdateSummary {
    let previous_schema_version = before.and_then(|status| status.schema_version.clone());
    let previous_built_at = before.and_then(|status| status.built_at.clone());
    let current_schema_version = after.schema_version.clone();
    let current_built_at = after.built_at.clone();

    let outcome = if !before.map(|status| status.present).unwrap_or(false) && after.present {
        GrypeDbUpdateOutcome::Downloaded
    } else if previous_schema_version != current_schema_version
        || previous_built_at != current_built_at
    {
        GrypeDbUpdateOutcome::Updated
    } else {
        GrypeDbUpdateOutcome::AlreadyCurrent
    };

    GrypeDbUpdateSummary {
        checked_at: timestamps::now(),
        outcome,
        previous_schema_version,
        previous_built_at,
        current_schema_version,
        current_built_at,
    }
}

fn grype_db_notification_content(
    summary: &GrypeDbUpdateSummary,
) -> (String, String, AppNotificationTone) {
    match summary.outcome {
        GrypeDbUpdateOutcome::Downloaded => (
            "Initial vulnerability DB downloaded".to_string(),
            "cvec downloaded the first local Grype database for the app-managed cache.".to_string(),
            AppNotificationTone::Success,
        ),
        GrypeDbUpdateOutcome::Updated => (
            "Vulnerability DB updated".to_string(),
            format!(
                "Grype DB changed from {} to {}.",
                format_db_build_summary(
                    summary.previous_schema_version.as_deref(),
                    summary.previous_built_at.as_deref()
                ),
                format_db_build_summary(
                    summary.current_schema_version.as_deref(),
                    summary.current_built_at.as_deref()
                )
            ),
            AppNotificationTone::Success,
        ),
        GrypeDbUpdateOutcome::AlreadyCurrent => (
            "Vulnerability DB already current".to_string(),
            "No newer Grype DB build was available, so the local database stayed unchanged."
                .to_string(),
            AppNotificationTone::Neutral,
        ),
    }
}

fn format_db_build_summary(schema_version: Option<&str>, built_at: Option<&str>) -> String {
    let version_label = schema_version.unwrap_or("no local DB");
    match built_at {
        Some(built_at) => format!("{version_label} ({built_at})"),
        None => version_label.to_string(),
    }
}

#[cfg(all(test, unix))]
mod real_scan_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{
        NormalizedFinding, RemediationVerificationState, RepoPackage, ScanRunStatus, Severity,
    };
    use std::path::Path;
    use std::process::Command;

    #[tokio::test]
    async fn repository_scan_requires_the_selected_checkout_to_exist() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent");
        let repo = parent.join("local-repo");
        let git_init = |args: &[&Path]| {
            assert!(Command::new("git")
                .args(["init", "-q", "--template="])
                .args(args)
                .status()
                .unwrap()
                .success());
        };
        git_init(&[&parent]);
        std::fs::create_dir(&repo).unwrap();
        let error = prepare_local_scan_root(repo.clone()).await.err().unwrap();
        assert!(error.to_string().contains("unavailable"));
        std::fs::create_dir(repo.join(".git")).unwrap();
        let error = prepare_local_scan_root(repo.clone()).await.err().unwrap();
        assert!(error.to_string().contains("could not be verified"));

        git_init(&[&repo]);
        let prepared = prepare_local_scan_root(repo.clone()).await.unwrap();
        assert_eq!(prepared.path(), repo);
        assert_eq!(prepared.source_snapshot().dirty, Some(false));
        assert!(prepared.source_snapshot().revision.is_none());

        let linked = temp.path().join("linked");
        let metadata = temp.path().join("linked-metadata");
        git_init(&[Path::new("--separate-git-dir"), &metadata, &linked]);
        assert!(linked.join(".git").is_file());
        let prepared = prepare_local_scan_root(linked.clone()).await.unwrap();
        assert_eq!(prepared.path(), linked);
        assert_eq!(prepared.source_snapshot().dirty, Some(false));
    }

    #[tokio::test]
    async fn changed_checkout_cannot_replace_last_completed_scan() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("repo");
        std::fs::create_dir(&path).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        let manifest = path.join("package.json");
        std::fs::write(&manifest, r#"{"name":"fixture","version":"1.0.0"}"#).unwrap();
        git(&["add", "package.json"]);
        git(&[
            "-c",
            "user.name=CVEC QA",
            "-c",
            "user.email=qa@example.invalid",
            "commit",
            "-qm",
            "baseline",
            "--no-gpg-sign",
        ]);

        let db = Database::new(temp.path().join("cvec.db")).unwrap();
        db.sync_repositories(&repo_discovery::discover_repositories(&path).unwrap())
            .unwrap();
        let repo = db.list_active_repositories().unwrap().pop().unwrap();
        let versions = ScannerVersions {
            syft: "test-syft".into(),
            grype: "test-grype".into(),
        };
        let first = db.insert_scan_start(repo.id, &versions).unwrap();
        let fingerprint = "fixture-finding";
        persist_prepared_scan(
            db.clone(),
            repo.clone(),
            first,
            prepare_local_scan_root(path.clone()).await.unwrap(),
            ScanOutput {
                evidence: None,
                findings: vec![NormalizedFinding {
                    fingerprint: fingerprint.into(),
                    advisory_id: "CVE-2026-1234".into(),
                    package_name: "fixture-dependency".into(),
                    package_version: Some("1.0.0".into()),
                    package_type: Some("npm".into()),
                    package_url: None,
                    severity: Severity::High,
                    fix_version: None,
                    title: None,
                    source_url: None,
                    advisory_details: None,
                }],
                packages: vec![RepoPackage {
                    name: "fixture-dependency".into(),
                    version: Some("1.0.0".into()),
                    package_type: Some("npm".into()),
                    package_url: None,
                    highest_severity: None,
                    finding_count: 0,
                    fix_available: false,
                }],
                remediation_evidence: vec![],
            },
        )
        .await
        .unwrap();

        std::fs::write(&manifest, r#"{"name":"fixture","version":"2.0.0"}"#).unwrap();
        git(&["add", "package.json"]);
        let run = db.insert_scan_start(repo.id, &versions).unwrap();
        let prepared = prepare_local_scan_root(path.clone()).await.unwrap();
        let source = prepared.source_snapshot();
        assert_eq!(source.dirty, Some(true));
        std::fs::write(&manifest, r#"{"name":"fixture","version":"3.0.0"}"#).unwrap();
        git(&["add", "package.json"]);
        assert_eq!(
            prepare_local_scan_root(path.clone())
                .await
                .unwrap()
                .source_snapshot(),
            source
        );

        let empty = ScanOutput {
            evidence: None,
            findings: vec![],
            packages: vec![],
            remediation_evidence: vec![],
        };
        let cause = persist_prepared_scan(db.clone(), repo.clone(), run, prepared, empty.clone())
            .await
            .unwrap_err();
        assert!(cause
            .to_string()
            .contains("checkout changed during the scan"));
        db.fail_scan(repo.id, run, &cause.to_string(), Some(&source))
            .unwrap();

        let detail = db.repo_detail(repo.id).unwrap();
        assert_eq!(detail.latest_scan.unwrap().id, run);
        assert_eq!(
            detail
                .scan_history
                .iter()
                .find(|scan| scan.status == ScanRunStatus::Completed)
                .unwrap()
                .id,
            first
        );
        assert_eq!(detail.findings[0].fingerprint, fingerprint);
        assert_eq!(detail.packages[0].name, "fixture-dependency");
        assert_eq!(detail.scan_history[0].id, run);
        assert_eq!(detail.scan_history[0].status, ScanRunStatus::Failed);
        assert!(!detail.scan_history[0].evidence_complete);
        assert_eq!(
            db.finding_remediation(repo.id, fingerprint, first)
                .unwrap()
                .verification_state,
            RemediationVerificationState::Unverifiable
        );

        let retry = db.insert_scan_start(repo.id, &versions).unwrap();
        let (changes, _) = persist_prepared_scan(
            db.clone(),
            repo.clone(),
            retry,
            prepare_local_scan_root(path.clone()).await.unwrap(),
            empty,
        )
        .await
        .unwrap();
        assert_eq!(changes.resolved_findings, 1);
        let latest = db.repo_detail(repo.id).unwrap().latest_scan.unwrap();
        assert_eq!(latest.id, retry);
        assert_eq!(latest.source_dirty, Some(true));
        assert!(latest.evidence_complete);
    }
}
