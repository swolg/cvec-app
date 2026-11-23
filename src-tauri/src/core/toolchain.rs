//! Filesystem-only discovery for GUI launches without a login-shell PATH.
use std::path::{Path, PathBuf};

pub(crate) fn search_dirs() -> Vec<PathBuf> {
    directories(
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("HOME").as_deref().map(Path::new),
    )
}

fn directories(path: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = path
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if cfg!(target_os = "macos") {
        dirs.extend(
            [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/usr/bin",
                "/bin",
                "/usr/sbin",
                "/sbin",
            ]
            .map(PathBuf::from),
        );
        if let Some(home) = home.filter(|home| home.is_absolute()) {
            dirs.extend([
                home.join(".local/bin"),
                home.join(".cargo/bin"),
                home.join(".opencode/bin"),
            ]);
        }
    }
    let mut result = Vec::new();
    for dir in dirs {
        if dir.is_absolute() && !result.contains(&dir) {
            result.push(dir);
        }
    }
    result
}

pub(crate) fn find(name: &str) -> Option<PathBuf> {
    find_in(&search_dirs(), name)
}

pub(crate) fn find_in(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|path| executable(path))
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Provider probes and sessions share the same filesystem-only GUI discovery.
pub(crate) fn command(binary: &str) -> std::process::Command {
    let executable = if Path::new(binary).components().count() == 1 {
        find(binary).unwrap_or_else(|| PathBuf::from(binary))
    } else {
        PathBuf::from(binary)
    };
    let mut command = std::process::Command::new(executable);
    command.env("PATH", joined_path());
    command
}

pub(crate) fn joined_path() -> std::ffi::OsString {
    std::env::join_paths(search_dirs()).unwrap_or_default()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn gui_defaults_keep_absolute_preferences_and_exclude_relative_entries() {
        let dirs = directories(
            Some(std::ffi::OsStr::new(
                ".:/custom/bin:relative:/usr/bin:/custom/bin",
            )),
            Some(Path::new("/synthetic/home")),
        );
        assert_eq!(dirs[0], Path::new("/custom/bin"));
        assert!(dirs.iter().all(|dir| dir.is_absolute()));
        assert_eq!(
            dirs.iter()
                .filter(|dir| **dir == Path::new("/custom/bin"))
                .count(),
            1
        );
        if cfg!(target_os = "macos") {
            assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
            assert!(dirs.contains(&PathBuf::from("/synthetic/home/.local/bin")));
            assert!(dirs.contains(&PathBuf::from("/synthetic/home/.cargo/bin")));
        }
    }

    #[test]
    fn discovery_skips_nonexecutables_and_preserves_search_order() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        for dir in [&first, &second] {
            std::fs::write(dir.join("tool"), "fixture").unwrap();
        }
        std::fs::set_permissions(second.join("tool"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        assert_eq!(
            find_in(&[first.clone(), second.clone()], "tool"),
            Some(second.clone().join("tool"))
        );
        std::fs::set_permissions(first.join("tool"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        assert_eq!(
            find_in(&[first.clone(), second], "tool"),
            Some(first.join("tool"))
        );
    }
}
