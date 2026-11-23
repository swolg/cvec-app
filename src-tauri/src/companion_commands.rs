use crate::core::timestamps;
use serde_json::Value;
use tauri::State;

use crate::core::companion::{
    CompanionAccountStatus, CompanionActivity, CompanionActivityKind,
    CompanionCachedThreadSnapshot, CompanionConversationIndex, CompanionLoginStart,
    CompanionMention, CompanionModel, CompanionProviderSegmentStatus, CompanionRuntimeStatus,
    CompanionSourceDetail, CompanionThread, CompanionThreadSnapshot, CompanionTimelineItem,
    CompanionTimelineItemKind, CompanionTimelineItemStatus, CompanionTurnRequest,
    CompanionTurnStart, CompanionUsageStatus, ContextAttachmentKind, ContextAttachmentRequest,
    ContextPreviewRequest, PrivacyConsentStatus, RepositoryChatContext,
};
use crate::core::provider_connections::{
    boundary_fingerprint, classify_upstream, InferenceBoundary, ProviderDriverKind,
    DEFAULT_CODEX_CONNECTION_ID,
};
use crate::state::SharedState;
use crate::Error;

async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    crate::runtime::run_io(operation)
        .await
        .map_err(|cause| format!("companion operation could not complete: {cause}"))?
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn companion_read_draft(
    state: State<'_, SharedState>,
    repository_id: i64,
    scope: String,
) -> Result<Option<String>, String> {
    let shared = state.inner().clone();
    run_blocking(move || shared.0.db.companion_read_draft(repository_id, &scope)).await
}

#[tauri::command]
pub async fn companion_write_draft(
    state: State<'_, SharedState>,
    repository_id: i64,
    scope: String,
    text: String,
) -> Result<(), String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        shared
            .0
            .db
            .companion_write_draft(repository_id, &scope, &text)
    })
    .await
}

#[tauri::command]
pub async fn companion_runtime_status(
    state: State<'_, SharedState>,
) -> Result<CompanionRuntimeStatus, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || shared.0.companion.status()).await
}

#[tauri::command]
pub async fn companion_start_runtime(
    state: State<'_, SharedState>,
) -> Result<CompanionRuntimeStatus, String> {
    let shared = state.inner().clone();
    run_blocking(move || shared.0.companion.start()).await
}

#[tauri::command]
pub async fn companion_stop_runtime(state: State<'_, SharedState>) -> Result<(), String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared.0.companion.shutdown();
        shared.0.grok.shutdown();
        shared.0.open_code.shutdown();
    })
    .await
}

#[tauri::command]
pub async fn companion_account_status(
    state: State<'_, SharedState>,
) -> Result<CompanionAccountStatus, String> {
    let shared = state.inner().clone();
    run_blocking(move || shared.0.companion.account_status()).await
}

#[tauri::command]
pub async fn companion_login_chatgpt(
    state: State<'_, SharedState>,
    device_code: Option<bool>,
) -> Result<CompanionLoginStart, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        shared
            .0
            .companion
            .login_chatgpt(device_code.unwrap_or(false))
    })
    .await
}

#[tauri::command]
pub async fn companion_login_api_key(
    state: State<'_, SharedState>,
    api_key: String,
) -> Result<CompanionLoginStart, String> {
    let shared = state.inner().clone();
    run_blocking(move || shared.0.companion.login_api_key(api_key)).await
}

#[tauri::command]
pub async fn companion_cancel_login(
    state: State<'_, SharedState>,
    login_id: String,
) -> Result<(), String> {
    let shared = state.inner().clone();
    run_blocking(move || shared.0.companion.cancel_login(&login_id)).await
}

#[tauri::command]
pub async fn companion_logout(
    state: State<'_, SharedState>,
    confirm_shared_account: bool,
) -> Result<(), String> {
    let shared = state.inner().clone();
    run_blocking(move || shared.0.companion.logout(confirm_shared_account)).await
}

#[tauri::command]
pub async fn companion_list_models(
    state: State<'_, SharedState>,
) -> Result<Vec<CompanionModel>, String> {
    let shared = state.inner().clone();
    run_blocking(move || shared.0.companion.list_models()).await
}

#[tauri::command]
pub async fn companion_usage(
    state: State<'_, SharedState>,
    force_refresh: Option<bool>,
) -> Result<CompanionUsageStatus, String> {
    let shared = state.inner().clone();
    run_blocking(move || shared.0.companion.usage(force_refresh.unwrap_or(false))).await
}

