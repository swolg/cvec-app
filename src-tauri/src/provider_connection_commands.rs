use std::time::Duration;

use tauri::State;

use crate::core::companion::COMPANION_DISCLOSURE_VERSION;
use crate::core::provider_connections::{
    boundary_fingerprint, is_builtin_connection_id, now_timestamp, probe_grok, probe_opencode,
    InferenceBoundary, ProviderConnection, ProviderConnectionDraft, ProviderConnectionModel,
    ProviderConnectionSnapshot, ProviderDriverKind, ProviderErrorCode, ProviderErrorInfo,
    ProviderHealthState, ProviderUpstream, DEFAULT_CODEX_CONNECTION_ID,
    PROVIDER_CONNECTION_SCHEMA_VERSION,
};
use crate::state::{AppState, SharedState};
use crate::Error;

async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    crate::runtime::run_io(operation)
        .await
        .map_err(|cause| format!("AI connection operation could not complete: {cause}"))?
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn provider_connection_list(
    state: State<'_, SharedState>,
) -> Result<Vec<ProviderConnectionSnapshot>, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        shared
            .0
            .db
            .list_ai_connections()?
            .into_iter()
            .map(|connection| snapshot(&shared.0, connection, true))
            .collect()
    })
    .await
}

#[tauri::command]
pub async fn provider_connection_get(
    state: State<'_, SharedState>,
    connection_id: String,
) -> Result<ProviderConnectionSnapshot, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        let connection = shared.0.db.ai_connection(&connection_id)?;
        snapshot(&shared.0, connection, false)
    })
    .await
}

#[tauri::command]
pub async fn provider_connection_validate(
    state: State<'_, SharedState>,
    draft: ProviderConnectionDraft,
) -> Result<ProviderConnectionSnapshot, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        let timestamp = now_timestamp();
        let connection = ProviderConnection {
            schema_version: PROVIDER_CONNECTION_SCHEMA_VERSION,
            id: crate::core::provider_connections::ProviderConnectionId::parse(
                draft.id.trim().to_string(),
            )?,
            driver: draft.driver,
            display_name: draft.display_name.trim().into(),
            accent_color: draft.accent_color.clone(),
            enabled: draft.enabled,
            built_in: is_builtin_connection_id(&draft.id),
            config: draft.config.clone(),
            has_server_password: draft
                .server_password
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
            created_at: timestamp.clone(),
            updated_at: timestamp,
        };
        connection.validate()?;
        snapshot(&shared.0, connection, true)
    })
    .await
}

#[tauri::command]
pub async fn provider_connection_save(
    state: State<'_, SharedState>,
    draft: ProviderConnectionDraft,
) -> Result<ProviderConnectionSnapshot, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        if draft.enabled || draft.server_password.is_some() {
            shared.0.require_external_ai()?;
        }
        let password = draft
            .server_password
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut non_secret_draft = draft.clone();
        non_secret_draft.server_password = None;
        let mut connection = shared.0.db.save_ai_connection(&non_secret_draft)?;
        if let Some(password) = password.as_deref() {
            shared
                .0
                .provider_secrets
                .set_server_password(&draft.id, password)?;
            connection = shared
                .0
                .db
                .set_ai_connection_secret_presence(&draft.id, true)?;
        }
        snapshot(&shared.0, connection, true)
    })
    .await
}

#[tauri::command]
pub async fn provider_connection_refresh(
    state: State<'_, SharedState>,
    connection_id: String,
) -> Result<ProviderConnectionSnapshot, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        let connection = shared.0.db.ai_connection(&connection_id)?;
        snapshot(&shared.0, connection, true)
    })
    .await
}

#[tauri::command]
pub async fn provider_connection_set_enabled(
    state: State<'_, SharedState>,
    connection_id: String,
    enabled: bool,
) -> Result<ProviderConnectionSnapshot, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        if enabled {
            shared.0.require_external_ai()?;
        }
        let connection = shared
            .0
            .db
            .set_ai_connection_enabled(&connection_id, enabled)?;
        snapshot(&shared.0, connection, enabled)
    })
    .await
}

#[tauri::command]
pub async fn provider_connection_reset(
    state: State<'_, SharedState>,
    connection_id: String,
) -> Result<ProviderConnectionSnapshot, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        let _ = shared
            .0
            .provider_secrets
            .delete_server_password(&connection_id);
        let mut connection = shared.0.db.reset_ai_connection(&connection_id)?;
        if shared.0.isolated_ui {
            connection = shared
                .0
                .db
                .set_ai_connection_enabled(&connection_id, false)?;
        }
        snapshot(&shared.0, connection, true)
    })
    .await
}

#[tauri::command]
pub async fn provider_connection_remove(
    state: State<'_, SharedState>,
    connection_id: String,
    confirm_bound_threads: bool,
) -> Result<u32, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        let bound = shared
            .0
            .db
            .ai_connection_bound_thread_count(&connection_id)?;
        if bound > 0 && !confirm_bound_threads {
            return Err(Error::App(format!(
                "connection is used by {bound} existing chat thread(s); confirm removal to continue"
            )));
        }
        shared
            .0
            .provider_secrets
            .delete_server_password(&connection_id)?;
        shared.0.db.remove_ai_connection(&connection_id)
    })
    .await
}

