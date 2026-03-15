use crate::{DBColumn, Error, StoreItem};
use serde::{Deserialize, Serialize};
use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use types::{Hash256, Slot};

pub const CURRENT_SCHEMA_VERSION: SchemaVersion = SchemaVersion(28);

// All the keys that get stored under the `BeaconMeta` column.
//
// We use `repeat_byte` because it's a const fn.
pub const SCHEMA_VERSION_KEY: Hash256 = Hash256::repeat_byte(0);
pub const CONFIG_KEY: Hash256 = Hash256::repeat_byte(1);
pub const SPLIT_KEY: Hash256 = Hash256::repeat_byte(2);
// DEPRECATED
// pub const PRUNING_CHECKPOINT_KEY: Hash256 = Hash256::repeat_byte(3);
pub const COMPACTION_TIMESTAMP_KEY: Hash256 = Hash256::repeat_byte(4);
pub const ANCHOR_INFO_KEY: Hash256 = Hash256::repeat_byte(5);
pub const BLOB_INFO_KEY: Hash256 = Hash256::repeat_byte(6);
pub const DATA_COLUMN_INFO_KEY: Hash256 = Hash256::repeat_byte(7);
pub const DATA_COLUMN_CUSTODY_INFO_KEY: Hash256 = Hash256::repeat_byte(8);

/// State upper limit value used to indicate that a node is not storing historic states.
pub const STATE_UPPER_LIMIT_NO_RETAIN: Slot = Slot::new(u64::MAX);

