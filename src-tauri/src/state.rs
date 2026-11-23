use crate::core::timestamps;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use tauri::{async_runtime::JoinHandle, AppHandle, Manager};
use time::OffsetDateTime;

use crate::core::{
    companion_context::CompanionContextService,
    companion_host::CompanionHost,
    db::Database,
    fix_workspaces::FixWorkspaces,
    github_auth::GithubAuthState,
    grok_host::GrokHost,
    models::{
        ScanActivity, ScanActivityStage, ScanActivityStatus, ScannerAvailability, ScheduleInterval,
    },
    opencode_host::OpenCodeHost,
    provider_secrets::ProviderSecretStore,
    scanner, scheduler,
};
use crate::Error;

pub(crate) const QA_DATA_DIR_ENV: &str = "CVEC_UI_QA_DATA_DIR";
pub(crate) const ISOLATED_UI_IDENTIFIER: &str = "dev.cvec.isolated-ui-qa";
pub(crate) const PERFORMANCE_QA_IDENTIFIER: &str = "dev.cvec.performance-qa";
#[cfg(debug_assertions)]
const QA_SCAN_ACTIVITY_ENV: &str = "CVEC_UI_QA_SCAN_ACTIVITY";
#[cfg(debug_assertions)]
const QA_REMEDIATION_SEED_ENV: &str = "CVEC_UI_QA_SEED_REMEDIATION";

#[cfg(debug_assertions)]
use crate::core::models::ScanRepositoryOutcome;

#[derive(Clone)]
pub struct SharedState(pub Arc<AppState>);

pub struct AppState {
    pub isolated_ui: bool,
    pub db: Database,
    pub companion: CompanionHost,
    pub fix_workspaces: FixWorkspaces,
    pub grok: GrokHost,
    pub open_code: OpenCodeHost,
    pub companion_context: CompanionContextService,
    pub provider_secrets: ProviderSecretStore,
    pub discovery_root: Mutex<PathBuf>,
    pub grype_db_cache_dir: PathBuf,
    pub github_blob_cache_dir: PathBuf,
    pub github_auth: GithubAuthState,
    pub scanner_status: Mutex<ScannerAvailability>,
    pub scan_running: AtomicBool,
    bootstrap_refresh_running: AtomicBool,
    pub scan_activity: Mutex<Option<ScanActivity>>,
    pub schedule: Mutex<ScheduleInterval>,
    pub next_scheduled_scan_at: Mutex<Option<OffsetDateTime>>,
    pub scheduled_task: Mutex<Option<JoinHandle<()>>>,
}

impl SharedState {
    pub fn new(app: &AppHandle) -> Result<Self, Error> {
        let identifier = app.config().identifier.as_str();
        let isolated_ui = identifier == ISOLATED_UI_IDENTIFIER;
        let qa_profile = isolated_ui || identifier == PERFORMANCE_QA_IDENTIFIER;
        let default_app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|cause| Error::App(format!("failed to resolve app data dir: {cause}")))?;
        let default_app_cache_dir = app
            .path()
            .app_cache_dir()
            .map_err(|cause| Error::App(format!("failed to resolve app cache dir: {cause}")))?;
        let (app_data_dir, app_cache_dir) = resolve_storage_directories(
            default_app_data_dir,
            default_app_cache_dir,
            if cfg!(debug_assertions) || qa_profile {
                qa_data_dir_override()
            } else {
                None
            },
        );
        ensure_private_directory(&app_data_dir)?;
        ensure_private_directory(&app_cache_dir)?;
        let db = Database::new(app_data_dir.join("cvec.db"))?;
        #[cfg(debug_assertions)]
        if qa_remediation_seed_enabled(
            identifier,
            std::env::var(QA_REMEDIATION_SEED_ENV).as_deref() == Ok("1"),
        ) {
            db.seed_native_remediation_qa(&app_data_dir.join("remediation-fixtures"))?;
        }
        if isolated_ui {
            prepare_isolated_ui(&db, &app_data_dir)?;
        }
        let scan_activity = match qa_profile.then(qa_scan_activity_fixture).flatten() {
            Some(fixture) => Some(fixture),
            None => db.interrupted_scan_activity()?,
        };
        let home = std::env::var("HOME").map_err(|_| Error::App("HOME is not set".into()))?;
        let default_root = PathBuf::from(home).join("code");
        let discovery_root = db
            .get_setting("discovery_root")?
            .map(PathBuf::from)
            .unwrap_or(default_root);
        let schedule = db
            .get_setting("schedule_interval")?
            .as_deref()
            .and_then(ScheduleInterval::parse)
            .unwrap_or(ScheduleInterval::Off);
        let next_scheduled_scan_at = match schedule.duration() {
            Some(duration) => {
                let persisted = db
                    .get_setting("next_scheduled_scan_at")?
                    .as_deref()
                    .and_then(parse_timestamp);
                let next = persisted.unwrap_or_else(|| OffsetDateTime::now_utc() + duration);
                db.set_setting("next_scheduled_scan_at", timestamps::format(&next))?;
                Some(next)
            }
            None => None,
        };
        let grype_db_cache_dir = app_cache_dir.join("grype").join("db");
        let mut scanner_status = db
            .get_setting("scanner_status")?
            .and_then(|value| serde_json::from_str::<ScannerAvailability>(&value).ok())
            .unwrap_or(ScannerAvailability {
                checked_at: None,
                checking: true,
                ready: false,
                message: None,
                versions: None,
                grype_db: None,
            });
        scanner_status.ready = false;
        scanner_status.checking = true;
        scanner_status.message = Some("Checking local scanners…".into());

