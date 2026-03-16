use crate::Error;
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use strum::{Display, EnumString, VariantNames};
use types::non_zero_usize::new_non_zero_usize;
use types::{Epoch, EthSpec, IndexedAttestation};

pub const DEFAULT_CHUNK_SIZE: usize = 16;
pub const DEFAULT_VALIDATOR_CHUNK_SIZE: usize = 256;
pub const DEFAULT_HISTORY_LENGTH: usize = 4096;
pub const DEFAULT_UPDATE_PERIOD: u64 = 12;
pub const DEFAULT_SLOT_OFFSET: f64 = 10.5;
pub const DEFAULT_MAX_DB_SIZE: usize = 512 * 1024; // 512 GiB
pub const DEFAULT_ATTESTATION_ROOT_CACHE_SIZE: NonZeroUsize = new_non_zero_usize(100_000);
pub const DEFAULT_BROADCAST: bool = false;

#[cfg(all(feature = "mdbx", not(any(feature = "lmdb", feature = "redb"))))]
pub const DEFAULT_BACKEND: DatabaseBackend = DatabaseBackend::Mdbx;
#[cfg(feature = "lmdb")]
pub const DEFAULT_BACKEND: DatabaseBackend = DatabaseBackend::Lmdb;
#[cfg(all(feature = "redb", not(any(feature = "mdbx", feature = "lmdb"))))]
pub const DEFAULT_BACKEND: DatabaseBackend = DatabaseBackend::Redb;
#[cfg(not(any(feature = "mdbx", feature = "lmdb", feature = "redb")))]
pub const DEFAULT_BACKEND: DatabaseBackend = DatabaseBackend::Disabled;

pub const MAX_HISTORY_LENGTH: usize = 1 << 16;
pub const MEGABYTE: usize = 1 << 20;
pub const MDBX_DATA_FILENAME: &str = "mdbx.dat";
pub const REDB_DATA_FILENAME: &str = "slasher.redb";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database_path: PathBuf,
    pub chunk_size: usize,
    pub validator_chunk_size: usize,
    /// Number of epochs of history to keep.
    pub history_length: usize,
    /// Update frequency in seconds.
    pub update_period: u64,
    /// Offset from the start of the slot to begin processing.
    pub slot_offset: f64,
    /// Maximum size of the database in megabytes.
    pub max_db_size_mbs: usize,
    /// Maximum size of the in-memory cache for attestation roots.
    pub attestation_root_cache_size: NonZeroUsize,
    /// Whether to broadcast slashings found to the network.
    pub broadcast: bool,
    /// Database backend to use.
    pub backend: DatabaseBackend,
}

/// Immutable configuration parameters which are stored on disk and checked for consistency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskConfig {
    pub chunk_size: usize,
    pub validator_chunk_size: usize,
    pub history_length: usize,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Display, EnumString, VariantNames,
)]
#[strum(serialize_all = "lowercase")]
pub enum DatabaseBackend {
    #[cfg(feature = "mdbx")]
    Mdbx,
    #[cfg(feature = "lmdb")]
    Lmdb,
    #[cfg(feature = "redb")]
    Redb,
    Disabled,
}

#[derive(Debug, PartialEq)]
pub enum DatabaseBackendOverride {
    Success(DatabaseBackend),
    Failure(PathBuf),
    Noop,
}

