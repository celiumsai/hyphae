// SPDX-License-Identifier: Apache-2.0

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use fs4::{FileExt, TryLockError};
use hyphae_core::{DISK_FORMAT_VERSION, MIN_DISK_FORMAT_VERSION};
use thiserror::Error;

use crate::{DurableLog, LogError, ManifestError, OpenedLog, manifest::StorageManifest};

const FORMAT_PREFIX: &str = "hyphae-disk-format=";
const REQUIRED_DIRECTORIES: [&str; 6] = ["manifest", "log", "snapshots", "indexes", "blobs", "tmp"];

/// Failure while opening or initializing a Hyphae data directory.
#[derive(Debug, Error)]
pub enum DataDirectoryError {
    /// Another writer already owns the directory lock.
    #[error("data directory is already locked by another writer: {0}")]
    AlreadyLocked(PathBuf),

    /// The `FORMAT` marker does not match the canonical representation.
    #[error("malformed data format marker: {0}")]
    MalformedFormat(PathBuf),

    /// The directory was created by a newer, unsupported disk format.
    #[error("unsupported disk format {found}; this binary supports {supported}")]
    UnsupportedFormat {
        /// Version found in `FORMAT`.
        found: u16,
        /// Highest version understood by this binary.
        supported: u16,
    },

    /// The immutable storage manifest could not be loaded or initialized.
    #[error(transparent)]
    Manifest(#[from] ManifestError),

    /// A filesystem operation failed.
    #[error("failed to {action} {path}: {source}")]
    Io {
        /// Operation that failed.
        action: &'static str,
        /// Path involved in the operation.
        path: PathBuf,
        /// Underlying operating-system error.
        #[source]
        source: io::Error,
    },
}

/// An exclusively owned Hyphae data directory.
///
/// The operating-system lock is held until this value is dropped. Opening the
/// same directory for a second writer fails instead of relying on cooperative
/// process behavior.
#[derive(Debug)]
pub struct DataDirectory {
    root: PathBuf,
    lock: File,
    manifest: StorageManifest,
    disk_format_version: u16,
}

impl DataDirectory {
    /// Opens an existing data directory or initializes a new one.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be initialized, is owned by
    /// another writer, has a malformed marker, or uses a future format.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DataDirectoryError> {
        let root = path.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|source| DataDirectoryError::Io {
            action: "create data directory",
            path: root.clone(),
            source,
        })?;

