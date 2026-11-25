//! Descriptor-relative access for repository files. An untrusted process may replace directory names
//! between validation and use, but cannot redirect an already opened parent.
#[cfg(unix)]
use std::path::Component;
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

pub(super) fn read_regular_text(file: File, max_bytes: u64) -> io::Result<String> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Expected a regular file within the supported size",
        ));
    }
    let mut contents = String::new();
    file.take(max_bytes + 1).read_to_string(&mut contents)?;
    if contents.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "File exceeded the supported size while reading",
        ));
    }
    Ok(contents)
}

// Git metadata may live outside the worktree. Follow its resolved path and
// use a nonblocking open so a FIFO cannot stall repository discovery.
pub(super) fn read_git_metadata(path: &Path) -> io::Result<String> {
    const MAX_GIT_METADATA_BYTES: u64 = 1024 * 1024;
    #[cfg(unix)]
    let file = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?
    .into();
    #[cfg(not(unix))]
    let file = File::open(path)?;
    read_regular_text(file, MAX_GIT_METADATA_BYTES)
}

#[cfg(unix)]
mod platform {
    use super::*;
    use rustix::fs::{self, AtFlags, Mode, OFlags};
    use std::{
        ffi::OsString,
        io::Write,
        os::unix::{ffi::OsStringExt, fs::MetadataExt},
    };

    pub(super) struct Root(File);
    pub(super) struct Entry {
        parent: File,
        name: OsString,
    }

    impl Root {
        pub(super) fn open(path: &Path) -> io::Result<Self> {
            Ok(Self(
                fs::open(
                    path,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )?
                .into(),
            ))
        }
        pub(super) fn entry(&self, relative: &Path, create: bool) -> io::Result<Entry> {
            self.entry_excluding(relative, create, &[])
        }
        pub(super) fn entry_excluding(
            &self,
            relative: &Path,
            create: bool,
            excluded: &[std::path::PathBuf],
        ) -> io::Result<Entry> {
            let components = relative
                .components()
                .map(|part| match part {
                    Component::Normal(name) => Ok(name.to_os_string()),
                    _ => Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Invalid relative workspace path",
                    )),
                })
                .collect::<io::Result<Vec<_>>>()?;
            let (name, parents) = components.split_last().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Empty workspace path")
            })?;
            let mut directory = self.0.try_clone()?;
            for component in parents {
                directory = open_directory(&directory, component, create)?;
                if !excluded.is_empty() {
                    self.check_excluded(&fs::fstat(&directory)?, excluded)?;
                }
            }
            let entry = Entry {
                parent: directory,
                name: name.clone(),
            };
            if !excluded.is_empty() {
                match fs::statat(&entry.parent, &entry.name, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(stat) => self.check_excluded(&stat, excluded)?,
                    Err(rustix::io::Errno::NOENT) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(entry)
        }
        fn check_excluded(
            &self,
            candidate: &fs::Stat,
            excluded: &[std::path::PathBuf],
        ) -> io::Result<()> {
            for path in excluded {
                // Resolve from the same root descriptor. Filesystem identity
                // covers case/normalization aliases and is checked after mkdir
                // as well, so absent submodule directories cannot be populated.
                let entry = match self.entry(path, false) {
                    Ok(entry) => entry,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                match fs::statat(&entry.parent, &entry.name, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(stat)
                        if stat.st_dev == candidate.st_dev && stat.st_ino == candidate.st_ino =>
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "Submodule paths are outside the isolated workspace tool boundary",
                        ));
                    }
                    Ok(_) | Err(rustix::io::Errno::NOENT) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(())
        }
        pub(super) fn directory(&self, name: &Path, create: bool) -> io::Result<Self> {
            let entry = self.entry(name, create)?;
            Ok(Self(open_directory(&entry.parent, &entry.name, create)?))
        }

        pub(super) fn entry_names(&self) -> io::Result<impl Iterator<Item = io::Result<OsString>>> {
            Ok(fs::Dir::read_from(&self.0)?.map(|entry| {
                entry
                    .map(|entry| OsString::from_vec(entry.file_name().to_bytes().to_vec()))
                    .map_err(io::Error::from)
            }))
        }
    }

    fn open_directory(parent: &File, name: &std::ffi::OsStr, create: bool) -> io::Result<File> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        match fs::openat(parent, name, flags, Mode::empty()) {
            Ok(fd) => Ok(fd.into()),
            Err(rustix::io::Errno::NOENT) if create => {
                match fs::mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(error) => return Err(error.into()),
                }
                Ok(fs::openat(parent, name, flags, Mode::empty())?.into())
            }
            Err(error) => Err(error.into()),
        }
    }

    impl Entry {
        pub(super) fn lease(&self) -> io::Result<Option<File>> {
            let file: File = fs::openat(
                &self.parent,
                &self.name,
                OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )?
            .into();
            if !file.metadata()?.is_file() {
                return Err(io::Error::other("Invalid fix workspace lease file"));
            }
            match fs::flock(&file, fs::FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => Ok(Some(file)),
                Err(rustix::io::Errno::WOULDBLOCK) => Ok(None),
                Err(error) => Err(error.into()),
            }
        }

        pub(super) fn open(&self) -> io::Result<File> {
            let file = self.open_regular()?;
            if file.metadata()?.nlink() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Workspace source must be a regular file with a single link",
                ));
            }
            Ok(file)
        }
        pub(super) fn open_regular(&self) -> io::Result<File> {
            let file: File = fs::openat(
                &self.parent,
                &self.name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )?
            .into();
            let metadata = file.metadata()?;
            if !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Source must be a regular file",
                ));
            }
            Ok(file)
        }
        pub(super) fn symlink_target(&self) -> io::Result<Option<Vec<u8>>> {
            match fs::readlinkat(&self.parent, &self.name, Vec::new()) {
                Ok(target) => Ok(Some(target.into_bytes())),
                Err(rustix::io::Errno::INVAL) => Ok(None),
                Err(error) => Err(error.into()),
            }
        }
        pub(super) fn is_directory(&self) -> io::Result<bool> {
            let stat = fs::statat(&self.parent, &self.name, AtFlags::SYMLINK_NOFOLLOW)?;
            Ok(fs::FileType::from_raw_mode(stat.st_mode) == fs::FileType::Directory)
        }
        pub(super) fn replace(
            &self,
            bytes: &[u8],
            permissions: Option<std::fs::Permissions>,
        ) -> io::Result<()> {
            let mut random = [0u8; 12];
            getrandom::fill(&mut random)
                .map_err(|_| io::Error::other("Could not allocate a workspace temporary file"))?;
            let name = OsString::from(format!(
                ".cvec-write-{}",
                crate::core::hashing::lowercase_hex(&random)
            ));
            let mut file: File = fs::openat(
                &self.parent,
                &name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )?
            .into();
            let result = (|| {
                file.write_all(bytes)?;
                if let Some(permissions) = permissions {
                    file.set_permissions(permissions)?;
                }
                file.sync_all()?;
                fs::renameat(&self.parent, &name, &self.parent, &self.name)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = fs::unlinkat(&self.parent, &name, AtFlags::empty());
            }
            result
        }
        pub(super) fn remove(&self) -> io::Result<()> {
            Ok(fs::unlinkat(&self.parent, &self.name, AtFlags::empty())?)
        }
    }
}

