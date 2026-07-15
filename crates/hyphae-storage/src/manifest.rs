// SPDX-License-Identifier: Apache-2.0

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use hyphae_core::DISK_FORMAT_VERSION;
use thiserror::Error;

const MAGIC: [u8; 8] = *b"HYMNFST1";
const MANIFEST_FORMAT_VERSION: u16 = 1;
const ENCODED_LENGTH: usize = 140;
const ENCODED_LENGTH_U64: u64 = 140;
const CHECKSUM_PREFIX_LENGTH: usize = 104;
const DIGEST_PREFIX_LENGTH: usize = 108;
const MANIFEST_EXTENSION: &str = "hymanifest";

/// Failure while loading or atomically creating a storage manifest.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// A filesystem operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// A committed manifest violates the canonical representation.
    #[error("invalid storage manifest {path}: {reason}")]
    Invalid {
        /// Invalid manifest path.
        path: PathBuf,
        /// Stable diagnostic reason.
        reason: &'static str,
    },

    /// The manifest uses a future format.
    #[error(
        "unsupported storage manifest version {found}; supported manifest version is {supported}"
    )]
    UnsupportedVersion {
        /// Version found on disk.
        found: u16,
        /// Highest version understood by this binary.
        supported: u16,
    },

    /// An immutable generation already exists with different content.
    #[error("storage manifest generation {generation} already exists with different content")]
    GenerationConflict {
        /// Conflicting manifest generation.
        generation: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StorageManifest {
    pub(crate) generation: u64,
    pub(crate) active_segment: u64,
    pub(crate) base_sequence: u64,
    pub(crate) base_digest: [u8; 32],
    pub(crate) snapshot_digest: [u8; 32],
}

impl StorageManifest {
    fn initial() -> Self {
        Self {
            generation: 1,
            active_segment: 1,
            base_sequence: 0,
            base_digest: [0; 32],
            snapshot_digest: [0; 32],
        }
    }

    pub(crate) fn load_or_initialize(root: &Path) -> Result<Self, ManifestError> {
        let directory = root.join("manifest");
        let mut generations = Vec::new();
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if let Some(generation) = generation_from_path(&path)? {
                generations.push((generation, path));
            }
        }
        generations.sort_unstable_by_key(|(generation, _)| *generation);
        if let Some((generation, path)) = generations.last() {
            return decode_manifest(path, *generation);
        }

        let initial = Self::initial();
        initial.write_new(root)?;
        Ok(initial)
    }

    pub(crate) fn write_new(&self, root: &Path) -> Result<(), ManifestError> {
        validate_semantics(self, &root.join(manifest_filename(self.generation)))?;
        let manifest_directory = root.join("manifest");
        let final_path = manifest_directory.join(manifest_filename(self.generation));
        if final_path.exists() {
            let existing = decode_manifest(&final_path, self.generation)?;
            return if existing == *self {
                Ok(())
            } else {
                Err(ManifestError::GenerationConflict {
                    generation: self.generation,
                })
            };
        }

        let temporary_path = root.join("tmp").join(format!(
            "manifest-{:020}-{}.tmp",
            self.generation,
            uuid::Uuid::now_v7()
        ));
        let encoded = encode_manifest(self);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary_path, &final_path)?;
        #[cfg(unix)]
        sync_directory(&manifest_directory)?;
        Ok(())
    }
}

fn manifest_filename(generation: u64) -> String {
    format!("{generation:020}.{MANIFEST_EXTENSION}")
}

fn generation_from_path(path: &Path) -> Result<Option<u64>, ManifestError> {
    if path.extension().and_then(|extension| extension.to_str()) != Some(MANIFEST_EXTENSION) {
        return Ok(None);
    }
    let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(invalid(path, "manifest filename is not UTF-8"));
    };
    let Some(raw_generation) = filename.strip_suffix(&format!(".{MANIFEST_EXTENSION}")) else {
        return Err(invalid(path, "malformed manifest filename"));
    };
    let generation = raw_generation
        .parse::<u64>()
        .map_err(|_| invalid(path, "malformed manifest generation"))?;
    if manifest_filename(generation) != filename {
        return Err(invalid(path, "noncanonical manifest filename"));
    }
    Ok(Some(generation))
}

fn encode_manifest(manifest: &StorageManifest) -> [u8; ENCODED_LENGTH] {
    let mut encoded = [0_u8; ENCODED_LENGTH];
    encoded[..8].copy_from_slice(&MAGIC);
    encoded[8..10].copy_from_slice(&MANIFEST_FORMAT_VERSION.to_le_bytes());
    encoded[10..12].copy_from_slice(&DISK_FORMAT_VERSION.to_le_bytes());
    encoded[12..16].copy_from_slice(&0_u32.to_le_bytes());
    encoded[16..24].copy_from_slice(&manifest.generation.to_le_bytes());
    encoded[24..32].copy_from_slice(&manifest.active_segment.to_le_bytes());
    encoded[32..40].copy_from_slice(&manifest.base_sequence.to_le_bytes());
    encoded[40..72].copy_from_slice(&manifest.base_digest);
    encoded[72..104].copy_from_slice(&manifest.snapshot_digest);
    let checksum = crc32c::crc32c(&encoded[..CHECKSUM_PREFIX_LENGTH]);
    encoded[104..108].copy_from_slice(&checksum.to_le_bytes());
    let digest = blake3::hash(&encoded[..DIGEST_PREFIX_LENGTH]);
    encoded[108..140].copy_from_slice(digest.as_bytes());
    encoded
}