        let lock_path = root.join("LOCK");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| DataDirectoryError::Io {
                action: "open lock file",
                path: lock_path.clone(),
                source,
            })?;

        match FileExt::try_lock(&lock) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(DataDirectoryError::AlreadyLocked(root));
            }
            Err(TryLockError::Error(source)) => {
                return Err(DataDirectoryError::Io {
                    action: "lock data directory",
                    path: lock_path,
                    source,
                });
            }
        }

        let opened_format = initialize_or_validate_format(&root)?;
        for name in REQUIRED_DIRECTORIES {
            let directory = root.join(name);
            fs::create_dir_all(&directory).map_err(|source| DataDirectoryError::Io {
                action: "create data subdirectory",
                path: directory,
                source,
            })?;
        }

        let manifest = StorageManifest::load_or_initialize(&root)?;

        Ok(Self {
            root,
            lock,
            manifest,
            disk_format_version: opened_format,
        })
    }

    /// Returns the canonical root path.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Returns the format currently committed by the directory marker.
    pub fn disk_format_version(&self) -> u16 {
        self.disk_format_version
    }

    /// Returns the path of the active append-only log segment.
    pub fn active_log_path(&self) -> PathBuf {
        self.log_path(self.manifest.active_segment)
    }

    /// Opens the initial durable log while borrowing this directory lock.
    ///
    /// The returned writer cannot outlive the exclusive data-directory owner.
    ///
    /// # Errors
    ///
    /// Returns an error for I/O failures or any complete invalid frame.
    pub fn open_log(&self) -> Result<OpenedLog<'_>, LogError> {
        let (log, recovery) = DurableLog::open_file_at_version(
            self.active_log_path(),
            self.manifest.base_sequence,
            self.manifest.base_digest,
            self.disk_format_version,
        )?;
        Ok(OpenedLog::new(log, recovery))
    }

    pub(crate) fn log_anchor(&self) -> (u64, [u8; 32]) {
        (self.manifest.base_sequence, self.manifest.base_digest)
    }

    pub(crate) fn manifest(&self) -> StorageManifest {
        self.manifest
    }

    pub(crate) fn log_path(&self, segment: u64) -> PathBuf {
        self.root.join("log").join(format!("{segment:020}.hylog"))
    }

    pub(crate) fn snapshot_path(&self, sequence: u64) -> PathBuf {
        self.root
            .join("snapshots")
            .join(format!("snapshot-{sequence:020}.hysnap"))
    }

    pub(crate) fn commit_manifest(
        &mut self,
        manifest: StorageManifest,
    ) -> Result<(), DataDirectoryError> {
        manifest.write_new(&self.root)?;
        self.manifest = manifest;
        Ok(())
    }

    pub(crate) fn promote_format(&mut self) -> Result<(), DataDirectoryError> {
        if self.disk_format_version == DISK_FORMAT_VERSION {
            return Ok(());
        }
        write_format_marker(&self.root, DISK_FORMAT_VERSION)?;
        self.disk_format_version = DISK_FORMAT_VERSION;
        Ok(())
    }

    pub(crate) fn cleanup_retired_logs(&self) -> bool {
        let log_directory = self.root.join("log");
        let Ok(entries) = fs::read_dir(&log_directory) else {
            return false;
        };
        #[cfg(unix)]
        let mut removed = false;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(segment) = segment_from_path(&path) else {
                continue;
            };
            if segment < self.manifest.active_segment {
                match fs::remove_file(&path) {
                    Ok(()) => {
                        #[cfg(unix)]
                        {
                            removed = true;
                        }
                    }
                    Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                    Err(_) => return false,
                }
            }
        }
        #[cfg(unix)]
        if removed && sync_directory(&log_directory).is_err() {
            return false;
        }
        true
    }
}

impl Drop for DataDirectory {
    fn drop(&mut self) {
        let _ignored = FileExt::unlock(&self.lock);
    }
}

fn initialize_or_validate_format(root: &Path) -> Result<u16, DataDirectoryError> {
    let format_path = root.join("FORMAT");
    if format_path.exists() {
        return validate_format(&format_path);
    }
    write_format_marker(root, DISK_FORMAT_VERSION)?;
    Ok(DISK_FORMAT_VERSION)
}

fn write_format_marker(root: &Path, version: u16) -> Result<(), DataDirectoryError> {
    let format_path = root.join("FORMAT");
    let temporary_path = root.join("FORMAT.new");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temporary_path)
        .map_err(|source| DataDirectoryError::Io {
            action: "create temporary format marker",
            path: temporary_path.clone(),
            source,
        })?;
    let marker = format!("{FORMAT_PREFIX}{version}\n");
    file.write_all(marker.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| DataDirectoryError::Io {
            action: "initialize temporary format marker",
            path: temporary_path.clone(),
            source,
        })?;
    drop(file);
    fs::rename(&temporary_path, &format_path).map_err(|source| DataDirectoryError::Io {
        action: "promote format marker",
        path: format_path,
        source,
    })?;
    #[cfg(unix)]
    sync_directory(root)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), DataDirectoryError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| DataDirectoryError::Io {
            action: "synchronize data directory",
            path: path.to_path_buf(),
            source,
        })
}