#[cfg(not(unix))]
mod platform {
    use super::*;
    pub(super) struct Root;
    pub(super) struct Entry;
    fn unavailable() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Fix workspace file access requires descriptor-relative filesystem support",
        )
    }
    impl Root {
        pub(super) fn open(_: &Path) -> io::Result<Self> {
            Err(unavailable())
        }
        pub(super) fn entry(&self, _: &Path, _: bool) -> io::Result<Entry> {
            Err(unavailable())
        }
        pub(super) fn entry_excluding(
            &self,
            _: &Path,
            _: bool,
            _: &[std::path::PathBuf],
        ) -> io::Result<Entry> {
            Err(unavailable())
        }
        pub(super) fn directory(&self, _: &Path, _: bool) -> io::Result<Self> {
            Err(unavailable())
        }
    }
    impl Entry {
        pub(super) fn lease(&self) -> io::Result<Option<File>> {
            Err(unavailable())
        }
        pub(super) fn open(&self) -> io::Result<File> {
            Err(unavailable())
        }
        pub(super) fn symlink_target(&self) -> io::Result<Option<Vec<u8>>> {
            Err(unavailable())
        }
        pub(super) fn is_directory(&self) -> io::Result<bool> {
            Err(unavailable())
        }
        pub(super) fn replace(&self, _: &[u8], _: Option<std::fs::Permissions>) -> io::Result<()> {
            Err(unavailable())
        }
        pub(super) fn remove(&self) -> io::Result<()> {
            Err(unavailable())
        }
    }
}

pub(super) struct AnchoredRoot(platform::Root);
pub(super) struct AnchoredEntry(platform::Entry);
pub(super) struct WorkspaceLease(File);
#[cfg(unix)]
impl Drop for WorkspaceLease {
    fn drop(&mut self) {
        // Release explicitly: a concurrent fork can briefly inherit this open
        // file before CLOEXEC takes effect, even after its owner is dropped.
        let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
    }
}
impl AnchoredRoot {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        Ok(Self(platform::Root::open(path)?))
    }
    pub(super) fn entry(&self, path: &Path, create: bool) -> io::Result<AnchoredEntry> {
        Ok(AnchoredEntry(self.0.entry(path, create)?))
    }
    pub(super) fn entry_excluding(
        &self,
        path: &Path,
        create: bool,
        excluded: &[std::path::PathBuf],
    ) -> io::Result<AnchoredEntry> {
        Ok(AnchoredEntry(
            self.0.entry_excluding(path, create, excluded)?,
        ))
    }
    pub(super) fn directory(&self, path: &Path, create: bool) -> io::Result<Self> {
        Ok(Self(self.0.directory(path, create)?))
    }

    #[cfg(unix)]
    pub(super) fn entry_names(
        &self,
    ) -> io::Result<impl Iterator<Item = io::Result<std::ffi::OsString>>> {
        self.0.entry_names()
    }
}
impl AnchoredEntry {
    pub(super) fn lease(&self) -> io::Result<Option<WorkspaceLease>> {
        Ok(self.0.lease()?.map(WorkspaceLease))
    }
    pub(super) fn open(&self) -> io::Result<File> {
        self.0.open()
    }
    #[cfg(unix)]
    pub(super) fn open_regular(&self) -> io::Result<File> {
        self.0.open_regular()
    }
    pub(super) fn symlink_target(&self) -> io::Result<Option<Vec<u8>>> {
        self.0.symlink_target()
    }
    pub(super) fn is_directory(&self) -> io::Result<bool> {
        self.0.is_directory()
    }
    pub(super) fn replace(
        &self,
        bytes: &[u8],
        permissions: Option<std::fs::Permissions>,
    ) -> io::Result<()> {
        self.0.replace(bytes, permissions)
    }
    pub(super) fn remove(&self) -> io::Result<()> {
        self.0.remove()
    }
}
