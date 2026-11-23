//! Read-only geometry for opted-in QA sessions. No window mutation or capture.
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Monitor, WebviewWindow};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Display {
    bounds_physical: Rect,
    work_area_physical: Rect,
    scale_factor: f64,
    // Tauri Monitor does not expose an OS display identifier. Never infer one
    // from image width, monitor name, or the ordering of available_monitors.
    native_display_id: Option<u32>,
}

fn display(monitor: &Monitor) -> Display {
    let area = monitor.work_area();
    Display {
        bounds_physical: Rect {
            x: monitor.position().x,
            y: monitor.position().y,
            width: monitor.size().width,
            height: monitor.size().height,
        },
        work_area_physical: Rect {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        },
        scale_factor: monitor.scale_factor(),
        native_display_id: None,
    }
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn allowed(identifier: &str, trace: bool, label: &str, sequence: u32) -> bool {
    identifier == "dev.cvec.performance-qa" && trace && label == "main" && sequence > 0
}

fn require_qa(app: &AppHandle, label: &str, sequence: u32) -> Result<(), String> {
    if !allowed(
        &app.config().identifier,
        std::env::var_os("CVEC_PERF_TRACE").is_some(),
        label,
        sequence,
    ) {
        return Err("Capture metadata requires an opted-in main QA window".into());
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeGeometry {
    schema_version: u8,
    sequence: u32,
    pid: u32,
    started_epoch_ms: u64,
    ended_epoch_ms: u64,
    // Native OS window ID is unavailable through these portable Tauri getters.
    native_window_id: Option<u32>,
    outer_physical: Option<Rect>,
    inner_physical: Option<Rect>,
    scale_factor: Option<f64>,
    focused: Option<bool>,
    visible: Option<bool>,
    minimized: Option<bool>,
    maximized: Option<bool>,
    fullscreen: Option<bool>,
    current_display: Option<Display>,
    displays: Option<Vec<Display>>,
    displays_truncated: bool,
}

#[tauri::command]
pub(crate) fn get_qa_capture_geometry(
    app: AppHandle,
    window: WebviewWindow,
    sequence: u32,
) -> Result<NativeGeometry, String> {
    require_qa(&app, window.label(), sequence)?;
    let started_epoch_ms = epoch_ms();
    let outer_physical = window
        .outer_position()
        .ok()
        .zip(window.outer_size().ok())
        .map(|(position, size)| Rect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        });
    let inner_physical = window
        .inner_position()
        .ok()
        .zip(window.inner_size().ok())
        .map(|(position, size)| Rect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        });
    let displays = window.available_monitors().ok();
    let displays_truncated = displays.as_ref().is_some_and(|items| items.len() > 32);
    let sample = NativeGeometry {
        schema_version: 1,
        sequence,
        pid: std::process::id(),
        started_epoch_ms,
        native_window_id: None,
        outer_physical,
        inner_physical,
        scale_factor: window
            .scale_factor()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0),
        focused: window.is_focused().ok(),
        visible: window.is_visible().ok(),
        minimized: window.is_minimized().ok(),
        maximized: window.is_maximized().ok(),
        fullscreen: window.is_fullscreen().ok(),
        current_display: window
            .current_monitor()
            .ok()
            .flatten()
            .as_ref()
            .map(display),
        displays: displays.map(|items| items.iter().take(32).map(display).collect()),
        displays_truncated,
        ended_epoch_ms: epoch_ms(),
    };
    eprintln!(
        "cvec-perf geometry-native {}",
        serde_json::to_string(&sample).map_err(|_| "Cannot serialize geometry")?
    );
    Ok(sample)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Viewport {
    epoch_ms: f64,
    inner_width_css: f64,
    inner_height_css: f64,
    outer_width_css: f64,
    outer_height_css: f64,
    screen_x_css: f64,
    screen_y_css: f64,
    device_pixel_ratio: f64,
    visual: Option<VisualViewport>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VisualViewport {
    width_css: f64,
    height_css: f64,
    offset_left_css: f64,
    offset_top_css: f64,
    page_left_css: f64,
    page_top_css: f64,
    scale: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ViewportRecord {
    sequence: u32,
    before: Viewport,
    after: Viewport,
    native_available: bool,
}

fn within(value: f64, min: f64, max: f64) -> bool {
    value.is_finite() && (min..=max).contains(&value)
}

impl Viewport {
    fn valid(&self) -> bool {
        within(self.epoch_ms, 1.0, 9_007_199_254_740_991.0)
            && [
                self.inner_width_css,
                self.inner_height_css,
                self.outer_width_css,
                self.outer_height_css,
            ]
            .iter()
            .all(|value| within(*value, 0.0, 1_000_000.0))
            && [self.screen_x_css, self.screen_y_css]
                .iter()
                .all(|value| within(*value, -1_000_000.0, 1_000_000.0))
            && within(self.device_pixel_ratio, 0.01, 32.0)
            && self.visual.as_ref().is_none_or(|v| {
                [v.width_css, v.height_css]
                    .iter()
                    .all(|value| within(*value, 0.0, 1_000_000.0))
                    && [
                        v.offset_left_css,
                        v.offset_top_css,
                        v.page_left_css,
                        v.page_top_css,
                    ]
                    .iter()
                    .all(|value| within(*value, -1_000_000.0, 1_000_000.0))
                    && within(v.scale, 0.01, 32.0)
            })
    }
}

#[tauri::command]
pub(crate) fn record_qa_capture_viewport(
    app: AppHandle,
    window: WebviewWindow,
    sample: ViewportRecord,
) -> Result<(), String> {
    require_qa(&app, window.label(), sample.sequence)?;
    if !sample.before.valid()
        || !sample.after.valid()
        || sample.after.epoch_ms < sample.before.epoch_ms
    {
        return Err("Invalid viewport metadata".into());
    }
    eprintln!(
        "cvec-perf geometry-viewport {}",
        serde_json::to_string(&sample).map_err(|_| "Cannot serialize viewport")?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> serde_json::Value {
        json!({"epochMs":1792093250851_f64,"innerWidthCss":1900,"innerHeightCss":949,
            "outerWidthCss":1900,"outerHeightCss":949,"screenXCss":-1900,"screenYCss":25,
            "devicePixelRatio":2,"visual":{"widthCss":1900,"heightCss":949,"offsetLeftCss":0,
            "offsetTopCss":0,"pageLeftCss":0,"pageTopCss":0,"scale":1}})
    }

    #[test]
    fn restricts_geometry_to_opted_in_qa_main_window() {
        assert!(allowed("dev.cvec.performance-qa", true, "main", 1));
        assert!(!allowed("dev.cvec.app", true, "main", 1));
        assert!(!allowed("dev.cvec.performance-qa", false, "main", 1));
        assert!(!allowed("dev.cvec.performance-qa", true, "other", 1));
        assert!(!allowed("dev.cvec.performance-qa", true, "main", 0));
    }

    #[test]
    fn viewport_preserves_negative_display_coordinates_and_rejects_text() {
        let value: Viewport = serde_json::from_value(fixture()).unwrap();
        assert!(value.valid());
        assert_eq!(value.screen_x_css, -1900.0);
        let mut bad = fixture();
        bad["repositoryPath"] = json!("private");
        assert!(serde_json::from_value::<Viewport>(bad).is_err());
        for (key, invalid) in [
            ("innerWidthCss", -1.0),
            ("devicePixelRatio", 0.0),
            ("epochMs", -1.0),
        ] {
            let mut bad = fixture();
            bad[key] = json!(invalid);
            assert!(!serde_json::from_value::<Viewport>(bad).unwrap().valid());
        }
        let mut value: Viewport = serde_json::from_value(fixture()).unwrap();
        value.screen_x_css = f64::NAN;
        assert!(!value.valid());
    }
}