        let companion_context = CompanionContextService::new(db.clone());
        let companion = CompanionHost::new(
            app.clone(),
            db.clone(),
            app_cache_dir.join("companion").join("sandbox"),
        )?;
        if isolated_ui {
            companion.disable_external_access();
        }
        let grok = GrokHost::new(
            app.clone(),
            db.clone(),
            app_cache_dir.join("companion").join("grok-sandbox"),
        )?;
        let fix_app = app.clone();
        let fix_workspaces = FixWorkspaces::new(
            db.clone(),
            app_data_dir.join("fix-workspaces"),
            grype_db_cache_dir.clone(),
            companion.clone(),
            move |workspace| {
                use tauri::Emitter;
                let _ = fix_app.emit("cvec://fix-workspace", workspace);
            },
        )?;
        let open_code = OpenCodeHost::new(
            app.clone(),
            db.clone(),
            app_cache_dir.join("companion").join("opencode-sandbox"),
        )?;

        let github_auth = if qa_profile {
            GithubAuthState::isolated()
        } else {
            GithubAuthState::from_settings(&db)?
        };
        Ok(Self(Arc::new(AppState {
            isolated_ui,
            db,
            companion,
            fix_workspaces,
            grok,
            open_code,
            companion_context,
            provider_secrets: if isolated_ui {
                ProviderSecretStore::isolated()
            } else {
                ProviderSecretStore::new()
            },
            discovery_root: Mutex::new(discovery_root),
            grype_db_cache_dir,
            github_blob_cache_dir: app_cache_dir.join("github").join("blobs"),
            github_auth,
            scanner_status: Mutex::new(scanner_status),
            scan_running: AtomicBool::new(false),
            bootstrap_refresh_running: AtomicBool::new(false),
            scan_activity: Mutex::new(scan_activity),
            schedule: Mutex::new(schedule),
            next_scheduled_scan_at: Mutex::new(next_scheduled_scan_at),
            scheduled_task: Mutex::new(None),
        })))
    }

    pub fn schedule(&self) -> ScheduleInterval {
        *self.0.schedule.lock().expect("schedule lock poisoned")
    }

    pub(crate) fn begin_bootstrap_refresh(&self) -> Option<BootstrapRefreshGuard> {
        self.0
            .bootstrap_refresh_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        Some(BootstrapRefreshGuard {
            state: self.0.clone(),
        })
    }

    pub fn set_schedule(&self, interval: ScheduleInterval) -> Result<(), Error> {
        let mut schedule = self.0.schedule.lock().expect("schedule lock poisoned");
        let next = interval
            .duration()
            .map(|duration| OffsetDateTime::now_utc() + duration);
        self.0
            .db
            .save_schedule(interval, next.as_ref().map(timestamps::format).as_deref())?;
        *self
            .0
            .next_scheduled_scan_at
            .lock()
            .expect("next schedule lock poisoned") = next;
        *schedule = interval;
        Ok(())
    }

    pub fn next_scheduled_scan_at(&self) -> Option<String> {
        self.0
            .next_scheduled_scan_at
            .lock()
            .expect("next schedule lock poisoned")
            .as_ref()
            .map(timestamps::format)
    }

    pub fn schedule_next_from_now(&self) -> Result<(), Error> {
        let schedule = self.0.schedule.lock().expect("schedule lock poisoned");
        let next = schedule
            .duration()
            .map(|duration| OffsetDateTime::now_utc() + duration);
        let saved = self.0.db.set_setting(
            "next_scheduled_scan_at",
            next.as_ref().map(timestamps::format).unwrap_or_default(),
        );
        // Advance the in-process deadline even if SQLite could not save it.
        // Leaving an expired deadline in place causes the scheduler to loop.
        *self
            .0
            .next_scheduled_scan_at
            .lock()
            .expect("next schedule lock poisoned") = next;
        saved
    }

    pub fn discovery_root(&self) -> PathBuf {
        self.0
            .discovery_root
            .lock()
            .expect("discovery_root lock poisoned")
            .clone()
    }

    pub fn grype_db_cache_dir(&self) -> PathBuf {
        self.0.grype_db_cache_dir.clone()
    }

    pub fn github_blob_cache_dir(&self) -> PathBuf {
        self.0.github_blob_cache_dir.clone()
    }

    pub fn scanner_status(&self) -> ScannerAvailability {
        self.0
            .scanner_status
            .lock()
            .expect("scanner status lock poisoned")
            .clone()
    }

    fn save_scanner_status(&self, status: ScannerAvailability) {
        if let Ok(json) = serde_json::to_string(&status) {
            let _ = self.0.db.set_setting("scanner_status", json);
        }
        *self
            .0
            .scanner_status
            .lock()
            .expect("scanner status lock poisoned") = status;
    }
    pub fn refresh_scanner_local_status(&self) {
        let mut status = scanner::availability();
        status.grype_db = scanner::grype_db_local_status(&self.grype_db_cache_dir()).ok();
        self.save_scanner_status(status);
    }
    pub fn refresh_scanner_update_status(&self) {
        let Some(before) = self.scanner_status().grype_db else {
            return;
        };
        let updated = scanner::grype_db_update_check(before.clone(), &self.grype_db_cache_dir());
        let mut current = self.scanner_status();
        // An explicit DB update may finish while the network probe is waiting.
        if current
            .grype_db
            .as_ref()
            .is_some_and(|value| value.built_at == before.built_at && value.path == before.path)
        {
            current.grype_db = Some(updated);
            self.save_scanner_status(current);
        }
    }
    pub fn refresh_scanner_status(&self) {
        self.refresh_scanner_local_status();
        self.refresh_scanner_update_status();
    }

    pub fn set_discovery_root(&self, path: PathBuf) -> Result<(), Error> {
        if self.0.isolated_ui {
            return Err(Error::App(
                "The isolated test profile uses only its fixture folder.".into(),
            ));
        }
        self.0
            .db
            .set_setting("discovery_root", path.display().to_string())?;
        *self
            .0
            .discovery_root
            .lock()
            .expect("discovery_root lock poisoned") = path;
        Ok(())
    }

    pub fn try_acquire_scan(&self) -> Result<ScanGuard, Error> {
        if self
            .0
            .scan_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(Error::App("a scan is already running".into()));
        }

        Ok(ScanGuard {
            state: Arc::clone(&self.0),
        })
    }

    pub fn scan_activity(&self) -> Option<ScanActivity> {
        self.0
            .scan_activity
            .lock()
            .expect("scan activity lock poisoned")
            .clone()
    }

    pub fn update_scan_activity<F>(&self, update: F) -> Option<ScanActivity>
    where
        F: FnOnce(&mut ScanActivity),
    {
        let mut activity = self
            .0
            .scan_activity
            .lock()
            .expect("scan activity lock poisoned");
        let current = activity.as_mut()?;
        update(current);
        current.sequence += 1;
        Some(current.clone())
    }

    pub fn begin_scan_activity(&self, scope_repo_id: Option<i64>, total: u32) -> ScanActivity {
        let mut current = self
            .0
            .scan_activity
            .lock()
            .expect("scan activity lock poisoned");
        let sequence = current.as_ref().map_or(1, |activity| activity.sequence + 1);
        let activity = ScanActivity {
            id: format!("scan-{sequence}"),
            sequence,
            status: ScanActivityStatus::Running,
            stage: ScanActivityStage::Queued,
            interrupted: false,
            scope_repo_id,
            started_at: timestamps::now(),
            finished_at: None,
            total_repositories: total,
            completed_repositories: 0,
            failed_repositories: 0,
            current_repo_id: None,
            current_repo_name: None,
            outcomes: Vec::new(),
            warnings: Vec::new(),
            new_findings: 0,
            resolved_findings: 0,
            worsened_findings: 0,
            terminal_error: None,
        };
        *current = Some(activity.clone());
        activity
    }

    pub fn record_scan_enrichment(
        &self,
        run_id: i64,
        reopened: u32,
        warnings: Vec<String>,
    ) -> Option<ScanActivity> {
        if reopened == 0 && warnings.is_empty() {
            return None;
        }
        let mut current = self
            .0
            .scan_activity
            .lock()
            .expect("scan activity lock poisoned");
        let activity = current.as_mut()?;
        let outcome = activity
            .outcomes
            .iter_mut()
            .find(|outcome| outcome.scan_run_id == Some(run_id) && outcome.succeeded)?;
        outcome.worsened_findings += reopened;
        activity.worsened_findings += reopened;
        activity.warnings.extend(warnings);
        activity.sequence += 1;
        Some(activity.clone())
    }

    pub fn reschedule(&self, app: AppHandle) {
        let interval = self.schedule();

        if let Some(handle) = self
            .0
            .scheduled_task
            .lock()
            .expect("scheduler lock poisoned")
            .take()
        {
            handle.abort();
        }

        let Some(_duration) = scheduler::duration_for(interval) else {
            return;
        };

        let shared = self.clone();
        let handle = tauri::async_runtime::spawn(async move {
            loop {
                let wait = shared
                    .0
                    .next_scheduled_scan_at
                    .lock()
                    .expect("next schedule lock poisoned")
                    .as_ref()
                    .map(duration_until)
                    .unwrap_or_default();
                tokio::time::sleep(wait).await;
                let scan_app = app.clone();
                let scan_state = shared.clone();
                if let Err(cause) = scheduler::finish_started_job(async move {
                    if let Err(cause) =
                        crate::commands::scan_all_internal(scan_app.clone(), scan_state.clone())
                            .await
                    {
                        crate::commands::emit_background_error(
                            &scan_app,
                            format!("Scheduled scan failed: {cause}"),
                        );
                        if let Err(cause) = scan_state.schedule_next_from_now() {
                            crate::commands::emit_background_error(
                                &scan_app,
                                format!("Failed to schedule the next scan: {cause}"),
                            );
                        }
                        if let Ok(snapshot) = crate::commands::refresh_snapshot(&scan_state) {
                            use tauri::Emitter;
                            let _ = scan_app.emit("cvec://state", snapshot);
                        }
                    }
                })
                .await
                {
                    crate::commands::emit_background_error(
                        &app,
                        format!("Scheduled scan worker failed: {cause}"),
                    );
                    let _ = shared.schedule_next_from_now();
                }
            }
        });

        *self
            .0
            .scheduled_task
            .lock()
            .expect("scheduler lock poisoned") = Some(handle);
    }
}

