mod commands;
mod companion_commands;
pub mod core;
mod desktop_window;
mod diagnostics;
mod fix_workspace_commands;
mod provider_connection_commands;
mod runtime;
mod state;

use crate::core::models::{AppSnapshot, ScheduleInterval, TrayStatus};
use crate::state::{
    SharedState, ISOLATED_UI_IDENTIFIER, PERFORMANCE_QA_IDENTIFIER, QA_DATA_DIR_ENV,
};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, WindowEvent,
};
use tauri_plugin_window_state::AppHandleExt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    App(String),
    #[error("{0}")]
    ProviderHistoryUnavailable(String),
    #[error("{0}")]
    Scanner(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

static EXIT_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    diagnostics::application_started();
    let context = tauri::generate_context!();
    let window_state_filename = desktop_window::state_filename(
        &context.config().identifier,
        std::env::var_os(QA_DATA_DIR_ENV).as_deref(),
    );
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(desktop_window::state_flags())
                .with_filename(window_state_filename)
                .skip_initial_state("main")
                .build(),
        )
        .setup(|app| {
            let shared = SharedState::new(app.handle())?;
            app.manage(shared.clone());
            if let Some(zoom) = qa_zoom_scale(app.handle()) {
                app.get_webview_window("main")
                    .ok_or_else(|| Error::App("main QA webview is unavailable".into()))?
                    .set_zoom(zoom)
                    .map_err(|cause| Error::App(format!("failed to set QA zoom: {cause}")))?;
            }
            shared.reschedule(app.handle().clone());
            create_tray(app.handle())?;
            desktop_window::restore(app.handle())?;
            if app.config().identifier == PERFORMANCE_QA_IDENTIFIER {
                if let Some(window) = app.get_webview_window("main") {
                    let profile = std::env::var(QA_DATA_DIR_ENV)
                        .ok()
                        .and_then(|value| {
                            std::path::Path::new(&value)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                        })
                        .unwrap_or_else(|| "empty".into());
                    let _ = window.set_title(&format!("CVEC Performance QA · {profile}"));
                }
            }
            diagnostics::start(app.handle());
            #[cfg(feature = "performance-profiler")]
            if app.config().identifier == PERFORMANCE_QA_IDENTIFIER
                && std::env::var("CVEC_PERF_INSPECT").as_deref() == Ok("1")
            {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }
            if let Err(cause) = diagnostics::replay(app.handle(), &shared) {
                eprintln!("Local QA replay setup failed: {cause}");
            }
            let tray_app = app.handle().clone();
            let tray_state = shared.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::runtime::run(move || refresh_tray(&tray_app, &tray_state)).await;
            });
            // Snoozes can expire even with scanning disabled and the window hidden.
            let badge_app = app.handle().clone();
            let badge_state = shared.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    let app = badge_app.clone();
                    let shared = badge_state.clone();
                    let _ = crate::runtime::run(move || {
                        if shared.0.db.has_due_snoozes().unwrap_or(false) {
                            if let Ok(snapshot) = commands::refresh_snapshot(&shared) {
                                set_review_badge(&app, snapshot.review_summary.needs_review);
                                let _ = app.emit("cvec://state", snapshot);
                            }
                        }
                    })
                    .await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            fix_workspace_commands::fix_workspace_list,
            fix_workspace_commands::fix_workspace_archived,
            fix_workspace_commands::fix_workspace_set_archived,
            fix_workspace_commands::fix_workspace_create,
            fix_workspace_commands::fix_workspace_open_directory,
            fix_workspace_commands::fix_workspace_open_checkout,
            fix_workspace_commands::fix_workspace_snapshot,
            fix_workspace_commands::fix_workspace_start,
            fix_workspace_commands::fix_workspace_stop,
            fix_workspace_commands::fix_workspace_check,
            fix_workspace_commands::fix_workspace_rescan,
            diagnostics::get_build_identity,
            diagnostics::record_ui_timings,
            diagnostics::capture::get_qa_capture_geometry,
            diagnostics::capture::record_qa_capture_viewport,
            commands::bootstrap,
            commands::get_snapshot,
            commands::get_repo_overview,
            commands::get_repo_scan_comparison,
            commands::list_repository_assets,
            commands::list_artifact_scans,
            commands::get_artifact_findings,
            commands::scan_artifact,
            commands::list_repository_dependencies,
            commands::list_risk_trend,
            commands::list_review_findings,
            commands::get_finding_remediation,
            commands::get_review_finding_detail,
            commands::open_finding_manifest,
            commands::mark_finding_reviewed,
            commands::snooze_finding,
            commands::return_finding_to_review,
            commands::search_local_entities,
            commands::get_scan_activity,
            commands::list_external_intel_feed,
            commands::get_external_intel_detail,
            commands::open_external_url,
            commands::clear_notification,
            commands::clear_all_notifications,
            commands::complete_onboarding,
            commands::skip_onboarding,
            commands::set_external_intel_topics,
            commands::refresh_external_intel,
            commands::start_github_login,
            commands::configure_github_app,
            commands::load_saved_github_auth,
            commands::poll_github_login,
            commands::logout_github,
            commands::list_github_repositories,
            commands::scan_all,
            commands::scan_repo,
            commands::add_github_repository,
            commands::remove_remote_repository,
            commands::set_schedule,
            commands::choose_discovery_root,
            commands::refresh_local_repositories,
            commands::check_vulnerability_db,
            commands::update_vulnerability_db,
            companion_commands::companion_read_draft,
            companion_commands::companion_write_draft,
            companion_commands::companion_runtime_status,
            companion_commands::companion_start_runtime,
            companion_commands::companion_stop_runtime,
            companion_commands::companion_account_status,
            companion_commands::companion_login_chatgpt,
            companion_commands::companion_login_api_key,
            companion_commands::companion_cancel_login,
            companion_commands::companion_logout,
            companion_commands::companion_list_models,
            companion_commands::companion_usage,
            companion_commands::companion_list_threads,
            companion_commands::companion_conversation_index,
            companion_commands::companion_cached_thread,
            companion_commands::companion_history_page,
            companion_commands::companion_create_thread,
            companion_commands::companion_resume_thread,
            companion_commands::companion_rename_thread,
            companion_commands::companion_set_thread_archived,
            companion_commands::companion_privacy_consent,
            companion_commands::companion_set_privacy_consent,
            companion_commands::companion_preview_context,
            companion_commands::companion_scan_status,
            companion_commands::companion_search_mentions,
            companion_commands::companion_source_detail,
            companion_commands::companion_start_turn,
            companion_commands::companion_interrupt_turn,
            provider_connection_commands::provider_connection_list,
            provider_connection_commands::provider_connection_get,
            provider_connection_commands::provider_connection_validate,
            provider_connection_commands::provider_connection_save,
            provider_connection_commands::provider_connection_refresh,
            provider_connection_commands::provider_connection_set_enabled,
            provider_connection_commands::provider_connection_reset,
            provider_connection_commands::provider_connection_remove,
            provider_connection_commands::provider_connection_consent,
            provider_connection_commands::provider_connection_set_consent
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) = window
                    .app_handle()
                    .save_window_state(desktop_window::state_flags())
                {
                    eprintln!("Could not save window geometry: {error}");
                }
                let _ = window.hide();
            }
        })
        .build(context)
        .expect("error while building cvec")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                if !EXIT_STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    api.prevent_exit();
                    let app = app.clone();
                    let shared = app.state::<SharedState>().inner().clone();
                    tauri::async_runtime::spawn(async move {
                        shared.0.fix_workspaces.shutdown();
                        if let Err(cause) = core::process::shutdown().await {
                            eprintln!("Scanner process cleanup failed: {cause}");
                        }
                        let _ = crate::runtime::run_io(move || {
                            shared.0.companion.shutdown();
                            shared.0.grok.shutdown();
                            shared.0.open_code.shutdown();
                            if let Err(cause) =
                                core::event_writer::flush_database(shared.0.db.path())
                            {
                                eprintln!("Conversation history flush failed: {cause}");
                            }
                        })
                        .await;
                        app.exit(0);
                    });
                }
            }
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                focus_main_window(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

fn qa_zoom_scale(app: &AppHandle) -> Option<f64> {
    let identifier = app.config().identifier.as_str();
    if ((cfg!(debug_assertions) && identifier == ISOLATED_UI_IDENTIFIER)
        || identifier == PERFORMANCE_QA_IDENTIFIER)
        && std::env::var("CVEC_UI_QA_ZOOM").as_deref() == Ok("2")
    {
        Some(2.0)
    } else {
        None
    }
}

// Badge errors must never turn a successful review or scan into a failed action.
pub(crate) fn set_review_badge(app: &AppHandle, needs_review: u32) {
    #[cfg(target_os = "macos")]
    if let Some(window) = app.get_webview_window("main") {
        let count = (needs_review > 0).then_some(i64::from(needs_review));
        if let Err(cause) = window.set_badge_count(count) {
            eprintln!("Failed to update review Dock badge: {cause}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, needs_review);
}

pub(crate) fn refresh_dock_badge(app: &AppHandle, shared: &SharedState) {
    match shared.0.db.review_summary() {
        Ok(summary) => set_review_badge(app, summary.needs_review),
        Err(cause) => eprintln!("Failed to refresh review Dock badge: {cause}"),
    }
}

pub fn refresh_tray(app: &AppHandle, shared: &SharedState) -> Result<(), Error> {
    refresh_dock_badge(app, shared);
    update_tray(app, shared.0.db.overall_tray_status()?)
}

pub(crate) fn refresh_tray_from_snapshot(
    app: &AppHandle,
    snapshot: &AppSnapshot,
) -> Result<(), Error> {
    set_review_badge(app, snapshot.review_summary.needs_review);
    update_tray(app, TrayStatus::from_repositories(&snapshot.repos))
}

fn update_tray(app: &AppHandle, state: TrayStatus) -> Result<(), Error> {
    let tray = app
        .tray_by_id("cvec-tray")
        .ok_or_else(|| Error::App("tray not initialized".into()))?;

    let tooltip = match state {
        TrayStatus::Clean => "cvec: no HIGH / CRITICAL issues",
        TrayStatus::Warning => "cvec: open HIGH / CRITICAL issues",
        TrayStatus::Red => "cvec: new CRITICAL issue detected",
    };

    tray.set_tooltip(Some(tooltip))
        .map_err(|cause| Error::App(format!("failed to update tray tooltip: {cause}")))?;
    tray.set_icon(Some(tray_icon(state)))
        .map_err(|cause| Error::App(format!("failed to update tray icon: {cause}")))?;
    Ok(())
}

fn create_tray(app: &AppHandle) -> Result<(), Error> {
    let dashboard = MenuItem::with_id(app, "dashboard", "Dashboard", true, None::<&str>)
        .map_err(|cause| Error::App(format!("failed to create dashboard item: {cause}")))?;
    let scan_now = MenuItem::with_id(app, "scan-now", "Scan now", true, None::<&str>)
        .map_err(|cause| Error::App(format!("failed to create scan menu item: {cause}")))?;
    let schedule_off = MenuItem::with_id(
        app,
        "schedule-off",
        "Scheduled scan: Off",
        true,
        None::<&str>,
    )
    .map_err(|cause| Error::App(format!("failed to create schedule off item: {cause}")))?;
    let schedule_twelve = MenuItem::with_id(
        app,
        "schedule-12h",
        "Scheduled scan: Every 12h",
        true,
        None::<&str>,
    )
    .map_err(|cause| Error::App(format!("failed to create schedule item: {cause}")))?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)
        .map_err(|cause| Error::App(format!("failed to create quit item: {cause}")))?;

    let menu = Menu::with_items(
        app,
        &[
            &dashboard,
            &scan_now,
            &schedule_off,
            &schedule_twelve,
            &quit,
        ],
    )
    .map_err(|cause| Error::App(format!("failed to create tray menu: {cause}")))?;

    TrayIconBuilder::with_id("cvec-tray")
        .icon(tray_icon(TrayStatus::Clean))
        .icon_as_template(false)
        .tooltip("cvec")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            if let Some(shared) = app.try_state::<SharedState>() {
                match event.id.as_ref() {
                    "dashboard" => {
                        focus_main_window(app);
                    }
                    "scan-now" => {
                        let app = app.clone();
                        let shared = shared.inner().clone();
                        tauri::async_runtime::spawn(async move {
                            match crate::commands::scan_all_internal(app.clone(), shared.clone())
                                .await
                            {
                                Ok(_) => {
                                    shared.reschedule(app);
                                }
                                Err(cause) => crate::commands::emit_background_error(
                                    &app,
                                    format!("Tray scan failed: {cause}"),
                                ),
                            }
                        });
                    }
                    "schedule-off" => match shared.set_schedule(ScheduleInterval::Off) {
                        Ok(()) => shared.reschedule(app.clone()),
                        Err(cause) => crate::commands::emit_background_error(
                            app,
                            format!("Failed to disable the scan schedule: {cause}"),
                        ),
                    },
                    "schedule-12h" => match shared.set_schedule(ScheduleInterval::Every12Hours) {
                        Ok(()) => shared.reschedule(app.clone()),
                        Err(cause) => crate::commands::emit_background_error(
                            app,
                            format!("Failed to update the scan schedule: {cause}"),
                        ),
                    },
                    "quit" => app.exit(0),
                    _ => {}
                }
            }
        })
        .build(app)
        .map_err(|cause| Error::App(format!("failed to build tray: {cause}")))?;

    Ok(())
}

fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn tray_icon(status: TrayStatus) -> Image<'static> {
    const SIZE: u32 = 32;
    const CENTER: f32 = (SIZE as f32 - 1.0) / 2.0;
    const RADIUS: f32 = 12.4;
    let [red, green, blue] = match status {
        TrayStatus::Clean => [34, 197, 94],
        TrayStatus::Warning => [229, 174, 46],
        TrayStatus::Red => [255, 77, 79],
    };
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        let dy = y as f32 - CENTER;
        for x in 0..SIZE {
            let dx = x as f32 - CENTER;
            let coverage = (RADIUS + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
            rgba.extend_from_slice(&[red, green, blue, (coverage * 255.0).round() as u8]);
        }
    }
    Image::new_owned(rgba, SIZE, SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icons_preserve_status_colors_and_transparent_edges() {
        for (status, color) in [
            (TrayStatus::Clean, [34, 197, 94, 255]),
            (TrayStatus::Warning, [229, 174, 46, 255]),
            (TrayStatus::Red, [255, 77, 79, 255]),
        ] {
            let icon = tray_icon(status);
            assert_eq!((icon.width(), icon.height()), (32, 32));
            let pixel = |x: usize, y: usize| {
                let offset = (y * 32 + x) * 4;
                &icon.rgba()[offset..offset + 4]
            };
            assert_eq!(pixel(16, 16), color);
            assert_eq!(pixel(0, 0)[3], 0);
            assert!((1..255).contains(&pixel(3, 16)[3]));
        }
    }
}