#[tauri::command]
pub async fn companion_list_threads(
    state: State<'_, SharedState>,
    repository_id: i64,
    include_archived: Option<bool>,
) -> Result<Vec<CompanionThread>, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .list_companion_threads(repository_id, include_archived.unwrap_or(false))
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn companion_conversation_index(
    state: State<'_, SharedState>,
) -> Result<CompanionConversationIndex, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .list_companion_conversation_index()
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn companion_cached_thread(
    state: State<'_, SharedState>,
    repository_id: i64,
    thread_id: i64,
) -> Result<CompanionCachedThreadSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        repository_thread(&shared, repository_id, thread_id).map_err(|cause| cause.to_string())?;
        shared
            .0
            .db
            .cached_companion_thread_page(thread_id, None)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn companion_history_page(
    state: State<'_, SharedState>,
    repository_id: i64,
    thread_id: i64,
    cursor: Option<String>,
) -> Result<CompanionCachedThreadSnapshot, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        repository_thread(&shared, repository_id, thread_id).map_err(|cause| cause.to_string())?;
        shared
            .0
            .db
            .cached_companion_thread_page(thread_id, cursor.as_deref())
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn companion_create_thread(
    state: State<'_, SharedState>,
    repository_id: i64,
    connection_id: Option<String>,
    model: Option<String>,
    initial_title: Option<String>,
) -> Result<CompanionThread, String> {
    let shared = state.inner().clone();
    shared
        .0
        .require_external_ai()
        .map_err(|cause| cause.to_string())?;
    run_blocking(move || {
        let connection_id = connection_id.unwrap_or_else(|| DEFAULT_CODEX_CONNECTION_ID.into());
        let connection = shared.0.db.ai_connection(&connection_id)?;
        if !connection.enabled {
            return Err(Error::App("The selected AI connection is disabled".into()));
        }
        let title = deterministic_thread_title(initial_title.as_deref());
        match connection.driver {
            ProviderDriverKind::Codex => {
                if connection.id.as_ref() != DEFAULT_CODEX_CONNECTION_ID {
                    return Err(Error::App("Additional Codex runtimes are not initialized yet; use the built-in Codex connection".into()));
                }
                let response = shared.0.companion.start_thread(repository_id, model.as_deref())?;
                let provider_thread_id = provider_thread_id(&response)?;
                shared.0.db.create_companion_thread_for_connection(
                    repository_id,
                    &connection_id,
                    "codex",
                    &provider_thread_id,
                    &title,
                    model.as_deref(),
                )
            }
            ProviderDriverKind::Grok => shared.0.grok.create_thread(
                repository_id,
                &connection_id,
                &title,
                model.as_deref(),
            ),
            ProviderDriverKind::OpenCode => shared.0.open_code.create_thread(
                repository_id,
                &connection_id,
                &title,
                model.as_deref(),
            ),
        }
    })
    .await
}

fn bounded_provider_snapshot(mut snapshot: CompanionThreadSnapshot) -> CompanionThreadSnapshot {
    let mut bytes = 0usize;
    let mut count = 0;
    for item in snapshot.items.iter().rev().take(200) {
        let size = serde_json::to_vec(item)
            .map(|value| value.len())
            .unwrap_or(usize::MAX);
        if count > 0 && bytes.saturating_add(size) > 512 * 1024 {
            break;
        }
        bytes = bytes.saturating_add(size);
        count += 1;
    }
    snapshot
        .items
        .drain(..snapshot.items.len().saturating_sub(count));
    snapshot
}