fn decode_manifest(
    path: &Path,
    filename_generation: u64,
) -> Result<StorageManifest, ManifestError> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() != ENCODED_LENGTH_U64 {
        return Err(invalid(path, "file length mismatch"));
    }
    let mut encoded = [0_u8; ENCODED_LENGTH];
    file.read_exact(&mut encoded)?;
    if encoded[..8] != MAGIC {
        return Err(invalid(path, "bad magic"));
    }
    let manifest_version = u16::from_le_bytes(copy_array(&encoded[8..10]));
    if manifest_version != MANIFEST_FORMAT_VERSION {
        return Err(ManifestError::UnsupportedVersion {
            found: manifest_version,
            supported: MANIFEST_FORMAT_VERSION,
        });
    }
    if u16::from_le_bytes(copy_array(&encoded[10..12])) != DISK_FORMAT_VERSION {
        return Err(invalid(path, "disk format mismatch"));
    }
    if u32::from_le_bytes(copy_array(&encoded[12..16])) != 0 {
        return Err(invalid(path, "unsupported flags"));
    }
    let expected_checksum = u32::from_le_bytes(copy_array(&encoded[104..108]));
    if crc32c::crc32c(&encoded[..CHECKSUM_PREFIX_LENGTH]) != expected_checksum {
        return Err(invalid(path, "CRC32C mismatch"));
    }
    let expected_digest: [u8; 32] = copy_array(&encoded[108..140]);
    if *blake3::hash(&encoded[..DIGEST_PREFIX_LENGTH]).as_bytes() != expected_digest {
        return Err(invalid(path, "BLAKE3 digest mismatch"));
    }

    let manifest = StorageManifest {
        generation: u64::from_le_bytes(copy_array(&encoded[16..24])),
        active_segment: u64::from_le_bytes(copy_array(&encoded[24..32])),
        base_sequence: u64::from_le_bytes(copy_array(&encoded[32..40])),
        base_digest: copy_array(&encoded[40..72]),
        snapshot_digest: copy_array(&encoded[72..104]),
    };
    if manifest.generation != filename_generation {
        return Err(invalid(path, "filename generation mismatch"));
    }
    validate_semantics(&manifest, path)?;
    Ok(manifest)
}

fn validate_semantics(manifest: &StorageManifest, path: &Path) -> Result<(), ManifestError> {
    if manifest.generation == 0 || manifest.active_segment != manifest.generation {
        return Err(invalid(path, "invalid generation or active segment"));
    }
    let empty_anchor = manifest.base_sequence == 0;
    if empty_anchor != (manifest.base_digest == [0; 32])
        || empty_anchor != (manifest.snapshot_digest == [0; 32])
    {
        return Err(invalid(path, "inconsistent snapshot anchor"));
    }
    if manifest.generation == 1 && !empty_anchor {
        return Err(invalid(path, "initial generation has a snapshot anchor"));
    }
    if manifest.generation > 1 && empty_anchor {
        return Err(invalid(
            path,
            "compacted generation lacks a snapshot anchor",
        ));
    }
    Ok(())
}

fn invalid(path: &Path, reason: &'static str) -> ManifestError {
    ManifestError::Invalid {
        path: path.to_path_buf(),
        reason,
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), ManifestError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn copy_array<const N: usize>(source: &[u8]) -> [u8; N] {
    let mut output = [0_u8; N];
    output.copy_from_slice(source);
    output
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, io::Write};

    use super::{ManifestError, StorageManifest};
    use crate::test_support::TestDirectory;

    fn initialize_layout(root: &std::path::Path) -> Result<(), Box<dyn Error>> {
        fs::create_dir_all(root.join("manifest"))?;
        fs::create_dir_all(root.join("tmp"))?;
        Ok(())
    }

    #[test]
    fn initializes_and_reloads_an_immutable_manifest() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("manifest-initial")?;
        initialize_layout(temporary.path())?;

        let created = StorageManifest::load_or_initialize(temporary.path())?;
        let reopened = StorageManifest::load_or_initialize(temporary.path())?;
        assert_eq!(created, reopened);
        assert_eq!(created.generation, 1);
        assert_eq!(created.base_sequence, 0);
        Ok(())
    }

    #[test]
    fn ignores_an_interrupted_temporary_manifest() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("manifest-interrupted")?;
        initialize_layout(temporary.path())?;
        let mut partial = fs::File::create(temporary.path().join("tmp/manifest-partial.tmp"))?;
        partial.write_all(b"partial")?;
        partial.sync_all()?;

        let manifest = StorageManifest::load_or_initialize(temporary.path())?;
        assert_eq!(manifest.generation, 1);
        Ok(())
    }

    #[test]
    fn rejects_corruption_in_the_latest_generation() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("manifest-corrupt")?;
        initialize_layout(temporary.path())?;
        StorageManifest::load_or_initialize(temporary.path())?;
        let path = temporary
            .path()
            .join("manifest/00000000000000000001.hymanifest");
        let mut bytes = fs::read(&path)?;
        bytes[40] ^= 1;
        fs::write(&path, bytes)?;

        assert!(matches!(
            StorageManifest::load_or_initialize(temporary.path()),
            Err(ManifestError::Invalid {
                reason: "CRC32C mismatch",
                ..
            })
        ));
        Ok(())
    }
}
