//! Conservative reuse of Syft output. Every source byte is included, including
//! dirty/untracked/generated/vendor/binary files. Unsupported inputs bypass the
//! cache; they still receive the normal complete scan and a fresh Grype match.
use super::hashing::{lowercase_hex, sha256_hex};
use crate::Error;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

pub(crate) fn digest(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}
pub(crate) fn default_configuration(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            if line == "check-for-app-update: false" {
                "check-for-app-update: true".into()
            } else if line == "pretty: false" {
                "pretty:".into()
            } else {
                line.to_owned()
            }
        })
        .collect()
}

pub(crate) fn fingerprint(root: &Path, tool: &Path, config: &[u8]) -> Result<String, Error> {
    let started = Instant::now();
    let mut hash = Sha256::new();
    let mut entries = 0;
    let mut bytes = 0u64;
    let root = root.canonicalize()?;
    let tool = tool.canonicalize()?;
    hash.update(b"cvec-inventory-v2-cdx-syft-json");
    hash.update(root.as_os_str().as_encoded_bytes());
    hash.update(config);
    let mut environment = std::env::vars_os().collect::<Vec<_>>();
    environment.sort();
    for (key, value) in environment {
        hash.update(key.as_encoded_bytes());
        hash.update([0]);
        hash.update(value.as_encoded_bytes());
        hash.update([0]);
    }
    hash.update(std::env::current_dir()?.as_os_str().as_encoded_bytes());
    visit(&root, &root, &mut hash, &mut entries, &mut bytes, started)?;
    hash.update(tool.as_os_str().as_encoded_bytes());
    visit(&tool, &tool, &mut hash, &mut entries, &mut bytes, started)?;
    Ok(lowercase_hex(&hash.finalize()))
}
fn visit(
    root: &Path,
    path: &Path,
    hash: &mut Sha256,
    entries: &mut usize,
    bytes: &mut u64,
    started: Instant,
) -> Result<(), Error> {
    if *entries >= 100_000
        || *bytes > 512 * 1024 * 1024
        || started.elapsed() > Duration::from_secs(3)
    {
        return Err(Error::Scanner(
            "Inventory cache fingerprint budget exceeded".into(),
        ));
    }
    *entries += 1;
    let before = fs::symlink_metadata(path)?;
    if before.file_type().is_symlink() || (!before.is_file() && !before.is_dir()) {
        return Err(Error::Scanner(
            "Inventory cache cannot prove this filesystem input".into(),
        ));
    }
    // Syft can resolve go.mod with the host toolchain and dependencies outside
    // the repository, even with the default configuration.
    if before.is_file()
        && path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("go.mod"))
    {
        return Err(Error::Scanner(
            "Inventory cache cannot prove external Go module inputs".into(),
        ));
    }
    hash.update(
        path.strip_prefix(root)
            .unwrap_or(path)
            .as_os_str()
            .as_encoded_bytes(),
    );
    hash.update([0]);
    hash.update(before.len().to_le_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        for value in [
            before.ino(),
            before.dev(),
            before.mode() as u64,
            before.uid() as u64,
            before.gid() as u64,
            before.mtime() as u64,
            before.mtime_nsec() as u64,
            before.ctime() as u64,
            before.ctime_nsec() as u64,
        ] {
            hash.update(value.to_le_bytes());
        }
    }
    if before.is_dir() {
        hash.update(b"directory");
        let mut children = fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        children.sort();
        for child in children {
            visit(root, &child, hash, entries, bytes, started)?;
        }
    } else {
        hash.update(b"file");
        *bytes = bytes.saturating_add(before.len());
        if *bytes > 512 * 1024 * 1024 {
            return Err(Error::Scanner(
                "Inventory cache byte budget exceeded".into(),
            ));
        }
        let mut file = fs::File::open(path)?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let length = file.read(&mut buffer)?;
            if length == 0 {
                break;
            }
            hash.update(&buffer[..length]);
            if started.elapsed() > Duration::from_secs(3) {
                return Err(Error::Scanner(
                    "Inventory cache time budget exceeded".into(),
                ));
            }
        }
    }
    let after = fs::symlink_metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.ino() != after.ino()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || before.len() != after.len()
        {
            return Err(Error::Scanner(
                "Source changed while proving inventory cache input".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn read(directory: &Path, key: &str) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    let entry = directory.join(key);
    if fs::metadata(&entry).ok()?.modified().ok()?.elapsed().ok()? > Duration::from_secs(24 * 3600)
    {
        return None;
    }
    let inventory =
        super::scanner::read_bounded_output(&entry.join("inventory.json"), 128 * 1024 * 1024)
            .ok()?;
    let evidence =
        super::scanner::read_bounded_output(&entry.join("evidence.json"), 128 * 1024 * 1024)
            .ok()?;
    let checksum = fs::read_to_string(entry.join("checksum")).ok()?;
    if checksum != format!("{}:{}", digest(&inventory), digest(&evidence)) {
        return None;
    }
    Some((inventory, Some(evidence)))
}
#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), Error> {
    Ok(())
}

pub(crate) fn write(
    directory: &Path,
    key: &str,
    inventory: &[u8],
    evidence: &[u8],
) -> Result<(), Error> {
    if inventory.len().saturating_add(evidence.len()) > 128 * 1024 * 1024 {
        return Ok(());
    }
    fs::create_dir_all(directory)?;
    set_mode(directory, 0o700)?;
    let temp = tempfile::Builder::new()
        .prefix("pending-")
        .tempdir_in(directory)?;
    set_mode(temp.path(), 0o700)?;
    let inventory_path = temp.path().join("inventory.json");
    let evidence_path = temp.path().join("evidence.json");
    let checksum_path = temp.path().join("checksum");
    fs::write(&inventory_path, inventory)?;
    set_mode(&inventory_path, 0o600)?;
    fs::write(&evidence_path, evidence)?;
    set_mode(&evidence_path, 0o600)?;
    fs::write(
        &checksum_path,
        format!("{}:{}", digest(inventory), digest(evidence)),
    )?;
    set_mode(&checksum_path, 0o600)?;
    let target = directory.join(key);
    if let Ok(meta) = fs::symlink_metadata(&target) {
        if meta.file_type().is_dir() {
            fs::remove_dir_all(&target)?;
        } else {
            fs::remove_file(&target)?;
        }
    }
    fs::rename(temp.path(), &target)?;
    prune(directory)?;
    Ok(())
}
fn prune(directory: &Path) -> Result<(), Error> {
    let mut entries = Vec::<(SystemTime, PathBuf, u64)>::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().len() != 64 {
            continue;
        }
        let metadata = entry.metadata()?;
        if !metadata.is_dir() {
            continue;
        }
        let size = fs::read_dir(entry.path())?
            .filter_map(Result::ok)
            .filter_map(|file| file.metadata().ok())
            .map(|value| value.len())
            .sum();
        entries.push((metadata.modified()?, entry.path(), size));
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut bytes = 0u64;
    for (index, (time, path, size)) in entries.into_iter().enumerate() {
        bytes = bytes.saturating_add(size);
        if index >= 16
            || bytes > 256 * 1024 * 1024
            || time.elapsed().unwrap_or_default() > Duration::from_secs(24 * 3600)
        {
            fs::remove_dir_all(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_file_config_and_binary_change_invalidates_reuse() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repository");
        fs::create_dir(&root).unwrap();
        let tool = temp.path().join("syft");
        fs::write(&tool, b"scanner").unwrap();
        let a = fingerprint(&root, &tool, b"default config").unwrap();
        for name in [
            "untracked",
            "node_modules/package.json",
            "target/binary",
            "vendor/library",
        ] {
            let file = root.join(name);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, b"input").unwrap();
            let b = fingerprint(&root, &tool, b"default config").unwrap();
            assert_ne!(a, b);
            assert_eq!(b, fingerprint(&root, &tool, b"default config").unwrap());
            fs::write(&file, b"dirty").unwrap();
            assert_ne!(b, fingerprint(&root, &tool, b"default config").unwrap());
        }
        let before = fingerprint(&root, &tool, b"default config").unwrap();
        assert_ne!(
            before,
            fingerprint(&root, &tool, b"changed config").unwrap()
        );
        fs::write(&tool, b"new scanner").unwrap();
        assert_ne!(
            before,
            fingerprint(&root, &tool, b"default config").unwrap()
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&tool, root.join("link")).unwrap();
            assert!(fingerprint(&root, &tool, b"default config").is_err());
        }
    }
    #[test]
    fn nested_go_module_bypasses_inventory_reuse() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repository");
        let nested = root.join("services/example");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("go.mod"), "module example.com/example\n").unwrap();
        let tool = temp.path().join("syft");
        fs::write(&tool, b"scanner").unwrap();
        assert!(fingerprint(&root, &tool, b"default config").is_err());
    }

    #[test]
    fn cache_is_atomic_checked_and_recovers_from_corruption() {
        let temp = tempfile::tempdir().unwrap();
        let key = digest(b"key");
        write(temp.path(), &key, b"inventory", b"evidence").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir_mode = fs::metadata(temp.path().join(&key))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            let file_mode = fs::metadata(temp.path().join(&key).join("inventory.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dir_mode, 0o700);
            assert_eq!(file_mode, 0o600);
        }
        assert_eq!(
            read(temp.path(), &key),
            Some((b"inventory".to_vec(), Some(b"evidence".to_vec())))
        );
        fs::write(temp.path().join(&key).join("inventory.json"), b"corrupt").unwrap();
        assert!(read(temp.path(), &key).is_none());
    }
}
