use crate::context_deserialize;
use crate::test_utils::TestRandom;
use crate::{BeaconState, EthSpec, ForkName, Hash256};
use compare_fields_derive::CompareFields;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash::TreeHash;
use tree_hash_derive::TreeHash;

/// `HistoricalSummary` matches the components of the phase0 `HistoricalBatch`
/// making the two hash_tree_root-compatible. This struct is introduced into the beacon state
/// in the Capella hard fork.
///
/// <https://github.com/ethereum/consensus-specs/blob/dev/specs/capella/beacon-chain.md#historicalsummary>
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Encode,
    Decode,
    TreeHash,
    TestRandom,
    CompareFields,
    Clone,
    Copy,
    Default,
)]
#[context_deserialize(ForkName)]
pub struct HistoricalSummary {
    block_summary_root: Hash256,
    state_summary_root: Hash256,
}

impl HistoricalSummary {
    pub fn new<E: EthSpec>(state: &BeaconState<E>) -> Self {
        Self {
            block_summary_root: state.block_roots().tree_hash_root(),
            state_summary_root: state.state_roots().tree_hash_root(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MinimalEthSpec;
    use tree_hash::TreeHash;

    type E = MinimalEthSpec;

    #[test]
    fn default_is_zero() {
        let summary = HistoricalSummary::default();
        assert_eq!(summary.block_summary_root, Hash256::ZERO);
        assert_eq!(summary.state_summary_root, Hash256::ZERO);
    }

    #[test]
    fn clone_and_eq() {
        let summary = HistoricalSummary {
            block_summary_root: Hash256::repeat_byte(0xaa),
            state_summary_root: Hash256::repeat_byte(0xbb),
        };
        assert_eq!(summary, summary.clone());
    }

    #[test]
    fn copy_semantics() {
        let summary = HistoricalSummary {
            block_summary_root: Hash256::repeat_byte(1),
            state_summary_root: Hash256::repeat_byte(2),
        };
        let copied = summary;
        assert_eq!(summary, copied);
    }

    #[test]
    fn ssz_round_trip() {
        let summary = HistoricalSummary {
            block_summary_root: Hash256::repeat_byte(0x11),
            state_summary_root: Hash256::repeat_byte(0x22),
        };
        let encoded = ssz::Encode::as_ssz_bytes(&summary);
        let decoded = <HistoricalSummary as ssz::Decode>::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(summary, decoded);
    }

    #[test]
    fn tree_hash_deterministic() {
        let summary = HistoricalSummary {
            block_summary_root: Hash256::repeat_byte(0xcc),
            state_summary_root: Hash256::repeat_byte(0xdd),
        };
        assert_eq!(summary.tree_hash_root(), summary.tree_hash_root());
    }

    #[test]
    fn tree_hash_different() {
        let s1 = HistoricalSummary {
            block_summary_root: Hash256::repeat_byte(1),
            state_summary_root: Hash256::repeat_byte(2),
        };
        let s2 = HistoricalSummary {
            block_summary_root: Hash256::repeat_byte(3),
            state_summary_root: Hash256::repeat_byte(2),
        };
        assert_ne!(s1.tree_hash_root(), s2.tree_hash_root());
    }

    #[test]
    fn serde_round_trip() {
        let summary = HistoricalSummary {
            block_summary_root: Hash256::repeat_byte(0x55),
            state_summary_root: Hash256::repeat_byte(0x66),
        };
        let json = serde_json::to_string(&summary).unwrap();
        // Deserialize with fork context
        let decoded: HistoricalSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(summary, decoded);
    }

    #[test]
    fn new_from_state() {
        let spec = E::default_spec();
        let state = BeaconState::<E>::new(0, Default::default(), &spec);
        let summary = HistoricalSummary::new(&state);
        // block_roots and state_roots are all zeros, but tree_hash_root of them is not zero
        assert_ne!(summary.block_summary_root, Hash256::ZERO);
        assert_ne!(summary.state_summary_root, Hash256::ZERO);
    }

    #[test]
    fn new_deterministic() {
        let spec = E::default_spec();
        let state = BeaconState::<E>::new(0, Default::default(), &spec);
        let s1 = HistoricalSummary::new(&state);
        let s2 = HistoricalSummary::new(&state);
        assert_eq!(s1, s2);
    }

    #[test]
    fn debug_format() {
        let summary = HistoricalSummary::default();
        let debug = format!("{:?}", summary);
        assert!(debug.contains("HistoricalSummary"));
    }
}