fn parse_timestamp(value: &str) -> Option<OffsetDateTime> {
    timestamps::parse(value).ok()
}

fn duration_until(target: &OffsetDateTime) -> std::time::Duration {
    std::time::Duration::try_from(*target - OffsetDateTime::now_utc()).unwrap_or_default()
}

fn qa_data_dir_override() -> Option<PathBuf> {
    std::env::var_os(QA_DATA_DIR_ENV)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[cfg(debug_assertions)]
fn qa_remediation_seed_enabled(identifier: &str, requested: bool) -> bool {
    identifier == ISOLATED_UI_IDENTIFIER || (identifier == PERFORMANCE_QA_IDENTIFIER && requested)
}

fn qa_scan_activity_fixture() -> Option<ScanActivity> {
    #[cfg(debug_assertions)]
    {
        let scenario = std::env::var(QA_SCAN_ACTIVITY_ENV).ok()?;
        let successful_outcome = ScanRepositoryOutcome {
            repo_id: 1,
            repo_name: "cvec-ui-qa-demo-service".into(),
            scan_run_id: Some(2),
            succeeded: true,
            error: None,
            new_findings: 2,
            resolved_findings: 1,
            worsened_findings: 1,
        };

        if scenario == "running" {
            return Some(ScanActivity {
                id: "scan-native-qa-running".into(),
                sequence: 1,
                status: ScanActivityStatus::Running,
                stage: ScanActivityStage::MatchingVulnerabilities,
                interrupted: false,
                scope_repo_id: None,
                started_at: "2026-08-12T08:00:00Z".into(),
                finished_at: None,
                total_repositories: 2,
                completed_repositories: 1,
                failed_repositories: 0,
                current_repo_id: Some(2),
                current_repo_name: Some("offline-worker".into()),
                outcomes: vec![successful_outcome],
                warnings: Vec::new(),
                new_findings: 2,
                resolved_findings: 1,
                worsened_findings: 1,
                terminal_error: None,
            });
        }

        if scenario == "completed" {
            return Some(ScanActivity {
                interrupted: false,
                id: "scan-native-qa-completed".into(),
                sequence: 1,
                status: ScanActivityStatus::Completed,
                stage: ScanActivityStage::Completed,
                scope_repo_id: None,
                started_at: "2026-08-12T08:00:00Z".into(),
                finished_at: Some("2026-08-12T08:04:00Z".into()),
                total_repositories: 2,
                completed_repositories: 2,
                failed_repositories: 0,
                current_repo_id: None,
                current_repo_name: None,
                outcomes: vec![
                    successful_outcome,
                    ScanRepositoryOutcome {
                        repo_id: 2,
                        repo_name: "offline-worker".into(),
                        scan_run_id: Some(3),
                        succeeded: true,
                        error: None,
                        new_findings: 0,
                        resolved_findings: 0,
                        worsened_findings: 0,
                    },
                ],
                warnings: Vec::new(),
                new_findings: 2,
                resolved_findings: 1,
                worsened_findings: 1,
                terminal_error: None,
            });
        }

        if scenario == "partial_failure" {
            return Some(ScanActivity {
                interrupted: false,
                id: "scan-native-qa-partial".into(),
                sequence: 1,
                status: ScanActivityStatus::PartialFailure,
                stage: ScanActivityStage::Completed,
                scope_repo_id: None,
                started_at: "2026-08-12T08:00:00Z".into(),
                finished_at: Some("2026-08-12T08:04:00Z".into()),
                total_repositories: 2,
                completed_repositories: 1,
                failed_repositories: 1,
                current_repo_id: None,
                current_repo_name: None,
                outcomes: vec![
                    successful_outcome,
                    ScanRepositoryOutcome {
                        repo_id: 2,
                        repo_name: "offline-worker".into(),
                        scan_run_id: None,
                        succeeded: false,
                        error: Some("Repository could not be prepared".into()),
                        new_findings: 0,
                        resolved_findings: 0,
                        worsened_findings: 0,
                    },
                ],
                warnings: vec![
                    "Advisory enrichment was unavailable; scanner results were saved locally."
                        .into(),
                ],
                new_findings: 2,
                resolved_findings: 1,
                worsened_findings: 1,
                terminal_error: None,
            });
        }
    }

    None
}

fn resolve_storage_directories(
    default_data_dir: PathBuf,
    default_cache_dir: PathBuf,
    qa_data_dir: Option<PathBuf>,
) -> (PathBuf, PathBuf) {
    match qa_data_dir.filter(|path| !path.as_os_str().is_empty()) {
        Some(data_dir) => {
            let cache_dir = data_dir.join("cache");
            (data_dir, cache_dir)
        }
        None => (default_data_dir, default_cache_dir),
    }
}

fn ensure_private_directory(path: &std::path::Path) -> Result<(), Error> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            if metadata.file_type().is_dir() {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            }
        }
    }
    Ok(())
}

