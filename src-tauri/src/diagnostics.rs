//! Opt-in local numeric diagnostics. No repository paths, message text or
//! credentials are collected, and ordinary sessions do not run the sampler.
use crate::core::timestamps;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant};
use tauri::AppHandle;
pub(crate) mod capture;

static APPLICATION_STARTED: OnceLock<Instant> = OnceLock::new();
pub(crate) fn application_started() {
    APPLICATION_STARTED.get_or_init(Instant::now);
}

#[tauri::command]
pub(crate) fn get_build_identity(app: AppHandle) -> serde_json::Value {
    serde_json::json!({"bundleIdentifier":app.config().identifier,"commit":env!("CVEC_BUILD_COMMIT"),"dirty":env!("CVEC_BUILD_DIRTY"),"profile":env!("CVEC_BUILD_PROFILE"),"cargoLockSha256":env!("CVEC_BUILD_CARGO_LOCK"),"npmLockSha256":env!("CVEC_BUILD_NPM_LOCK"),"sourceSha256":env!("CVEC_BUILD_SOURCE_HASH"),"trace":std::env::var_os("CVEC_PERF_TRACE").is_some(),"startupAutoExit":false,"diagnosticForceFocus":false})
}
#[tauri::command]
pub(crate) fn record_ui_timings(sample: serde_json::Value) {
    if std::env::var_os("CVEC_PERF_TRACE").is_some() {
        let clean = sanitize_sample(&sample);
        eprintln!("cvec-perf webview {clean}");
        if clean.get("firstAppPaintMs").is_some() {
            if let Some(started) = APPLICATION_STARTED.get() {
                eprintln!(
                    "cvec-perf startup application_entry_to_shell_ms={:.3}",
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
        // Diagnostics only observe. In particular, inherited startup benchmark
        // environment variables must never exit, restart or focus this app.
    }
}

fn sanitize_sample(sample: &serde_json::Value) -> serde_json::Value {
    // Only known bounded numeric/boolean diagnostic keys are accepted.
    let mut clean = serde_json::Map::new();
    for key in [
        "completedItemCommits",
        "streamingItemCommits",
        "railCommits",
        "reactActualDurationMs",
        "reactMaxRenderDurationMs",
        "reactProfiling",
        "uiAction",
        "sessionTimeOriginMs",
        "uiTransitionId",
        "uiRoute",
        "uiStartedElapsedMs",
        "uiSettledElapsedMs",
        "uiReadyState",
        "workspaceReadyMs",
        "presentationReadyMs",
        "inspectorReadyMs",
        "inspectorDetailsReadyMs",
        "uiTransitionFailed",
        "uiTransitionTimeout",
        "ipcOperation",
        "ipcMs",
        "validationMs",
        "completedItemRenders",
        "cachedHistoryReads",
        "cachedHistoryHydrations",
        "markdownParses",
        "markdownCacheHits",
        "markdownParsedCharacters",
        "markdownParseDurationMs",
        "streamingItemRenders",
        "railRenders",
        "composerRenders",
        "timelineRenders",
        "timelineProjections",
        "modelPickerProjections",
        "composerCommits",
        "timelineCommits",
        "composerDurationMs",
        "timelineDurationMs",
        "historyCacheEntries",
        "historyCacheBytes",
        "deltaBufferRoutes",
        "deltaCadenceRoutes",
        "historyInFlightRequests",
        "firstAppPaintMs",
        "visible",
        "elapsedMs",
        "frames",
        "frameP50Ms",
        "frameP95Ms",
        "frameP99Ms",
        "frameMaxMs",
        "longTaskSupported",
        "longTasks",
        "longTaskMaxMs",
        "interactionCount",
        "interactionP95Ms",
        "interactionMaxMs",
        "reviewRows",
        "dependencyRows",
        "messageNodes",
        "domNodes",
        "heapBytes",
        "streamToPaintMs",
    ] {
        if let Some(value) = sample.get(key).filter(|value| {
            let numeric = value
                .as_f64()
                .is_some_and(|number| number.is_finite() && number >= 0.0);
            match key {
                "uiTransitionId" => value
                    .as_u64()
                    .is_some_and(|id| (1..=9_007_199_254_740_991).contains(&id)),
                "uiRoute" => value.as_u64().is_some_and(|id| (1..=4).contains(&id)),
                "uiReadyState" => value.as_u64().is_some_and(|id| (1..=4).contains(&id)),
                "sessionTimeOriginMs" | "uiStartedElapsedMs" | "uiSettledElapsedMs" => numeric,
                _ => value.is_boolean() || numeric,
            }
        }) {
            clean.insert(key.into(), value.clone());
        }
    }
    serde_json::Value::Object(clean)
}
pub(crate) fn start(app: &AppHandle) {
    if std::env::var_os("CVEC_PERF_TRACE").is_none() {
        return;
    }
    eprintln!("cvec-perf build {}", get_build_identity(app.clone()));
    let app = app.clone();
    let pending = Arc::new(AtomicBool::new(false));
    let samples = Arc::new(Mutex::new(Vec::with_capacity(120)));
    let started = Instant::now();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(16)).await;
            if pending.swap(true, Ordering::Relaxed) {
                continue;
            }
            let queued = Instant::now();
            let pending = pending.clone();
            let samples = samples.clone();
            let _=app.run_on_main_thread(move||{
                let ms=queued.elapsed().as_secs_f64()*1000.0;
                if let Ok(mut samples)=samples.lock(){samples.push(ms);if samples.len()>=120{samples.sort_by(f64::total_cmp);eprintln!("cvec-perf appkit elapsed_ms={:.0} samples={} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} max_ms={:.3}",started.elapsed().as_secs_f64()*1000.0,samples.len(),samples[59],samples[113],samples[118],samples[119]);samples.clear();}}
                pending.store(false,Ordering::Relaxed);
            });
        }
    });
}

