// SPDX-License-Identifier: Apache-2.0

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use fs4::{FileExt, TryLockError};
use hyphae_core::DISK_FORMAT_VERSION;
use thiserror::Error;

use crate::{DurableLog, LogError, OpenedLog};

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

        initialize_or_validate_format(&root)?;
        for name in REQUIRED_DIRECTORIES {
            let directory = root.join(name);
            fs::create_dir_all(&directory).map_err(|source| DataDirectoryError::Io {
                action: "create data subdirectory",
                path: directory,
                source,
            })?;
        }

        Ok(Self { root, lock })
    }

    /// Returns the canonical root path.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Returns the path of the first append-only log segment.
    pub fn initial_log_path(&self) -> PathBuf {
        self.root.join("log").join("0000000000000001.hylog")
    }

    /// Opens the initial durable log while borrowing this directory lock.
    ///
    /// The returned writer cannot outlive the exclusive data-directory owner.
    ///
    /// # Errors
    ///
    /// Returns an error for I/O failures or any complete invalid frame.
    pub fn open_log(&self) -> Result<OpenedLog<'_>, LogError> {
        let (log, recovery) = DurableLog::open_file(self.initial_log_path())?;
        Ok(OpenedLog::new(log, recovery))
    }
}

impl Drop for DataDirectory {
    fn drop(&mut self) {
        let _ignored = FileExt::unlock(&self.lock);
    }
}

fn initialize_or_validate_format(root: &Path) -> Result<(), DataDirectoryError> {
    let format_path = root.join("FORMAT");
    if format_path.exists() {
        return validate_format(&format_path);
    }

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
    let marker = format!("{FORMAT_PREFIX}{DISK_FORMAT_VERSION}\n");
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

fn validate_format(path: &Path) -> Result<(), DataDirectoryError> {
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
    if version != DISK_FORMAT_VERSION {
        return Err(DataDirectoryError::MalformedFormat(path.to_path_buf()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs};

    use uuid::Uuid;

    use super::{DataDirectory, DataDirectoryError, REQUIRED_DIRECTORIES};
    use crate::test_support::TestDirectory;

    #[test]
    fn initializes_canonical_layout() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("data-layout")?;
        let root = temporary.path().join("data");
        let directory = DataDirectory::open(&root)?;

        assert_eq!(directory.path(), root);
        assert_eq!(
            fs::read_to_string(root.join("FORMAT"))?,
            "hyphae-disk-format=1\n"
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
        fs::write(temporary.path().join("FORMAT"), "hyphae-disk-format=2\n")?;

        let result = DataDirectory::open(temporary.path());
        assert!(matches!(
            result,
            Err(DataDirectoryError::UnsupportedFormat {
                found: 2,
                supported: 1
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
}
