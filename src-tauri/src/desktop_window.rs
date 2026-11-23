use crate::core::hashing::sha256_hex;
use crate::state::{ISOLATED_UI_IDENTIFIER, PERFORMANCE_QA_IDENTIFIER};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, ffi::OsStr, io::Read, path::Path};
use tauri::{Manager, PhysicalPosition, PhysicalSize};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

type Frame = (i32, i32, u32, u32);

pub fn state_flags() -> StateFlags {
    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED
}

// The plugin joins this filename to app_config_dir for both loading and saving.
// Keep production's existing file, but never share geometry between QA profiles.
pub fn state_filename(identifier: &str, profile: Option<&OsStr>) -> String {
    if matches!(
        identifier,
        PERFORMANCE_QA_IDENTIFIER | ISOLATED_UI_IDENTIFIER
    ) {
        if let Some(profile) = profile.filter(|value| Path::new(value).is_absolute()) {
            return format!(
                ".window-state-qa-{}.json",
                sha256_hex(profile.as_encoded_bytes())
            );
        }
    }
    tauri_plugin_window_state::DEFAULT_FILENAME.into()
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
struct SavedFrame {
    x: i32,
    y: i32,
    width: u32, // The plugin saves INNER physical dimensions.
    height: u32,
    #[serde(default)]
    maximized: bool,
    #[serde(default)]
    prev_x: Option<i32>,
    #[serde(default)]
    prev_y: Option<i32>,
}

fn read_saved(path: &Path) -> Result<Option<SavedFrame>, Box<dyn std::error::Error>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(65_537).read_to_end(&mut bytes)?;
    if bytes.len() > 65_536 {
        return Err("Window state exceeds 64 KiB".into());
    }
    let frames: HashMap<String, SavedFrame> = serde_json::from_slice(&bytes)?;
    Ok(frames
        .get("main")
        .copied()
        .filter(|f| f.width > 0 && f.height > 0))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RestorePlan {
    initial_outer: Frame,
    saved_state_loaded: bool,
    target_outer: Frame,
    target_inner: (u32, u32),
    work_area: Option<Frame>,
    maximize_last: bool,
}

#[derive(Debug, PartialEq)]
enum Action {
    Size(u32, u32),
    Position(i32, i32),
    Maximize,
    Show,
}

impl RestorePlan {
    fn actions(&self) -> Vec<Action> {
        let mut actions = vec![
            Action::Size(self.target_inner.0, self.target_inner.1),
            Action::Position(self.target_outer.0, self.target_outer.1),
        ];
        if self.maximize_last {
            actions.push(Action::Maximize);
        }
        actions.push(Action::Show);
        actions
    }
}

// Plan from immutable saved input and the initial decoration inset. Never read
// native geometry immediately after a setter: macOS Tao enqueues those on GCD.
fn plan_restore(
    initial: Frame,
    inner: (u32, u32),
    saved: Option<SavedFrame>,
    areas: &[Frame],
) -> RestorePlan {
    let inset = (
        initial.2.saturating_sub(inner.0),
        initial.3.saturating_sub(inner.1),
    );
    let desired = saved.map_or(initial, |f| {
        (
            if f.maximized {
                f.prev_x.unwrap_or(f.x)
            } else {
                f.x
            },
            if f.maximized {
                f.prev_y.unwrap_or(f.y)
            } else {
                f.y
            },
            f.width.saturating_add(inset.0),
            f.height.saturating_add(inset.1),
        )
    });
    let area = areas
        .iter()
        .copied()
        .filter(|a| a.2 > inset.0 && a.3 > inset.1)
        .max_by_key(|a| {
            (
                intersection(desired, *a),
                std::cmp::Reverse(distance(desired, *a)),
            )
        });
    let target = area.map_or(desired, |a| fit_frame(desired, a));
    RestorePlan {
        initial_outer: initial,
        saved_state_loaded: saved.is_some(),
        target_outer: target,
        target_inner: (
            target.2.saturating_sub(inset.0).max(1),
            target.3.saturating_sub(inset.1).max(1),
        ),
        work_area: area,
        maximize_last: saved.is_some_and(|f| f.maximized),
    }
}

fn intersection(a: Frame, b: Frame) -> i64 {
    let w = (i64::from(a.0) + i64::from(a.2)).min(i64::from(b.0) + i64::from(b.2))
        - i64::from(a.0.max(b.0));
    let h = (i64::from(a.1) + i64::from(a.3)).min(i64::from(b.1) + i64::from(b.3))
        - i64::from(a.1.max(b.1));
    w.max(0).saturating_mul(h.max(0))
}

fn distance(a: Frame, b: Frame) -> i64 {
    // Manhattan distance between doubled centers; no floating point or overflow.
    let x = (2 * i64::from(a.0) + i64::from(a.2)) - (2 * i64::from(b.0) + i64::from(b.2));
    let y = (2 * i64::from(a.1) + i64::from(a.3)) - (2 * i64::from(b.1) + i64::from(b.3));
    x.abs() + y.abs()
}

pub fn restore(app: &tauri::AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    let path = app.path().app_config_dir()?.join(app.filename());
    let saved = read_saved(&path).unwrap_or_else(|_| {
        eprintln!("Could not read saved window geometry; using initial frame");
        None
    });
    // Capture all native input before issuing ANY geometry mutation.
    let position = window.outer_position()?;
    let outer = window.outer_size()?;
    let inner = window.inner_size()?;
    let areas: Vec<Frame> = window
        .available_monitors()?
        .iter()
        .map(|monitor| {
            let area = monitor.work_area();
            (
                area.position.x,
                area.position.y,
                area.size.width,
                area.size.height,
            )
        })
        .collect();
    let plan = plan_restore(
        (position.x, position.y, outer.width, outer.height),
        (inner.width, inner.height),
        saved,
        &areas,
    );
    if app.config().identifier == "dev.cvec.performance-qa"
        && std::env::var_os("CVEC_PERF_TRACE").is_some()
    {
        // Requested plan, NOT confirmation that native setters have settled.
        let record = serde_json::json!({
            "schemaVersion": 1,
            "pid": std::process::id(),
            "requestedEpochMs": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),
            "appliedGeometryConfirmed": false,
            "plan": plan,
        });
        eprintln!("cvec-perf window-restore-plan {record}");
    }
    // No plugin restore/maximize followed by stale getters, no corrective loop,
    // and no position/size mutation after Maximize. QA sampler records actuals.
    for action in plan.actions() {
        match action {
            Action::Size(w, h) => window.set_size(PhysicalSize::new(w, h))?,
            Action::Position(x, y) => window.set_position(PhysicalPosition::new(x, y))?,
            Action::Maximize => window.maximize()?,
            Action::Show => window.show()?,
        }
    }
    Ok(())
}