/// The `AnchorInfo` encoding an uninitialized anchor.
///
/// This value should never exist except on initial start-up prior to the anchor being initialised
/// by `init_anchor_info`.
pub const ANCHOR_UNINITIALIZED: AnchorInfo = AnchorInfo {
    anchor_slot: Slot::new(u64::MAX),
    oldest_block_slot: Slot::new(u64::MAX),
    oldest_block_parent: Hash256::ZERO,
    state_upper_limit: Slot::new(u64::MAX),
    state_lower_limit: Slot::new(0),
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SchemaVersion(pub u64);

impl SchemaVersion {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl StoreItem for SchemaVersion {
    fn db_column() -> DBColumn {
        DBColumn::BeaconMeta
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.0.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Ok(SchemaVersion(u64::from_ssz_bytes(bytes)?))
    }
}

/// The last time the database was compacted.
pub struct CompactionTimestamp(pub u64);

impl StoreItem for CompactionTimestamp {
    fn db_column() -> DBColumn {
        DBColumn::BeaconMeta
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.0.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Ok(CompactionTimestamp(u64::from_ssz_bytes(bytes)?))
    }
}

/// Database parameters relevant to weak subjectivity sync.
#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode, Serialize, Deserialize)]
pub struct AnchorInfo {
    /// The slot at which the anchor state is present and which we cannot revert. Values on start:
    /// - Genesis start: 0
    /// - Checkpoint sync: Slot of the finalized state advanced to the checkpoint epoch
    /// - Existing DB prior to v23: Finalized state slot at the migration moment
    ///
    /// Immutable
    pub anchor_slot: Slot,
    /// All blocks with slots greater than or equal to this value are available in the database.
    /// Additionally, the genesis block is always available.
    ///
    /// Values on start:
    /// - Genesis start: 0
    /// - Checkpoint sync: Slot of the finalized checkpoint block
    ///
    /// Progressively decreases during backfill sync until reaching 0.
    pub oldest_block_slot: Slot,
    /// The block root of the next block that needs to be added to fill in the history.
    ///
    /// Zero if we know all blocks back to genesis.
    pub oldest_block_parent: Hash256,
    /// All states with slots _greater than or equal to_ `min(split.slot, state_upper_limit)` are
    /// available in the database. If `state_upper_limit` is higher than `split.slot`, states are
    /// not being written to the freezer database.
    ///
    /// Values on start if state reconstruction is enabled:
    /// - Genesis start: 0
    /// - Checkpoint sync: Slot of the next scheduled snapshot
    ///
    /// Value on start if state reconstruction is disabled:
    /// - 2^64 - 1 representing no historic state storage.
    ///
    /// Immutable until state reconstruction completes.
    pub state_upper_limit: Slot,
    /// All states with slots _less than or equal to_ this value are available in the database.
    /// The minimum value is 0, indicating that the genesis state is always available.
    ///
    /// Values on start:
    /// - Genesis start: 0
    /// - Checkpoint sync: 0
    ///
    /// When full block backfill completes (`oldest_block_slot == 0`) state reconstruction starts and
    /// this value will progressively increase until reaching `state_upper_limit`.
    pub state_lower_limit: Slot,
}

impl AnchorInfo {
    /// Returns true if the block backfill has completed.
    /// This is a comparison between the oldest block slot and the target backfill slot (which is
    /// likely to be the closest WSP).
    pub fn block_backfill_complete(&self, target_slot: Slot) -> bool {
        self.oldest_block_slot <= target_slot
    }

    /// Return true if all historic states are stored, i.e. if state reconstruction is complete.
    pub fn all_historic_states_stored(&self) -> bool {
        self.state_lower_limit == self.state_upper_limit
    }

    /// Return true if no historic states other than genesis are stored in the database.
    pub fn no_historic_states_stored(&self, split_slot: Slot) -> bool {
        self.state_lower_limit == 0 && self.state_upper_limit >= split_slot
    }

    /// Return true if no historic states other than genesis *will ever be stored*.
    pub fn full_state_pruning_enabled(&self) -> bool {
        self.state_lower_limit == 0 && self.state_upper_limit == STATE_UPPER_LIMIT_NO_RETAIN
    }

    /// Compute the correct `AnchorInfo` for an archive node created from the current node.
    ///
    /// This method ensures that the `anchor_slot` which is used for the hot database's diff grid is
    /// preserved.
    pub fn as_archive_anchor(&self) -> Self {
        Self {
            // Anchor slot MUST be the same. It is immutable.
            anchor_slot: self.anchor_slot,
            oldest_block_slot: Slot::new(0),
            oldest_block_parent: Hash256::ZERO,
            state_upper_limit: Slot::new(0),
            state_lower_limit: Slot::new(0),
        }
    }
}

impl StoreItem for AnchorInfo {
    fn db_column() -> DBColumn {
        DBColumn::BeaconMeta
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Ok(Self::from_ssz_bytes(bytes)?)
    }
}

/// Database parameters relevant to blob sync.
#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode, Serialize, Deserialize, Default)]
pub struct BlobInfo {
    /// The slot after which blobs are or *will be* available (>=).
    ///
    /// If this slot is in the future, then it is the first slot of the Deneb fork, from which blobs
    /// will be available.
    ///
    /// If the `oldest_blob_slot` is `None` then this means that the Deneb fork epoch is not yet
    /// known.
    pub oldest_blob_slot: Option<Slot>,
    /// A separate blobs database is in use (deprecated, always `true`).
    pub blobs_db: bool,
}

impl StoreItem for BlobInfo {
    fn db_column() -> DBColumn {
        DBColumn::BeaconMeta
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Ok(Self::from_ssz_bytes(bytes)?)
    }
}

/// Database parameter relevant to data column custody sync. There is only at most a single
/// `DataColumnCustodyInfo` stored in the db. `earliest_data_column_slot` is updated when cgc
/// count changes and is updated incrementally during data column custody backfill. Once custody backfill
/// is complete `earliest_data_column_slot` is set to `None`.
#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode, Serialize, Deserialize, Default)]
pub struct DataColumnCustodyInfo {
    /// The earliest slot for which data columns are available.
    pub earliest_data_column_slot: Option<Slot>,
}

impl StoreItem for DataColumnCustodyInfo {
    fn db_column() -> DBColumn {
        DBColumn::BeaconDataColumnCustodyInfo
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Ok(DataColumnCustodyInfo::from_ssz_bytes(bytes)?)
    }
}

/// Database parameters relevant to data column sync.
#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode, Serialize, Deserialize, Default)]
pub struct DataColumnInfo {
    /// The slot after which data columns are or *will be* available (>=).
    ///
    /// If this slot is in the future, then it is the first slot of the Fulu fork, from which
    /// data columns will be available.
    ///
    /// If the `oldest_data_column_slot` is `None` then this means that the Fulu fork epoch is
    /// not yet known.
    pub oldest_data_column_slot: Option<Slot>,
}

impl StoreItem for DataColumnInfo {
    fn db_column() -> DBColumn {
        DBColumn::BeaconMeta
    }

    fn as_store_bytes(&self) -> Vec<u8> {
        self.as_ssz_bytes()
    }

    fn from_store_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Ok(Self::from_ssz_bytes(bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SchemaVersion ---

    #[test]
    fn schema_version_as_u64() {
        assert_eq!(SchemaVersion(42).as_u64(), 42);
        assert_eq!(SchemaVersion(0).as_u64(), 0);
    }

    #[test]
    fn schema_version_store_roundtrip() {
        let v = SchemaVersion(28);
        let bytes = v.as_store_bytes();
        let decoded = SchemaVersion::from_store_bytes(&bytes).unwrap();
        assert_eq!(decoded.as_u64(), 28);
    }

    #[test]
    fn schema_version_ordering() {
        assert!(SchemaVersion(1) < SchemaVersion(2));
        assert_eq!(SchemaVersion(5), SchemaVersion(5));
    }

    #[test]
    fn current_schema_version_value() {
        assert_eq!(CURRENT_SCHEMA_VERSION, SchemaVersion(28));
    }

    // --- CompactionTimestamp ---

    #[test]
    fn compaction_timestamp_store_roundtrip() {
        let ts = CompactionTimestamp(1710000000);
        let bytes = ts.as_store_bytes();
        let decoded = CompactionTimestamp::from_store_bytes(&bytes).unwrap();
        assert_eq!(decoded.0, 1710000000);
    }

    #[test]
    fn compaction_timestamp_zero() {
        let ts = CompactionTimestamp(0);
        let bytes = ts.as_store_bytes();
        let decoded = CompactionTimestamp::from_store_bytes(&bytes).unwrap();
        assert_eq!(decoded.0, 0);
    }

    // --- AnchorInfo ---

    #[test]
    fn anchor_info_block_backfill_complete() {
        let anchor = AnchorInfo {
            anchor_slot: Slot::new(1000),
            oldest_block_slot: Slot::new(50),
            oldest_block_parent: Hash256::ZERO,
            state_upper_limit: Slot::new(0),
            state_lower_limit: Slot::new(0),
        };
        assert!(anchor.block_backfill_complete(Slot::new(50)));
        assert!(anchor.block_backfill_complete(Slot::new(100)));
        assert!(!anchor.block_backfill_complete(Slot::new(10)));
    }

    #[test]
    fn anchor_info_all_historic_states_stored() {
        let mut anchor = ANCHOR_UNINITIALIZED;
        assert!(!anchor.all_historic_states_stored());

        anchor.state_lower_limit = Slot::new(500);
        anchor.state_upper_limit = Slot::new(500);
        assert!(anchor.all_historic_states_stored());
    }

    #[test]
    fn anchor_info_no_historic_states_stored() {
        let anchor = AnchorInfo {
            anchor_slot: Slot::new(1000),
            oldest_block_slot: Slot::new(0),
            oldest_block_parent: Hash256::ZERO,
            state_upper_limit: Slot::new(2000),
            state_lower_limit: Slot::new(0),
        };
        // split_slot <= state_upper_limit: no historic states
        assert!(anchor.no_historic_states_stored(Slot::new(1500)));
        // split_slot > state_upper_limit: some states stored
        assert!(!anchor.no_historic_states_stored(Slot::new(3000)));
    }

    #[test]
    fn anchor_info_full_state_pruning_enabled() {
        let anchor = AnchorInfo {
            anchor_slot: Slot::new(1000),
            oldest_block_slot: Slot::new(0),
            oldest_block_parent: Hash256::ZERO,
            state_upper_limit: STATE_UPPER_LIMIT_NO_RETAIN,
            state_lower_limit: Slot::new(0),
        };
        assert!(anchor.full_state_pruning_enabled());

        let anchor2 = AnchorInfo {
            state_upper_limit: Slot::new(100),
            ..anchor
        };
        assert!(!anchor2.full_state_pruning_enabled());
    }

    #[test]
    fn anchor_info_as_archive_anchor() {
        let anchor = AnchorInfo {
            anchor_slot: Slot::new(1000),
            oldest_block_slot: Slot::new(500),
            oldest_block_parent: Hash256::repeat_byte(0xff),
            state_upper_limit: Slot::new(2000),
            state_lower_limit: Slot::new(100),
        };
        let archive = anchor.as_archive_anchor();
        // anchor_slot preserved
        assert_eq!(archive.anchor_slot, Slot::new(1000));
        // everything else zeroed
        assert_eq!(archive.oldest_block_slot, Slot::new(0));
        assert_eq!(archive.oldest_block_parent, Hash256::ZERO);
        assert_eq!(archive.state_upper_limit, Slot::new(0));
        assert_eq!(archive.state_lower_limit, Slot::new(0));
    }

    #[test]
    fn anchor_info_store_roundtrip() {
        let anchor = AnchorInfo {
            anchor_slot: Slot::new(1000),
            oldest_block_slot: Slot::new(500),
            oldest_block_parent: Hash256::repeat_byte(0xab),
            state_upper_limit: Slot::new(2000),
            state_lower_limit: Slot::new(100),
        };
        let bytes = anchor.as_store_bytes();
        let decoded = AnchorInfo::from_store_bytes(&bytes).unwrap();
        assert_eq!(decoded, anchor);
    }

    #[test]
    fn anchor_uninitialized_constant() {
        assert_eq!(ANCHOR_UNINITIALIZED.anchor_slot, Slot::new(u64::MAX));
        assert_eq!(ANCHOR_UNINITIALIZED.oldest_block_slot, Slot::new(u64::MAX));
        assert_eq!(ANCHOR_UNINITIALIZED.oldest_block_parent, Hash256::ZERO);
        assert_eq!(ANCHOR_UNINITIALIZED.state_upper_limit, Slot::new(u64::MAX));
        assert_eq!(ANCHOR_UNINITIALIZED.state_lower_limit, Slot::new(0));
    }

    // --- BlobInfo ---

    #[test]
    fn blob_info_store_roundtrip() {
        let info = BlobInfo {
            oldest_blob_slot: Some(Slot::new(100)),
            blobs_db: true,
        };
        let bytes = info.as_store_bytes();
        let decoded = BlobInfo::from_store_bytes(&bytes).unwrap();
        assert_eq!(decoded, info);
    }

    #[test]
    fn blob_info_default() {
        let info = BlobInfo::default();
        assert_eq!(info.oldest_blob_slot, None);
        assert!(!info.blobs_db);
    }

    #[test]
    fn blob_info_none_slot_roundtrip() {
        let info = BlobInfo {
            oldest_blob_slot: None,
            blobs_db: true,
        };
        let bytes = info.as_store_bytes();
        let decoded = BlobInfo::from_store_bytes(&bytes).unwrap();
        assert_eq!(decoded.oldest_blob_slot, None);
    }

    // --- DataColumnInfo ---

    #[test]
    fn data_column_info_store_roundtrip() {
        let info = DataColumnInfo {
            oldest_data_column_slot: Some(Slot::new(200)),
        };
        let bytes = info.as_store_bytes();
        let decoded = DataColumnInfo::from_store_bytes(&bytes).unwrap();
        assert_eq!(decoded, info);
    }

    #[test]
    fn data_column_info_default() {
        let info = DataColumnInfo::default();
        assert_eq!(info.oldest_data_column_slot, None);
    }

    // --- DataColumnCustodyInfo ---

    #[test]
    fn data_column_custody_info_store_roundtrip() {
        let info = DataColumnCustodyInfo {
            earliest_data_column_slot: Some(Slot::new(300)),
        };
        let bytes = info.as_store_bytes();
        let decoded = DataColumnCustodyInfo::from_store_bytes(&bytes).unwrap();
        assert_eq!(decoded, info);
    }

    #[test]
    fn data_column_custody_info_default() {
        let info = DataColumnCustodyInfo::default();
        assert_eq!(info.earliest_data_column_slot, None);
    }

    // --- Key constants ---

    #[test]
    fn meta_keys_are_distinct() {
        let keys = [
            SCHEMA_VERSION_KEY,
            CONFIG_KEY,
            SPLIT_KEY,
            COMPACTION_TIMESTAMP_KEY,
            ANCHOR_INFO_KEY,
            BLOB_INFO_KEY,
            DATA_COLUMN_INFO_KEY,
            DATA_COLUMN_CUSTODY_INFO_KEY,
        ];
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "keys at index {} and {} collide", i, j);
            }
        }
    }
}
