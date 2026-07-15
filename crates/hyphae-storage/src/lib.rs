// SPDX-License-Identifier: Apache-2.0

//! Durable log, recovery, snapshot, and materialized-index implementation.

mod data_directory;
mod engine;
mod index;
mod log;
mod manifest;
mod mutation;
mod snapshot;

pub use data_directory::{DataDirectory, DataDirectoryError};
pub use engine::{
    CompactionOutcome, CompactionReport, OpenedStorage, StorageEngine, StorageError,
    StorageRecoveryReport,
};
pub use index::MaterializedIndexError;
pub use log::{
    AppendOutcome, CommitReceipt, DurableLog, LogError, OpenedLog, RecoveredTransaction,
    RecoveryReport,
};
pub use manifest::ManifestError;
pub use mutation::{MAX_KEY_BYTES, Mutation, MutationError};
pub use snapshot::{SnapshotError, SnapshotInfo, verify_snapshot};

#[cfg(test)]
mod test_support {
    use std::{fs, io, path::PathBuf};

    pub(crate) struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        pub(crate) fn new(name: &str) -> io::Result<Self> {
            let path = std::env::temp_dir().join(format!(
                "hyphae-{name}-{}-{}",
                std::process::id(),
                uuid::Uuid::now_v7()
            ));
            fs::create_dir_all(&path)?;
            Ok(Self { path })
        }

        pub(crate) fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ignored = fs::remove_dir_all(&self.path);
        }
    }
}