#[tauri::command]
pub async fn companion_resume_thread(
    state: State<'_, SharedState>,
    repository_id: i64,
    thread_id: i64,
) -> Result<CompanionThreadSnapshot, String> {
    let shared = state.inner().clone();
    shared
        .0
        .require_external_ai()
        .map_err(|cause| cause.to_string())?;
    run_blocking(move || {
        let mut thread = repository_thread(&shared, repository_id, thread_id)?;
        if thread.provider == "grok" {
            let snapshot = shared.0.grok.resume_thread(thread);
            shared
                .0
                .db
                .cache_companion_thread_snapshot(thread_id, &snapshot)?;
            return Ok(bounded_provider_snapshot(snapshot));
        }
        if thread.provider == "opencode" {
            let snapshot = shared.0.open_code.resume_thread(thread);
            shared
                .0
                .db
                .cache_companion_thread_snapshot(thread_id, &snapshot)?;
            return Ok(bounded_provider_snapshot(snapshot));
        }
        if !shared.0.db.companion_thread_has_provider_turn(thread_id)? {
            let response = shared
                .0
                .companion
                .start_thread(repository_id, thread.last_model_id.as_deref())?;
            let fresh_provider_thread_id = provider_thread_id(&response)?;
            thread = shared
                .0
                .db
                .rebind_empty_companion_thread(thread_id, &fresh_provider_thread_id)?;
            let snapshot = shared
                .0
                .companion
                .thread_snapshot(thread, &serde_json::json!({ "thread": { "turns": [] } }));
            shared
                .0
                .db
                .cache_companion_thread_snapshot(thread_id, &snapshot)?;
            return Ok(bounded_provider_snapshot(snapshot));
        }
        let provider = match shared.0.companion.resume_thread(&thread.provider_thread_id) {
            Ok(provider) => provider,
            Err(cause) => {
                let segment = shared.0.db.current_companion_provider_segment(thread_id)?;
                shared.0.db.set_companion_provider_segment_status(
                    segment.id,
                    CompanionProviderSegmentStatus::HistoryUnavailable,
                    thread.last_model_id.as_deref(),
                )?;
                return Err(cause);
            }
        };
        shared.0.db.touch_companion_thread(thread_id, None)?;
        let snapshot = shared.0.companion.thread_snapshot(thread, &provider);
        shared
            .0
            .db
            .cache_companion_thread_snapshot(thread_id, &snapshot)?;
        Ok(bounded_provider_snapshot(snapshot))
    })
    .await
}

#[tauri::command]
pub async fn companion_rename_thread(
    state: State<'_, SharedState>,
    repository_id: i64,
    thread_id: i64,
    title: String,
) -> Result<CompanionThread, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        repository_thread(&shared, repository_id, thread_id).map_err(|cause| cause.to_string())?;
        shared
            .0
            .db
            .rename_companion_thread(thread_id, &title)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn companion_set_thread_archived(
    state: State<'_, SharedState>,
    repository_id: i64,
    thread_id: i64,
    archived: bool,
) -> Result<CompanionThread, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        repository_thread(&shared, repository_id, thread_id).map_err(|cause| cause.to_string())?;
        shared
            .0
            .db
            .set_companion_thread_archived(thread_id, archived)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn companion_privacy_consent(
    state: State<'_, SharedState>,
) -> Result<PrivacyConsentStatus, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .companion_privacy_consent()
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn companion_set_privacy_consent(
    state: State<'_, SharedState>,
    accepted: bool,
) -> Result<PrivacyConsentStatus, String> {
    let shared = state.inner().clone();
    crate::runtime::run(move || {
        shared
            .0
            .db
            .set_companion_privacy_consent(accepted)
            .map_err(|cause| cause.to_string())
    })
    .await?
}

#[tauri::command]
pub async fn companion_scan_status(
    state: State<'_, SharedState>,
    repository_id: i64,
) -> Result<crate::core::companion::RepositoryChatScanStatus, String> {
    let service = state.inner().0.companion_context.clone();
    crate::runtime::run(move || service.scan_status(repository_id))
        .await?
        .map_err(|cause| cause.to_string())
}

#[tauri::command]
pub async fn companion_preview_context(
    state: State<'_, SharedState>,
    request: ContextPreviewRequest,
) -> Result<RepositoryChatContext, String> {
    let shared = state.inner().clone();
    run_blocking(move || shared.0.companion_context.preview(&request)).await
}

#[tauri::command]
pub async fn companion_search_mentions(
    state: State<'_, SharedState>,
    repository_id: i64,
    kind: ContextAttachmentKind,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<CompanionMention>, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        shared
            .0
            .companion_context
            .mentions(repository_id, kind, &query, limit.unwrap_or(30))
    })
    .await
}

#[tauri::command]
pub async fn companion_source_detail(
    state: State<'_, SharedState>,
    repository_id: i64,
    source_id: String,
) -> Result<CompanionSourceDetail, String> {
    let shared = state.inner().clone();
    run_blocking(move || {
        shared
            .0
            .companion_context
            .source_detail(repository_id, &source_id)
    })
    .await
}

