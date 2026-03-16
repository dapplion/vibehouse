use crate::*;
use ethereum_hashing::{ZERO_HASHES, hash32_concat};
use int_to_bytes::int_to_bytes32;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use test_utils::TestRandom;

#[derive(Encode, Decode, Deserialize, Serialize, Clone, Copy, Debug, PartialEq, TestRandom)]
pub struct FinalizedExecutionBlock {
    pub deposit_root: Hash256,
    pub deposit_count: u64,
    pub block_hash: Hash256,
    pub block_height: u64,
}

impl From<&DepositTreeSnapshot> for FinalizedExecutionBlock {
    fn from(snapshot: &DepositTreeSnapshot) -> Self {
        Self {
            deposit_root: snapshot.deposit_root,
            deposit_count: snapshot.deposit_count,
            block_hash: snapshot.execution_block_hash,
            block_height: snapshot.execution_block_height,
        }
    }
}

#[derive(Encode, Decode, Deserialize, Serialize, Clone, Debug, PartialEq, TestRandom)]
pub struct DepositTreeSnapshot {
    pub finalized: Vec<Hash256>,
    pub deposit_root: Hash256,
    #[serde(with = "serde_utils::quoted_u64")]
    pub deposit_count: u64,
    pub execution_block_hash: Hash256,
    #[serde(with = "serde_utils::quoted_u64")]
    pub execution_block_height: u64,
}

impl Default for DepositTreeSnapshot {
    fn default() -> Self {
        let mut result = Self {
            finalized: vec![],
            deposit_root: Hash256::zero(),
            deposit_count: 0,
            execution_block_hash: Hash256::zero(),
            execution_block_height: 0,
        };
        // properly set the empty deposit root
        result.deposit_root = result.calculate_root().unwrap();
        result
    }
}

impl DepositTreeSnapshot {
    // Calculates the deposit tree root from the hashes in the snapshot
    pub fn calculate_root(&self) -> Option<Hash256> {
        let mut size = self.deposit_count;
        let mut index = self.finalized.len();
        let mut deposit_root = [0; 32];
        for height in 0..DEPOSIT_TREE_DEPTH {
            deposit_root = if (size & 1) == 1 {
                index = index.checked_sub(1)?;
                hash32_concat(self.finalized.get(index)?.as_slice(), &deposit_root)
            } else {
                hash32_concat(&deposit_root, ZERO_HASHES.get(height)?)
            };
            size /= 2;
        }
        // add mix-in-length
        deposit_root = hash32_concat(&deposit_root, &int_to_bytes32(self.deposit_count));

        Some(Hash256::from(deposit_root))
    }
    pub fn is_valid(&self) -> bool {
        self.calculate_root() == Some(self.deposit_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::{Decode, Encode};
    ssz_tests!(DepositTreeSnapshot);

    #[test]
    fn default_is_valid() {
        let snapshot = DepositTreeSnapshot::default();
        assert!(snapshot.is_valid(), "default snapshot should be valid");
        assert_eq!(snapshot.deposit_count, 0);
        assert!(snapshot.finalized.is_empty());
        assert_eq!(snapshot.execution_block_hash, Hash256::zero());
        assert_eq!(snapshot.execution_block_height, 0);
    }

    #[test]
    fn default_deposit_root_matches_calculated() {
        let snapshot = DepositTreeSnapshot::default();
        let calculated = snapshot.calculate_root().unwrap();
        assert_eq!(snapshot.deposit_root, calculated);
    }

    #[test]
    fn invalid_snapshot_wrong_root() {
        let mut snapshot = DepositTreeSnapshot::default();
        // Corrupt the deposit root
        snapshot.deposit_root = Hash256::from_low_u64_be(42);
        assert!(
            !snapshot.is_valid(),
            "snapshot with wrong root should be invalid"
        );
    }

    #[test]
    fn calculate_root_returns_none_for_bad_count() {
        // deposit_count implies more finalized hashes than provided
        let snapshot = DepositTreeSnapshot {
            finalized: vec![], // No hashes
            deposit_root: Hash256::zero(),
            deposit_count: 3, // But count is 3, needs finalized hashes
            execution_block_hash: Hash256::zero(),
            execution_block_height: 0,
        };
        // When deposit_count has bits set, it tries to index into finalized
        // which may cause None if index goes below 0
        let result = snapshot.calculate_root();
        assert!(
            result.is_none(),
            "should return None when finalized hashes don't match count"
        );
    }

    #[test]
    fn from_finalized_execution_block() {
        let snapshot = DepositTreeSnapshot {
            finalized: vec![Hash256::from_low_u64_be(1)],
            deposit_root: Hash256::from_low_u64_be(99),
            deposit_count: 42,
            execution_block_hash: Hash256::from_low_u64_be(100),
            execution_block_height: 500,
        };
        let block = FinalizedExecutionBlock::from(&snapshot);
        assert_eq!(block.deposit_root, snapshot.deposit_root);
        assert_eq!(block.deposit_count, snapshot.deposit_count);
        assert_eq!(block.block_hash, snapshot.execution_block_hash);
        assert_eq!(block.block_height, snapshot.execution_block_height);
    }

    #[test]
    fn finalized_execution_block_ssz_roundtrip() {
        let block = FinalizedExecutionBlock {
            deposit_root: Hash256::from_low_u64_be(1),
            deposit_count: 100,
            block_hash: Hash256::from_low_u64_be(2),
            block_height: 9999,
        };
        let bytes = block.as_ssz_bytes();
        let decoded = FinalizedExecutionBlock::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(block, decoded);
    }

    #[test]
    fn finalized_execution_block_serde_roundtrip() {
        let block = FinalizedExecutionBlock {
            deposit_root: Hash256::from_low_u64_be(1),
            deposit_count: 100,
            block_hash: Hash256::from_low_u64_be(2),
            block_height: 9999,
        };
        let json = serde_json::to_string(&block).unwrap();
        let decoded: FinalizedExecutionBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, decoded);
    }

    #[test]
    fn deposit_tree_snapshot_serde_roundtrip() {
        let snapshot = DepositTreeSnapshot {
            finalized: vec![Hash256::from_low_u64_be(1), Hash256::from_low_u64_be(2)],
            deposit_root: Hash256::from_low_u64_be(99),
            deposit_count: 42,
            execution_block_hash: Hash256::from_low_u64_be(100),
            execution_block_height: 500,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let decoded: DepositTreeSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snapshot, decoded);
    }

    #[test]
    fn deposit_tree_snapshot_zero_count_valid() {
        let snapshot = DepositTreeSnapshot {
            finalized: vec![],
            deposit_root: Hash256::zero(),
            deposit_count: 0,
            execution_block_hash: Hash256::zero(),
            execution_block_height: 0,
        };
        // Zero count with empty finalized should produce a valid root
        let root = snapshot.calculate_root().unwrap();
        // The default also has zero count so roots should match
        let default_snapshot = DepositTreeSnapshot::default();
        assert_eq!(root, default_snapshot.deposit_root);
    }
}