fn validate_format(path: &Path) -> Result<u16, DataDirectoryError> {
    let mut marker = String::new();
    File::open(path)
        .and_then(|mut file| file.read_to_string(&mut marker))
        .map_err(|source| DataDirectoryError::Io {
            action: "read format marker",
            path: path.to_path_buf(),
            source,
        })?;

    let Some(raw_version) = marker
        .strip_prefix(FORMAT_PREFIX)
        .and_then(|value| value.strip_suffix('\n'))
    else {
        return Err(DataDirectoryError::MalformedFormat(path.to_path_buf()));
    };
    let version = raw_version
        .parse::<u16>()
        .map_err(|_| DataDirectoryError::MalformedFormat(path.to_path_buf()))?;
    if version > DISK_FORMAT_VERSION {
        return Err(DataDirectoryError::UnsupportedFormat {
            found: version,
            supported: DISK_FORMAT_VERSION,
        });
    }
    if version < MIN_DISK_FORMAT_VERSION {
        return Err(DataDirectoryError::MalformedFormat(path.to_path_buf()));
    }
    Ok(version)
}

fn segment_from_path(path: &Path) -> Option<u64> {
    let filename = path.file_name()?.to_str()?;
    let raw_segment = filename.strip_suffix(".hylog")?;
    if raw_segment.len() != 20 || !raw_segment.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let segment = raw_segment.parse().ok()?;
    (format!("{segment:020}.hylog") == filename).then_some(segment)
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs};

    use uuid::Uuid;

    use super::{DataDirectory, DataDirectoryError, REQUIRED_DIRECTORIES};
    use crate::{DurableLog, test_support::TestDirectory};

    #[test]
    fn initializes_canonical_layout() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("data-layout")?;
        let root = temporary.path().join("data");
        let directory = DataDirectory::open(&root)?;

        assert_eq!(directory.path(), root);
        assert_eq!(
            fs::read_to_string(root.join("FORMAT"))?,
            "hyphae-disk-format=2\n"
        );
        assert!(root.join("LOCK").is_file());
        for name in REQUIRED_DIRECTORIES {
            assert!(root.join(name).is_dir());
        }
        Ok(())
    }

    #[test]
    fn rejects_a_second_writer() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("data-lock")?;
        let first = DataDirectory::open(temporary.path())?;
        let second = DataDirectory::open(temporary.path());

        assert!(matches!(second, Err(DataDirectoryError::AlreadyLocked(_))));
        drop(first);
        DataDirectory::open(temporary.path())?;
        Ok(())
    }

    #[test]
    fn rejects_future_format() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("future-format")?;
        fs::write(temporary.path().join("FORMAT"), "hyphae-disk-format=3\n")?;

        let result = DataDirectory::open(temporary.path());
        assert!(matches!(
            result,
            Err(DataDirectoryError::UnsupportedFormat {
                found: 3,
                supported: 2
            })
        ));
        Ok(())
    }

    #[test]
    fn durable_log_cannot_outlive_directory_lock() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("locked-log")?;
        let directory = DataDirectory::open(temporary.path())?;
        let mut opened = directory.open_log()?;
        opened
            .log
            .append_transaction(Uuid::now_v7(), &[b"operation".to_vec()])?;

        assert_eq!(opened.recovery.transactions.len(), 0);
        assert!(matches!(
            DataDirectory::open(temporary.path()),
            Err(DataDirectoryError::AlreadyLocked(_))
        ));
        Ok(())
    }

    #[test]
    fn migrates_an_existing_format_one_directory_without_a_manifest() -> Result<(), Box<dyn Error>>
    {
        let temporary = TestDirectory::new("data-manifest-migration")?;
        let root = temporary.path().join("data");
        fs::create_dir_all(root.join("log"))?;
        fs::write(root.join("FORMAT"), "hyphae-disk-format=1\n")?;
        let log_path = root.join("log/00000000000000000001.hylog");
        let (mut legacy_log, _) = DurableLog::open_file_at_version(&log_path, 0, [0; 32], 1)?;
        legacy_log.append_transaction(Uuid::now_v7(), &[b"preserved".to_vec()])?;
        drop(legacy_log);

        let directory = DataDirectory::open(&root)?;
        assert!(
            root.join("manifest/00000000000000000001.hymanifest")
                .is_file()
        );
        let opened = directory.open_log()?;
        assert_eq!(opened.recovery.transactions.len(), 1);
        assert_eq!(opened.recovery.transactions[0].operations[0], b"preserved");
        Ok(())
    }
}
