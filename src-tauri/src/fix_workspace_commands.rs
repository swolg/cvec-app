use crate::{
    core::fix_workspaces::{FixArchivePage, FixSnapshot, FixStartRequest, FixWorkspace},
    state::SharedState,
    Error,
};
use tauri::State;

async fn run<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, String> {
    crate::runtime::run_io(operation)
        .await?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn fix_workspace_list(
    state: State<'_, SharedState>,
    repository_id: i64,
) -> Result<Vec<FixWorkspace>, String> {
    let service = state.0.fix_workspaces.clone();
    run(move || service.list(repository_id)).await
}
#[tauri::command]
pub async fn fix_workspace_archived(
    state: State<'_, SharedState>,
    repository_id: i64,
    cursor: Option<String>,
    finding_fingerprint: Option<String>,
) -> Result<FixArchivePage, String> {
    let service = state.0.fix_workspaces.clone();
    run(move || service.archived(repository_id, cursor, finding_fingerprint)).await
}
#[tauri::command]
pub async fn fix_workspace_set_archived(
    state: State<'_, SharedState>,
    repository_id: i64,
    workspace_id: String,
    archived: bool,
) -> Result<FixSnapshot, String> {
    let service = state.0.fix_workspaces.clone();
    run(move || service.set_archived(repository_id, &workspace_id, archived)).await
}
#[tauri::command]
pub async fn fix_workspace_create(
    state: State<'_, SharedState>,
    repository_id: i64,
    finding_fingerprint: Option<String>,
    confirmed: bool,
    expected_scan_run_id: Option<i64>,
) -> Result<FixSnapshot, String> {
    let service = state.0.fix_workspaces.clone();
    run(move || {
        service.create(
            repository_id,
            finding_fingerprint,
            confirmed,
            expected_scan_run_id,
        )
    })
    .await
}
#[tauri::command]
pub async fn fix_workspace_snapshot(
    state: State<'_, SharedState>,
    repository_id: i64,
    workspace_id: String,
) -> Result<FixSnapshot, String> {
    let service = state.0.fix_workspaces.clone();
    run(move || service.snapshot(repository_id, &workspace_id)).await
}
#[tauri::command]
pub async fn fix_workspace_open_checkout(
    state: State<'_, SharedState>,
    repository_id: i64,
) -> Result<(), String> {
    let service = state.0.fix_workspaces.clone();
    run(move || crate::commands::open_local_file(&service.checkout_directory(repository_id)?)).await
}

#[tauri::command]
pub async fn fix_workspace_open_directory(
    state: State<'_, SharedState>,
    repository_id: i64,
    workspace_id: String,
) -> Result<(), String> {
    let service = state.0.fix_workspaces.clone();
    run(move || crate::commands::open_local_file(&service.directory(repository_id, &workspace_id)?))
        .await
}
#[tauri::command]
pub async fn fix_workspace_start(
    state: State<'_, SharedState>,
    repository_id: i64,
    workspace_id: String,
    request: FixStartRequest,
) -> Result<(), String> {
    state
        .0
        .require_external_ai()
        .map_err(|error| error.to_string())?;
    let service = state.0.fix_workspaces.clone();
    run(move || service.start(repository_id, &workspace_id, request)).await
}
#[tauri::command]
pub async fn fix_workspace_stop(
    state: State<'_, SharedState>,
    repository_id: i64,
    workspace_id: String,
) -> Result<(), String> {
    let service = state.0.fix_workspaces.clone();
    // Stop must remain available even when both ordinary I/O workers are busy.
    crate::runtime::run(move || service.stop(repository_id, &workspace_id))
        .await?
        .map_err(|error| error.to_string())
}
#[tauri::command]
pub async fn fix_workspace_check(
    state: State<'_, SharedState>,
    repository_id: i64,
    workspace_id: String,
    action: String,
) -> Result<FixSnapshot, String> {
    let service = state.0.fix_workspaces.clone();
    run(move || service.run_check(repository_id, &workspace_id, &action)).await
}
#[tauri::command]
pub async fn fix_workspace_rescan(
    state: State<'_, SharedState>,
    repository_id: i64,
    workspace_id: String,
) -> Result<FixSnapshot, String> {
    let service = state.0.fix_workspaces.clone();
    crate::runtime::run_rescan(move || service.rescan(repository_id, &workspace_id))
        .await?
        .map_err(|error| error.to_string())
}