fn fit_frame(frame: Frame, area: Frame) -> Frame {
    let (x, y, width, height) = frame;
    let (left, top, available_width, available_height) = area;
    if available_width == 0 || available_height == 0 {
        return frame;
    }
    let width = width.min(available_width);
    let height = height.min(available_height);
    let max_x =
        (i64::from(left) + i64::from(available_width - width)).min(i64::from(i32::MAX)) as i32;
    let max_y =
        (i64::from(top) + i64::from(available_height - height)).min(i64::from(i32::MAX)) as i32;
    (x.clamp(left, max_x), y.clamp(top, max_y), width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_large_or_offscreen_frames_inside_the_work_area() {
        assert_eq!(
            fit_frame((2200, 100, 2560, 1600), (0, 25, 1440, 850)),
            (0, 25, 1440, 850)
        );
        assert_eq!(
            fit_frame((1300, 800, 900, 600), (0, 25, 1440, 850)),
            (540, 275, 900, 600)
        );
        assert_eq!(
            fit_frame((640, 208, 3840, 1938), (0, 60, 3840, 1938)),
            (0, 60, 3840, 1938)
        );
    }

    #[test]
    fn saved_maximize_is_last_without_a_stale_post_restore_read() {
        let saved = SavedFrame {
            x: 0,
            y: 60,
            width: 3344,
            height: 1938,
            maximized: true,
            ..SavedFrame::default()
        };
        let plan = plan_restore(
            (640, 208, 2560, 1640),
            (2560, 1640),
            Some(saved),
            &[(0, 60, 3840, 1938)],
        );
        assert_eq!(plan.target_outer, (0, 60, 3344, 1938));
        assert_eq!(
            plan.actions(),
            vec![
                Action::Size(3344, 1938),
                Action::Position(0, 60),
                Action::Maximize,
                Action::Show
            ]
        );
        // The old synchronous fit of the initial frame retained (640,208),
        // overriding the asynchronously restored/maximized position.
        assert_eq!(fit_frame(plan.initial_outer, (0, 60, 3840, 1938)).0, 640);
    }

    #[test]
    fn saved_inner_size_keeps_decorations_inside_work_area() {
        let saved = SavedFrame {
            x: 5000,
            y: 5000,
            width: 8000,
            height: 8000,
            maximized: false,
            ..SavedFrame::default()
        };
        let plan = plan_restore(
            (0, 0, 1000, 730),
            (1000, 700),
            Some(saved),
            &[(0, 60, 1920, 1000)],
        );
        assert_eq!(plan.target_outer, (0, 60, 1920, 1000));
        assert_eq!(plan.target_inner, (1920, 970));
        assert!(!plan.actions().contains(&Action::Maximize));
    }

    #[test]
    fn chooses_saved_monitor_or_nearest_remaining_monitor() {
        let areas = [(-1920, 25, 1920, 1055), (0, 25, 1920, 1055)];
        let saved = SavedFrame {
            x: -1500,
            y: 60,
            width: 1000,
            height: 700,
            maximized: false,
            ..SavedFrame::default()
        };
        assert_eq!(
            plan_restore((300, 300, 1000, 700), (1000, 700), Some(saved), &areas).target_outer,
            (-1500, 60, 1000, 700)
        );
        let saved = SavedFrame { x: 8000, ..saved };
        assert_eq!(
            plan_restore((0, 0, 1000, 700), (1000, 700), Some(saved), &areas).target_outer,
            (920, 60, 1000, 700)
        );
    }

    #[test]
    fn missing_state_preserves_valid_frame_and_invalid_areas_are_ignored() {
        let p = plan_restore(
            (-1500, 60, 1000, 700),
            (1000, 700),
            None,
            &[(-1920, 25, 1920, 1055)],
        );
        assert_eq!(p.target_outer, (-1500, 60, 1000, 700));
        assert!(!p.saved_state_loaded);
        assert!(
            plan_restore((0, 0, 1000, 700), (1000, 700), None, &[(0, 0, 0, 0)])
                .work_area
                .is_none()
        );
    }

    #[test]
    fn profile_files_are_distinct_and_production_ignores_qa_override() {
        assert_eq!(
            state_filename(
                "dev.cvec.performance-qa",
                Some(OsStr::new("/tmp/cvec-qa-profile"))
            ),
            ".window-state-qa-cba6d93f1d7aa17679c4f5117c1dc2c6d9362e3a27744eed4dc9214af9f733b9.json"
        );
        let a = state_filename("dev.cvec.performance-qa", Some(OsStr::new("/tmp/a")));
        let b = state_filename("dev.cvec.performance-qa", Some(OsStr::new("/tmp/b")));
        assert_ne!(a, b);
        assert_eq!(
            a,
            state_filename("dev.cvec.performance-qa", Some(OsStr::new("/tmp/a")))
        );
        assert!(!a.contains("/tmp/a"));
        assert_eq!(
            state_filename("dev.cvec.app", Some(OsStr::new("/tmp/a"))),
            ".window-state.json"
        );
        assert_eq!(
            state_filename("dev.cvec.performance-qa", Some(OsStr::new("relative"))),
            ".window-state.json"
        );
    }

    #[test]
    fn parses_existing_plugin_state_without_writing_or_migrating_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        assert!(read_saved(&path).unwrap().is_none());
        let data=br#"{"main":{"x":6,"y":62,"width":3344,"height":1938,"prev_x":6,"prev_y":62,"maximized":true,"visible":false,"decorated":true,"fullscreen":false}}"#;
        std::fs::write(&path, data).unwrap();
        let saved = read_saved(&path).unwrap().unwrap();
        assert_eq!(saved.width, 3344);
        assert!(saved.maximized);
        let plan = plan_restore(
            (640, 208, 2560, 1640),
            (2560, 1640),
            Some(saved),
            &[(0, 60, 3840, 1938)],
        );
        assert_eq!(plan.target_outer, (6, 60, 3344, 1938));
        assert_eq!(std::fs::read(&path).unwrap(), data);
        std::fs::write(&path, br#"{"main":{"x":0,"y":0,"width":0,"height":0}}"#).unwrap();
        assert!(read_saved(&path).unwrap().is_none());
        std::fs::write(&path, b"bad json").unwrap();
        assert!(read_saved(&path).is_err());
        std::fs::write(&path, vec![b' '; 65_537]).unwrap();
        assert!(read_saved(&path).is_err());
    }
}