impl Config {
    pub fn new(database_path: PathBuf) -> Self {
        Self {
            database_path,
            chunk_size: DEFAULT_CHUNK_SIZE,
            validator_chunk_size: DEFAULT_VALIDATOR_CHUNK_SIZE,
            history_length: DEFAULT_HISTORY_LENGTH,
            update_period: DEFAULT_UPDATE_PERIOD,
            slot_offset: DEFAULT_SLOT_OFFSET,
            max_db_size_mbs: DEFAULT_MAX_DB_SIZE,
            attestation_root_cache_size: DEFAULT_ATTESTATION_ROOT_CACHE_SIZE,
            broadcast: DEFAULT_BROADCAST,
            backend: DEFAULT_BACKEND,
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.chunk_size == 0
            || self.validator_chunk_size == 0
            || self.history_length == 0
            || self.max_db_size_mbs == 0
        {
            Err(Error::ConfigInvalidZeroParameter {
                config: self.clone(),
            })
        } else if !self.history_length.is_multiple_of(self.chunk_size) {
            Err(Error::ConfigInvalidChunkSize {
                chunk_size: self.chunk_size,
                history_length: self.history_length,
            })
        } else if self.history_length > MAX_HISTORY_LENGTH {
            Err(Error::ConfigInvalidHistoryLength {
                history_length: self.history_length,
                max_history_length: MAX_HISTORY_LENGTH,
            })
        } else {
            Ok(())
        }
    }

    pub fn disk_config(&self) -> DiskConfig {
        DiskConfig {
            chunk_size: self.chunk_size,
            validator_chunk_size: self.validator_chunk_size,
            history_length: self.history_length,
        }
    }

    pub fn chunk_index(&self, epoch: Epoch) -> usize {
        (epoch.as_usize() % self.history_length) / self.chunk_size
    }

    pub fn validator_chunk_index(&self, validator_index: u64) -> usize {
        validator_index as usize / self.validator_chunk_size
    }

    pub fn chunk_offset(&self, epoch: Epoch) -> usize {
        epoch.as_usize() % self.chunk_size
    }

    pub fn validator_offset(&self, validator_index: u64) -> usize {
        validator_index as usize % self.validator_chunk_size
    }

    /// Map the validator and epoch chunk indexes into a single value for use as a database key.
    pub fn disk_key(&self, validator_chunk_index: usize, chunk_index: usize) -> usize {
        let width = self.history_length / self.chunk_size;
        validator_chunk_index * width + chunk_index
    }

    /// Map the validator and epoch offsets into an index for `Chunk::data`.
    pub fn cell_index(&self, validator_offset: usize, chunk_offset: usize) -> usize {
        validator_offset * self.chunk_size + chunk_offset
    }

    /// Return an iterator over all the validator indices in a validator chunk.
    pub fn validator_indices_in_chunk(
        &self,
        validator_chunk_index: usize,
    ) -> impl Iterator<Item = u64> {
        (validator_chunk_index * self.validator_chunk_size
            ..(validator_chunk_index + 1) * self.validator_chunk_size)
            .map(|index| index as u64)
    }

    /// Iterate over the attesting indices which belong to the `validator_chunk_index` chunk.
    pub fn attesting_validators_in_chunk<'a, E: EthSpec>(
        &'a self,
        attestation: &'a IndexedAttestation<E>,
        validator_chunk_index: usize,
    ) -> impl Iterator<Item = u64> + 'a {
        attestation
            .attesting_indices_iter()
            .filter(move |v| self.validator_chunk_index(**v) == validator_chunk_index)
            .copied()
    }

    pub fn override_backend(&mut self) -> DatabaseBackendOverride {
        let mdbx_path = self.database_path.join(MDBX_DATA_FILENAME);

        #[cfg(feature = "mdbx")]
        let already_mdbx = self.backend == DatabaseBackend::Mdbx;
        #[cfg(not(feature = "mdbx"))]
        let already_mdbx = false;

        if !already_mdbx && mdbx_path.exists() {
            #[cfg(feature = "mdbx")]
            {
                let old_backend = self.backend;
                self.backend = DatabaseBackend::Mdbx;
                DatabaseBackendOverride::Success(old_backend)
            }
            #[cfg(not(feature = "mdbx"))]
            {
                DatabaseBackendOverride::Failure(mdbx_path)
            }
        } else {
            DatabaseBackendOverride::Noop
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn default_config() -> Config {
        Config::new(PathBuf::from("/tmp/slasher-test"))
    }

    // --- Config::new defaults ---

    #[test]
    fn new_uses_default_values() {
        let config = default_config();
        assert_eq!(config.chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(config.validator_chunk_size, DEFAULT_VALIDATOR_CHUNK_SIZE);
        assert_eq!(config.history_length, DEFAULT_HISTORY_LENGTH);
        assert_eq!(config.update_period, DEFAULT_UPDATE_PERIOD);
        assert!((config.slot_offset - DEFAULT_SLOT_OFFSET).abs() < f64::EPSILON);
        assert_eq!(config.max_db_size_mbs, DEFAULT_MAX_DB_SIZE);
        assert_eq!(
            config.attestation_root_cache_size,
            DEFAULT_ATTESTATION_ROOT_CACHE_SIZE
        );
        assert_eq!(config.broadcast, DEFAULT_BROADCAST);
        assert_eq!(config.backend, DEFAULT_BACKEND);
    }

    // --- validate ---

    #[test]
    fn validate_default_config_succeeds() {
        assert!(default_config().validate().is_ok());
    }

    #[test]
    fn validate_zero_chunk_size_fails() {
        let mut config = default_config();
        config.chunk_size = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_zero_validator_chunk_size_fails() {
        let mut config = default_config();
        config.validator_chunk_size = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_zero_history_length_fails() {
        let mut config = default_config();
        config.history_length = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_zero_max_db_size_fails() {
        let mut config = default_config();
        config.max_db_size_mbs = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_history_length_not_multiple_of_chunk_size_fails() {
        let mut config = default_config();
        config.chunk_size = 16;
        config.history_length = 100; // not divisible by 16
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_history_length_exceeds_max_fails() {
        let mut config = default_config();
        config.chunk_size = 1;
        config.history_length = MAX_HISTORY_LENGTH + 1;
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_history_length_at_max_succeeds() {
        let mut config = default_config();
        config.chunk_size = 1;
        config.history_length = MAX_HISTORY_LENGTH;
        assert!(config.validate().is_ok());
    }

    // --- disk_config ---

    #[test]
    fn disk_config_matches_fields() {
        let config = default_config();
        let disk = config.disk_config();
        assert_eq!(disk.chunk_size, config.chunk_size);
        assert_eq!(disk.validator_chunk_size, config.validator_chunk_size);
        assert_eq!(disk.history_length, config.history_length);
    }

    // --- chunk_index ---

    #[test]
    fn chunk_index_basic() {
        let config = default_config();
        // chunk_size=16, history_length=4096
        // epoch 0 -> chunk 0
        assert_eq!(config.chunk_index(Epoch::new(0)), 0);
        // epoch 15 -> chunk 0 (15 / 16 = 0)
        assert_eq!(config.chunk_index(Epoch::new(15)), 0);
        // epoch 16 -> chunk 1
        assert_eq!(config.chunk_index(Epoch::new(16)), 1);
        // epoch 4095 -> chunk 255 (last in history)
        assert_eq!(config.chunk_index(Epoch::new(4095)), 255);
        // epoch 4096 wraps -> chunk 0
        assert_eq!(config.chunk_index(Epoch::new(4096)), 0);
    }

    #[test]
    fn chunk_index_custom_sizes() {
        let mut config = default_config();
        config.chunk_size = 8;
        config.history_length = 64;
        // epoch 7 -> chunk 0
        assert_eq!(config.chunk_index(Epoch::new(7)), 0);
        // epoch 8 -> chunk 1
        assert_eq!(config.chunk_index(Epoch::new(8)), 1);
        // epoch 63 -> chunk 7 (last)
        assert_eq!(config.chunk_index(Epoch::new(63)), 7);
        // epoch 64 wraps -> chunk 0
        assert_eq!(config.chunk_index(Epoch::new(64)), 0);
    }

    // --- validator_chunk_index ---

    #[test]
    fn validator_chunk_index_basic() {
        let config = default_config();
        // validator_chunk_size=256
        assert_eq!(config.validator_chunk_index(0), 0);
        assert_eq!(config.validator_chunk_index(255), 0);
        assert_eq!(config.validator_chunk_index(256), 1);
        assert_eq!(config.validator_chunk_index(512), 2);
    }

    // --- chunk_offset ---

    #[test]
    fn chunk_offset_basic() {
        let config = default_config();
        // chunk_size=16
        assert_eq!(config.chunk_offset(Epoch::new(0)), 0);
        assert_eq!(config.chunk_offset(Epoch::new(15)), 15);
        assert_eq!(config.chunk_offset(Epoch::new(16)), 0);
        assert_eq!(config.chunk_offset(Epoch::new(17)), 1);
    }

    // --- validator_offset ---

    #[test]
    fn validator_offset_basic() {
        let config = default_config();
        // validator_chunk_size=256
        assert_eq!(config.validator_offset(0), 0);
        assert_eq!(config.validator_offset(255), 255);
        assert_eq!(config.validator_offset(256), 0);
        assert_eq!(config.validator_offset(257), 1);
    }

    // --- disk_key ---

    #[test]
    fn disk_key_maps_two_indices() {
        let config = default_config();
        // width = history_length / chunk_size = 4096 / 16 = 256
        assert_eq!(config.disk_key(0, 0), 0);
        assert_eq!(config.disk_key(0, 1), 1);
        assert_eq!(config.disk_key(1, 0), 256);
        assert_eq!(config.disk_key(1, 1), 257);
        assert_eq!(config.disk_key(2, 5), 2 * 256 + 5);
    }

    // --- cell_index ---

    #[test]
    fn cell_index_maps_offsets() {
        let config = default_config();
        // chunk_size=16
        assert_eq!(config.cell_index(0, 0), 0);
        assert_eq!(config.cell_index(0, 15), 15);
        assert_eq!(config.cell_index(1, 0), 16);
        assert_eq!(config.cell_index(3, 5), 3 * 16 + 5);
    }

    // --- validator_indices_in_chunk ---

    #[test]
    fn validator_indices_in_chunk_range() {
        let config = default_config();
        let indices: Vec<u64> = config.validator_indices_in_chunk(0).collect();
        assert_eq!(indices.len(), 256);
        assert_eq!(indices[0], 0);
        assert_eq!(indices[255], 255);

        let indices: Vec<u64> = config.validator_indices_in_chunk(2).collect();
        assert_eq!(indices[0], 512);
        assert_eq!(indices[255], 767);
    }

    // --- override_backend ---

    #[test]
    fn override_backend_noop_when_no_mdbx_file() {
        let dir = tempdir().unwrap();
        let mut config = Config::new(dir.path().into());
        assert_eq!(config.override_backend(), DatabaseBackendOverride::Noop);
    }

    #[test]
    fn override_backend_with_mdbx_file_present() {
        let dir = tempdir().unwrap();
        let mdbx_path = dir.path().join(MDBX_DATA_FILENAME);
        // Create the mdbx data file and ensure it's flushed to disk
        {
            let file = std::fs::File::create(&mdbx_path).unwrap();
            std::io::Write::write_all(&mut &file, b"data").unwrap();
            file.sync_all().unwrap();
        }
        assert!(
            mdbx_path.exists(),
            "mdbx data file should exist at {}",
            mdbx_path.display()
        );
        let mut config = Config::new(dir.path().into());
        // Force a non-mdbx backend so the override has something to change
        config.backend = DatabaseBackend::Disabled;

        let result = config.override_backend();

        // If mdbx feature is enabled, it should override; otherwise fail
        #[cfg(feature = "mdbx")]
        assert!(matches!(result, DatabaseBackendOverride::Success(_)));
        #[cfg(not(feature = "mdbx"))]
        assert!(matches!(result, DatabaseBackendOverride::Failure(_)));
    }

    // --- DiskConfig equality ---

    #[test]
    fn disk_config_equality() {
        let a = DiskConfig {
            chunk_size: 16,
            validator_chunk_size: 256,
            history_length: 4096,
        };
        let b = a.clone();
        assert_eq!(a, b);

        let c = DiskConfig {
            chunk_size: 8,
            ..a.clone()
        };
        assert_ne!(a, c);
    }

    // --- DatabaseBackend display ---

    #[test]
    fn database_backend_display() {
        assert_eq!(DatabaseBackend::Disabled.to_string(), "disabled");
        #[cfg(feature = "lmdb")]
        assert_eq!(DatabaseBackend::Lmdb.to_string(), "lmdb");
        #[cfg(feature = "redb")]
        assert_eq!(DatabaseBackend::Redb.to_string(), "redb");
    }

    #[test]
    fn database_backend_from_str() {
        assert_eq!(
            "disabled".parse::<DatabaseBackend>().unwrap(),
            DatabaseBackend::Disabled
        );
        #[cfg(feature = "lmdb")]
        assert_eq!(
            "lmdb".parse::<DatabaseBackend>().unwrap(),
            DatabaseBackend::Lmdb
        );
        assert!("nonexistent".parse::<DatabaseBackend>().is_err());
    }
}