#[tauri::command]
pub async fn provider_connection_consent(
    state: State<'_, SharedState>,
    connection_id: String,
    model_id: String,
) -> Result<crate::core::companion::PrivacyConsentStatus, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        let connection = shared
            .0
            .db
            .ai_connection(&connection_id)
            .map_err(|cause| cause.to_string())?;
        let (upstream, boundary) = model_boundary(&connection, &model_id);
        let fingerprint = boundary_fingerprint(
            &connection.id,
            connection.driver,
            upstream.as_deref(),
            boundary,
            COMPANION_DISCLOSURE_VERSION,
        );
        shared
            .0
            .db
            .connection_privacy_consent(&connection_id, &fingerprint, COMPANION_DISCLOSURE_VERSION)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn provider_connection_set_consent(
    state: State<'_, SharedState>,
    connection_id: String,
    model_id: String,
    accepted: bool,
) -> Result<crate::core::companion::PrivacyConsentStatus, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        let connection = shared
            .0
            .db
            .ai_connection(&connection_id)
            .map_err(|cause| cause.to_string())?;
        let (upstream, boundary) = model_boundary(&connection, &model_id);
        let fingerprint = boundary_fingerprint(
            &connection.id,
            connection.driver,
            upstream.as_deref(),
            boundary,
            COMPANION_DISCLOSURE_VERSION,
        );
        shared
            .0
            .db
            .set_connection_privacy_consent(
                &connection_id,
                &fingerprint,
                COMPANION_DISCLOSURE_VERSION,
                accepted,
            )
            .map_err(|cause| cause.to_string())
    })
    .await?
}

fn snapshot(
    state: &AppState,
    mut connection: ProviderConnection,
    refresh: bool,
) -> Result<ProviderConnectionSnapshot, Error> {
    if state.isolated_ui {
        connection.enabled = false;
    }
    if !connection.enabled {
        return Ok(ProviderConnectionSnapshot::unchecked(connection));
    }
    match connection.driver {
        ProviderDriverKind::Grok => Ok(if refresh {
            probe_grok(&connection)
        } else {
            ProviderConnectionSnapshot::unchecked(connection)
        }),
        ProviderDriverKind::OpenCode => Ok(if refresh {
            probe_opencode(&connection)
        } else {
            ProviderConnectionSnapshot::unchecked(connection)
        }),
        ProviderDriverKind::Codex => snapshot_codex(state, connection, refresh),
    }
}

fn snapshot_codex(
    state: &AppState,
    connection: ProviderConnection,
    refresh: bool,
) -> Result<ProviderConnectionSnapshot, Error> {
    let mut result = ProviderConnectionSnapshot::unchecked(connection.clone());
    if connection.id.as_ref() != DEFAULT_CODEX_CONNECTION_ID {
        let binary = match &connection.config {
            crate::core::provider_connections::ProviderConnectionConfig::Codex(config) => {
                config.binary_path.as_deref().unwrap_or("codex")
            }
            _ => "codex",
        };
        match configured_codex_version(binary, Duration::from_secs(4)) {
            Ok(version) => {
                result.installed_version = Some(version);
                result.health = ProviderHealthState::SetupRequired;
                result.message = Some(
                    "Codex version detected. Additional Codex runtimes are not initialized by CVEC. Select the built-in Codex connection."
                        .into(),
                );
            }
            Err(error) => {
                result.health = ProviderHealthState::Unavailable;
                result.error = Some(error);
            }
        }
        result.checked_at = Some(now_timestamp());
        return Ok(result);
    }
    if refresh {
        let runtime = match state.companion.start() {
            Ok(runtime) => runtime,
            Err(cause) => {
                result.health = ProviderHealthState::Unavailable;
                result.checked_at = Some(now_timestamp());
                result.error = Some(ProviderErrorInfo {
                    code: ProviderErrorCode::NotInstalled,
                    message: cause.to_string(),
                    retryable: true,
                });
                return Ok(result);
            }
        };
        let account = match state.companion.account_status() {
            Ok(account) => account,
            Err(cause) => {
                result.installed_version = runtime.codex_version;
                result.health = ProviderHealthState::SetupRequired;
                result.checked_at = Some(now_timestamp());
                result.error = Some(ProviderErrorInfo {
                    code: ProviderErrorCode::AuthenticationRequired,
                    message: cause.to_string(),
                    retryable: true,
                });
                return Ok(result);
            }
        };
        let models = state.companion.list_models().unwrap_or_default();
        result.installed_version = runtime.codex_version;
        result.auth_kind = account.auth_kind;
        result.account_label = account.account_label;
        result.health = if account.signed_in {
            ProviderHealthState::Ready
        } else {
            ProviderHealthState::SetupRequired
        };
        result.models = models
            .into_iter()
            .map(|model| ProviderConnectionModel {
                connection_id: connection.id.clone(),
                driver: ProviderDriverKind::Codex,
                upstream_provider_id: Some("openai".into()),
                inference_boundary: InferenceBoundary::External,
                model,
            })
            .collect();
        result.upstream_providers = vec![ProviderUpstream {
            id: "openai".into(),
            display_name: "OpenAI".into(),
            connected: account.signed_in,
            inference_boundary: InferenceBoundary::External,
        }];
        result.checked_at = Some(now_timestamp());
        result.message = account.message;
    } else {
        let runtime = state.companion.status();
        result.installed_version = runtime.codex_version;
        result.health = match runtime.phase {
            crate::core::companion::CompanionRuntimePhase::Ready
            | crate::core::companion::CompanionRuntimePhase::ActiveTurn => {
                ProviderHealthState::Ready
            }
            crate::core::companion::CompanionRuntimePhase::SignedOut => {
                ProviderHealthState::SetupRequired
            }
            crate::core::companion::CompanionRuntimePhase::Incompatible => {
                ProviderHealthState::Incompatible
            }
            crate::core::companion::CompanionRuntimePhase::Unavailable => {
                ProviderHealthState::Unavailable
            }
            _ => ProviderHealthState::Unchecked,
        };
        result.message = runtime.message;
    }
    Ok(result)
}