/// Replay through the production persistence/IPC path in the separately named
/// QA app only. Providers are disabled; no prompt or login reaches a provider.
pub(crate) fn replay(
    app: &AppHandle,
    shared: &crate::state::SharedState,
) -> Result<(), crate::Error> {
    use crate::core::companion::*;
    if app.config().identifier != "dev.cvec.performance-qa"
        || std::env::var("CVEC_PERF_REPLAY").as_deref() != Ok("1")
    {
        return Ok(());
    }
    let Some(repo) = shared.0.db.list_active_repositories()?.first().cloned() else {
        return Ok(());
    };
    let provider_id = format!("local-performance-replay-{}", timestamps::unix_millis());
    shared
        .0
        .db
        .set_ai_connection_enabled("codex_default", true)?;
    let thread = shared.0.db.create_companion_thread(
        repo.id,
        &provider_id,
        "Performance replay · local only",
        None,
    )?;
    for connection in shared.0.db.list_ai_connections()? {
        shared
            .0
            .db
            .set_ai_connection_enabled(connection.id.as_ref(), false)?;
    }
    let item = |id: String, text: String, status| CompanionTimelineItem {
        id,
        kind: CompanionTimelineItemKind::AssistantMessage,
        status,
        text,
        activity: None,
        citations: Vec::new(),
        error: None,
        model_id: None,
        created_at: timestamps::now(),
    };
    let snapshot = CompanionThreadSnapshot {
        thread: thread.clone(),
        items: (0..2000)
            .map(|index| {
                item(
                    format!("history-{index}"),
                    format!(
                        "Historical replay message {index}. {}",
                        "Bounded local history fixture. ".repeat(12)
                    ),
                    CompanionTimelineItemStatus::Completed,
                )
            })
            .collect(),
        usage: None,
    };
    shared
        .0
        .db
        .cache_companion_thread_snapshot(thread.id, &snapshot)?;
    let sink = crate::core::event_writer::sink(app.clone(), shared.0.db.clone())?;
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(std::time::Duration::from_secs(20));
        let mut sequence = 1;
        let event = |sequence, kind, payload| CompanionEventEnvelope {
            cache_epoch: None,
            provider_segment_id: None,
            schema_version: COMPANION_SCHEMA_VERSION,
            repository_id: Some(repo.id),
            thread_id: Some(thread.id),
            provider_thread_id: Some(provider_id.clone()),
            connection_id: Some("codex_default".into()),
            driver: Some("codex".into()),
            model_id: None,
            turn_id: Some("local-performance-turn".into()),
            sequence,
            kind,
            payload,
        };
        sink(event(
            sequence,
            CompanionEventKind::ItemStarted,
            serde_json::to_value(item(
                "live-answer".into(),
                String::new(),
                CompanionTimelineItemStatus::Streaming,
            ))
            .unwrap(),
        ));
        let mut text = String::new();
        for index in 0..6000 {
            sequence += 1;
            let delta = format!("{index} ");
            text.push_str(&delta);
            sink(event(
                sequence,
                CompanionEventKind::ItemDelta,
                serde_json::json!({"itemId":"live-answer","text":delta,"perfReceivedAt":timestamps::unix_millis()}),
            ));
            if index % 10 == 9 {
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        }
        sequence += 1;
        sink(event(
            sequence,
            CompanionEventKind::ItemCompleted,
            serde_json::to_value(item(
                "live-answer".into(),
                text,
                CompanionTimelineItemStatus::Completed,
            ))
            .unwrap(),
        ));
        sequence += 1;
        sink(event(
            sequence,
            CompanionEventKind::TurnCompleted,
            serde_json::json!({"status":"completed"}),
        ));
        eprintln!("cvec-perf replay completed events={sequence}");
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diagnostic_payload_keeps_numeric_metrics_without_identity_or_text() {
        let clean = sanitize_sample(&json!({
            "firstAppPaintMs": 0,
            "inspectorDetailsReadyMs": 32.5,
            "historyInFlightRequests": 0,
            "visible": true,
            "repositoryPath": "/private/repository",
            "message": "private chat text",
            "uiAction": "private identity"
        }));
        assert_eq!(
            clean,
            json!({
                "firstAppPaintMs": 0, "inspectorDetailsReadyMs": 32.5,
                "historyInFlightRequests": 0, "visible": true
            })
        );
    }

    #[test]
    fn diagnostic_payload_rejects_invalid_measurements() {
        let clean = sanitize_sample(&json!({
            "ipcMs": -1, "validationMs": null,
            "heapBytes": {"contents": "private"}, "frameP95Ms": [17],
            "uiTransitionFailed": true, "longTaskSupported": false
        }));
        assert_eq!(
            clean,
            json!({"uiTransitionFailed": true, "longTaskSupported": false})
        );
        assert_eq!(sanitize_sample(&json!("unexpected envelope")), json!({}));
    }

    #[test]
    fn transition_correlation_keeps_only_numeric_clock_and_enum_fields() {
        let sample = json!({
            "sessionTimeOriginMs": 1_792_080_000_123.5,
            "uiTransitionId": 7, "uiRoute": 3, "uiReadyState": 3,
            "uiStartedElapsedMs": 120.25, "uiSettledElapsedMs": 164.5
        });
        assert_eq!(sanitize_sample(&sample), sample);
        for invalid in [json!(true), json!("private"), json!(-1), json!(null)] {
            assert_eq!(
                sanitize_sample(&json!({
                    "sessionTimeOriginMs": invalid, "uiTransitionId": invalid,
                    "uiRoute": invalid, "uiReadyState": invalid,
                    "uiStartedElapsedMs": invalid, "uiSettledElapsedMs": invalid
                })),
                json!({})
            );
        }
        assert_eq!(
            sanitize_sample(&json!({
                "uiTransitionId": 0, "uiRoute": 5, "uiReadyState": 0
            })),
            json!({})
        );
        assert_eq!(
            sanitize_sample(&json!({
                "uiTransitionId": 1.5, "uiRoute": 1.5, "uiReadyState": 1.5
            })),
            json!({})
        );
    }
}