fn prepare_isolated_ui(db: &Database, data: &std::path::Path) -> Result<(), Error> {
    let fixtures = data.join("remediation-fixtures");
    std::fs::create_dir_all(&fixtures)?;
    db.set_setting("discovery_root", fixtures.to_string_lossy().into_owned())?;
    db.set_setting("schedule_interval", "off".into())?;
    for connection in db.list_ai_connections()? {
        db.set_ai_connection_enabled(connection.id.as_ref(), false)?;
    }
    Ok(())
}

impl AppState {
    pub(crate) fn require_external_ai(&self) -> Result<(), Error> {
        if self.isolated_ui {
            return Err(Error::App(
                "External AI connections are unavailable in this isolated test profile.".into(),
            ));
        }
        Ok(())
    }
}

pub struct ScanGuard {
    state: Arc<AppState>,
}

pub(crate) struct BootstrapRefreshGuard {
    state: Arc<AppState>,
}

impl Drop for BootstrapRefreshGuard {
    fn drop(&mut self) {
        self.state
            .bootstrap_refresh_running
            .store(false, Ordering::SeqCst);
    }
}

impl Drop for ScanGuard {
    fn drop(&mut self) {
        self.state.scan_running.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod storage_directory_tests {
    use super::{resolve_storage_directories, AppState, SharedState};
    use crate::core::{
        companion_context::CompanionContextService,
        companion_host::{CompanionHost, CompanionHostConfig},
        db::Database,
        github_auth::GithubAuthState,
        grok_host::GrokHost,
        models::{ScanActivityStage, ScanActivityStatus, ScheduleInterval},
        opencode_host::OpenCodeHost,
        provider_secrets::ProviderSecretStore,
    };
    use std::path::PathBuf;
    use std::sync::{atomic::AtomicBool, Arc, Mutex};
    use tempfile::tempdir;

    #[cfg(debug_assertions)]
    #[test]
    fn only_dedicated_qa_profiles_can_seed_remediation_fixtures() {
        use super::{
            qa_remediation_seed_enabled, ISOLATED_UI_IDENTIFIER, PERFORMANCE_QA_IDENTIFIER,
        };

        assert!(!qa_remediation_seed_enabled("dev.cvec.app", true));
        assert!(qa_remediation_seed_enabled(ISOLATED_UI_IDENTIFIER, false));
        assert!(!qa_remediation_seed_enabled(
            PERFORMANCE_QA_IDENTIFIER,
            false
        ));
        assert!(qa_remediation_seed_enabled(PERFORMANCE_QA_IDENTIFIER, true));
    }

    fn test_state(path: &std::path::Path) -> SharedState {
        let db = Database::new(path.join("test.sqlite")).expect("database");
        let companion = CompanionHost::with_event_sink(
            db.clone(),
            CompanionHostConfig::production(path.join("companion-sandbox")),
            |_| {},
        );
        let fix_workspaces = crate::core::fix_workspaces::FixWorkspaces::new(
            db.clone(),
            path.join("fix-workspaces"),
            path.join("cache"),
            companion.clone(),
            |_| {},
        )
        .expect("fix service");
        SharedState(Arc::new(AppState {
            isolated_ui: false,
            db: db.clone(),
            companion,
            fix_workspaces,
            grok: GrokHost::with_event_sink(db.clone(), path.join("grok-sandbox"), |_| {}),
            open_code: OpenCodeHost::with_event_sink(
                db.clone(),
                path.join("opencode-sandbox"),
                |_| {},
            ),
            companion_context: CompanionContextService::new(db),
            provider_secrets: ProviderSecretStore::new(),
            discovery_root: Mutex::new(path.to_path_buf()),
            grype_db_cache_dir: path.join("cache"),
            github_blob_cache_dir: path.join("github-cache"),
            github_auth: GithubAuthState::new(),
            scanner_status: Mutex::new(crate::core::models::ScannerAvailability {
                checked_at: None,
                checking: false,
                ready: false,
                message: None,
                versions: None,
                grype_db: None,
            }),
            scan_running: AtomicBool::new(false),
            bootstrap_refresh_running: AtomicBool::new(false),
            scan_activity: Mutex::new(None),
            schedule: Mutex::new(ScheduleInterval::Off),
            next_scheduled_scan_at: Mutex::new(None),
            scheduled_task: Mutex::new(None),
        }))
    }

    #[test]
    fn cached_snapshot_remains_available_while_one_bootstrap_refresh_is_running() {
        let temporary = tempdir().unwrap();
        let state = test_state(temporary.path());
        let refresh = state.begin_bootstrap_refresh().expect("first refresh");
        assert!(state.begin_bootstrap_refresh().is_none());
        let snapshot = crate::commands::refresh_snapshot(&state).expect("cached snapshot");
        assert_eq!(snapshot.review_summary.needs_review, 0);
        drop(refresh);
        assert!(state.begin_bootstrap_refresh().is_some());
    }

    #[test]
    fn qa_storage_override_is_explicit_and_keeps_cache_isolated() {
        let defaults = (
            PathBuf::from("default-data"),
            PathBuf::from("default-cache"),
        );

        assert_eq!(
            resolve_storage_directories(defaults.0.clone(), defaults.1.clone(), None),
            defaults
        );
        assert_eq!(
            resolve_storage_directories(
                PathBuf::from("default-data"),
                PathBuf::from("default-cache"),
                Some(PathBuf::new()),
            ),
            (
                PathBuf::from("default-data"),
                PathBuf::from("default-cache")
            )
        );
        assert_eq!(
            resolve_storage_directories(
                PathBuf::from("default-data"),
                PathBuf::from("default-cache"),
                Some(PathBuf::from("qa-data")),
            ),
            (PathBuf::from("qa-data"), PathBuf::from("qa-data/cache"))
        );
    }

    #[test]
    fn scan_activity_survives_frontend_remount_and_orders_real_stages() {
        let temporary = tempdir().unwrap();
        let state = test_state(temporary.path());
        let initial = state.begin_scan_activity(None, 2);
        assert_eq!(initial.status, ScanActivityStatus::Running);
        assert_eq!(initial.stage, ScanActivityStage::Queued);
        let stages = [
            ScanActivityStage::PreparingSource,
            ScanActivityStage::InventoryingPackages,
            ScanActivityStage::MatchingVulnerabilities,
            ScanActivityStage::EnrichingAdvisories,
            ScanActivityStage::SavingResults,
        ];
        for (index, stage) in stages.into_iter().enumerate() {
            state.update_scan_activity(|activity| activity.stage = stage);
            let progress = state.scan_activity().expect("activity");
            assert_eq!(progress.stage, stage);
            assert_eq!(progress.sequence, initial.sequence + index as u64 + 1);
        }
        assert_eq!(
            state.scan_activity().expect("bootstrap activity").id,
            initial.id
        );
        assert!(state.begin_scan_activity(Some(1), 1).sequence > initial.sequence);
    }

    #[test]
    fn late_advisory_updates_only_the_matching_scan_activity() {
        let temporary = tempdir().unwrap();
        let state = test_state(temporary.path());
        state.begin_scan_activity(Some(1), 1);
        let completed = state
            .update_scan_activity(|activity| {
                activity.status = ScanActivityStatus::Completed;
                activity
                    .outcomes
                    .push(crate::core::models::ScanRepositoryOutcome {
                        repo_id: 1,
                        repo_name: "fixture".into(),
                        scan_run_id: Some(7),
                        succeeded: true,
                        error: None,
                        new_findings: 0,
                        resolved_findings: 0,
                        worsened_findings: 0,
                    });
            })
            .unwrap();
        let enriched = state
            .record_scan_enrichment(7, 1, vec!["Optional fixture warning".into()])
            .unwrap();
        assert_eq!(enriched.id, completed.id);
        assert_eq!(enriched.sequence, completed.sequence + 1);
        assert_eq!(enriched.status, ScanActivityStatus::Completed);
        assert_eq!(enriched.worsened_findings, 1);
        assert_eq!(enriched.outcomes[0].worsened_findings, 1);
        let next = state.begin_scan_activity(Some(2), 1);
        assert!(state
            .record_scan_enrichment(7, 1, vec!["Another warning".into()])
            .is_none());
        assert_eq!(state.scan_activity().unwrap(), next);
    }

    #[test]
    fn scan_guard_rejects_duplicates_and_releases_after_drop() {
        let temporary = tempdir().unwrap();
        let state = test_state(temporary.path());
        let guard = state.try_acquire_scan().expect("first scan");
        assert!(state.try_acquire_scan().is_err());
        drop(guard);
        assert!(state.try_acquire_scan().is_ok());
    }

    #[test]
    fn failed_schedule_write_keeps_settings_consistent_and_advances_the_deadline() {
        use crate::core::models::ScheduleInterval;
        use time::{Duration, OffsetDateTime};

        let temporary = tempdir().unwrap();
        let state = test_state(temporary.path());
        state.set_schedule(ScheduleInterval::Every12Hours).unwrap();
        *state.0.next_scheduled_scan_at.lock().unwrap() =
            Some(OffsetDateTime::now_utc() - Duration::minutes(1));
        let saved_deadline = state.0.db.get_setting("next_scheduled_scan_at").unwrap();
        let connection = rusqlite::Connection::open(state.0.db.path()).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_schedule BEFORE UPDATE OF value ON app_settings
             WHEN OLD.key = 'next_scheduled_scan_at'
             BEGIN SELECT RAISE(ABORT, 'fixture schedule write failure'); END;",
            )
            .unwrap();

        assert!(state.set_schedule(ScheduleInterval::Off).is_err());
        assert_eq!(state.schedule(), ScheduleInterval::Every12Hours);
        assert_eq!(
            state
                .0
                .db
                .get_setting("schedule_interval")
                .unwrap()
                .as_deref(),
            Some("12h")
        );
        assert!(state.schedule_next_from_now().is_err());
        assert!(
            state.0.next_scheduled_scan_at.lock().unwrap().unwrap() > OffsetDateTime::now_utc()
        );
        assert_eq!(
            state.0.db.get_setting("next_scheduled_scan_at").unwrap(),
            saved_deadline
        );
    }

    #[test]
    fn isolated_profile_pins_its_fixture_root_and_disables_existing_connections() {
        let temporary = tempdir().unwrap();
        let db = Database::new(temporary.path().join("qa.db")).unwrap();
        db.set_setting("discovery_root", "/private-user-repository".into())
            .unwrap();
        db.set_setting("schedule_interval", "hourly".into())
            .unwrap();
        super::prepare_isolated_ui(&db, temporary.path()).unwrap();
        assert_eq!(
            db.get_setting("discovery_root").unwrap().unwrap(),
            temporary
                .path()
                .join("remediation-fixtures")
                .to_string_lossy()
        );
        assert_eq!(
            db.get_setting("schedule_interval").unwrap().as_deref(),
            Some("off")
        );
        assert!(db
            .list_ai_connections()
            .unwrap()
            .iter()
            .all(|connection| !connection.enabled));
        let state_directory = tempdir().unwrap();
        let mut state = test_state(state_directory.path());
        Arc::get_mut(&mut state.0).unwrap().isolated_ui = true;
        assert!(state.0.require_external_ai().is_err());
        assert!(state
            .set_discovery_root(temporary.path().to_path_buf())
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_private_directory_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempdir().unwrap();
        let private_dir = temporary.path().join("app-data");
        super::ensure_private_directory(&private_dir).unwrap();
        let mode = std::fs::symlink_metadata(&private_dir)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }
}