#[tauri::command]
pub async fn companion_start_turn(
    state: State<'_, SharedState>,
    mut request: CompanionTurnRequest,
) -> Result<CompanionTurnStart, String> {
    let shared = state.inner().clone();
    shared
        .0
        .require_external_ai()
        .map_err(|cause| cause.to_string())?;
    run_blocking(move || {
        let thread = repository_thread(&shared, request.repository_id, request.thread_id)?;
        let connection = shared.0.db.ai_connection(&request.connection_id)?;
        if !connection.enabled {
            return Err(Error::App(format!(
                "The selected AI connection '{}' is unavailable",
                connection.display_name
            )));
        }
        let model_id = request.model.as_deref().unwrap_or_default();
        let upstream = match connection.driver {
            ProviderDriverKind::Codex => Some("openai"),
            ProviderDriverKind::Grok => Some("xai"),
            ProviderDriverKind::OpenCode => model_id.split_once('/').map(|value| value.0),
        };
        let boundary = match connection.driver {
            ProviderDriverKind::Codex => InferenceBoundary::External,
            ProviderDriverKind::Grok => InferenceBoundary::External,
            ProviderDriverKind::OpenCode => upstream
                .map(|value| classify_upstream(value, None))
                .unwrap_or(InferenceBoundary::Unknown),
        };
        let fingerprint = boundary_fingerprint(
            &connection.id,
            connection.driver,
            upstream,
            boundary,
            crate::core::companion::COMPANION_DISCLOSURE_VERSION,
        );
        if !shared
            .0
            .db
            .connection_privacy_consent(
                &request.connection_id,
                &fingerprint,
                crate::core::companion::COMPANION_DISCLOSURE_VERSION,
            )?
            .valid
        {
            return Err(Error::App(
                "Data-sharing consent is required for this AI connection and inference boundary"
                    .into(),
            ));
        }
        let attachments = request
            .context
            .attachments
            .iter()
            .filter(|attachment| attachment.optional)
            .map(|attachment| ContextAttachmentRequest {
                kind: attachment.kind.clone(),
                id: attachment.id.clone(),
            })
            .collect();
        request.context = shared.0.companion_context.preview_for_send(
            &ContextPreviewRequest {
                repository_id: request.repository_id,
                query: request.prompt.clone(),
                attachments,
            },
            &request.context,
        )?;
        if request.context.attachments.iter().any(|attachment| {
            attachment.kind == crate::core::companion::ContextAttachmentKind::Finding
                && attachment.id.starts_with("scan:")
                && !attachment.available
        }) {
            return Err(Error::App(
                "The selected finding evidence is unavailable. Open the finding again or remove it from this message.".into(),
            ));
        }

        let current_segment = shared
            .0
            .db
            .current_companion_provider_segment(thread.id)?;
        let current_segment_turn_started = shared
            .0
            .db
            .companion_thread_has_provider_turn(thread.id)?;
        let needs_new_segment = thread.provider_connection_id != request.connection_id
            || thread.provider != connection.driver.as_str()
            || current_segment.status == CompanionProviderSegmentStatus::HistoryUnavailable;
        let (active_segment, created_new_segment) = if needs_new_segment {
            let segment = match connection.driver {
                ProviderDriverKind::Codex => {
                    if connection.id.as_ref() != DEFAULT_CODEX_CONNECTION_ID {
                        return Err(Error::App("Additional Codex runtimes are not initialized yet; use the built-in Codex connection".into()));
                    }
                    let response = shared
                        .0
                        .companion
                        .start_thread(request.repository_id, request.model.as_deref())?;
                    let provider_thread_id = provider_thread_id(&response)?;
                    shared.0.db.create_companion_provider_segment(
                        thread.id,
                        connection.id.as_ref(),
                        "codex",
                        &provider_thread_id,
                        request.model.as_deref(),
                    )?
                }
                ProviderDriverKind::Grok => shared.0.grok.create_provider_segment(
                    thread.id,
                    request.repository_id,
                    connection.id.as_ref(),
                    request.model.as_deref(),
                )?,
                ProviderDriverKind::OpenCode => shared.0.open_code.create_provider_segment(
                    thread.id,
                    request.repository_id,
                    connection.id.as_ref(),
                    request.model.as_deref(),
                )?,
            };
            let model_label = request.model.as_deref().unwrap_or("default model");
            shared.0.db.upsert_companion_timeline_item(
                thread.id,
                segment.id,
                &CompanionTimelineItem {
                    id: format!("provider-transition:{}", segment.id),
                    kind: CompanionTimelineItemKind::Activity,
                    status: CompanionTimelineItemStatus::Completed,
                    text: String::new(),
                    activity: Some(CompanionActivity {
                        kind: CompanionActivityKind::ProviderTransition,
                        label: format!("Continued with {}", connection.display_name),
                        detail: Some(model_label.into()),
                        source_ids: Vec::new(),
                    }),
                    citations: Vec::new(),
                    error: None,
                    model_id: None,
                    created_at: timestamps::now(),
                },
            )?;
            (segment, true)
        } else {
            (current_segment.clone(), false)
        };

        let optimistic_item = CompanionTimelineItem {
            id: format!("local:{}", request.client_turn_id),
            kind: CompanionTimelineItemKind::UserMessage,
            status: CompanionTimelineItemStatus::Completed,
            text: request.prompt.trim().to_string(),
            activity: None,
            citations: Vec::new(),
            error: None,
            model_id: request.model.clone(),
            created_at: timestamps::now(),
        };
        shared.0.db.upsert_companion_timeline_item(
            thread.id,
            active_segment.id,
            &optimistic_item,
        )?;
        let requested_model = request.model.clone();
        let started = match connection.driver {
            ProviderDriverKind::Codex => shared.0.companion.start_turn(request),
            ProviderDriverKind::Grok => shared.0.grok.start_turn(connection, request),
            ProviderDriverKind::OpenCode => {
                let secret = if connection.has_server_password {
                    shared
                        .0
                        .provider_secrets
                        .server_password(connection.id.as_ref())?
                } else {
                    None
                };
                shared.0.open_code.start_turn(connection, request, secret)
            }
        };
        if let Err(cause) = &started {
            shared.0.db.delete_companion_timeline_item(
                thread.id,
                active_segment.id,
                &optimistic_item.id,
            )?;
            if created_new_segment {
                shared.0.db.rollback_companion_provider_segment(
                    thread.id,
                    active_segment.id,
                    &current_segment,
                    current_segment_turn_started,
                )?;
            } else if matches!(cause, Error::ProviderHistoryUnavailable(_)) {
                shared.0.db.set_companion_provider_segment_status(
                    active_segment.id,
                    CompanionProviderSegmentStatus::HistoryUnavailable,
                    requested_model.as_deref(),
                )?;
            }
        }
        started
    })
    .await
}

