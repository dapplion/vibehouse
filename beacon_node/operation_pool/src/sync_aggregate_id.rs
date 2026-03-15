use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use types::{Hash256, Slot};

/// Used to key `SyncAggregate`s in the `naive_sync_aggregation_pool`.
#[derive(
    PartialEq, Eq, Clone, Hash, Debug, PartialOrd, Ord, Encode, Decode, Serialize, Deserialize,
)]
pub struct SyncAggregateId {
    pub slot: Slot,
    pub beacon_block_root: Hash256,
}

impl SyncAggregateId {
    pub fn new(slot: Slot, beacon_block_root: Hash256) -> Self {
        Self {
            slot,
            beacon_block_root,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::{Decode, Encode};

    fn make_id(slot: u64, byte: u8) -> SyncAggregateId {
        SyncAggregateId::new(Slot::new(slot), Hash256::repeat_byte(byte))
    }

    #[test]
    fn new_sets_fields() {
        let id = make_id(42, 0xab);
        assert_eq!(id.slot, Slot::new(42));
        assert_eq!(id.beacon_block_root, Hash256::repeat_byte(0xab));
    }

    #[test]
    fn clone_and_eq() {
        let id = make_id(1, 0x01);
        assert_eq!(id.clone(), id);
    }

    #[test]
    fn inequality_by_slot() {
        assert_ne!(make_id(1, 0x01), make_id(2, 0x01));
    }

    #[test]
    fn inequality_by_root() {
        assert_ne!(make_id(1, 0x01), make_id(1, 0x02));
    }

    #[test]
    fn ordering() {
        let a = make_id(1, 0x01);
        let b = make_id(2, 0x01);
        assert!(a < b);
    }

    #[test]
    fn hash_is_usable_in_hashset() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(make_id(1, 0x01));
        set.insert(make_id(1, 0x01));
        assert_eq!(set.len(), 1);
        set.insert(make_id(2, 0x01));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn ssz_roundtrip() {
        let id = make_id(99, 0xff);
        let bytes = id.as_ssz_bytes();
        let decoded = SyncAggregateId::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(id, decoded);
    }

    #[test]
    fn debug_format() {
        let id = make_id(1, 0x00);
        let dbg = format!("{:?}", id);
        assert!(dbg.contains("SyncAggregateId"));
    }
}