fn configured_codex_version(binary: &str, deadline: Duration) -> Result<String, ProviderErrorInfo> {
    use crate::core::provider_connections::{bounded_process_output, ProcessProbeError};
    let output = bounded_process_output(
        crate::core::toolchain::command(binary).arg("--version"),
        deadline,
    )
    .map_err(|cause| ProviderErrorInfo {
        code: if cause == ProcessProbeError::Spawn {
            ProviderErrorCode::NotInstalled
        } else {
            ProviderErrorCode::ProcessExited
        },
        message: match cause {
            ProcessProbeError::Spawn => "Codex is not installed at the configured path.",
            ProcessProbeError::Timeout => {
                "The configured Codex version check timed out. Check its executable and retry."
            }
            ProcessProbeError::Cancelled => "The configured Codex version check was cancelled.",
        }
        .into(),
        retryable: true,
    })?;
    if !output.status.success() {
        return Err(ProviderErrorInfo {
            code: ProviderErrorCode::ProcessExited,
            message: "The configured Codex version check failed. Check its executable and retry."
                .into(),
            retryable: true,
        });
    }
    let version = std::str::from_utf8(&output.stdout)
        .ok()
        .map(str::trim)
        .filter(|version| !version.is_empty() && version.len() <= 4096)
        .ok_or_else(|| ProviderErrorInfo {
            code: ProviderErrorCode::ProtocolError,
            message: "The configured Codex returned an empty or oversized version response.".into(),
            retryable: true,
        })?;
    Ok(crate::core::provider_connections::redact(version))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, process::Command, time::Instant};

    fn executable(directory: &std::path::Path, script: &str) -> String {
        let path = directory.join("configured codex");
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn configured_codex_probe_times_out_and_reaps_its_process() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("pid");
        let binary = executable(
            directory.path(),
            &format!(
                "printf '%s' \"$$\" > '{}'\nexec /bin/sleep 3",
                marker.display()
            ),
        );
        let started = Instant::now();
        let result = configured_codex_version(&binary, Duration::from_secs(1));
        let error = result.unwrap_err();
        assert_eq!(error.code, ProviderErrorCode::ProcessExited);
        assert!(error.message.contains("timed out"));
        assert!(started.elapsed() < Duration::from_millis(2500));
        let pid = fs::read_to_string(marker).unwrap();
        assert!(!Command::new("/bin/kill")
            .args(["-0", &pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn configured_codex_probe_accepts_only_bounded_successful_version_output() {
        let directory = tempfile::tempdir().unwrap();
        let binary = executable(directory.path(), "printf 'codex 0.166.0\\n'");
        assert_eq!(
            configured_codex_version(&binary, Duration::from_secs(1)).unwrap(),
            "codex 0.166.0"
        );
        for script in [
            "exit 0",
            "printf 'not a version'; exit 7",
            "i=0; while [ \"$i\" -lt 5000 ]; do printf x; i=$((i+1)); done",
        ] {
            let binary = executable(directory.path(), script);
            assert!(configured_codex_version(&binary, Duration::from_secs(1)).is_err());
        }
    }
}

fn model_boundary(
    connection: &ProviderConnection,
    model_id: &str,
) -> (Option<String>, InferenceBoundary) {
    match connection.driver {
        ProviderDriverKind::Codex => (Some("openai".into()), InferenceBoundary::External),
        ProviderDriverKind::Grok => (Some("xai".into()), InferenceBoundary::External),
        ProviderDriverKind::OpenCode => {
            let upstream = model_id.split_once('/').map(|value| value.0.to_string());
            let boundary = upstream
                .as_deref()
                .map(|provider| {
                    crate::core::provider_connections::classify_upstream(provider, None)
                })
                .unwrap_or(InferenceBoundary::Unknown);
            (upstream, boundary)
        }
    }
}