#[tauri::command]
pub async fn companion_interrupt_turn(
    state: State<'_, SharedState>,
    repository_id: i64,
    thread_id: i64,
    turn_id: String,
) -> Result<(), String> {
    let shared = state.inner().clone();
    shared
        .0
        .require_external_ai()
        .map_err(|cause| cause.to_string())?;
    run_blocking(move || {
        let thread = repository_thread(&shared, repository_id, thread_id)?;
        if thread.provider == "grok" {
            shared.0.grok.interrupt_turn(thread.id)
        } else if thread.provider == "opencode" {
            shared.0.open_code.interrupt_turn(thread.id)
        } else {
            shared
                .0
                .companion
                .interrupt_turn(&thread.provider_thread_id, &turn_id)
        }
    })
    .await
}

fn repository_thread(
    shared: &SharedState,
    repository_id: i64,
    thread_id: i64,
) -> Result<CompanionThread, Error> {
    let thread = shared.0.db.companion_thread(thread_id)?;
    if thread.repository_id != repository_id {
        return Err(Error::App(
            "companion thread belongs to a different repository".into(),
        ));
    }
    Ok(thread)
}

fn provider_thread_id(value: &Value) -> Result<String, Error> {
    value
        .pointer("/thread/id")
        .or_else(|| value.pointer("/result/thread/id"))
        .or_else(|| value.get("threadId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 512)
        .map(str::to_string)
        .ok_or_else(|| Error::App("Codex returned a malformed thread identity".into()))
}

fn deterministic_thread_title(value: Option<&str>) -> String {
    let value = value.unwrap_or_default().trim();
    let title = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(64)
        .collect::<String>();
    if title.is_empty() {
        "New security chat".into()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_thread_identity_is_normalized_without_returning_protocol_data() {
        assert_eq!(
            provider_thread_id(&serde_json::json!({
                "thread": { "id": "provider-1", "raw": "ignored" }
            }))
            .expect("thread id"),
            "provider-1"
        );
        assert!(provider_thread_id(&serde_json::json!({ "thread": {} })).is_err());
    }

    #[test]
    fn initial_title_is_stable_and_bounded() {
        assert_eq!(deterministic_thread_title(None), "New security chat");
        assert_eq!(
            deterministic_thread_title(Some("  explain   CVE-2025-1  ")),
            "explain CVE-2025-1"
        );
        assert!(deterministic_thread_title(Some(&"x".repeat(100))).len() <= 64);
    }
}
